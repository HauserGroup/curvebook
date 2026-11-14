//! Deterministic headless & desktop UI accessibility/inspection tree (`ui.inspect`, `R8.7`, §2).
//!
//! Produces [`UiTreeSnapshot`] from the live [`UiApp`] without requiring an OS window, attaching
//! declarative enablement (`enabled`, `disabled_reason`) from [`crate::modal_predicates`] and
//! deterministic `[x, y, width, height]` screen bounds from [`compute_workspace_layout_map`].

use lumencraft_doc::{AnalysisKind, OneWayMethod, PValueFormat, ProjectDocument, SheetId, TableArchetype, TransformAnalysisSpec, XColumnMode};
use lumencraft_tools::GridCellCoord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::compositor::{WorkspacePixelRect, compute_workspace_layout_map};
use crate::dialogs_canvas::{ActiveDialog, format_compact_f64};
use crate::grid_view::format_cell_display;
use crate::interaction::{ViewportMetrics, WorkspaceLayoutMap};
use crate::menus::tooltips::{menu_bar_tooltips, toolbar_tooltips};
use crate::modal_predicates::{
    ControlEnablement, evaluate_dialog_button, evaluate_edit_params_tab, r1_paired_or_matched_allowed, r1_sphericity_or_reml_control,
    r2_post_hoc_option_enabled, r3_control_column_picker, r4_two_group_graph_differences, r4_two_group_nonparametric_options,
    r4_two_group_welch_correction, r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend, r6_contingency_ci_selector,
    r7_correlation_heatmap_toggle, r8_subcolumn_aggregation, r9_pvalue_precision_spinner, r10_pcr_outcome_column, source_table_shape,
};
use crate::power_wizard::PowerWizardStage;
use lumencraft_tools::SelectionQuickStats;

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

/// Build a deterministic [`UiTreeSnapshot`] from `app` filtered by `scope` (`all`, `menus`,
/// `toolbar`, `navigator`, `tabs`, `canvas`, `grid`, `dialog`).
#[must_use]
pub fn inspect_ui_tree(app: &UiApp, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    let layout = compute_workspace_layout_map(app, ViewportMetrics::default());
    let doc = app.session().document();
    let active_id = doc.active_sheet_id.as_ref();
    let active_kind = active_id.and_then(|id| doc.sheet_section(id)).map(sheet_section_name);
    let active_dialog_name = active_dialog_slug(&app.canvas_dialogs.active_dialog);

    let grid_sel = build_grid_selection_snapshot(app, doc);
    let brushed_points = collect_brushed_points(app);
    let scope_norm = scope.map(|s| s.trim().to_ascii_lowercase());
    let want = |name: &str| match scope_norm.as_deref() {
        None | Some("" | "all" | "workspace") => true,
        Some(target) => target == name,
    };

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
        menus: want("menus").then(|| build_menus_snapshot(app, &layout, include_bounds)),
        toolbar: want("toolbar").then(|| build_toolbar_snapshot(app, &layout, include_bounds)),
        navigator_sheets: want("navigator").then(|| build_navigator_snapshot(app, doc, &layout, include_bounds)),
        tabs: want("tabs").then(|| build_tabs_snapshot(app, doc, &layout, include_bounds)),
        canvas_elements: want("canvas").then(|| build_canvas_snapshot(app, doc, &layout, include_bounds)),
        grid_cells: want("grid").then(|| build_grid_cells_snapshot(app, doc, &layout, include_bounds)),
        dialog: want("dialog").then(|| build_dialog_snapshot(app, doc, &layout, include_bounds)).flatten(),
    }
}

fn rect_bounds(r: WorkspacePixelRect, scale: f64, include_bounds: bool) -> Option<[f64; 4]> {
    if !include_bounds {
        return None;
    }
    let s = scale.max(0.5);
    Some([f64::from(r.x) / s, f64::from(r.y) / s, f64::from(r.width) / s, f64::from(r.height) / s])
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
        selected_rows,
        selected_coords: coords.iter().copied().map(GridCoordSnapshot::from).collect(),
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

fn build_menus_snapshot(app: &UiApp, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Vec<UiMenuSnapshot> {
    let bar_rect = layout.frame.toolbar;
    let scale = layout.metrics.clamped_scale_factor();
    menu_bar_tooltips(app)
        .into_iter()
        .enumerate()
        .map(|(sec_idx, sec)| {
            let menu_bounds = include_bounds.then(|| {
                let x = f64::from(bar_rect.x) + (sec_idx as f64) * 68.0;
                [x / scale, 0.0, 64.0, 22.0]
            });
            let items = sec
                .items
                .into_iter()
                .enumerate()
                .map(|(idx, tip)| UiWidgetNode {
                    widget_id: format!("menu.{}", tip.command_id),
                    role: String::from("menuitem"),
                    label: tip.localized_label.to_owned(),
                    command_id: Some(tip.command_id.to_owned()),
                    value: None,
                    enabled: tip.enabled,
                    disabled_reason: (!tip.enabled).then(|| format!("{} is currently unavailable in this context.", tip.label)),
                    bounds: include_bounds.then(|| {
                        let mx = f64::from(bar_rect.x) + (sec_idx as f64) * 68.0;
                        let my = 24.0 + (idx as f64) * 22.0;
                        [mx / scale, my, 210.0, 20.0]
                    }),
                })
                .collect();
            UiMenuSnapshot { menu_key: sec.menu_key.to_owned(), title: sec.title.to_owned(), bounds: menu_bounds, items }
        })
        .collect()
}

fn build_toolbar_snapshot(app: &UiApp, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Vec<UiWidgetNode> {
    let bar = layout.frame.toolbar;
    let scale = layout.metrics.clamped_scale_factor();
    toolbar_tooltips(app)
        .into_iter()
        .enumerate()
        .map(|(idx, entry)| {
            let bw = 72.0;
            let bx = f64::from(bar.x) / scale + 68.0 + (idx as f64) * (bw + 6.0);
            let by = f64::from(bar.y) / scale + 4.0;
            UiWidgetNode {
                widget_id: entry.button.id.to_owned(),
                role: String::from("button"),
                label: entry.caption.to_owned(),
                command_id: Some(entry.button.command_id.to_owned()),
                value: None,
                enabled: entry.tooltip.enabled,
                disabled_reason: (!entry.tooltip.enabled).then(|| format!("{} requires a compatible active sheet.", entry.caption)),
                bounds: include_bounds.then_some([bx, by, bw, 24.0]),
            }
        })
        .collect()
}

fn build_navigator_snapshot(app: &UiApp, doc: &ProjectDocument, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Vec<UiNavigatorSheetNode> {
    let scale = layout.metrics.clamped_scale_factor();
    let active_id = doc.active_sheet_id.as_ref();
    let nav_rect = layout.frame.navigator;
    let mut out = Vec::new();
    let mut fallback_idx = 0usize;
    for (folder, sheet_ids) in [
        ("data", doc.data_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect::<Vec<_>>()),
        ("info", doc.info_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect()),
        ("results", doc.result_sheets.iter().map(|s| (&s.id, s.title())).collect()),
        ("graphs", doc.graph_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect()),
        ("layouts", doc.layout_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect()),
    ] {
        for (id, title) in sheet_ids {
            let hit = layout.navigator_entries.iter().find(|e| &e.sheet_id == id);
            let bounds = hit.and_then(|h| rect_bounds(h.rect, scale, include_bounds)).or_else(|| {
                include_bounds.then(|| {
                    let y = f64::from(nav_rect.y) / scale + 28.0 + (fallback_idx as f64) * 20.0;
                    [f64::from(nav_rect.x) / scale + 8.0, y, 164.0, 18.0]
                })
            });
            fallback_idx += 1;
            out.push(UiNavigatorSheetNode {
                widget_id: format!("nav.sheet.{}", id.as_str()),
                sheet_id: id.as_str().to_owned(),
                section: folder.to_owned(),
                title: title.to_owned(),
                selected: active_id == Some(id),
                role: String::from("treeitem"),
                label: title.to_owned(),
                enabled: app.nav.search_query.is_empty() || title.to_ascii_lowercase().contains(&app.nav.search_query.to_ascii_lowercase()),
                bounds,
            });
        }
    }
    out
}

fn build_tabs_snapshot(app: &UiApp, doc: &ProjectDocument, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Vec<UiWidgetNode> {
    let canvas = layout.frame.canvas;
    let scale = layout.metrics.clamped_scale_factor();
    let mut tabs = Vec::new();
    if let Some(rs_id) = doc.active_sheet_id.as_ref()
        && let Some(rs) = doc.result_sheet(rs_id)
    {
        for (idx, tab) in rs.tabs.iter().enumerate() {
            let active = idx == app.canvas_dialogs.result_view.active_tab_index;
            let bounds = include_bounds.then(|| {
                let tx = f64::from(canvas.x) / scale + 12.0 + (idx as f64) * 116.0;
                let ty = f64::from(canvas.y) / scale + 8.0;
                [tx, ty, 110.0, 22.0]
            });
            tabs.push(UiWidgetNode {
                widget_id: format!("result.tab.{}", tab.id),
                role: String::from("tab"),
                label: tab.title.clone(),
                command_id: None,
                value: Some(json!({ "selected": active, "index": idx, "tab_id": tab.id })),
                enabled: true,
                disabled_reason: None,
                bounds,
            });
        }
    }
    if let Some(dlg) = build_dialog_snapshot(app, doc, layout, include_bounds) {
        tabs.extend(dlg.tabs);
    }
    tabs
}

fn build_canvas_snapshot(app: &UiApp, doc: &ProjectDocument, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Vec<UiWidgetNode> {
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    let scale = layout.metrics.clamped_scale_factor();
    if let Some(paper) = &layout.scene_paper
        && let Ok(scene) = app.session().render_sheet_scene(active_id)
    {
        for (idx, pt) in scene.plotted_points.iter().enumerate() {
            let pix = paper.scene_pt_to_pixel(pt.center_pt);
            let bounds = include_bounds.then(|| {
                let (cx, cy) =
                    pix.map_or((f64::from(layout.frame.canvas.x) + 40.0, f64::from(layout.frame.canvas.y) + 40.0), |[x, y]| (x / scale, y / scale));
                [cx - 6.0, cy - 6.0, 12.0, 12.0]
            });
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
                bounds,
            });
        }
    }
    nodes
}

fn build_grid_cells_snapshot(app: &UiApp, doc: &ProjectDocument, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Vec<UiWidgetNode> {
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return Vec::new();
    };
    let Some(ds) = doc.data_sheet(active_id) else {
        return Vec::new();
    };
    let frame = app.grid.tool.format_visible_window(ds);
    let win_start_row = frame.window.start_row;
    let win_end_row = frame.window.end_row;
    let scale = layout.metrics.clamped_scale_factor();
    let mut nodes: Vec<UiWidgetNode> = frame
        .cells
        .into_iter()
        .take(64)
        .map(|cell| {
            let bounds = include_bounds.then(|| {
                if let Some(grid) = &layout.grid_geometry
                    && let Some(r) = grid.cell_rect(cell.coord.row, cell.flat_col)
                {
                    return [f64::from(r.x) / scale, f64::from(r.y) / scale, f64::from(r.width) / scale, f64::from(r.height) / scale];
                }
                [
                    f64::from(layout.frame.canvas.x) / scale + 52.0 + (cell.flat_col as f64) * 76.0,
                    f64::from(layout.frame.canvas.y) / scale + 48.0 + (cell.coord.row as f64) * 18.0,
                    74.0,
                    18.0,
                ]
            });
            UiWidgetNode {
                widget_id: format!("grid.cell.{}.{}.{}", cell.coord.row, cell.coord.col, cell.coord.subcol),
                role: String::from("gridcell"),
                label: format!("R{}C{}", cell.coord.row + 1, cell.coord.col + 1),
                command_id: None,
                value: Some(json!({
                    "row": cell.coord.row,
                    "col": cell.coord.col,
                    "subcol": cell.coord.subcol,
                    "text": cell.display_text,
                    "excluded": cell.excluded,
                    "selected": cell.is_selected,
                })),
                enabled: !ds.frozen,
                disabled_reason: ds.frozen.then(|| String::from("Data sheet is frozen (read-only).")),
                bounds,
            }
        })
        .collect();

    let (x_subcols, is_generated_x) = match &ds.archetype {
        TableArchetype::Xy(xy) => (xy.x_mode.subcolumn_count().max(1), matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. })),
        TableArchetype::Survival(_) => (1, false),
        _ => (0, false),
    };
    if x_subcols > 0 {
        let row_limit = win_end_row.min(win_start_row + 16);
        for row in win_start_row..row_limit {
            for subcol in 0..x_subcols {
                let bounds = include_bounds.then(|| {
                    if let Some(grid) = &layout.grid_geometry
                        && let Some(r) = grid.x_cell_rect(row, subcol)
                    {
                        return [f64::from(r.x) / scale, f64::from(r.y) / scale, f64::from(r.width) / scale, f64::from(r.height) / scale];
                    }
                    [
                        f64::from(layout.frame.canvas.x) / scale + 42.0 + (subcol as f64) * 60.0,
                        f64::from(layout.frame.canvas.y) / scale + 52.0 + ((row - win_start_row) as f64) * 18.0,
                        59.0,
                        17.0,
                    ]
                });
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
                    widget_id: format!("grid.xcell.{row}.{subcol}"),
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
                    bounds,
                });
            }
        }
    }
    nodes
}

fn build_dialog_snapshot(app: &UiApp, doc: &ProjectDocument, layout: &WorkspaceLayoutMap, include_bounds: bool) -> Option<UiDialogSnapshot> {
    let dialogs = &app.canvas_dialogs;
    if dialogs.active_dialog == ActiveDialog::None {
        return None;
    }
    let scale = layout.metrics.clamped_scale_factor();
    let modal_rect =
        layout.frame.modal_overlay.unwrap_or_else(|| WorkspacePixelRect::new(layout.frame.canvas.x + 24, layout.frame.canvas.y + 24, 480, 320));
    let dlg_bounds = rect_bounds(modal_rect, scale, include_bounds);
    let base_x = f64::from(modal_rect.x) / scale + 16.0;
    let base_y = f64::from(modal_rect.y) / scale + 36.0;

    let (title, target_sheet_id, stage, tabs, controls, buttons_list) = match &dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => build_power_dialog_nodes(app, base_x, base_y, include_bounds),
        ActiveDialog::FormatSelectedPoints { graph_sheet_id } => build_format_points_nodes(app, graph_sheet_id, base_x, base_y, include_bounds),
        ActiveDialog::AnalyzeSheet { source_sheet_id } => build_analyze_dialog_nodes(app, doc, source_sheet_id, base_x, base_y, include_bounds),
        ActiveDialog::EditAnalysisParameters { result_sheet_id } => {
            build_edit_params_dialog_nodes(app, doc, result_sheet_id, base_x, base_y, include_bounds)
        }
        ActiveDialog::ImportReshape => build_import_reshape_nodes(app, base_x, base_y, include_bounds),
        ActiveDialog::ConvertArchetype { sheet_id } => build_convert_archetype_nodes(app, sheet_id, base_x, base_y, include_bounds),
        ActiveDialog::ChangeGraphType { graph_sheet_id } => build_change_graph_type_nodes(app, graph_sheet_id, base_x, base_y, include_bounds),
        ActiveDialog::FormatGraph { graph_sheet_id } => build_format_graph_nodes(app, graph_sheet_id, base_x, base_y, include_bounds),
        ActiveDialog::Print { sheet_id } => build_print_dialog_nodes(app, sheet_id.as_ref(), base_x, base_y, include_bounds),
        other => build_generic_dialog_nodes(other, base_x, base_y, include_bounds),
    };

    let buttons = buttons_list
        .into_iter()
        .enumerate()
        .map(|(idx, (id, label))| {
            let pred = evaluate_dialog_button(dialogs, doc, id).unwrap_or_else(ControlEnablement::enabled);
            let bx = f64::from(modal_rect.right()) / scale - 92.0 * (idx as f64 + 1.0);
            let by = f64::from(modal_rect.bottom()) / scale - 34.0;
            UiWidgetNode {
                widget_id: format!("dialog.button.{id}"),
                role: String::from("button"),
                label: label.to_owned(),
                command_id: None,
                value: None,
                enabled: pred.enabled,
                disabled_reason: pred.disabled_reason,
                bounds: include_bounds.then_some([bx, by, 84.0, 26.0]),
            }
        })
        .collect();

    Some(UiDialogSnapshot {
        dialog_kind: active_dialog_slug(&dialogs.active_dialog),
        title: title.to_owned(),
        target_sheet_id,
        stage,
        bounds: dlg_bounds,
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

    let btns = vec![("create", "Create"), ("next", "Next"), ("back", "Back"), ("cancel", "Cancel")];
    ("Power and Sample Size", None, Some(pw.stage.label().to_owned()), tabs, controls, btns)
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
    ("Format Selected Points...", Some(graph_id.as_str().to_owned()), None, Vec::new(), controls, vec![("ok", "OK"), ("cancel", "Cancel")])
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
    (
        "Analyze...",
        Some(source_sheet_id.as_str().to_owned()),
        st.extracted_result_tab_id.clone(),
        Vec::new(),
        controls,
        vec![("ok", "OK"), ("cancel", "Cancel")],
    )
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
    let gg_checked = ep.draft.as_ref().is_none_or(|d| {
        !matches!(
            d.kind,
            AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
        )
    });
    let mut controls = vec![
        node("edit.assume_gaussian", "checkbox", "Assume Gaussian", json!(ep.assume_gaussian), ControlEnablement::enabled(), pos(1)),
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
            r1_sphericity_or_reml_control(ep.paired_or_matched),
            pos(3),
        ),
        node(
            "edit.post_hoc.tukey",
            "radio",
            "Tukey post-hoc",
            json!(matches!(method, lumencraft_doc::MultiplicityMethod::TukeyHsd)),
            r2_post_hoc_option_enabled(ep.assume_gaussian, true),
            pos(4),
        ),
        node(
            "edit.post_hoc.dunn",
            "radio",
            "Dunn's rank comparisons",
            json!(matches!(method, lumencraft_doc::MultiplicityMethod::DunnRankSum)),
            r2_post_hoc_option_enabled(ep.assume_gaussian, false),
            pos(5),
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
            json!(cont_spec.method == lumencraft_doc::ContingencyMethod::FisherExact),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(14),
        ),
        node(
            "edit.contingency.mcnemar",
            "radio",
            "McNemar's test",
            json!(cont_spec.method == lumencraft_doc::ContingencyMethod::McNemarPaired),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(15),
        ),
        node(
            "edit.contingency.cochran_armitage",
            "radio",
            "Cochran-Armitage trend test",
            json!(cont_spec.method == lumencraft_doc::ContingencyMethod::CochranArmitageTrend),
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
            TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype } => {
                controls.push(node(
                    "edit.transform.formula",
                    "textbox",
                    "Row filter formula",
                    json!(filter_formula),
                    ControlEnablement::enabled(),
                    pos(24),
                ));
                controls.push(node(
                    "edit.transform.target_archetype",
                    "combobox",
                    "Target archetype",
                    json!(target_archetype),
                    ControlEnablement::enabled(),
                    pos(25),
                ));
            }
            _ => {}
        }
    }

    (
        "Edit Analysis Parameters (Cmd+T)",
        Some(result_sheet_id.as_str().to_owned()),
        Some(ep.active_tab.clone()),
        tabs,
        controls,
        vec![("ok", "OK"), ("apply", "Apply"), ("cancel", "Cancel")],
    )
}

fn extract_edit_params_details(
    ep: &crate::dialogs_canvas::EditAnalysisParametersState,
) -> (lumencraft_doc::MultiplicityMethod, PValueFormat, lumencraft_doc::ContingencySpec, Option<lumencraft_doc::TwoGroupMethod>) {
    let Some(spec) = ep.draft.as_ref() else {
        return (lumencraft_doc::MultiplicityMethod::None, PValueFormat::FourDecimal, lumencraft_doc::ContingencySpec::default(), None);
    };
    let mult = match &spec.kind {
        AnalysisKind::OneWayAnova(a) => a.multiplicity.method,
        AnalysisKind::FactorialAnova(f) => f.multiplicity.method,
        _ => lumencraft_doc::MultiplicityMethod::None,
    };
    let cont = match &spec.kind {
        AnalysisKind::Contingency(c) => *c,
        _ => lumencraft_doc::ContingencySpec::default(),
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

fn dialog_ctrl_bounds(bx: f64, by: f64, row: usize, include_bounds: bool) -> Option<[f64; 4]> {
    include_bounds.then_some([bx, by + (row as f64) * 24.0, 240.0, 20.0])
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
    ("Import Reshape (Plate Map / Tidy Long)", None, None, Vec::new(), controls, vec![("ok", "Import"), ("cancel", "Cancel")])
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
        "Convert Table Archetype...",
        Some(sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", "Convert"), ("preview", "Preview"), ("cancel", "Cancel")],
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
    ("Change Graph Type", Some(graph_sheet_id.as_str().to_owned()), None, Vec::new(), controls, vec![("ok", "OK"), ("cancel", "Cancel")])
}

fn build_format_graph_nodes(app: &UiApp, graph_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.format_graph;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
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
    ];
    (
        "Format Graph",
        Some(graph_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", "OK"), ("apply", "Apply"), ("cancel", "Cancel")],
    )
}

fn build_print_dialog_nodes(app: &UiApp, sheet_id: Option<&SheetId>, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.print;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node("print.crop_marks", "checkbox", "Crop marks", json!(st.job.crop_marks), ControlEnablement::enabled(), pos(0)),
        node("print.bleed_pt", "spinbutton", "Bleed (pt)", json!(st.job.bleed_pt), ControlEnablement::enabled(), pos(1)),
        node("print.copies", "spinbutton", "Copies", json!(st.copies), ControlEnablement::enabled(), pos(2)),
        node("print.scale_percent", "spinbutton", "Scale (%)", json!(st.job.scale_percent), ControlEnablement::enabled(), pos(3)),
        node("print.selected_printer", "combobox", "Printer", json!(st.selected_printer), ControlEnablement::enabled(), pos(4)),
    ];
    ("Print...", sheet_id.map(|id| id.as_str().to_owned()), None, Vec::new(), controls, vec![("ok", "Print"), ("cancel", "Cancel")])
}

fn build_generic_dialog_nodes(dialog: &ActiveDialog, _bx: f64, _by: f64, _include_bounds: bool) -> DialogBuildParts {
    let (title, target) = match dialog {
        ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, .. } => ("Analysis Already Exists", Some(source_sheet_id)),
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id } => ("Change Data Analyzed", Some(result_sheet_id)),
        ActiveDialog::ResultViewer { result_sheet_id } => ("Result Sheet Viewer", Some(result_sheet_id)),
        ActiveDialog::ConstantHookPicker { target_sheet_id } => ("Insert Hooked Constant", Some(target_sheet_id)),
        ActiveDialog::ChangeGraphType { graph_sheet_id } => ("Change Graph Type", Some(graph_sheet_id)),
        ActiveDialog::FormatGraph { graph_sheet_id } => ("Format Graph", Some(graph_sheet_id)),
        ActiveDialog::FormatAxes { sheet_id } => ("Format Axes", Some(sheet_id)),
        ActiveDialog::PairwiseComparisons { graph_sheet_id } => ("Pairwise Comparisons", Some(graph_sheet_id)),
        ActiveDialog::BubblePlotInspector { graph_sheet_id } => ("Bubble Plot Channels", Some(graph_sheet_id)),
        ActiveDialog::ApplyStyleFrom { target_graph_id } => ("Apply Style From...", Some(target_graph_id)),
        ActiveDialog::PageLayoutComposer { layout_sheet_id } => ("Page Layout Composer", Some(layout_sheet_id)),
        ActiveDialog::ExportFigure { sheet_id } => ("Export Publication Figure", Some(sheet_id)),
        ActiveDialog::ConvertArchetype { sheet_id } => ("Convert Table Archetype...", Some(sheet_id)),
        ActiveDialog::ImportReshape => ("Import Reshape (Plate Map / Tidy Long)", None),
        ActiveDialog::ExcludeWithReason { sheet_id } => ("Exclude Selection with Reason", Some(sheet_id)),
        ActiveDialog::Print { sheet_id } => ("Print...", sheet_id.as_ref()),
        ActiveDialog::CommandPalette => ("Command Palette (Cmd+K)", None),
        ActiveDialog::About => ("About LumenCraft", None),
        _ => ("", None),
    };
    (title, target.map(|id| id.as_str().to_owned()), None, Vec::new(), Vec::new(), vec![("ok", "OK"), ("cancel", "Cancel")])
}

fn sheet_section_name(sec: lumencraft_doc::SheetSection) -> String {
    match sec {
        lumencraft_doc::SheetSection::Data => String::from("data"),
        lumencraft_doc::SheetSection::Info => String::from("info"),
        lumencraft_doc::SheetSection::Analyses => String::from("results"),
        lumencraft_doc::SheetSection::Graphs => String::from("graphs"),
        lumencraft_doc::SheetSection::Layouts => String::from("layouts"),
    }
}

fn active_dialog_slug(dialog: &ActiveDialog) -> String {
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
