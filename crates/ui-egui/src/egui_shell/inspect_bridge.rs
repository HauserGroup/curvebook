//! AccessKit + `FrameInspectMeta` -> `UiTreeSnapshot` bridge & Workspace Coexistence Arbiter (`R15.1`–`R15.5`).
//!
//! Coordinates native `egui` workspace surfaces and modal focus ownership, building settled
//! `ui.inspect` [`UiTreeSnapshot`] output, enforcing `100%` `ZLayer::ModalBackdrop` outside
//! capture, and verifying AccessKit `Role::ScrollBar` non-overlap.

use std::collections::BTreeMap;

use curvebook_doc::{ErrorCode, TableArchetype};
use curvebook_engine::{EngineError, cmd};
use curvebook_render::{RenderTargetSpec, encode_png};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use serde_json::{Value, json};

use crate::UiApp;
use crate::control_io::host_routing::contextual_command_params;
use crate::control_io::{
    dispatch_ui_command_as_ui, opt_bool_param, opt_finite_f64_param, opt_non_empty_str_param, opt_str_param, parse_bounded_u32,
    require_params_object_or_null,
};
use crate::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner};
#[cfg(any(test, feature = "test-support"))]
use crate::egui_shell::FrameInspectEntryInput;
use crate::egui_shell::app_modals::render_app_modal;
use crate::egui_shell::center_stage::{CenterStageContext, render_center_stage};
use crate::egui_shell::command_palette::render_command_palette;
use crate::egui_shell::fit_modal::{is_nonlinear_fit_dialog_active, render_fit_modal, render_hook_constant_modal};
use crate::egui_shell::graph_modals::{is_graph_modal_active, render_graph_modal};
use crate::egui_shell::import_wizard::render_import_or_convert_modal;
use crate::egui_shell::leading_dock::{LeadingDockContext, render_leading_dock};
use crate::egui_shell::manage_vars_modal::render_manage_vars_modal;
use crate::egui_shell::power_studio::{is_power_studio_dialog_active, render_power_studio_modal};
use crate::egui_shell::rasterizer::EguiScreenshotRegions;
use crate::egui_shell::stat_anova_modal::{is_one_way_anova_dialog_active, render_one_way_anova_modal};
use crate::egui_shell::stats_modals::{
    render_analyze_sheet_modal, render_change_data_analyzed_modal, render_dialogs_r9_modal, render_duplicate_analysis_modal,
    render_result_viewer_modal, render_stat_params_modal,
};
use crate::egui_shell::status_footer::{StatusFooterContext, render_status_footer};
use crate::egui_shell::top_chrome::{TopChromeContext, render_top_chrome};
use crate::egui_shell::trailing_studio::{TrailingStudioContext, render_trailing_studio};
use crate::egui_shell::transform_modals::{is_transform_edit_dialog_active, render_exclude_with_reason_modal, render_transform_modal};
use crate::egui_shell::{
    EguiShellViewState, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, ShellLayoutInput, Tokens, UiIntent, UiIntentOutcome,
};
use crate::grid_view::{parse_grid_cell_widget_id, parse_x_cell_widget_id, selection_cell_count};
use crate::inspect::{
    BrushedPointSnapshot, GridCoordSnapshot, GridSelectionSnapshot, MAX_INSPECT_SELECTED_COORDS, ScreenshotRegionTarget, UiDialogSnapshot,
    UiMenuSnapshot, UiNavigatorSheetNode, UiTreeSnapshot, UiWidgetNode, ZLayer, active_dialog_slug,
};
use crate::interaction::{KeyModifiers, UiEventOutcome, ViewportMetrics};
use crate::menus::{CuratedMenuNode, UI_COMMANDS, command_tooltip, curated_menu_bar, is_desktop_host_routed_command, menu_bar_tooltips};
use crate::modal_predicates::evaluate_dialog_button_for_app;
use crate::tr;
use crate::workspace_nav::ActiveWorkspacePane;

/// Which center-stage sheet backend renders the current frame in `egui` (`R15.5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HybridSheetSurface {
    /// Ported `egui` virtualized `Column` `DataSheet` grid.
    EguiColumnGrid,
    /// Native `egui` virtualized grid for the remaining 7 `DataSheet` archetypes (`R15.4`).
    EguiDataGrid,
    /// Non-grid native `egui` stage (`Welcome`, `PortfolioGallery`, `InfoSheet`, `ResultSheet`, `CanvasViewport`).
    EguiStage,
}

/// Which modal surface currently holds focus & `ZLayer::ModalBackdrop` ownership (`R15.2+`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HybridModalSurface {
    /// No modal dialog open; center stage & chrome receive input normally.
    None,
    /// Standardized `egui` `Parameters: One-Way ANOVA` modal (`Case A`).
    EguiOneWayAnova,
    /// `egui` `Cmd/Ctrl+K` Command Palette modal overlay (`R15.3`).
    EguiCommandPalette,
    /// `egui` `About` modal (`R15.1` minimal shell parity + `app_modals`).
    EguiAbout,
    /// Native `egui` unsaved-changes confirmation dialog (`close_dialog_open` / `unsaved_changes_open`).
    HostUnsavedClose,
    /// Native `egui` `Manage Variables...` master-detail modal (`R15.4`).
    EguiManageVariables,
    /// Native `egui` 5-stage `Import & Paste Special` wizard / `ImportReshape` / `ConvertArchetype` (`R15.4`).
    EguiImportWizard,
    /// Native `egui` Transform / Baseline / MV Filter / Extract / Exclude modal (`R15.4`).
    EguiTransformModal,
    /// Native `egui` `Analyze...` picker modal (`R15.4`).
    EguiAnalyzeSheet,
    /// Native `egui` `DuplicateAnalysisPrompt` modal (`R15.4`).
    EguiDuplicateAnalysis,
    /// Native `egui` `ChangeDataAnalyzed` retargeting modal (`R15.4`).
    EguiChangeDataAnalyzed,
    /// Native `egui` statistical / GLM / Cox / PCA / PCR / Clustering / Survival / R9 modal (`R15.4`).
    EguiStatParams,
    /// Native `egui` 7-tab `Nonlinear Curve Fitting` modal (`R15.4`).
    EguiFitModal,
    /// Native `egui` 3-branch `Hook Constant` browser modal (`R15.4`).
    EguiHookPicker,
    /// Native `egui` 4-stage `Power & Sample Size Studio` (`R15.4`).
    EguiPowerStudio,
    /// Native `egui` Graph / Axes / Additional Ticks / Layout configuration modal (`R15.4`).
    EguiGraphModal,
    /// Native `egui` `Export Figure` / `Print` utility modal (`R15.4`).
    EguiAppModal,
    /// Native `egui` 4-category `Preferences` modal (`R15.4`).
    EguiPreferences,
    /// Native `egui` `Find & Replace` modal (`R15.4`).
    EguiFindReplace,
    /// Native `egui` Navigator sheet delete confirmation modal (`FA-SEC16-04`).
    EguiNavDeleteConfirm,
    /// Native `egui` blocking progress modal overlay (`AUD-08-15`).
    EguiProgressModal,
}

/// Shared metadata descriptor passed into [`finalize_egui_modal_response`].
#[derive(Debug, Clone, Copy)]
pub struct ModalCaptureSpec<'a> {
    pub widget_id: &'a str,
    pub title: &'a str,
    pub z_dialog: ZLayer,
}

/// Shared backdrop capture and dialog bounds emitted by [`finalize_egui_modal_response`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModalCaptureReport {
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Record `modal.backdrop` + `dialog.*` into [`FrameInspectMeta`] and absorb outside clicks/wheel.
pub fn finalize_egui_modal_response<T>(
    ctx: &egui::Context,
    modal_resp: &egui::ModalResponse<T>,
    spec: ModalCaptureSpec<'_>,
    meta: &mut FrameInspectMeta,
) -> ModalCaptureReport {
    let viewport = ctx.content_rect();
    meta.record(FrameInspectNodeMeta {
        egui_id: modal_resp.backdrop_response.id,
        accesskit_id: modal_resp.backdrop_response.id.accesskit_id(),
        widget_id: String::from("modal.backdrop"),
        role: String::from("backdrop"),
        label: String::from("Modal Backdrop"),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        unclipped_rect: viewport,
        clipped_rect: viewport,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
        action_rank: None,
    });

    let pointer_pos = ctx.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()));
    let dlg_rect = modal_resp.response.rect;
    let outside = pointer_pos.is_some_and(|p| !dlg_rect.contains(p));
    let outside_click_captured = modal_resp.backdrop_response.clicked()
        || (outside
            && (modal_resp.backdrop_response.secondary_clicked()
                || ctx.input(|i| {
                    i.pointer.primary_clicked()
                        || i.pointer.secondary_clicked()
                        || i.events.iter().any(|ev| matches!(ev, egui::Event::PointerButton { pos, pressed: false, .. } if !dlg_rect.contains(*pos)))
                })));

    let has_wheel = ctx.input(|i| i.smooth_scroll_delta.length_sq() > 0.0 || i.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })));
    let mut outside_wheel_captured = false;
    if has_wheel && (outside || !dlg_rect.is_positive()) {
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
            i.events.retain(|ev| !matches!(ev, egui::Event::MouseWheel { .. }));
        });
        outside_wheel_captured = true;
    }

    meta.record(FrameInspectNodeMeta {
        egui_id: modal_resp.response.id,
        accesskit_id: modal_resp.response.id.accesskit_id(),
        widget_id: spec.widget_id.to_owned(),
        role: String::from("dialog"),
        label: spec.title.to_owned(),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        unclipped_rect: dlg_rect,
        clipped_rect: meta.viewport_rect.intersect(dlg_rect),
        z_layer: spec.z_dialog,
        ellipsized: false,
        full_label: None,
        action_rank: None,
    });

    ModalCaptureReport { dialog_rect_pt: Some(dlg_rect), outside_click_captured, outside_wheel_captured }
}

/// Arbitration decision resolved once per frame by [`HybridCoexistenceArbiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridCoexistenceDecision {
    pub sheet_surface: HybridSheetSurface,
    pub modal_surface: HybridModalSurface,
    pub modal_backdrop_active: bool,
    pub blocks_egui_grid: bool,
    pub renders_egui_column_grid: bool,
    pub renders_egui_anova_modal: bool,
    pub renders_egui_about_modal: bool,
}

/// Pure, stateless arbiter controlling focus ownership, Z-order, and raw-input routing
/// across native `egui` workspace surfaces and modals (`R15.5`).
pub struct HybridCoexistenceArbiter;

impl HybridCoexistenceArbiter {
    /// Classify the active sheet and modal layers for `app` and optional host close modal.
    #[must_use]
    pub fn evaluate(app: &UiApp, host_close_dialog_open: bool) -> HybridCoexistenceDecision {
        Self::evaluate_with_views(app, &app.egui_views, host_close_dialog_open)
    }

    /// Classify the active sheet and modal layers using explicit `views` state alongside `app`.
    #[must_use]
    pub fn evaluate_with_views(app: &UiApp, views: &EguiShellViewState, host_close_dialog_open: bool) -> HybridCoexistenceDecision {
        let doc = app.document();
        let active_ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
        let sheet_surface = match active_ds.map(|ds| &ds.archetype) {
            Some(TableArchetype::Column(_)) => HybridSheetSurface::EguiColumnGrid,
            Some(_) => HybridSheetSurface::EguiDataGrid,
            None => HybridSheetSurface::EguiStage,
        };

        let modal_surface = if host_close_dialog_open || views.app_modals.unsaved_changes_open {
            HybridModalSurface::HostUnsavedClose
        } else if views.app_modals.preferences_open {
            HybridModalSurface::EguiPreferences
        } else if views.app_modals.find_replace_open {
            HybridModalSurface::EguiFindReplace
        } else if app.grid.variables_dialog.open {
            HybridModalSurface::EguiManageVariables
        } else if app.grid.import_wizard.open {
            HybridModalSurface::EguiImportWizard
        } else if views.transform_modals.open_kind.is_some() {
            HybridModalSurface::EguiTransformModal
        } else if views.leading_dock.delete_confirm.is_some() {
            HybridModalSurface::EguiNavDeleteConfirm
        } else if views.progress_modal.is_some() {
            HybridModalSurface::EguiProgressModal
        } else {
            classify_active_dialog_surface(app)
        };

        let modal_backdrop_active = modal_surface != HybridModalSurface::None;
        let blocks_egui_grid = modal_backdrop_active;

        HybridCoexistenceDecision {
            sheet_surface,
            modal_surface,
            modal_backdrop_active,
            blocks_egui_grid,
            renders_egui_column_grid: matches!(sheet_surface, HybridSheetSurface::EguiColumnGrid | HybridSheetSurface::EguiDataGrid),
            renders_egui_anova_modal: modal_surface == HybridModalSurface::EguiOneWayAnova,
            renders_egui_about_modal: modal_surface == HybridModalSurface::EguiAbout,
        }
    }

    /// Return `true` when `Cmd/Ctrl+T` is eligible to open `EditAnalysisParameters` on an active `ResultSheet`.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn allows_cmd_t_edit_params(app: &UiApp, decision: &HybridCoexistenceDecision, egui_wants_keyboard: bool) -> bool {
        !decision.modal_backdrop_active && !egui_wants_keyboard && crate::interaction::resolve_result_sheet_for_shortcut(app, true).is_some()
    }
}

fn classify_active_dialog_surface(app: &UiApp) -> HybridModalSurface {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::None => HybridModalSurface::None,
        ActiveDialog::About => HybridModalSurface::EguiAbout,
        ActiveDialog::ExportFigure { .. }
        | ActiveDialog::Print { .. }
        | ActiveDialog::Checkpoints
        | ActiveDialog::ImportWarnings
        | ActiveDialog::ImportProjectMode { .. }
        | ActiveDialog::CsvExportOptions(_)
        | ActiveDialog::ExportAllTablesExistingFiles { .. } => HybridModalSurface::EguiAppModal,
        ActiveDialog::CommandPalette => HybridModalSurface::EguiCommandPalette,
        ActiveDialog::AnalyzeSheet { .. } => HybridModalSurface::EguiAnalyzeSheet,
        ActiveDialog::DuplicateAnalysisPrompt { .. } => HybridModalSurface::EguiDuplicateAnalysis,
        ActiveDialog::ChangeDataAnalyzed { .. } => HybridModalSurface::EguiChangeDataAnalyzed,
        ActiveDialog::ConstantHookPicker { .. } => HybridModalSurface::EguiHookPicker,
        ActiveDialog::PowerAndSampleSize => HybridModalSurface::EguiPowerStudio,
        ActiveDialog::ImportReshape | ActiveDialog::ConvertArchetype { .. } => HybridModalSurface::EguiImportWizard,
        ActiveDialog::ExcludeWithReason { .. } => HybridModalSurface::EguiTransformModal,
        ActiveDialog::EditAnalysisParameters { .. } => {
            if is_one_way_anova_dialog_active(app) {
                HybridModalSurface::EguiOneWayAnova
            } else if is_transform_edit_dialog_active(app) {
                HybridModalSurface::EguiTransformModal
            } else if is_nonlinear_fit_dialog_active(app) {
                HybridModalSurface::EguiFitModal
            } else if is_power_studio_dialog_active(app) {
                HybridModalSurface::EguiPowerStudio
            } else {
                HybridModalSurface::EguiStatParams
            }
        }
        ActiveDialog::RandomizeGroups
        | ActiveDialog::RandomSampleRows { .. }
        | ActiveDialog::SimulateDistributions
        | ActiveDialog::ResultViewer { .. } => HybridModalSurface::EguiStatParams,
        other if is_graph_modal_active(other) => HybridModalSurface::EguiGraphModal,
        _ => HybridModalSurface::EguiGraphModal,
    }
}

/// Result emitted by `render_minimal_shell_and_about` and [`render_egui_shell_frame`] for
/// testing, `ui.inspect`, `ui.screenshot`, and `apps/curvebook` viewport sync.
#[derive(Debug, Clone, Default)]
pub struct ShellAboutFrameReport {
    pub regions: EguiScreenshotRegions,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub scrollbar_rect_pt: Option<egui::Rect>,
    pub resolved_test_banner: String,
    pub decision: Option<HybridCoexistenceDecision>,
    pub tier1_rect_pt: Option<egui::Rect>,
    pub tier2_rect_pt: Option<egui::Rect>,
    pub leading_dock_rect_pt: Option<egui::Rect>,
    pub trailing_dock_rect_pt: Option<egui::Rect>,
    pub footer_rect_pt: Option<egui::Rect>,
    pub menu_popup_open: bool,
    pub center_stage_handled_natively: bool,
    pub grid_overlay_shown: bool,
}

impl ShellAboutFrameReport {
    const fn apply_modal_capture(&mut self, dialog_rect_pt: Option<egui::Rect>, outside_click_captured: bool, outside_wheel_captured: bool) {
        self.regions.dialog_rect_pt = dialog_rect_pt;
        self.outside_click_captured = outside_click_captured;
        self.outside_wheel_captured = outside_wheel_captured;
    }
}

/// Convert a logical coordinate `[x_pt, y_pt]` to physical pixels using `pixels_per_point`.
#[must_use]
pub fn logical_pt_to_physical_px(pt: [f64; 2], pixels_per_point: f32) -> [f64; 2] {
    let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { f64::from(pixels_per_point) } else { 1.0 };
    [pt[0] * scale, pt[1] * scale]
}

/// Convert a physical pixel coordinate `[x_px, y_px]` to logical points using `pixels_per_point`.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn physical_px_to_logical_pt(px: [f64; 2], pixels_per_point: f32) -> [f64; 2] {
    let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { f64::from(pixels_per_point) } else { 1.0 };
    [px[0] / scale, px[1] / scale]
}

/// Convert a clipped logical-point `egui::Rect` into physical `[x, y, width, height]` inspect bounds.
#[must_use]
pub fn clipped_rect_to_bounds_px(rect: egui::Rect, pixels_per_point: f32) -> Option<[f64; 4]> {
    if !rect.is_positive() {
        return None;
    }
    let min_px = logical_pt_to_physical_px([f64::from(rect.min.x), f64::from(rect.min.y)], pixels_per_point);
    let max_px = logical_pt_to_physical_px([f64::from(rect.max.x), f64::from(rect.max.y)], pixels_per_point);
    let w = (max_px[0] - min_px[0]).max(0.0);
    let h = (max_px[1] - min_px[1]).max(0.0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some([min_px[0].round(), min_px[1].round(), w.round(), h.round()])
}

/// Map an `accesskit::Role` to CurveBook's canonical `UiWidgetNode.role` string.
#[must_use]
pub const fn accesskit_role_to_inspect_role(role: egui::accesskit::Role) -> &'static str {
    use egui::accesskit::Role;
    match role {
        Role::Button | Role::DefaultButton => "button",
        Role::Dialog | Role::AlertDialog => "dialog",
        Role::Menu | Role::MenuBar => "menu",
        Role::MenuItem | Role::MenuItemCheckBox | Role::MenuItemRadio => "menuitem",
        Role::Tab | Role::TabList => "tab",
        Role::CheckBox => "checkbox",
        Role::RadioButton => "radio",
        Role::TextInput | Role::MultilineTextInput | Role::SearchInput => "textbox",
        Role::ComboBox => "combobox",
        Role::Slider => "slider",
        Role::ScrollBar => "scrollbar",
        Role::Label | Role::Heading => "label",
        Role::ScrollView => "scrollview",
        Role::Tree | Role::TreeItem => "treeitem",
        Role::Canvas => "canvas",
        _ => "widget",
    }
}

fn scope_wants(scope: Option<&str>, target: &str) -> bool {
    match scope.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("all" | "workspace") => true,
        Some(other) => other.eq_ignore_ascii_case(target),
    }
}

fn entry_belongs_to_scope(entry: &FrameInspectNodeMeta, scope_tag: &str) -> bool {
    let wid = entry.widget_id.as_str();
    match scope_tag {
        "toolbar" => wid.starts_with("toolbar."),
        "menus" => wid.starts_with("menu."),
        "tabs" => wid.starts_with("tab.") || wid.starts_with("result.tab."),
        "canvas" => {
            wid.starts_with("canvas.")
                || wid.starts_with("center.pane.")
                || wid == "split.divider"
                || wid.starts_with("result.diagnostic_banner")
                || wid.starts_with("info.")
                || wid.starts_with("welcome.")
        }
        "grid" => wid.starts_with("grid.") || wid.starts_with("formula_bar.") || wid.starts_with("mv_formula."),
        "dialog" => is_dialog_meta_entry(entry),
        "navigator" => wid.starts_with("nav."),
        "inspector" => wid.starts_with("inspector."),
        "status" => wid.starts_with("status."),
        _ => false,
    }
}

const DIALOG_WIDGET_PREFIXES: &[&str] = &[
    "dialog.",
    "modal.",
    "edit.",
    "palette.",
    "power.",
    "import.",
    "transform.",
    "analyze.",
    "fit.",
    "axes.",
    "graph_type.",
    "nav.delete_confirm.",
    "unsaved.",
    "prefs.",
    "find.",
];

fn is_dialog_meta_entry(entry: &FrameInspectNodeMeta) -> bool {
    let wid = entry.widget_id.as_str();
    matches!(entry.z_layer, ZLayer::ModalDialog | ZLayer::ModalBackdrop) || DIALOG_WIDGET_PREFIXES.iter().any(|p| wid.starts_with(p))
}

struct AccessKitLookup<'a> {
    nodes: BTreeMap<egui::accesskit::NodeId, &'a egui::accesskit::Node>,
    focus: Option<egui::accesskit::NodeId>,
}

impl<'a> AccessKitLookup<'a> {
    fn from_update(update: Option<&'a egui::accesskit::TreeUpdate>) -> Self {
        let mut nodes = BTreeMap::new();
        let mut focus = None;
        if let Some(tree) = update {
            focus = Some(tree.focus);
            for (id, node) in &tree.nodes {
                nodes.insert(*id, node);
            }
        }
        Self { nodes, focus }
    }
}

fn widget_node_from_meta(entry: &FrameInspectNodeMeta, ak: &AccessKitLookup<'_>, scale_factor: f32, include_bounds: bool) -> UiWidgetNode {
    let ak_node = ak.nodes.get(&entry.accesskit_id).copied();
    let role = ak_node.map_or_else(|| entry.role.clone(), |n| String::from(accesskit_role_to_inspect_role(n.role())));
    let label = ak_node.and_then(|n| n.label()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| entry.label.clone());
    let value = entry.value.clone().or_else(|| ak_node.and_then(|n| n.value().map(|s| json!(s)).or_else(|| n.numeric_value().map(|v| json!(v)))));
    let ak_disabled = ak_node.is_some_and(egui::accesskit::Node::is_disabled);
    let enabled = entry.enabled && entry.disabled_reason.is_none() && !ak_disabled;

    #[allow(clippy::cast_possible_truncation)]
    let effective_rect = ak_node
        .and_then(egui::accesskit::Node::bounds)
        .map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
        .map_or(entry.clipped_rect, |ak_rect| {
            let clipped = entry.clipped_rect.intersect(ak_rect);
            if clipped.is_positive() { clipped } else { entry.clipped_rect }
        });

    let bounds = if include_bounds { clipped_rect_to_bounds_px(effective_rect, scale_factor) } else { None };

    UiWidgetNode {
        widget_id: entry.widget_id.clone(),
        role,
        label,
        command_id: entry.command_id.clone(),
        value,
        enabled,
        disabled_reason: entry.disabled_reason.clone(),
        bounds,
    }
}

fn resolve_dialog_target_sheet_id(app: &UiApp) -> Option<String> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatAxes { sheet_id }
        | ActiveDialog::ExportFigure { sheet_id }
        | ActiveDialog::ConvertArchetype { sheet_id }
        | ActiveDialog::ExcludeWithReason { sheet_id } => Some(sheet_id.as_str().to_owned()),
        ActiveDialog::AnalyzeSheet { source_sheet_id } | ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, .. } => {
            Some(source_sheet_id.as_str().to_owned())
        }
        ActiveDialog::EditAnalysisParameters { result_sheet_id }
        | ActiveDialog::ChangeDataAnalyzed { result_sheet_id }
        | ActiveDialog::ResultViewer { result_sheet_id } => Some(result_sheet_id.as_str().to_owned()),
        ActiveDialog::ConstantHookPicker { target_sheet_id } => Some(target_sheet_id.as_str().to_owned()),
        ActiveDialog::ChangeGraphType { graph_sheet_id }
        | ActiveDialog::FormatGraph { graph_sheet_id }
        | ActiveDialog::PairwiseComparisons { graph_sheet_id }
        | ActiveDialog::BubblePlotInspector { graph_sheet_id }
        | ActiveDialog::FormatSelectedPoints { graph_sheet_id }
        | ActiveDialog::InsertModelEquation { graph_sheet_id } => Some(graph_sheet_id.as_str().to_owned()),
        ActiveDialog::ApplyStyleFrom { target_graph_id } => Some(target_graph_id.as_str().to_owned()),
        ActiveDialog::PageLayoutComposer { layout_sheet_id } | ActiveDialog::EqualizeScaling { layout_sheet_id } => {
            Some(layout_sheet_id.as_str().to_owned())
        }
        ActiveDialog::FormatEmbeddedTable { sheet_id, .. } | ActiveDialog::FormatImage { sheet_id, .. } => Some(sheet_id.as_str().to_owned()),
        ActiveDialog::CreateGraph { source_sheet_id } | ActiveDialog::RandomSampleRows { source_sheet_id } => {
            source_sheet_id.as_ref().map(|s| s.as_str().to_owned())
        }
        ActiveDialog::Print { sheet_id } => sheet_id.as_ref().map(|s| s.as_str().to_owned()),
        _ => app.egui_views.leading_dock.delete_confirm.as_ref().map(|d| d.sheet_id.as_str().to_owned()),
    }
}

fn resolve_dialog_stage(app: &UiApp) -> Option<String> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => Some(app.canvas_dialogs.power_wizard.stage.label().to_owned()),
        ActiveDialog::AnalyzeSheet { .. } => {
            let doc = app.document();
            let src_id = app.canvas_dialogs.analyze.source_sheet_id.as_ref().or(doc.active_sheet_id.as_ref())?;
            doc.result_sheet(src_id).and_then(|rs| rs.tabs.get(app.canvas_dialogs.result_view.active_tab_index)).map(|t| t.id.clone())
        }
        _ if app.grid.import_wizard.open => Some(format!("{:?}", app.grid.import_wizard.stage)),
        _ => None,
    }
}

fn resolve_dialog_slug(app: &UiApp, decision: &HybridCoexistenceDecision, dialog_entry: Option<&FrameInspectNodeMeta>) -> String {
    match decision.modal_surface {
        HybridModalSurface::HostUnsavedClose => String::from("unsaved_changes"),
        HybridModalSurface::EguiPreferences => String::from("preferences"),
        HybridModalSurface::EguiFindReplace => String::from("find_replace"),
        HybridModalSurface::EguiManageVariables => String::from("manage_variables"),
        HybridModalSurface::EguiNavDeleteConfirm => String::from("nav_delete_confirm"),
        HybridModalSurface::EguiProgressModal => String::from("progress_modal"),
        HybridModalSurface::EguiCommandPalette => String::from("command_palette"),
        _ if app.canvas_dialogs.active_dialog != ActiveDialog::None => active_dialog_slug(&app.canvas_dialogs.active_dialog),
        HybridModalSurface::EguiImportWizard => String::from("import_paste_special"),
        HybridModalSurface::EguiTransformModal => String::from("transform_modal"),
        HybridModalSurface::None => String::from("none"),
        _ => dialog_entry
            .map(|e| match e.widget_id.as_str() {
                "palette.dialog" => String::from("command_palette"),
                raw => raw.strip_prefix("dialog.").or_else(|| raw.strip_prefix("modal.")).unwrap_or(raw).to_owned(),
            })
            .unwrap_or_else(|| String::from("shell_modal")),
    }
}

const FALLBACK_DIALOG_TITLES: &[(&str, &str)] = &[
    ("command_palette", "Command Palette"),
    ("preferences", "Preferences"),
    ("find_replace", "Find & Replace"),
    ("manage_variables", "Manage Variables"),
    ("import_paste_special", "Import & Paste Special"),
    ("import_reshape", "Import & Paste Special"),
    ("transform_modal", "Transform"),
    ("power_and_sample_size", "Power & Sample Size"),
    ("analyze_sheet", "Analyze Data"),
    ("edit_analysis_parameters", "Edit Analysis Parameters"),
    ("change_data_analyzed", "Change Data Analyzed"),
    ("format_axes", "Format Axes"),
    ("format_graph", "Format Graph"),
    ("format_selected_points", "Format Selected Points"),
    ("change_graph_type", "Change Graph Type"),
    ("create_graph", "Create New Graph"),
    ("unsaved_changes", "Unsaved Changes"),
    ("nav_delete_confirm", "Delete Sheet"),
];

fn fallback_dialog_title(app: &UiApp, dialog_kind: &str) -> String {
    match dialog_kind {
        "about" => tr(&app.locale, "menu.help.about").to_owned(),
        "duplicate_analysis_prompt" => tr(&app.locale, "w6.view.modal.analysis_already_exists").to_owned(),
        _ => FALLBACK_DIALOG_TITLES
            .iter()
            .find_map(|(k, title)| (*k == dialog_kind).then(|| (*title).to_owned()))
            .unwrap_or_else(|| dialog_kind.replace('_', " ")),
    }
}

const BUTTON_ACTION_ALIASES: &[(&str, &[&str])] = &[
    ("ok", &["modal.ok", "graph_type.apply", "import.commit", "transform.commit", "nav.delete_confirm.confirm"]),
    ("cancel", &["modal.cancel", "nav.delete_confirm.cancel"]),
    ("apply", &["modal.apply"]),
    ("create", &["modal.create", "power.commit"]),
    ("next", &["modal.next"]),
    ("back", &["modal.back"]),
    ("close", &["modal.close"]),
    ("restore", &["modal.restore"]),
    ("discard", &["modal.discard"]),
    ("preview", &["modal.preview"]),
];

fn canonical_action_for_button_widget(wid: &str) -> Option<&'static str> {
    BUTTON_ACTION_ALIASES.iter().find_map(|(action, wids)| wids.contains(&wid).then_some(*action))
}

fn build_dialog_snapshot_from_meta(
    app: &UiApp,
    views: &EguiShellViewState,
    meta: &FrameInspectMeta,
    ak: &AccessKitLookup<'_>,
    include_bounds: bool,
    host_close_dialog_open: bool,
) -> Option<UiDialogSnapshot> {
    let decision = HybridCoexistenceArbiter::evaluate_with_views(app, views, host_close_dialog_open);
    let dialog_entry = meta
        .find_by_widget_id("modal.dialog")
        .or_else(|| meta.find_by_widget_id("dialog.about"))
        .or_else(|| meta.find_by_widget_id("palette.dialog"))
        .or_else(|| meta.entries.iter().find(|e| (e.widget_id.starts_with("dialog.") || e.widget_id.starts_with("modal.")) && e.role == "dialog"));
    let has_shell_modal = dialog_entry.is_some() || decision.modal_surface != HybridModalSurface::None;
    if app.canvas_dialogs.active_dialog == ActiveDialog::None && !has_shell_modal {
        return None;
    }

    let dialog_kind = resolve_dialog_slug(app, &decision, dialog_entry);

    let title = dialog_entry
        .and_then(|e| {
            ak.nodes
                .get(&e.accesskit_id)
                .copied()
                .and_then(|n| n.label())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| (!e.label.trim().is_empty()).then(|| e.label.clone()))
        })
        .unwrap_or_else(|| fallback_dialog_title(app, &dialog_kind));

    let bounds = if include_bounds { dialog_entry.and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor)) } else { None };

    let mut tabs = Vec::new();
    let mut controls = Vec::new();
    let mut buttons = Vec::new();
    let mut rendered_buttons_by_action: BTreeMap<&'static str, UiWidgetNode> = BTreeMap::new();

    for entry in &meta.entries {
        let wid = entry.widget_id.as_str();
        if matches!(wid, "modal.backdrop" | "modal.dialog" | "dialog.about" | "palette.dialog" | "modal.title") || entry.role == "dialog" {
            continue;
        }
        if !is_dialog_meta_entry(entry) {
            continue;
        }
        let node = widget_node_from_meta(entry, ak, meta.scale_factor, include_bounds);
        if wid.starts_with("edit.tab.") || wid.contains(".tab.") || node.role == "tab" {
            tabs.push(node);
        } else if let Some(action) = canonical_action_for_button_widget(wid) {
            rendered_buttons_by_action.entry(action).or_insert_with(|| node.clone());
            buttons.push(node);
        } else if wid.starts_with("dialog.button.") {
            buttons.push(node);
        } else {
            controls.push(node);
        }
    }

    append_synthetic_dialog_fallbacks(app, &rendered_buttons_by_action, &mut controls, &mut buttons);

    Some(UiDialogSnapshot {
        dialog_kind,
        title,
        target_sheet_id: resolve_dialog_target_sheet_id(app),
        stage: resolve_dialog_stage(app),
        bounds,
        tabs,
        controls,
        buttons,
    })
}

struct SyntheticNodeSpec<'a> {
    widget_id: &'a str,
    role: &'a str,
    label: &'a str,
    value: Option<Value>,
    enabled: bool,
    disabled_reason: Option<String>,
    bounds: Option<[f64; 4]>,
}

fn push_synthetic_node(list: &mut Vec<UiWidgetNode>, spec: SyntheticNodeSpec<'_>) {
    if !list.iter().any(|n| n.widget_id == spec.widget_id) {
        list.push(UiWidgetNode {
            widget_id: spec.widget_id.to_owned(),
            role: spec.role.to_owned(),
            label: spec.label.to_owned(),
            command_id: None,
            value: spec.value,
            enabled: spec.enabled,
            disabled_reason: spec.disabled_reason,
            bounds: spec.bounds,
        });
    }
}

fn append_synthetic_dialog_fallbacks(
    app: &UiApp,
    rendered_buttons_by_action: &BTreeMap<&'static str, UiWidgetNode>,
    controls: &mut Vec<UiWidgetNode>,
    buttons: &mut Vec<UiWidgetNode>,
) {
    for (action, src) in rendered_buttons_by_action {
        let canon_id = format!("dialog.button.{action}");
        let eval = evaluate_dialog_button_for_app(app, action);
        let enabled = eval.as_ref().map_or(src.enabled, |e| e.enabled && src.enabled);
        let disabled_reason = eval.and_then(|e| e.disabled_reason).or_else(|| src.disabled_reason.clone());
        if !buttons.iter().any(|b| b.widget_id == canon_id) {
            buttons.push(UiWidgetNode {
                widget_id: canon_id,
                role: String::from("button"),
                label: src.label.clone(),
                command_id: src.command_id.clone(),
                value: None,
                enabled,
                disabled_reason,
                bounds: src.bounds,
            });
        }
    }

    match app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => {
            if let Some(ok_src) = rendered_buttons_by_action.get("ok") {
                let eval = evaluate_dialog_button_for_app(app, "create");
                push_synthetic_node(
                    buttons,
                    SyntheticNodeSpec {
                        widget_id: "dialog.button.create",
                        role: "button",
                        label: tr(&app.locale, "w6.view.btn.create"),
                        value: None,
                        enabled: eval.as_ref().map_or(ok_src.enabled, |e| e.enabled),
                        disabled_reason: eval.and_then(|e| e.disabled_reason),
                        bounds: ok_src.bounds,
                    },
                );
            }
            for action in ["back", "next", "cancel", "create"] {
                let canon_id = format!("dialog.button.{action}");
                if let Some(eval) = evaluate_dialog_button_for_app(app, action) {
                    push_synthetic_node(
                        buttons,
                        SyntheticNodeSpec {
                            widget_id: &canon_id,
                            role: "button",
                            label: action,
                            value: None,
                            enabled: eval.enabled,
                            disabled_reason: eval.disabled_reason,
                            bounds: None,
                        },
                    );
                }
            }
        }
        ActiveDialog::DuplicateAnalysisPrompt { .. } => {
            let dup_val = match app.canvas_dialogs.analyze.duplicate_choice {
                crate::dialogs_canvas::DuplicateChoice::CreateNew => "create_new",
                crate::dialogs_canvas::DuplicateChoice::ModifyExisting => "modify_existing",
            };
            let bounds = controls
                .iter()
                .find(|c| matches!(c.widget_id.as_str(), "analyze_picker.dup.create_new" | "analyze_picker.dup.modify_existing"))
                .and_then(|c| c.bounds);
            push_synthetic_node(
                controls,
                SyntheticNodeSpec {
                    widget_id: "duplicate.choice",
                    role: "radio_group",
                    label: "Duplicate analysis choice",
                    value: Some(json!(dup_val)),
                    enabled: true,
                    disabled_reason: None,
                    bounds,
                },
            );
        }
        ActiveDialog::ChangeGraphType { .. } => {
            let bounds = controls.iter().find(|c| c.widget_id == "graph_type.preview_pane").and_then(|c| c.bounds);
            let cgt = &app.canvas_dialogs.change_graph_type;
            for (widget_id, role, label, val) in [
                ("graph_type.selected", "listbox", "Selected graph type", json!(cgt.selected_option_id)),
                ("graph_type.summary_measure", "combobox", "Summary measure", json!(cgt.summary_measure)),
                ("graph_type.error_bar_metric", "combobox", "Error bar metric", json!(cgt.error_bar_metric)),
            ] {
                push_synthetic_node(
                    controls,
                    SyntheticNodeSpec { widget_id, role, label, value: Some(val), enabled: true, disabled_reason: None, bounds },
                );
            }
        }
        _ => {}
    }
}

/// Convert CurveBook [`KeyModifiers`] to `egui::Modifiers`.
#[must_use]
pub const fn to_egui_modifiers(m: KeyModifiers) -> Modifiers {
    Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: m.meta, command: m.meta || m.ctrl }
}

/// Convert CurveBook [`crate::interaction::PointerButton`] to `egui::PointerButton`.
#[must_use]
pub const fn to_egui_pointer_button(button: crate::interaction::PointerButton) -> egui::PointerButton {
    match button {
        crate::interaction::PointerButton::Primary => egui::PointerButton::Primary,
        crate::interaction::PointerButton::Secondary => egui::PointerButton::Secondary,
        crate::interaction::PointerButton::Middle => egui::PointerButton::Middle,
    }
}

fn parse_pointer_button_str(button: &str) -> Result<egui::PointerButton, EngineError> {
    match button.trim().to_ascii_lowercase().as_str() {
        "primary" | "left" => Ok(egui::PointerButton::Primary),
        "secondary" | "right" => Ok(egui::PointerButton::Secondary),
        "middle" => Ok(egui::PointerButton::Middle),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported pointer button `{other}`"))),
    }
}

const ALPHA_KEYS: [(Key, &str); 26] = [
    (Key::A, "A"),
    (Key::B, "B"),
    (Key::C, "C"),
    (Key::D, "D"),
    (Key::E, "E"),
    (Key::F, "F"),
    (Key::G, "G"),
    (Key::H, "H"),
    (Key::I, "I"),
    (Key::J, "J"),
    (Key::K, "K"),
    (Key::L, "L"),
    (Key::M, "M"),
    (Key::N, "N"),
    (Key::O, "O"),
    (Key::P, "P"),
    (Key::Q, "Q"),
    (Key::R, "R"),
    (Key::S, "S"),
    (Key::T, "T"),
    (Key::U, "U"),
    (Key::V, "V"),
    (Key::W, "W"),
    (Key::X, "X"),
    (Key::Y, "Y"),
    (Key::Z, "Z"),
];

const DIGIT_KEYS: [(Key, &str); 10] = [
    (Key::Num0, "0"),
    (Key::Num1, "1"),
    (Key::Num2, "2"),
    (Key::Num3, "3"),
    (Key::Num4, "4"),
    (Key::Num5, "5"),
    (Key::Num6, "6"),
    (Key::Num7, "7"),
    (Key::Num8, "8"),
    (Key::Num9, "9"),
];

const FUNCTION_KEYS: [(Key, &str, &str); 12] = [
    (Key::F1, "F1", "f1"),
    (Key::F2, "F2", "f2"),
    (Key::F3, "F3", "f3"),
    (Key::F4, "F4", "f4"),
    (Key::F5, "F5", "f5"),
    (Key::F6, "F6", "f6"),
    (Key::F7, "F7", "f7"),
    (Key::F8, "F8", "f8"),
    (Key::F9, "F9", "f9"),
    (Key::F10, "F10", "f10"),
    (Key::F11, "F11", "f11"),
    (Key::F12, "F12", "f12"),
];

const PUNCT_KEYS: [(Key, &str, &[&str]); 11] = [
    (Key::Plus, "+", &["plus", "+"]),
    (Key::Minus, "-", &["minus", "-"]),
    (Key::Equals, "=", &["equals", "="]),
    (Key::Comma, ",", &["comma", ","]),
    (Key::Period, ".", &["period", "."]),
    (Key::Slash, "/", &["slash", "/"]),
    (Key::Backslash, "\\", &["backslash", "\\"]),
    (Key::Semicolon, ";", &["semicolon", ";"]),
    (Key::Quote, "'", &["quote", "'"]),
    (Key::OpenBracket, "[", &["openbracket", "open_bracket", "["]),
    (Key::CloseBracket, "]", &["closebracket", "close_bracket", "]"]),
];

const NAV_KEYS: [(Key, &[&str]); 15] = [
    (Key::Enter, &["enter", "return"]),
    (Key::Escape, &["escape", "esc"]),
    (Key::Tab, &["tab"]),
    (Key::Backspace, &["backspace"]),
    (Key::Delete, &["delete", "del"]),
    (Key::Insert, &["insert", "ins"]),
    (Key::Home, &["home"]),
    (Key::End, &["end"]),
    (Key::PageUp, &["pageup", "page_up"]),
    (Key::PageDown, &["pagedown", "page_down"]),
    (Key::Space, &["space", " "]),
    (Key::ArrowUp, &["arrowup", "arrow_up", "up"]),
    (Key::ArrowDown, &["arrowdown", "arrow_down", "down"]),
    (Key::ArrowLeft, &["arrowleft", "arrow_left", "left"]),
    (Key::ArrowRight, &["arrowright", "arrow_right", "right"]),
];

/// Parse a symbolic key name into `egui::Key` (case-insensitive for named keys and ASCII letters).
#[must_use]
pub fn parse_egui_key(raw: &str) -> Option<Key> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let tok = lower.as_str();
    if let Some((k, _)) = NAV_KEYS.iter().find(|(_, aliases)| aliases.contains(&tok)) {
        return Some(*k);
    }
    if let Some((k, _, _)) = PUNCT_KEYS.iter().find(|(_, _, aliases)| aliases.contains(&tok)) {
        return Some(*k);
    }
    if let Some((k, _, _)) = FUNCTION_KEYS.iter().find(|(_, _, alias)| *alias == tok) {
        return Some(*k);
    }
    if trimmed.len() == 1
        && let Some(b) = trimmed.as_bytes().first().copied()
    {
        if b.is_ascii_alphabetic() {
            return ALPHA_KEYS.get(usize::from(b.to_ascii_uppercase() - b'A')).map(|(k, _)| *k);
        }
        if b.is_ascii_digit() {
            return DIGIT_KEYS.get(usize::from(b - b'0')).map(|(k, _)| *k);
        }
    }
    None
}

fn viewport_geom_for_app(app: &UiApp, metrics: ViewportMetrics) -> (f32, f32, f32) {
    #[allow(clippy::cast_possible_truncation)]
    let w_pt = metrics.logical_width.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let h_pt = metrics.logical_height.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    let zoom = app.ui_scale();
    let native_ppp = (ppp / zoom).max(0.05);
    (w_pt, h_pt, native_ppp)
}

fn configure_ctx_for_app(ctx: &egui::Context, app: &UiApp) {
    let zoom = app.ui_scale();
    ctx.options_mut(|o| {
        o.zoom_with_keyboard = false;
        o.zoom_factor = zoom;
    });
}

fn configure_offscreen_egui_ctx(app: &UiApp, metrics: ViewportMetrics) -> (egui::Context, f32, f32, f32) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
    configure_ctx_for_app(&ctx, app);
    let (w_pt, h_pt, native_ppp) = viewport_geom_for_app(app, metrics);
    (ctx, w_pt, h_pt, native_ppp)
}

fn acquire_headless_egui_ctx(app: &UiApp, metrics: ViewportMetrics) -> (egui::Context, f32, f32, f32, f64) {
    let (w_pt, h_pt, native_ppp) = viewport_geom_for_app(app, metrics);
    let existing = app.headless_egui.inner.lock().ok().map(|mut guard| {
        if let Some((ctx, t)) = guard.as_ref() {
            (ctx.clone(), *t)
        } else {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
            *guard = Some((ctx.clone(), 0.0));
            (ctx, 0.0)
        }
    });
    let (ctx, time_sec) = existing.unwrap_or_else(|| {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
        (ctx, 0.0)
    });
    let monotonic_t = time_sec.max(ctx.input(|i| i.time));
    configure_ctx_for_app(&ctx, app);
    (ctx, w_pt, h_pt, native_ppp, monotonic_t)
}

fn store_headless_egui_time(app: &UiApp, time_sec: f64) {
    if let Ok(mut guard) = app.headless_egui.inner.lock()
        && let Some((_, ref mut slot_t)) = *guard
    {
        *slot_t = time_sec;
    }
}

fn step_egui_shell_with_events(
    ctx: &egui::Context,
    app: &mut UiApp,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    geom: (f32, f32, f32),
    time_sec: f64,
    initial_events: Vec<egui::Event>,
) -> Result<Vec<UiIntentOutcome>, EngineError> {
    let (outcomes, _ak) = step_egui_shell_with_events_and_ak(ctx, app, views, meta, geom, time_sec, initial_events)?;
    Ok(outcomes)
}

fn step_egui_shell_with_events_and_ak(
    ctx: &egui::Context,
    app: &mut UiApp,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    geom: (f32, f32, f32),
    time_sec: f64,
    initial_events: Vec<egui::Event>,
) -> Result<(Vec<UiIntentOutcome>, Option<egui::accesskit::TreeUpdate>), EngineError> {
    let (outcomes, ak, _) = step_egui_shell_with_events_and_ak_repaint(ctx, app, views, meta, geom, time_sec, initial_events)?;
    Ok((outcomes, ak))
}

fn step_egui_shell_with_events_and_ak_repaint(
    ctx: &egui::Context,
    app: &mut UiApp,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    geom: (f32, f32, f32),
    time_sec: f64,
    initial_events: Vec<egui::Event>,
) -> Result<(Vec<UiIntentOutcome>, Option<egui::accesskit::TreeUpdate>, bool), EngineError> {
    let (w_pt, h_pt, native_ppp) = geom;
    let mut pending_events = initial_events;
    let mut carried_intents = PendingUiIntents::default();
    for pass in 0..6_u32 {
        let events = std::mem::take(&mut pending_events);
        let pass_time = time_sec + f64::from(pass) * 0.0005;
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(w_pt, h_pt))),
            time: Some(pass_time),
            predicted_dt: 1.0 / 60.0,
            focused: true,
            events,
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(native_ppp), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut settled = false;
        let mut output = ctx.run_ui(raw_input, |ui| {
            let mut pass_intents = PendingUiIntents::default();
            let _ = render_egui_shell_frame(ui, app, views, meta, &mut pass_intents);
            carried_intents.merge_pass(pass_intents);
            settled = !ui.ctx().will_discard();
        });
        let immediate_repaint = output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay == std::time::Duration::ZERO);
        output.textures_delta.clear();
        if (settled && !output.platform_output.requested_discard()) || pass == 5 {
            app.egui_views = views.clone();
            let ak_update = output.platform_output.accesskit_update.take();
            let outcomes = carried_intents.commit_if_settled(app, false)?;
            *views = app.egui_views.clone();
            return Ok((outcomes, ak_update, immediate_repaint));
        }
    }
    Ok((Vec::new(), None, false))
}

/// Clear a stale non-grid textbox `app.focus_widget_id` after `egui` keyboard focus drops.
pub fn clear_stale_non_grid_textbox_focus(app: &mut UiApp, meta: &FrameInspectMeta, ever_wanted_kb: bool, now_wants_kb: bool) {
    if ever_wanted_kb
        && !now_wants_kb
        && let Some(fid) = app.focus_widget_id.as_deref()
        && !fid.starts_with("grid.")
        && meta.find_by_widget_id(fid).is_none_or(|e| matches!(e.role.as_str(), "textbox" | "input" | "textinput" | "searchbox" | "text"))
    {
        app.focus_widget_id = None;
    }
}

fn step_headless_batches(
    app: &mut UiApp,
    metrics: ViewportMetrics,
    batches: Vec<Vec<egui::Event>>,
) -> Result<(FrameInspectMeta, Option<egui::accesskit::TreeUpdate>, Vec<UiIntentOutcome>), EngineError> {
    app.sync_views();
    let (ctx, w_pt, h_pt, native_ppp, mut time_sec) = acquire_headless_egui_ctx(app, metrics);
    let geom = (w_pt, h_pt, native_ppp);
    let mut views = app.egui_views.clone();
    let mut meta = FrameInspectMeta::default();
    let mut all_outcomes = Vec::new();
    let mut last_ak = None;

    let mut ever_wanted_kb = ctx.egui_wants_keyboard_input();

    if time_sec == 0.0 {
        time_sec = time_sec.max(ctx.input(|i| i.time)) + 1.0 / 60.0;
        let _ = step_egui_shell_with_events_and_ak_repaint(&ctx, app, &mut views, &mut meta, geom, time_sec, Vec::new())?;
        ever_wanted_kb |= ctx.egui_wants_keyboard_input();
    }

    for events in batches {
        time_sec = time_sec.max(ctx.input(|i| i.time)) + 1.0 / 60.0;
        let (outcomes, ak, _) = step_egui_shell_with_events_and_ak_repaint(&ctx, app, &mut views, &mut meta, geom, time_sec, events)?;
        all_outcomes.extend(outcomes);
        last_ak = ak;
        ever_wanted_kb |= ctx.egui_wants_keyboard_input();
    }

    // Settle layout / modal auto_shrink / immediate repaint passes cleanly before returning `meta`.
    for _ in 0..8_u32 {
        time_sec = time_sec.max(ctx.input(|i| i.time)) + 1.0 / 60.0;
        let (tail_outcomes, tail_ak, immediate_repaint) =
            step_egui_shell_with_events_and_ak_repaint(&ctx, app, &mut views, &mut meta, geom, time_sec, Vec::new())?;
        all_outcomes.extend(tail_outcomes);
        if tail_ak.is_some() {
            last_ak = tail_ak;
        }
        ever_wanted_kb |= ctx.egui_wants_keyboard_input();
        if !immediate_repaint {
            break;
        }
    }
    clear_stale_non_grid_textbox_focus(app, &meta, ever_wanted_kb, ctx.egui_wants_keyboard_input());
    app.egui_views = views;
    let final_t = time_sec.max(ctx.input(|i| i.time));
    store_headless_egui_time(app, final_t);
    Ok((meta, last_ak, all_outcomes))
}

/// Drive a real `egui` pointer drag (`start_xy -> end_xy`) across settled shell frames,
/// returning a layout drag outcome when a Page Layout item move/resize commits `layout.setSpec`.
///
/// # Errors
/// Returns [`EngineError`] if any committed engine command fails.
pub fn dispatch_egui_pointer_drag(
    app: &mut UiApp,
    metrics: ViewportMetrics,
    start_xy: [f64; 2],
    end_xy: [f64; 2],
    modifiers: KeyModifiers,
) -> Result<Option<UiEventOutcome>, EngineError> {
    let (ctx, w_pt, h_pt, native_ppp) = configure_offscreen_egui_ctx(app, metrics);
    let geom = (w_pt, h_pt, native_ppp);
    let mut views = app.egui_views.clone();
    let mut meta = FrameInspectMeta::default();
    let egui_mods = to_egui_modifiers(modifiers);

    #[allow(clippy::cast_possible_truncation)]
    let start = Pos2::new(start_xy[0] as f32, start_xy[1] as f32);
    #[allow(clippy::cast_possible_truncation)]
    let end = Pos2::new(end_xy[0] as f32, end_xy[1] as f32);
    let delta = end - start;
    let dist = delta.length();

    let mut t = 0.0_f64;
    let _ = step_egui_shell_with_events(&ctx, app, &mut views, &mut meta, geom, t, Vec::new())?;
    t += 0.02;
    let _ = step_egui_shell_with_events(&ctx, app, &mut views, &mut meta, geom, t, vec![egui::Event::PointerMoved(start)])?;
    t += 0.02;
    let _ = step_egui_shell_with_events(
        &ctx,
        app,
        &mut views,
        &mut meta,
        geom,
        t,
        vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui_mods }],
    )?;
    if dist > 6.1 {
        t += 0.02;
        let step = (dist - 0.1).min(6.2);
        let trigger = start + delta.normalized() * step;
        let _ = step_egui_shell_with_events(&ctx, app, &mut views, &mut meta, geom, t, vec![egui::Event::PointerMoved(trigger)])?;
    }
    t += 0.02;
    let _ = step_egui_shell_with_events(&ctx, app, &mut views, &mut meta, geom, t, vec![egui::Event::PointerMoved(end)])?;
    t += 0.02;
    let outcomes = step_egui_shell_with_events(
        &ctx,
        app,
        &mut views,
        &mut meta,
        geom,
        t,
        vec![egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui_mods }],
    )?;

    for item in outcomes {
        if let UiIntentOutcome::Engine { command_id, outcome } = item
            && command_id == "layout.setSpec"
        {
            app.interaction.last_logical_pos = Some(end_xy);
            return Ok(Some(UiEventOutcome {
                handled: true,
                action: String::from("layout.item_drag_committed"),
                command_id: Some(command_id),
                command_result: serde_json::to_value(outcome).ok(),
            }));
        }
    }
    Ok(None)
}

/// Run a settled offscreen `egui` frame (reusing cached font definitions and persistent headless
/// context when initialized) and construct a canonical [`UiTreeSnapshot`] (`ui.inspect`).
#[must_use]
pub fn inspect_live_egui_snapshot(app: &UiApp, metrics: ViewportMetrics, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    let existing_ctx = app.headless_egui.inner.lock().ok().and_then(|g| g.as_ref().map(|(c, t)| (c.clone(), *t)));
    let (ctx, w_pt, h_pt, native_ppp, time_sec) = match existing_ctx {
        Some((ctx, t)) => {
            configure_ctx_for_app(&ctx, app);
            let (w_pt, h_pt, native_ppp) = viewport_geom_for_app(app, metrics);
            (ctx, w_pt, h_pt, native_ppp, t)
        }
        None => {
            let (ctx, w_pt, h_pt, native_ppp) = configure_offscreen_egui_ctx(app, metrics);
            (ctx, w_pt, h_pt, native_ppp, 0.0)
        }
    };

    let mut views = app.egui_views.clone();
    let mut meta = FrameInspectMeta::default();
    let mut last_ak = None;
    for _pass in 0..8_u32 {
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(w_pt, h_pt))),
            time: Some(time_sec),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(native_ppp), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut intents = PendingUiIntents::default();
        let mut settled = false;
        let mut output = ctx.run_ui(raw_input, |ui| {
            let _ = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
            settled = !ui.ctx().will_discard();
        });
        output.textures_delta.clear();
        last_ak = output.platform_output.accesskit_update.take();
        if settled {
            break;
        }
    }
    build_ui_tree_snapshot_from_egui_with_views(app, &views, &meta, last_ak.as_ref(), scope, include_bounds, metrics)
}

fn navigator_section_slug(section: curvebook_doc::SheetSection) -> &'static str {
    match section {
        curvebook_doc::SheetSection::Data => "data",
        curvebook_doc::SheetSection::Info => "info",
        curvebook_doc::SheetSection::Analyses => "results",
        curvebook_doc::SheetSection::Graphs => "graphs",
        curvebook_doc::SheetSection::Layouts => "layouts",
    }
}

fn build_menus_snapshot(app: &UiApp, meta: &FrameInspectMeta, include_bounds: bool) -> Vec<UiMenuSnapshot> {
    menu_bar_tooltips(app)
        .into_iter()
        .map(|sec| {
            let bounds = if include_bounds {
                meta.find_by_widget_id(sec.menu_key).and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor))
            } else {
                None
            };
            let items = sec
                .items
                .into_iter()
                .map(|item| {
                    let widget_id = format!("menu.{}", item.command_id);
                    let item_bounds = if include_bounds {
                        meta.find_by_widget_id(&widget_id).and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor))
                    } else {
                        None
                    };
                    UiWidgetNode {
                        widget_id,
                        role: String::from("menuitem"),
                        label: item.localized_label.to_owned(),
                        command_id: Some(item.command_id.to_owned()),
                        value: None,
                        enabled: item.enabled,
                        disabled_reason: (!item.enabled).then_some(String::from("unavailable")),
                        bounds: item_bounds,
                    }
                })
                .collect();
            UiMenuSnapshot { menu_key: sec.menu_key.to_owned(), title: sec.title.to_owned(), bounds, items }
        })
        .collect()
}

fn build_navigator_snapshot(app: &UiApp, meta: &FrameInspectMeta, include_bounds: bool) -> Vec<UiNavigatorSheetNode> {
    let mut out = Vec::new();
    for folder in &app.nav.folders {
        let sec_slug = navigator_section_slug(folder.section);
        for entry in &folder.entries {
            let wid = format!("nav.sheet.{}", entry.sheet_id.as_str());
            let bounds = if include_bounds {
                meta.find_by_widget_id(&wid).and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor))
            } else {
                None
            };
            out.push(UiNavigatorSheetNode {
                widget_id: wid,
                sheet_id: entry.sheet_id.as_str().to_owned(),
                section: sec_slug.to_owned(),
                title: entry.title.clone(),
                selected: entry.is_active,
                role: String::from("treeitem"),
                label: entry.title.clone(),
                enabled: true,
                bounds,
            });
        }
    }
    out
}

fn build_canvas_snapshot(
    app: &UiApp,
    cv: &crate::egui_shell::canvas_view::CanvasViewEguiState,
    meta: &FrameInspectMeta,
    mut nodes: Vec<UiWidgetNode>,
    include_bounds: bool,
) -> Option<Vec<UiWidgetNode>> {
    let doc = app.document();
    if let Some(active_id) = doc.active_sheet_id.as_ref()
        && let Some(gs) = doc.graph_sheet(active_id)
        && let Ok(scene) = app.session().render_sheet_scene(&gs.id)
    {
        let paper_and_scene =
            cv.last_active_paper_sheet_id.as_ref().filter(|id| *id == &gs.id).and_then(|_| cv.last_paper_rect_pt.zip(cv.last_scene_size_pt));
        for pt in scene.plotted_points.iter().take(500) {
            let wid = if pt.source_cell.subcol > 0 {
                format!("canvas.point.{}.{}.{}", pt.source_cell.col, pt.source_cell.row, pt.source_cell.subcol)
            } else {
                format!("canvas.point.{}.{}", pt.source_cell.col, pt.source_cell.row)
            };
            if nodes.iter().any(|n| n.widget_id == wid) {
                continue;
            }
            let bounds = if include_bounds
                && let Some((paper_rect, [sw, sh])) = paper_and_scene
                && sw > 0.0
                && sh > 0.0
            {
                let sx = f64::from(paper_rect.left()) + (pt.center_pt.x / sw) * f64::from(paper_rect.width());
                let sy = f64::from(paper_rect.top()) + (pt.center_pt.y / sh) * f64::from(paper_rect.height());
                let r_pt = (pt.radius_pt / sw * f64::from(paper_rect.width())).max(4.0);
                #[allow(clippy::cast_possible_truncation)]
                let rect = Rect::from_center_size(Pos2::new(sx as f32, sy as f32), Vec2::splat((r_pt * 2.0) as f32));
                clipped_rect_to_bounds_px(rect, meta.scale_factor)
            } else {
                None
            };
            nodes.push(UiWidgetNode {
                widget_id: wid,
                role: String::from("point"),
                label: pt.series_label.clone(),
                command_id: Some(String::from("ui.brush.point")),
                value: Some(json!({
                    "sheet_id": pt.source_cell.sheet_id.as_str(),
                    "row": pt.source_cell.row,
                    "col": pt.source_cell.col,
                    "subcol": pt.source_cell.subcol,
                    "summary": pt.value_summary,
                })),
                enabled: true,
                disabled_reason: None,
                bounds,
            });
        }
    }
    if nodes.is_empty() { None } else { Some(nodes) }
}

fn build_grid_selection_snapshot(app: &UiApp) -> GridSelectionSnapshot {
    let doc = app.document();
    let active_ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
    let matches_active = active_ds.is_some_and(|ds| app.grid.selection_belongs_to(&ds.id));
    let selected_coords: Vec<GridCoordSnapshot> = active_ds
        .map(|ds| app.grid.all_selected_coords(ds).into_iter().take(MAX_INSPECT_SELECTED_COORDS).map(GridCoordSnapshot::from).collect())
        .unwrap_or_default();
    let selected_cells_count = if !matches_active {
        0
    } else if !app.grid.selected_coords.is_empty() {
        app.grid.selected_coords.len()
    } else if let (Some(ds), Some(a), Some(b)) = (active_ds, app.grid.tool.anchor, app.grid.tool.active) {
        selection_cell_count(ds, a, b)
    } else {
        0
    };
    GridSelectionSnapshot {
        active_cell: matches_active.then_some(app.grid.tool.active).flatten().map(GridCoordSnapshot::from),
        anchor_cell: matches_active.then_some(app.grid.tool.anchor).flatten().map(GridCoordSnapshot::from),
        selected_rows: if matches_active { app.grid.selected_rows.iter().copied().take(MAX_INSPECT_SELECTED_COORDS).collect() } else { Vec::new() },
        selected_coords,
        selected_cells_count,
        quick_stats: matches_active.then_some(app.grid.quick_stats).flatten(),
    }
}

/// Build a [`UiTreeSnapshot`] (`ui.inspect`) directly from settled `egui` AccessKit output and
/// [`FrameInspectMeta`].
#[must_use]
pub fn build_ui_tree_snapshot_from_egui(
    app: &UiApp,
    meta: &FrameInspectMeta,
    accesskit_update: Option<&egui::accesskit::TreeUpdate>,
    scope: Option<&str>,
    include_bounds: bool,
    metrics: ViewportMetrics,
) -> UiTreeSnapshot {
    build_ui_tree_snapshot_from_egui_with_views(app, &app.egui_views, meta, accesskit_update, scope, include_bounds, metrics)
}

/// Input bundle for building a [`UiTreeSnapshot`] from a live settled `egui` window frame.
pub struct LiveInspectSnapshotInput<'a> {
    pub app: &'a UiApp,
    pub views: &'a EguiShellViewState,
    pub meta: &'a FrameInspectMeta,
    pub focused_egui_id: Option<egui::Id>,
    pub host_close_dialog_open: bool,
    pub scope: Option<&'a str>,
    pub include_bounds: bool,
    pub metrics: ViewportMetrics,
}

/// Build a [`UiTreeSnapshot`] (`ui.inspect`) directly from a live settled window's
/// [`FrameInspectMeta`], [`EguiShellViewState`], and active `egui::Context` focus.
#[must_use]
pub fn build_live_ui_tree_snapshot(input: LiveInspectSnapshotInput<'_>) -> UiTreeSnapshot {
    build_ui_tree_snapshot_inner(
        input.app,
        input.views,
        input.meta,
        None,
        input.focused_egui_id,
        input.host_close_dialog_open,
        input.scope,
        input.include_bounds,
    )
}

/// Build a [`UiTreeSnapshot`] (`ui.inspect`) using the freshly rendered [`EguiShellViewState`]
/// alongside [`FrameInspectMeta`] and AccessKit output.
#[must_use]
pub fn build_ui_tree_snapshot_from_egui_with_views(
    app: &UiApp,
    views: &EguiShellViewState,
    meta: &FrameInspectMeta,
    accesskit_update: Option<&egui::accesskit::TreeUpdate>,
    scope: Option<&str>,
    include_bounds: bool,
    _metrics: ViewportMetrics,
) -> UiTreeSnapshot {
    build_ui_tree_snapshot_inner(app, views, meta, accesskit_update, None, false, scope, include_bounds)
}

#[allow(clippy::too_many_arguments)]
fn build_ui_tree_snapshot_inner(
    app: &UiApp,
    views: &EguiShellViewState,
    meta: &FrameInspectMeta,
    accesskit_update: Option<&egui::accesskit::TreeUpdate>,
    live_focused_egui_id: Option<egui::Id>,
    host_close_dialog_open: bool,
    scope: Option<&str>,
    include_bounds: bool,
) -> UiTreeSnapshot {
    let doc = app.document();
    let ak = AccessKitLookup::from_update(accesskit_update);

    let mut focus_widget_id = app.focus_widget_id.clone();
    if let Some(focused_ak_id) = ak.focus
        && let Some(entry) = meta.find_by_accesskit_id(focused_ak_id)
    {
        focus_widget_id = Some(entry.widget_id.clone());
    } else if let Some(focused_egui_id) = live_focused_egui_id
        && let Some(entry) = meta.find_by_egui_id(focused_egui_id)
    {
        focus_widget_id = Some(entry.widget_id.clone());
    }

    let collect_scope = |tag: &str| -> Vec<UiWidgetNode> {
        meta.entries
            .iter()
            .filter(|e| entry_belongs_to_scope(e, tag))
            .map(|e| widget_node_from_meta(e, &ak, meta.scale_factor, include_bounds))
            .collect()
    };

    let toolbar = scope_wants(scope, "toolbar").then(|| collect_scope("toolbar"));

    let dialog = build_dialog_snapshot_from_meta(app, views, meta, &ak, include_bounds, host_close_dialog_open);
    let active_dialog = dialog.as_ref().map_or_else(|| String::from("none"), |d| d.dialog_kind.clone());

    let tabs = if scope_wants(scope, "tabs") {
        let mut combined = collect_scope("tabs");
        if let Some(rs) = doc.active_sheet_id.as_ref().and_then(|id| doc.result_sheet(id)) {
            let active_idx = if rs.tabs.is_empty() { 0 } else { app.canvas_dialogs.result_view.active_tab_index.min(rs.tabs.len() - 1) };
            for (idx, tab) in rs.tabs.iter().enumerate() {
                let wid = format!("result.tab.{}", tab.id);
                if !combined.iter().any(|n| n.widget_id == wid) {
                    let bounds = if include_bounds {
                        meta.find_by_widget_id(&wid).and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor))
                    } else {
                        None
                    };
                    combined.push(UiWidgetNode {
                        widget_id: wid,
                        role: String::from("tab"),
                        label: tab.title.clone(),
                        command_id: None,
                        value: Some(json!(idx == active_idx)),
                        enabled: true,
                        disabled_reason: None,
                        bounds,
                    });
                }
            }
        }
        if let Some(ref dlg) = dialog {
            for dt in &dlg.tabs {
                if !combined.iter().any(|n| n.widget_id == dt.widget_id) {
                    combined.push(dt.clone());
                }
            }
        }
        if combined.is_empty() { None } else { Some(combined) }
    } else {
        None
    };

    let dialog_out = if scope_wants(scope, "dialog") { dialog } else { None };

    let grid_cells = scope_wants(scope, "grid").then(|| build_egui_grid_cells_snapshot(app, meta, include_bounds, collect_scope("grid"))).flatten();

    let menus: Option<Vec<UiMenuSnapshot>> = scope_wants(scope, "menus").then(|| build_menus_snapshot(app, meta, include_bounds));
    let navigator_sheets: Option<Vec<UiNavigatorSheetNode>> =
        scope_wants(scope, "navigator").then(|| build_navigator_snapshot(app, meta, include_bounds));

    let canvas_elements = if scope_wants(scope, "canvas") {
        build_canvas_snapshot(app, &views.canvas_view, meta, collect_scope("canvas"), include_bounds)
    } else {
        None
    };

    let active_sheet_id = doc.active_sheet_id.as_ref().map(|id| id.as_str().to_owned());
    let active_sheet_kind = doc.active_sheet_id.as_ref().and_then(|id| {
        if doc.data_sheet(id).is_some() {
            Some(String::from("data"))
        } else if doc.info_sheet(id).is_some() {
            Some(String::from("info"))
        } else if doc.result_sheet(id).is_some() {
            Some(String::from("result"))
        } else if doc.graph_sheet(id).is_some() {
            Some(String::from("graph"))
        } else if doc.layout_sheet(id).is_some() {
            Some(String::from("layout"))
        } else {
            None
        }
    });

    let egui_or_ak_focused =
        ak.focus.and_then(|id| meta.find_by_accesskit_id(id)).is_some() || live_focused_egui_id.and_then(|id| meta.find_by_egui_id(id)).is_some();
    if !egui_or_ak_focused
        && focus_widget_id.as_deref().is_some_and(|wid| wid.starts_with("grid."))
        && (active_dialog != "none" || active_sheet_kind.as_deref() != Some("data"))
    {
        focus_widget_id = None;
    }

    let cursor_readout = if matches!(active_sheet_kind.as_deref(), Some("graph" | "layout")) { app.interaction.last_logical_pos } else { None };

    let brushed_points = app
        .brushed_points
        .iter()
        .map(|p| BrushedPointSnapshot { sheet_id: p.sheet_id.as_str().to_owned(), row: p.row, col: p.col, subcol: p.subcol })
        .collect();

    let result_diagnostic_banner = doc.active_sheet_id.as_ref().and_then(|id| doc.result_sheet(id)).and_then(AssumptionDiagnosticBanner::from_result);
    let import_warnings_open = app.import_warnings.panel_open || app.canvas_dialogs.active_dialog == ActiveDialog::ImportWarnings;
    let import_warnings = if !app.import_warnings.warnings.is_empty() || import_warnings_open {
        Some(crate::inspect::ImportWarningsSnapshot {
            open: import_warnings_open,
            command_id: app.import_warnings.command_id.clone(),
            source_label: app.import_warnings.source_label.clone(),
            warnings: app.import_warnings.warnings.clone(),
        })
    } else {
        None
    };

    UiTreeSnapshot {
        active_sheet_id,
        active_sheet_kind,
        active_dialog,
        focus_widget_id,
        zoom: app.canvas_dialogs.effective_zoom_and_pan().0,
        undo_depth: app.session().undo_depth(),
        redo_depth: app.session().redo_depth(),
        dirty: app.session().is_dirty(),
        cursor_readout,
        grid_selection: if scope_wants(scope, "grid") {
            build_grid_selection_snapshot(app)
        } else {
            GridSelectionSnapshot {
                active_cell: None,
                anchor_cell: None,
                selected_rows: Vec::new(),
                selected_coords: Vec::new(),
                selected_cells_count: 0,
                quick_stats: None,
            }
        },
        brushed_points,
        status_banner: app.canvas_dialogs.status_banner.clone(),
        context_menu_target: app.interaction.context_menu_target.clone(),
        result_diagnostic_banner,
        import_warnings,
        menus,
        toolbar,
        navigator_sheets,
        tabs,
        canvas_elements,
        grid_cells,
        dialog: dialog_out,
    }
}

fn build_egui_grid_cells_snapshot(
    app: &UiApp,
    meta: &FrameInspectMeta,
    include_bounds: bool,
    mut egui_grid: Vec<UiWidgetNode>,
) -> Option<Vec<UiWidgetNode>> {
    let ds = app.document().active_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id))?;

    // Alias `grid.xcell.{r}.{s}` -> `grid.x.{r}.{s}` when absent so legacy callers resolve bounds.
    let mut x_aliases = Vec::new();
    for node in &egui_grid {
        if let Some(rest) = node.widget_id.strip_prefix("grid.xcell.") {
            let alias_id = format!("grid.x.{rest}");
            if !egui_grid.iter().any(|n| n.widget_id == alias_id) {
                let mut alias = node.clone();
                alias.widget_id = alias_id;
                x_aliases.push(alias);
            }
        }
    }
    egui_grid.extend(x_aliases);

    if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
        for (col_idx, var) in mv.variables.iter().enumerate() {
            let wid = format!("formula_bar.open_col.{col_idx}");
            if !egui_grid.iter().any(|n| n.widget_id == wid) {
                let badge_id = format!("mv_formula.role_badge.{col_idx}");
                let bounds = if include_bounds {
                    meta.find_by_widget_id(&badge_id).and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor))
                } else {
                    None
                };
                egui_grid.push(UiWidgetNode {
                    widget_id: wid,
                    role: String::from("button"),
                    label: format!("Formula Bar ({})", var.title),
                    command_id: Some(String::from("ui.table.openFormulaBar")),
                    value: None,
                    enabled: !ds.frozen,
                    disabled_reason: ds.frozen.then_some(String::from("sheet is frozen")),
                    bounds,
                });
            }
        }
    }

    if app.grid.formula_bar.active || matches!(ds.archetype, TableArchetype::MultipleVariables(_)) {
        let active = app.grid.formula_bar.active;
        let fb_nodes = [
            (
                "formula_bar.input",
                "textbox",
                "Formula Bar Input",
                Some(json!(app.grid.formula_bar.input)),
                !ds.frozen && active,
                if ds.frozen {
                    Some(String::from("sheet is frozen"))
                } else if !active {
                    Some(String::from("Formula bar is inactive"))
                } else {
                    None
                },
            ),
            (
                "formula_bar.commit",
                "button",
                "Apply Formula",
                None,
                !ds.frozen && active && app.grid.formula_bar.is_valid,
                if ds.frozen {
                    Some(String::from("sheet is frozen"))
                } else if !active {
                    Some(String::from("Formula bar is inactive"))
                } else if !app.grid.formula_bar.is_valid {
                    app.grid.formula_bar.parse_error.clone().or_else(|| Some(String::from("Formula is invalid")))
                } else {
                    None
                },
            ),
            (
                "formula_bar.cancel",
                "button",
                "Cancel Formula",
                None,
                !ds.frozen && active,
                if ds.frozen {
                    Some(String::from("sheet is frozen"))
                } else if !active {
                    Some(String::from("Formula bar is inactive"))
                } else {
                    None
                },
            ),
        ];
        for (wid, role, label, val, enabled, disabled_reason) in fb_nodes {
            if let Some(existing) = egui_grid.iter_mut().find(|n| n.widget_id == wid) {
                existing.enabled = enabled;
                existing.disabled_reason = disabled_reason;
            } else {
                let bounds = if include_bounds {
                    meta.find_by_widget_id(wid).and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor))
                } else {
                    None
                };
                egui_grid.push(UiWidgetNode {
                    widget_id: String::from(wid),
                    role: String::from(role),
                    label: String::from(label),
                    command_id: None,
                    value: val,
                    enabled,
                    disabled_reason,
                    bounds,
                });
            }
        }
    }

    Some(egui_grid)
}

fn bad_input_err(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, msg)
}

fn parse_viewport_metrics_from_params(params: &Value) -> Result<ViewportMetrics, EngineError> {
    let vp_obj = params.get("viewport").or_else(|| params.get("metrics")).unwrap_or(params);
    let w = opt_finite_f64_param(vp_obj, &["logical_width", "width", "width_pt"]).map_err(bad_input_err)?.unwrap_or(1024.0);
    let h = opt_finite_f64_param(vp_obj, &["logical_height", "height", "height_pt"]).map_err(bad_input_err)?.unwrap_or(640.0);
    if !(128.0..=16384.0).contains(&w) || !(128.0..=16384.0).contains(&h) {
        return Err(bad_input_err(format!("viewport dimensions ({w}x{h}) out of allowed bounds [128, 16384]")));
    }
    let scale = opt_finite_f64_param(vp_obj, &["scale_factor", "scale"])
        .or_else(|_| opt_finite_f64_param(params, &["scale_factor", "scale"]))
        .map_err(bad_input_err)?
        .unwrap_or(1.0);
    Ok(ViewportMetrics::new(w, h, scale))
}

fn parse_modifiers_from_params(params: &Value) -> Result<KeyModifiers, EngineError> {
    let src = params.get("modifiers").unwrap_or(params);
    Ok(KeyModifiers {
        shift: opt_bool_param(src, &["shift"]).map_err(bad_input_err)?.unwrap_or(false),
        ctrl: opt_bool_param(src, &["ctrl"]).map_err(bad_input_err)?.unwrap_or(false),
        alt: opt_bool_param(src, &["alt"]).map_err(bad_input_err)?.unwrap_or(false),
        meta: opt_bool_param(src, &["meta", "cmd", "command"]).map_err(bad_input_err)?.unwrap_or(false),
    })
}

/// Resolve a canonical or legacy/aliased `widget_id` inside [`FrameInspectMeta`].
#[must_use]
pub fn resolve_widget_rect_in_meta<'a>(meta: &'a FrameInspectMeta, wid: &str) -> Option<&'a FrameInspectNodeMeta> {
    if let Some(hit) = meta.find_by_widget_id(wid) {
        return Some(hit);
    }
    if let Some(rest) = wid.strip_prefix("grid.x.") {
        return meta.find_by_widget_id(&format!("grid.xcell.{rest}"));
    }
    if let Some(rest) = wid.strip_prefix("dialog.button.") {
        if let Some(hit) = meta.find_by_widget_id(&format!("modal.{rest}")) {
            return Some(hit);
        }
        if let Some((_, aliases)) = BUTTON_ACTION_ALIASES.iter().find(|(action, _)| *action == rest)
            && let Some(hit) = aliases.iter().find_map(|alias| meta.find_by_widget_id(alias))
        {
            return Some(hit);
        }
        if rest == "create" {
            return meta.find_by_widget_id("modal.ok");
        }
    }
    if let Some(col_str) = wid.strip_prefix("formula_bar.open_col.") {
        return meta.find_by_widget_id(&format!("mv_formula.role_badge.{col_str}"));
    }
    if wid == "custom_title" {
        return meta.find_by_widget_id("edit.custom_title");
    }
    None
}

/// Resolve the center point (logical `pt`) and enabled state of `wid` inside [`FrameInspectMeta`].
///
/// # Errors
/// Returns [`EngineError`] with `ErrorCode::NotFound` if `wid` is absent.
pub fn resolve_widget_center_in_meta(
    app: &UiApp,
    meta: &FrameInspectMeta,
    metrics: ViewportMetrics,
    wid: &str,
) -> Result<(Pos2, bool, Option<String>), EngineError> {
    resolve_widget_center_pt(app, meta, metrics, wid)
}

fn resolve_widget_center_pt(
    app: &UiApp,
    meta: &FrameInspectMeta,
    metrics: ViewportMetrics,
    wid: &str,
) -> Result<(Pos2, bool, Option<String>), EngineError> {
    if let Some(entry) = resolve_widget_rect_in_meta(meta, wid) {
        return Ok((entry.clipped_rect.center(), entry.enabled, entry.disabled_reason.clone()));
    }
    if wid.starts_with("canvas.point.") {
        let snap = build_ui_tree_snapshot_from_egui(app, meta, None, Some("canvas"), true, metrics);
        if let Some(node) = snap.find_widget(wid)
            && let Some(bounds) = node.bounds
        {
            let scale = meta.scale_factor.max(0.05);
            #[allow(clippy::cast_possible_truncation)]
            let cx = ((bounds[0] + bounds[2] * 0.5) as f32) / scale;
            #[allow(clippy::cast_possible_truncation)]
            let cy = ((bounds[1] + bounds[3] * 0.5) as f32) / scale;
            return Ok((Pos2::new(cx, cy), node.enabled, node.disabled_reason));
        }
    }
    Err(EngineError::with_code(ErrorCode::NotFound, format!("widget `{wid}` not found in current UI tree")))
}

/// Build synthetic `egui` pointer click event batches at `pos`.
#[must_use]
pub fn click_pos_batches(pos: Pos2, button: egui::PointerButton, mods: Modifiers) -> Vec<Vec<egui::Event>> {
    let mut batches = vec![
        vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerMoved(pos)],
        vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerButton { pos, button, pressed: true, modifiers: mods }],
        vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerButton { pos, button, pressed: false, modifiers: mods }],
    ];
    if !mods.is_none() {
        batches.push(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    }
    batches
}

fn drag_pos_batches(start: Pos2, end: Pos2, button: egui::PointerButton, mods: Modifiers) -> Vec<Vec<egui::Event>> {
    let delta = end - start;
    let dist = delta.length();
    let mut batches = vec![
        vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerMoved(start)],
        vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerButton { pos: start, button, pressed: true, modifiers: mods }],
    ];
    if dist > 6.1 {
        let step = (dist - 0.1).min(6.2);
        let trigger = start + delta.normalized() * step;
        batches.push(vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerMoved(trigger)]);
    }
    batches.push(vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerMoved(end)]);
    batches.push(vec![egui::Event::ModifiersChanged(mods), egui::Event::PointerButton { pos: end, button, pressed: false, modifiers: mods }]);
    if !mods.is_none() {
        batches.push(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    }
    batches
}

/// If `app.focus_widget_id` points to an enabled non-grid textbox while `egui` lacks keyboard focus,
/// return its center coordinates so a live host can click-focus it.
#[must_use]
pub fn find_non_grid_textbox_focus_center(app: &UiApp, meta: &FrameInspectMeta, metrics: ViewportMetrics, egui_wants_kb: bool) -> Option<Pos2> {
    if egui_wants_kb {
        return None;
    }
    let fid = app.focus_widget_id.as_deref()?;
    if fid.starts_with("grid.") {
        return None;
    }
    if !meta.find_by_widget_id(fid).is_some_and(|e| matches!(e.role.as_str(), "textbox" | "input" | "textinput" | "searchbox" | "text")) {
        return None;
    }
    resolve_widget_center_pt(app, meta, metrics, fid).ok().and_then(|(pos, enabled, _)| enabled.then_some(pos))
}

fn ensure_egui_focus_if_needed(
    app: &mut UiApp,
    metrics: ViewportMetrics,
) -> Result<(FrameInspectMeta, Option<egui::accesskit::TreeUpdate>), EngineError> {
    let (meta, ak, _) = step_headless_batches(app, metrics, Vec::new())?;
    let wants_kb = app.headless_egui.inner.lock().ok().and_then(|g| g.as_ref().map(|(c, _)| c.egui_wants_keyboard_input())).unwrap_or(false);
    if let Some(pos) = find_non_grid_textbox_focus_center(app, &meta, metrics, wants_kb) {
        let (m2, ak2, _) = step_headless_batches(app, metrics, click_pos_batches(pos, egui::PointerButton::Primary, Modifiers::NONE))?;
        return Ok((m2, ak2));
    }
    Ok((meta, ak))
}

/// Dispatch any of the 7 interactive GUI actions (`ui.inspect`, `ui.screenshot`, `ui.invoke_menu`,
/// `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.switch_tab`) through settled
/// `egui::Context::run` passes (`F8`).
///
/// # Errors
/// Returns [`EngineError`] on bad parameters, missing/disabled widgets, invalid actions, or
/// policy/path violations.
pub(crate) fn dispatch_gui_action(app: &mut UiApp, method: &str, params: Value) -> Result<Value, EngineError> {
    require_params_object_or_null(&params, method).map_err(bad_input_err)?;
    match method {
        "ui.inspect" => dispatch_gui_inspect(app, &params),
        "ui.screenshot" => dispatch_gui_screenshot(app, &params),
        "ui.invoke_menu" => dispatch_gui_invoke_menu(app, &params),
        "ui.pointer_gesture" => dispatch_gui_pointer_gesture(app, &params),
        "ui.type_text" => dispatch_gui_type_text(app, &params),
        "ui.press_key" => dispatch_gui_press_key(app, &params),
        "ui.switch_tab" => dispatch_gui_switch_tab(app, &params),
        "ui.answer_dialog" if app.is_human_path_only() => Err(human_path_only_err("direct `ui.answer_dialog` is blocked in human-path-only mode")),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported GUI action `{method}`"))),
    }
}

/// Parsed and validated `ui.inspect` parameters.
pub struct ParsedInspectParams<'a> {
    pub scope: Option<&'a str>,
    pub include_bounds: bool,
    pub metrics: ViewportMetrics,
}

/// Parse and validate `ui.inspect` request parameters.
///
/// # Errors
/// Returns [`EngineError`] with `ErrorCode::BadInput` on malformed parameters.
pub fn parse_inspect_params(params: &Value) -> Result<ParsedInspectParams<'_>, EngineError> {
    require_params_object_or_null(params, "ui.inspect").map_err(bad_input_err)?;
    let scope = opt_non_empty_str_param(params, &["scope"]).map_err(bad_input_err)?;
    let include_bounds = opt_bool_param(params, &["include_bounds"]).map_err(bad_input_err)?.unwrap_or(true);
    let metrics = parse_viewport_metrics_from_params(params)?;
    Ok(ParsedInspectParams { scope, include_bounds, metrics })
}

fn dispatch_gui_inspect(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let parsed = parse_inspect_params(params)?;
    let snap = inspect_live_egui_snapshot(app, parsed.metrics, parsed.scope, parsed.include_bounds);
    serde_json::to_value(snap).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))
}

fn normalize_menu_token(raw: &str) -> String {
    raw.trim().trim_end_matches("...").trim_end_matches('…').trim().to_ascii_lowercase()
}

fn normalize_menu_path_str(raw: &str) -> String {
    raw.split(['>', '/']).map(normalize_menu_token).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

fn strip_ci_prefix<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    let mut h_chars = haystack.chars();
    for p_ch in prefix.chars() {
        let h_ch = h_chars.next()?;
        if !h_ch.eq_ignore_ascii_case(&p_ch) && h_ch.to_lowercase().ne(p_ch.to_lowercase()) {
            return None;
        }
    }
    Some(h_chars.as_str())
}

fn extract_menu_path_param(params: &Value) -> Result<Option<(String, Vec<String>)>, EngineError> {
    let Some(val) = params.get("path").or_else(|| params.get("menu_path")) else {
        return Ok(None);
    };
    match val {
        Value::Null => Ok(None),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(bad_input_err("`path` must be a non-empty string"));
            }
            Ok(Some((trimmed.to_owned(), Vec::new())))
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                return Err(bad_input_err("`path` array must not be empty"));
            }
            let mut segs = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad_input_err("`path` must be a string"))?;
                segs.push(s.to_owned());
            }
            Ok(Some((segs.join("/"), segs)))
        }
        _ => Err(bad_input_err("`path` must be a string")),
    }
}

fn match_curated_nodes_recursive(
    nodes: &[CuratedMenuNode],
    rem_str: Option<&str>,
    rem_segs: Option<&[String]>,
    norm_full: &str,
) -> Option<&'static str> {
    for node in nodes {
        match node {
            CuratedMenuNode::Separator => {}
            CuratedMenuNode::Action(tip) => {
                if let Some(segs) = rem_segs
                    && segs.len() == 1
                {
                    let seg_norm = normalize_menu_token(&segs[0]);
                    if seg_norm == normalize_menu_token(tip.localized_label) || seg_norm == normalize_menu_token(tip.label) {
                        return Some(tip.command_id);
                    }
                }
                if let Some(rem) = rem_str {
                    let rem_norm = normalize_menu_token(rem);
                    if rem_norm == normalize_menu_token(tip.localized_label) || rem_norm == normalize_menu_token(tip.label) {
                        return Some(tip.command_id);
                    }
                }
                if !tip.menu_path.is_empty() && norm_full == normalize_menu_path_str(tip.menu_path) {
                    return Some(tip.command_id);
                }
            }
            CuratedMenuNode::Submenu { title, children, .. } => {
                let next_segs = rem_segs.and_then(|segs| {
                    if segs.len() >= 2 && normalize_menu_token(&segs[0]) == normalize_menu_token(title) { Some(&segs[1..]) } else { None }
                });
                let title_clean = title.trim_end_matches("...").trim_end_matches('…').trim();
                let next_str = rem_str.and_then(|s| {
                    strip_ci_prefix(s.trim_start(), title).or_else(|| strip_ci_prefix(s.trim_start(), title_clean)).and_then(|after| {
                        let t = after.trim_start();
                        t.strip_prefix('/').or_else(|| t.strip_prefix('>')).map(str::trim_start)
                    })
                });
                if let Some(hit) = match_curated_nodes_recursive(children, next_str, next_segs, norm_full) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

fn resolve_menu_command_by_path(app: &UiApp, raw_path: &str, explicit_segs: &[String]) -> Option<String> {
    let norm_full = normalize_menu_path_str(raw_path);
    for sec in curated_menu_bar(app) {
        let next_segs = (!explicit_segs.is_empty()).then_some(explicit_segs).and_then(|segs| {
            if segs.len() >= 2 && normalize_menu_token(&segs[0]) == normalize_menu_token(sec.title) { Some(&segs[1..]) } else { None }
        });
        let next_str = strip_ci_prefix(raw_path.trim_start(), sec.title).and_then(|after| {
            let t = after.trim_start();
            t.strip_prefix('/').or_else(|| t.strip_prefix('>')).map(str::trim_start)
        });
        if let Some(cmd_id) = match_curated_nodes_recursive(&sec.nodes, next_str, next_segs, &norm_full) {
            return Some(cmd_id.to_owned());
        }
    }

    for spec in cmd::registry().specs() {
        if let Some(tip) = command_tooltip(app, spec.id)
            && ((!tip.menu_path.is_empty() && normalize_menu_path_str(tip.menu_path) == norm_full)
                || normalize_menu_token(tip.localized_label) == norm_full
                || normalize_menu_token(tip.label) == norm_full)
        {
            return Some(spec.id.to_owned());
        }
    }
    for spec in UI_COMMANDS {
        if let Some(tip) = command_tooltip(app, spec.id)
            && ((!tip.menu_path.is_empty() && normalize_menu_path_str(tip.menu_path) == norm_full)
                || normalize_menu_token(tip.localized_label) == norm_full
                || normalize_menu_token(tip.label) == norm_full)
        {
            return Some(spec.id.to_owned());
        }
    }
    None
}

/// Finalize a `ui.screenshot` response payload while enforcing `max_line_bytes` (`AUD-13-12`).
///
/// - When the response with inline `data_base64` fits within `max_line_bytes`: writes `validated_path`
///   (if provided) via `write_file` and returns a single `data_base64` field (never duplicating
///   `png_base64`).
/// - When the inline response exceeds `max_line_bytes` and `validated_path` is `Some(dest)`: checks
///   that path-only metadata fits within `max_line_bytes` before writing, writes `dest` atomically
///   via `write_file`, and returns path-only metadata without `data_base64` (`"data_base64_omitted": true`,
///   `"omitted_reason"`).
/// - When the inline response exceeds `max_line_bytes` and `validated_path` is `None`: fails with
///   `ErrorCode::ResourceLimit` and performs zero disk writes.
pub fn finalize_screenshot_payload_with_limit<F>(
    region_raw: &str,
    target: RenderTargetSpec,
    png: &[u8],
    validated_path: Option<&str>,
    max_line_bytes: usize,
    write_file: F,
) -> Result<Value, EngineError>
where
    F: FnOnce(&str, &[u8]) -> Result<(), EngineError>,
{
    let mut base_payload = json!({
        "region": region_raw,
        "width_px": target.width_px,
        "height_px": target.height_px,
        "dpi": target.dpi,
        "format": "png",
        "mime_type": "image/png",
        "bytes_len": png.len(),
        "path": validated_path,
    });

    let b64 = curvebook_platform::base64::encode(png);
    let mut with_inline = base_payload.clone();
    if let Some(map) = with_inline.as_object_mut() {
        map.insert(String::from("data_base64"), Value::String(b64));
    }

    // Reserve headroom for the JSON-lines ControlResponse envelope (`{"id":...,"ok":true,"result":...}\n`).
    const ENVELOPE_HEADROOM_BYTES: usize = 256;
    let inline_bytes = serde_json::to_string(&with_inline)
        .map(|s| s.len().saturating_add(ENVELOPE_HEADROOM_BYTES))
        .map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;

    if inline_bytes <= max_line_bytes {
        if let Some(dest_path) = validated_path {
            write_file(dest_path, png)?;
        }
        return Ok(with_inline);
    }

    let reason =
        format!("screenshot frame ({inline_bytes} bytes) exceeds control line limit ({max_line_bytes} bytes); supply `path` or reduce dimensions");
    if let Some(dest_path) = validated_path {
        if let Some(map) = base_payload.as_object_mut() {
            map.insert(String::from("data_base64_omitted"), Value::Bool(true));
            map.insert(String::from("omitted_reason"), Value::String(reason));
        }
        let meta_bytes = serde_json::to_string(&base_payload)
            .map(|s| s.len().saturating_add(ENVELOPE_HEADROOM_BYTES))
            .map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;
        if meta_bytes > max_line_bytes {
            return Err(EngineError::with_code(
                ErrorCode::ResourceLimit,
                format!("screenshot response metadata ({meta_bytes} bytes) exceeds control frame limit ({max_line_bytes} bytes)"),
            ));
        }
        write_file(dest_path, png)?;
        Ok(base_payload)
    } else {
        Err(EngineError::with_code(ErrorCode::ResourceLimit, reason))
    }
}

fn dispatch_gui_screenshot(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let region_raw = opt_non_empty_str_param(params, &["region", "target"]).map_err(bad_input_err)?.unwrap_or("full_window");
    let region = match region_raw.to_ascii_lowercase().as_str() {
        "full_window" | "full" | "window" | "workspace" | "viewport" => ScreenshotRegionTarget::FullWindow,
        "canvas" => ScreenshotRegionTarget::Canvas,
        "dialog" => ScreenshotRegionTarget::Dialog,
        other => return Err(bad_input_err(format!("unsupported screenshot region `{other}`"))),
    };
    let width_px = parse_bounded_u32(params.get("width_px"), 1280, 64, 4096, "width_px").map_err(bad_input_err)?;
    let height_px = parse_bounded_u32(params.get("height_px"), 800, 64, 4096, "height_px").map_err(bad_input_err)?;
    let dpi = if params.get("dpi").is_some_and(|v| !v.is_null()) {
        parse_bounded_u32(params.get("dpi"), 144, 72, 600, "dpi").map_err(bad_input_err)?
    } else if let Some(sf) = opt_finite_f64_param(params, &["scale_factor"]).map_err(bad_input_err)? {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let computed = (96.0 * sf.clamp(0.5, 6.0)).round() as u32;
        computed.clamp(72, 600)
    } else {
        144
    };
    let dpi_u16 = u16::try_from(dpi).unwrap_or(144);
    let path_arg = opt_non_empty_str_param(params, &["path"]).map_err(bad_input_err)?;
    let validated_path = if let Some(raw_path) = path_arg { Some(app.session().validate_host_write_path(raw_path)?) } else { None };

    let target = RenderTargetSpec { width_px, height_px, dpi: dpi_u16 };
    let img = app.render_egui_shell_rgba(target, region).map_err(|err| {
        let msg = err.to_string();
        if msg.contains("no modal dialog") || msg.contains("unavailable") {
            EngineError::with_code(ErrorCode::Conflict, msg)
        } else {
            EngineError::with_code(ErrorCode::BadInput, msg)
        }
    })?;
    let png = encode_png(&img).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;
    let actual_target = RenderTargetSpec { width_px: img.width_px, height_px: img.height_px, dpi: img.dpi };
    finalize_screenshot_payload_with_limit(
        region_raw,
        actual_target,
        &png,
        validated_path.as_deref(),
        crate::control_io::MAX_CONTROL_LINE_BYTES,
        |dest_path, bytes| app.session().write_host_file_atomic(dest_path, bytes),
    )
}

fn human_path_only_err(message: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::Conflict, message).with_details(json!({ "code": "human_path_only_policy" }))
}

fn dispatch_gui_invoke_menu(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let explicit_cmd = opt_non_empty_str_param(params, &["command_id", "command"]).map_err(bad_input_err)?;
    let menu_path = extract_menu_path_param(params)?;
    let explicit_params = params.get("params").filter(|v| match v {
        Value::Null => false,
        Value::Object(map) => !map.is_empty(),
        _ => true,
    });
    if app.is_human_path_only() && explicit_params.is_some() {
        return Err(human_path_only_err("direct command parameter injection via `ui.invoke_menu(params)` is blocked in human-path-only mode"));
    }
    let resolved_cmd: String = if let Some(cmd) = explicit_cmd {
        cmd.to_owned()
    } else if let Some((raw_path, segs)) = menu_path {
        resolve_menu_command_by_path(app, &raw_path, &segs)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("menu path `{raw_path}` not found")))?
    } else {
        return Err(bad_input_err("`ui.invoke_menu` requires non-empty `command_id` or `path`"));
    };
    let command_id = resolved_cmd.as_str();
    let cmd_params = match explicit_params {
        Some(v) => v.clone(),
        None => contextual_command_params(app, command_id),
    };
    if HybridCoexistenceArbiter::evaluate(app, false).modal_backdrop_active {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("cannot invoke menu command `{command_id}` while a modal dialog is open")));
    }
    if let Some(tip) = command_tooltip(app, command_id)
        && !tip.enabled
    {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("menu command `{command_id}` is currently disabled")));
    }

    let (is_ui, result, mutated) = match dispatch_ui_command_as_ui(app, command_id, &cmd_params)? {
        Some(res) => (true, res, false),
        None => {
            if cmd::registry().get(command_id).is_some() {
                let out = app.execute_session_ui_cmd(command_id, cmd_params)?;
                (false, out.payload, out.mutated)
            } else {
                return Err(EngineError::with_code(ErrorCode::NotFound, format!("unknown menu command `{command_id}`")));
            }
        }
    };
    let _ = step_headless_batches(app, ViewportMetrics::default(), Vec::new())?;
    let mut response = json!({
        "invoked": true,
        "command_id": command_id,
        "ui": is_ui,
        "payload": result,
        "result": result,
    });
    if let Some(obj) = response.as_object_mut() {
        if is_ui {
            obj.insert(String::from("active_dialog"), json!(format!("{:?}", app.canvas_dialogs.active_dialog)));
        } else {
            obj.insert(String::from("mutated"), json!(mutated));
        }
    }
    Ok(response)
}

/// Schedule a scroll-into-view state adjustment on `app` for an offscreen Navigator sheet or
/// grid cell widget (`wid`), returning `true` when scroll state was modified.
pub fn ensure_widget_scrolled_into_view_state(app: &mut UiApp, metrics: ViewportMetrics, wid: &str) -> bool {
    if let Some(sheet_str) = wid.strip_prefix("nav.sheet.") {
        let sid = curvebook_doc::SheetId::from(sheet_str);
        if app.nav.folders.iter().any(|f| f.entries.iter().any(|e| e.sheet_id == sid)) {
            app.nav.navigator_open = true;
            for folder in &mut app.nav.folders {
                if folder.entries.iter().any(|e| e.sheet_id == sid) {
                    folder.collapsed = false;
                }
            }
            app.egui_views.leading_dock.nested_collapsed_parents.clear();
            app.egui_views.leading_dock.scroll_to_sheet = Some(sid);
            return true;
        }
    }
    let active_ds = app.document().active_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id)).cloned();
    if let Some(ds) = active_ds {
        let mut grid_state = std::mem::take(&mut app.egui_views.column_grid);
        let layout_metrics = Tokens::from_theme_spec(&app.theme, &app.locale).metrics;
        #[allow(clippy::cast_possible_truncation)]
        let vp_w = metrics.logical_width as f32;
        #[allow(clippy::cast_possible_truncation)]
        let vp_h = metrics.logical_height as f32;
        let scrolled = if let Some(coord) = parse_grid_cell_widget_id(wid) {
            super::data_grid::scroll_cell_into_view(
                app,
                &ds,
                &mut grid_state,
                layout_metrics,
                (vp_w, vp_h),
                super::data_grid::ScrollCellTarget { is_x: false, row: coord.row, col: coord.col, subcol: coord.subcol },
            )
        } else if let Some(col) = wid.strip_prefix("grid.col_title.").and_then(|s| s.parse::<usize>().ok()) {
            super::data_grid::scroll_cell_into_view(
                app,
                &ds,
                &mut grid_state,
                layout_metrics,
                (vp_w, vp_h),
                super::data_grid::ScrollCellTarget { is_x: false, row: 0, col, subcol: 0 },
            )
        } else if let Some((row, subcol)) =
            parse_x_cell_widget_id(wid).or_else(|| wid.strip_prefix("grid.x.").and_then(|rest| parse_x_cell_widget_id(&format!("grid.xcell.{rest}"))))
        {
            super::data_grid::scroll_cell_into_view(
                app,
                &ds,
                &mut grid_state,
                layout_metrics,
                (vp_w, vp_h),
                super::data_grid::ScrollCellTarget { is_x: true, row, col: 0, subcol },
            )
        } else {
            false
        };
        app.egui_views.column_grid = grid_state;
        return scrolled;
    }
    false
}

fn ensure_widget_scrolled_into_view(app: &mut UiApp, metrics: ViewportMetrics, wid: &str, meta: &mut FrameInspectMeta) -> Result<(), EngineError> {
    if resolve_widget_rect_in_meta(meta, wid).is_some() {
        return Ok(());
    }
    if ensure_widget_scrolled_into_view_state(app, metrics, wid) {
        let (m2, _, _) = step_headless_batches(app, metrics, Vec::new())?;
        *meta = m2;
    }
    Ok(())
}

/// Parsed and validated `ui.pointer_gesture` parameters.
pub struct ParsedPointerGestureParams<'a> {
    pub kind: String,
    pub button: egui::PointerButton,
    pub egui_mods: Modifiers,
    pub metrics: ViewportMetrics,
    pub widget_id: Option<&'a str>,
    pub to_widget_id: Option<&'a str>,
    pub explicit_xy: Option<(f64, f64)>,
    pub to_xy: Option<(f64, f64)>,
    pub scale_delta: f64,
    pub delta_xy: (f64, f64),
}

/// Parse and validate `ui.pointer_gesture` request parameters.
///
/// # Errors
/// Returns [`EngineError`] on invalid actions, buttons, coordinates, or target ambiguity.
pub fn parse_pointer_gesture_params(params: &Value, default_scroll_dy: f64) -> Result<ParsedPointerGestureParams<'_>, EngineError> {
    require_params_object_or_null(params, "ui.pointer_gesture").map_err(bad_input_err)?;
    let kind = opt_non_empty_str_param(params, &["kind", "action", "gesture"]).map_err(bad_input_err)?.unwrap_or("click").to_ascii_lowercase();
    if !matches!(kind.as_str(), "click" | "double_click" | "right_click" | "hover" | "move" | "scroll" | "wheel" | "drag" | "pinch") {
        return Err(bad_input_err(format!("unsupported pointer gesture kind `{kind}`")));
    }
    let button_str = opt_non_empty_str_param(params, &["button"]).map_err(bad_input_err)?.unwrap_or("primary");
    let button = parse_pointer_button_str(button_str)?;
    let key_mods = parse_modifiers_from_params(params)?;
    let egui_mods = to_egui_modifiers(key_mods);
    let metrics = parse_viewport_metrics_from_params(params)?;
    let widget_id = opt_non_empty_str_param(params, &["widget_id", "target_id"]).map_err(bad_input_err)?;
    let to_widget_id = opt_non_empty_str_param(params, &["to_widget_id", "end_widget_id"]).map_err(bad_input_err)?;
    let ensure_f32_coord = |v: f64, name: &str| -> Result<f64, EngineError> {
        if !v.is_finite() || v.abs() > f64::from(f32::MAX) {
            return Err(bad_input_err(format!("`{name}` ({v}) exceeds finite f32 coordinate range")));
        }
        Ok(v)
    };
    let explicit_x = opt_finite_f64_param(params, &["x"]).map_err(bad_input_err)?.map(|v| ensure_f32_coord(v, "x")).transpose()?;
    let explicit_y = opt_finite_f64_param(params, &["y"]).map_err(bad_input_err)?.map(|v| ensure_f32_coord(v, "y")).transpose()?;
    if explicit_x.is_some() != explicit_y.is_some() {
        return Err(bad_input_err("both `x` and `y` coordinates are required when targeting by coordinates"));
    }
    if widget_id.is_some() && explicit_x.is_some() {
        return Err(bad_input_err("ambiguous pointer gesture target: specify either `widget_id` or `x`/`y` coordinates, not both"));
    }
    if widget_id.is_none() && explicit_x.is_none() {
        return Err(bad_input_err("ui.pointer_gesture requires either `widget_id` or finite `x`/`y` coordinates"));
    }
    let dx = ensure_f32_coord(opt_finite_f64_param(params, &["delta_x"]).map_err(bad_input_err)?.unwrap_or(0.0), "delta_x")?;
    let dy = ensure_f32_coord(
        opt_finite_f64_param(params, &["delta_y"]).map_err(bad_input_err)?.unwrap_or(if params.get("delta_x").is_none() {
            default_scroll_dy
        } else {
            0.0
        }),
        "delta_y",
    )?;
    let scale_delta = opt_finite_f64_param(params, &["scale_delta"]).map_err(bad_input_err)?.unwrap_or(1.0);
    if kind == "pinch" && (!scale_delta.is_finite() || scale_delta <= 0.0) {
        return Err(bad_input_err("`scale_delta` must be a positive finite number"));
    }
    let end_x = opt_finite_f64_param(params, &["to_x", "end_x"]).map_err(bad_input_err)?.map(|v| ensure_f32_coord(v, "to_x")).transpose()?;
    let end_y = opt_finite_f64_param(params, &["to_y", "end_y"]).map_err(bad_input_err)?.map(|v| ensure_f32_coord(v, "to_y")).transpose()?;
    if end_x.is_some() != end_y.is_some() {
        return Err(bad_input_err("both `to_x`/`end_x` and `to_y`/`end_y` must be provided together for `drag`"));
    }
    let to_xy = end_x.zip(end_y);
    if kind == "drag" && to_xy.is_none() && to_widget_id.is_none() {
        return Err(bad_input_err("`drag` gesture requires `to_x`/`to_y` (or `end_x`/`end_y`) or `to_widget_id`"));
    }
    Ok(ParsedPointerGestureParams {
        kind,
        button,
        egui_mods,
        metrics,
        widget_id,
        to_widget_id,
        explicit_xy: explicit_x.zip(explicit_y),
        to_xy,
        scale_delta,
        delta_xy: (dx, dy),
    })
}

/// Build synthetic `egui::Event` batches for a validated `ui.pointer_gesture`.
#[must_use]
pub fn build_pointer_gesture_batches(parsed: &ParsedPointerGestureParams<'_>, start_pos: Pos2, end_pos: Option<Pos2>) -> Vec<Vec<egui::Event>> {
    let button = parsed.button;
    let egui_mods = parsed.egui_mods;
    match parsed.kind.as_str() {
        "click" => click_pos_batches(start_pos, button, egui_mods),
        "right_click" => click_pos_batches(start_pos, egui::PointerButton::Secondary, egui_mods),
        "double_click" => vec![
            vec![egui::Event::PointerMoved(start_pos)],
            vec![egui::Event::PointerButton { pos: start_pos, button, pressed: true, modifiers: egui_mods }],
            vec![egui::Event::PointerButton { pos: start_pos, button, pressed: false, modifiers: egui_mods }],
            vec![egui::Event::PointerButton { pos: start_pos, button, pressed: true, modifiers: egui_mods }],
            vec![egui::Event::PointerButton { pos: start_pos, button, pressed: false, modifiers: egui_mods }],
        ],
        "move" | "hover" => vec![vec![egui::Event::PointerMoved(start_pos)]],
        "wheel" | "scroll" => {
            #[allow(clippy::cast_possible_truncation)]
            let delta = Vec2::new(parsed.delta_xy.0 as f32, parsed.delta_xy.1 as f32);
            vec![
                vec![egui::Event::PointerMoved(start_pos)],
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    modifiers: egui_mods,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                }],
            ]
        }
        "pinch" => {
            #[allow(clippy::cast_possible_truncation)]
            let zoom_f32 = parsed.scale_delta.clamp(0.1, 10.0) as f32;
            vec![vec![egui::Event::PointerMoved(start_pos)], vec![egui::Event::Zoom(zoom_f32)]]
        }
        "drag" => {
            let end = end_pos.unwrap_or(start_pos);
            drag_pos_batches(start_pos, end, button, egui_mods)
        }
        _ => Vec::new(),
    }
}

/// Clear `interaction.context_menu_target` after a coordinate `right_click` if no egui context menu opened.
pub fn clear_right_click_miss_context_menu(app: &mut UiApp) {
    if app.egui_views.column_grid.context_menu.is_none() && app.egui_views.leading_dock.context_menu.is_none() {
        app.interaction.context_menu_target = None;
    }
}

/// Return whether `wid` addresses the already-selected active grid cell without an open inline editor.
#[must_use]
pub fn is_grid_cell_widget_selected_without_inline_edit(app: &UiApp, wid: &str) -> bool {
    let active_for_sheet = app.document().active_sheet_id.as_ref().and_then(|sid| app.grid.active_cell_for_sheet(sid));
    crate::grid_view::parse_grid_cell_widget_id(wid).is_some_and(|coord| active_for_sheet == Some(coord))
        && app.egui_views.column_grid.inline_edit.is_none()
}

/// Commit any active grid inline editor cleanly before switching sheets or tabs.
pub fn commit_active_grid_inline_edit(app: &mut UiApp) {
    let _ = crate::interaction::commit_inline_edit_if_active(app);
}

/// Format the JSON response payload for `ui.pointer_gesture` from committed intent outcomes.
#[must_use]
pub fn format_pointer_gesture_result(kind: &str, widget_id: Option<&str>, start_pos: Pos2, outcomes: &[UiIntentOutcome]) -> Value {
    let cmd_ids: Vec<String> = outcomes
        .iter()
        .filter_map(|o| match o {
            UiIntentOutcome::Engine { command_id, .. } | UiIntentOutcome::Ui { command_id, .. } => Some(command_id.clone()),
            UiIntentOutcome::ShellUpdated => None,
        })
        .collect();
    let mut out = json!({
        "handled": true,
        "kind": kind,
        "action": kind,
        "widget_id": widget_id,
        "x": f64::from(start_pos.x),
        "y": f64::from(start_pos.y),
        "commands": cmd_ids,
        "mutated": false,
    });
    if let Some(out_obj) = out.as_object_mut() {
        for o in outcomes {
            match o {
                UiIntentOutcome::Engine { outcome, .. } => {
                    if outcome.mutated {
                        out_obj.insert(String::from("mutated"), json!(true));
                    }
                    if let Some(map) = outcome.payload.as_object() {
                        for (k, v) in map {
                            if !matches!(k.as_str(), "handled" | "kind" | "action" | "widget_id" | "x" | "y" | "commands") {
                                out_obj.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
                UiIntentOutcome::Ui { result: Some(val), .. } => {
                    if let Some(map) = val.as_object() {
                        for (k, v) in map {
                            if !matches!(k.as_str(), "handled" | "kind" | "action" | "widget_id" | "x" | "y" | "commands") {
                                out_obj.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
                UiIntentOutcome::Ui { result: None, .. } | UiIntentOutcome::ShellUpdated => {}
            }
        }
    }
    out
}

/// Resolved pointer target and readout for `power.curve_hover`.
#[derive(Debug, Clone, Copy)]
pub struct PowerCurveHoverTarget {
    pub pos: Pos2,
    pub fraction_x: f64,
    pub hovered_pair: Option<(f64, f64)>,
}

/// Compute `power.curve_hover` fraction, pointer coordinates, and `(n, power)` readout.
///
/// # Errors
/// Returns [`EngineError`] if `power.curve_hover` is absent or `hover_fraction_x` is invalid.
pub fn resolve_power_curve_hover_target(app: &mut UiApp, meta: &FrameInspectMeta, params: &Value) -> Result<PowerCurveHoverTarget, EngineError> {
    let entry = meta
        .find_by_widget_id("power.curve_hover")
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, "widget `power.curve_hover` not found in current UI tree"))?;
    let explicit_frac = opt_finite_f64_param(params, &["hover_fraction_x", "fraction_x"]).map_err(bad_input_err)?;
    let hover_n_opt = opt_finite_f64_param(params, &["hover_n"]).map_err(bad_input_err)?;
    let frac = explicit_frac
        .map(|f| f.clamp(0.0, 1.0))
        .or_else(|| {
            hover_n_opt.and_then(|n| {
                let pts = &app.canvas_dialogs.power_wizard.result.as_ref()?.power_curve;
                let min_n = pts.first().map_or(2.0, |p| p.n);
                let max_n = pts.last().map_or(min_n + 1.0, |p| p.n).max(min_n + 1.0);
                Some(((n - min_n) / (max_n - min_n)).clamp(0.0, 1.0))
            })
        })
        .unwrap_or(0.5);
    #[allow(clippy::cast_possible_truncation)]
    let hover_x = entry.clipped_rect.left() + entry.clipped_rect.width() * (frac as f32);
    let hover_pos = Pos2::new(hover_x, entry.clipped_rect.center().y);
    let hovered_pair = app.canvas_dialogs.power_wizard.hover_at_fraction(frac);
    Ok(PowerCurveHoverTarget { pos: hover_pos, fraction_x: frac, hovered_pair })
}

fn dispatch_gui_pointer_gesture(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let parsed = parse_pointer_gesture_params(params, 0.0)?;
    if app.is_human_path_only() && (parsed.widget_id.is_some() || parsed.to_widget_id.is_some()) {
        return Err(human_path_only_err("direct `ui.pointer_gesture(widget_id)` is blocked in human-path-only mode"));
    }

    if parsed.kind == "click" {
        let (_, _, _, _, t) = acquire_headless_egui_ctx(app, parsed.metrics);
        store_headless_egui_time(app, t + 0.5);
    }

    let (mut meta, _ak, _) = step_headless_batches(app, parsed.metrics, Vec::new())?;

    if let Some(wid) = parsed.widget_id
        && parsed.explicit_xy.is_none()
    {
        ensure_widget_scrolled_into_view(app, parsed.metrics, wid, &mut meta)?;
        if let Some(col_str) = wid.strip_prefix("formula_bar.open_col.")
            && let Ok(col) = col_str.parse::<usize>()
        {
            if !matches!(parsed.kind.as_str(), "click" | "double_click") {
                return Ok(json!({
                    "handled": true,
                    "kind": parsed.kind,
                    "action": parsed.kind,
                    "widget_id": wid,
                    "formula_bar_active": app.grid.formula_bar.active,
                }));
            }
            if HybridCoexistenceArbiter::evaluate(app, false).modal_backdrop_active {
                return Err(EngineError::with_code(ErrorCode::Conflict, format!("cannot open formula bar `{wid}` while a modal dialog is active")));
            }
            let badge_wid = format!("mv_formula.role_badge.{col}");
            if let Ok((pos, enabled, reason)) = resolve_widget_center_pt(app, &meta, parsed.metrics, &badge_wid) {
                if !enabled {
                    return Err(EngineError::with_code(ErrorCode::Conflict, reason.unwrap_or_else(|| format!("widget `{wid}` is disabled"))));
                }
                let _ = step_headless_batches(app, parsed.metrics, click_pos_batches(pos, egui::PointerButton::Primary, parsed.egui_mods))?;
            }
            if !app.grid.formula_bar.active {
                let _ = dispatch_ui_command_as_ui(app, "ui.table.openFormulaBar", &json!({ "col": col }))?;
                let _ = step_headless_batches(app, parsed.metrics, Vec::new())?;
            }
            return Ok(json!({
                "handled": true,
                "kind": parsed.kind,
                "action": parsed.kind,
                "widget_id": wid,
                "formula_bar_active": app.grid.formula_bar.active,
            }));
        }

        if wid == "power.curve_hover" {
            let hover = resolve_power_curve_hover_target(app, &meta, params)?;
            let _ = step_headless_batches(app, parsed.metrics, vec![vec![egui::Event::PointerMoved(hover.pos)]])?;
            app.interaction.last_logical_pos = Some([f64::from(hover.pos.x), f64::from(hover.pos.y)]);
            return Ok(json!({
                "handled": true,
                "kind": parsed.kind,
                "action": parsed.kind,
                "widget_id": wid,
                "x": f64::from(hover.pos.x),
                "y": f64::from(hover.pos.y),
                "hover_fraction_x": hover.fraction_x,
                "hover_n": hover.hovered_pair.map(|(n, _)| n),
                "hover_power": hover.hovered_pair.map(|(_, p)| p),
            }));
        }
    }

    let start_pos = match (parsed.explicit_xy, parsed.widget_id) {
        (Some((x, y)), _) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            Pos2::new(x as f32, y as f32)
        }
        (None, Some(wid)) => {
            let (center, enabled, reason) = resolve_widget_center_pt(app, &meta, parsed.metrics, wid)?;
            if !enabled && !matches!(parsed.kind.as_str(), "hover" | "move") {
                return Err(EngineError::with_code(ErrorCode::Conflict, reason.unwrap_or_else(|| format!("widget `{wid}` is disabled"))));
            }
            center
        }
        _ => return Err(bad_input_err("`ui.pointer_gesture` requires `widget_id` or numeric `x` and `y`")),
    };

    let end_pos = if parsed.kind == "drag" {
        let ep = if let Some((tx, ty)) = parsed.to_xy {
            #[allow(clippy::cast_possible_truncation)]
            Pos2::new(tx as f32, ty as f32)
        } else if let Some(to_wid) = parsed.to_widget_id {
            let (center, _, _) = resolve_widget_center_pt(app, &meta, parsed.metrics, to_wid)?;
            center
        } else {
            return Err(bad_input_err("`drag` gesture requires `to_x`/`to_y` (or `end_x`/`end_y`) or `to_widget_id`"));
        };
        app.interaction.last_logical_pos = Some([f64::from(ep.x), f64::from(ep.y)]);
        Some(ep)
    } else {
        app.interaction.last_logical_pos = Some([f64::from(start_pos.x), f64::from(start_pos.y)]);
        None
    };

    let batches = build_pointer_gesture_batches(&parsed, start_pos, end_pos);
    let (_m2, _ak2, outcomes) = step_headless_batches(app, parsed.metrics, batches)?;
    if parsed.kind == "right_click"
        && parsed.widget_id.is_none()
        && !outcomes.iter().any(|o| !matches!(o, UiIntentOutcome::ShellUpdated))
        && app.egui_views.column_grid.context_menu.is_none()
        && app.egui_views.leading_dock.context_menu.is_none()
    {
        app.interaction.context_menu_target = None;
    }
    Ok(format_pointer_gesture_result(&parsed.kind, parsed.widget_id, start_pos, &outcomes))
}

/// Parsed and validated `ui.type_text` parameters.
pub struct ParsedTypeTextParams<'a> {
    pub text: &'a str,
    pub clear_param: Option<bool>,
    pub submit: bool,
    pub widget_id: Option<&'a str>,
    pub metrics: ViewportMetrics,
}

/// Parse and validate `ui.type_text` request parameters.
///
/// # Errors
/// Returns [`EngineError`] on invalid types or missing `text`.
pub fn parse_type_text_params(params: &Value) -> Result<ParsedTypeTextParams<'_>, EngineError> {
    require_params_object_or_null(params, "ui.type_text").map_err(bad_input_err)?;
    let text = opt_str_param(params, &["text"]).map_err(bad_input_err)?.ok_or_else(|| bad_input_err("`ui.type_text` requires string `text`"))?;
    let clear_param = opt_bool_param(params, &["clear_first", "clear_existing", "replace"]).map_err(bad_input_err)?;
    let submit = opt_bool_param(params, &["submit", "commit_enter", "press_enter"]).map_err(bad_input_err)?.unwrap_or(false);
    let widget_id = opt_non_empty_str_param(params, &["widget_id", "target_widget_id"]).map_err(bad_input_err)?;
    let metrics = parse_viewport_metrics_from_params(params)?;
    Ok(ParsedTypeTextParams { text, clear_param, submit, widget_id, metrics })
}

/// Validate read-only `LinkedDataRegion` constraints and classify grid typing targets.
///
/// # Errors
/// Returns [`EngineError`] with `ErrorCode::Conflict` if the targeted cell belongs to a read-only `LinkedDataRegion`.
pub fn prepare_grid_type_text_target(
    app: &mut UiApp,
    widget_id: Option<&str>,
    live_focus_wid: Option<&str>,
    modal_backdrop_active: bool,
    egui_wants_kb: bool,
) -> Result<(bool, bool), EngineError> {
    let resolved_grid_coord =
        widget_id.and_then(parse_grid_cell_widget_id).or_else(|| live_focus_wid.and_then(parse_grid_cell_widget_id)).or_else(|| {
            if widget_id.is_none() && !modal_backdrop_active && !app.grid.formula_bar.active && live_focus_wid.is_none_or(|f| f.starts_with("grid."))
            {
                app.document().active_sheet_id.as_ref().and_then(|sid| app.grid.active_cell_for_sheet(sid))
            } else {
                None
            }
        });
    if let Some(coord) = resolved_grid_coord
        && let Some(ds) = app.document().active_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id))
        && let Some(reg) = crate::grid_view::linked_region_for_cell_subcol(ds, coord.row, coord.col, coord.subcol)
    {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("cell ({}, {}, {}) belongs to read-only LinkedDataRegion `{}`", coord.row, coord.col, coord.subcol, reg.id),
        ));
    }

    let is_x_cell_target = widget_id.or(live_focus_wid).is_some_and(|w| parse_x_cell_widget_id(w).is_some());
    let is_y_cell_target = widget_id.or(live_focus_wid).is_some_and(|w| parse_grid_cell_widget_id(w).is_some())
        || (widget_id.is_none() && resolved_grid_coord.is_some() && !egui_wants_kb);

    if let Some(wid) = widget_id.or(live_focus_wid)
        && let Some((row, subcol)) = parse_x_cell_widget_id(wid)
        && let Some(sheet_id) = app
            .document()
            .active_sheet_id
            .as_ref()
            .and_then(|id| app.document().data_sheet(id))
            .filter(|ds| matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)))
            .map(|ds| ds.id.clone())
        && app.egui_views.column_grid.inline_edit.is_none()
    {
        // Bind the X-cell selection to the active sheet so a selection left on another sheet
        // (split view) never keeps `active_x_cell` pointing at that sheet (`GAP-07-02`, `R0.36`).
        app.grid.select_x_cell_for_sheet(&sheet_id, row, subcol);
    }

    Ok((is_x_cell_target, is_y_cell_target))
}

/// Build `egui::Event` batches for `ui.type_text`.
#[must_use]
pub fn build_type_text_batches(
    text: &str,
    clear_first: bool,
    submit: bool,
    is_x_cell_target: bool,
    is_y_cell_target: bool,
    inline_edit_open: bool,
) -> Vec<Vec<egui::Event>> {
    let is_grid_cell = is_x_cell_target || is_y_cell_target;
    let is_grid_tsv = is_grid_cell && (text.contains('\t') || text.contains('\n') || text.contains('\r'));
    if is_grid_tsv {
        return vec![vec![egui::Event::Paste(text.to_owned())]];
    }
    if is_grid_cell && text.is_empty() && (submit || clear_first) {
        return vec![vec![
            egui::Event::Key { key: Key::Backspace, physical_key: Some(Key::Backspace), pressed: true, repeat: false, modifiers: Modifiers::NONE },
            egui::Event::Key { key: Key::Backspace, physical_key: Some(Key::Backspace), pressed: false, repeat: false, modifiers: Modifiers::NONE },
        ]];
    }
    let mut batches = Vec::new();
    if clear_first && (!is_grid_cell || inline_edit_open) {
        batches.push(vec![
            egui::Event::Key { key: Key::A, physical_key: Some(Key::A), pressed: true, repeat: false, modifiers: Modifiers::COMMAND },
            egui::Event::Key { key: Key::A, physical_key: Some(Key::A), pressed: false, repeat: false, modifiers: Modifiers::COMMAND },
            egui::Event::Key { key: Key::Backspace, physical_key: Some(Key::Backspace), pressed: true, repeat: false, modifiers: Modifiers::NONE },
            egui::Event::Key { key: Key::Backspace, physical_key: Some(Key::Backspace), pressed: false, repeat: false, modifiers: Modifiers::NONE },
        ]);
    }
    if !text.is_empty() {
        batches.push(vec![egui::Event::Text(text.to_owned())]);
    }
    if !batches.is_empty() {
        // Settle pass opens and focuses freshly activated inline cell editors (`first_frame = false`)
        // so a subsequent `Enter` submit pass commits cleanly instead of being ignored on `first_frame`.
        batches.push(Vec::new());
    }
    if submit {
        batches.push(vec![
            egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE },
            egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: false, repeat: false, modifiers: Modifiers::NONE },
        ]);
    }
    batches
}

/// Format the JSON response payload for `ui.type_text`.
#[must_use]
pub fn format_type_text_result(app: &UiApp, text: &str, widget_id: Option<&str>, submit: bool, outcomes: &[UiIntentOutcome]) -> Value {
    let committed_cmd = outcomes.iter().rev().find_map(|o| match o {
        UiIntentOutcome::Engine { command_id, .. } | UiIntentOutcome::Ui { command_id, .. } => Some(command_id.clone()),
        UiIntentOutcome::ShellUpdated => None,
    });
    let mut out = json!({
        "typed": true,
        "text": text,
        "widget_id": widget_id,
        "submit": submit,
        "committed": committed_cmd.is_some(),
        "command_id": committed_cmd,
    });
    if (widget_id == Some("formula_bar.input") || app.grid.formula_bar.active)
        && let Some(obj) = out.as_object_mut()
    {
        obj.insert(String::from("is_valid"), json!(app.grid.formula_bar.is_valid));
        obj.insert(String::from("parse_error"), json!(app.grid.formula_bar.parse_error));
        obj.insert(String::from("formula_bar_active"), json!(app.grid.formula_bar.active));
    }
    out
}

fn dispatch_gui_type_text(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let parsed = parse_type_text_params(params)?;
    if app.is_human_path_only()
        && let Some(wid) = parsed.widget_id
        && app.focus_widget_id.as_deref() != Some(wid)
    {
        return Err(human_path_only_err(format!("focus-jumping `ui.type_text(widget_id = {wid})` is blocked in human-path-only mode")));
    }

    if let Some(wid) = parsed.widget_id {
        if app.is_human_path_only() {
            let _ = ensure_egui_focus_if_needed(app, parsed.metrics)?;
        } else {
            if parse_grid_cell_widget_id(wid).is_some() || parse_x_cell_widget_id(wid).is_some() {
                // Exceed egui's double-click interval so focusing a previously clicked cell never opens inline_edit.
                let (_, _, _, _, t) = acquire_headless_egui_ctx(app, parsed.metrics);
                store_headless_egui_time(app, t + 0.5);
            }
            let _ = dispatch_gui_pointer_gesture(app, &json!({ "kind": "click", "widget_id": wid, "viewport": params.get("viewport") }))?;
        }
    } else {
        let _ = ensure_egui_focus_if_needed(app, parsed.metrics)?;
    }

    let modal_active = HybridCoexistenceArbiter::evaluate(app, false).modal_backdrop_active;
    let egui_wants_kb = app.headless_egui.inner.lock().ok().and_then(|g| g.as_ref().map(|(c, _)| c.egui_wants_keyboard_input())).unwrap_or(false);
    let focus_copy = app.focus_widget_id.clone();
    let (is_x_cell_target, is_y_cell_target) =
        prepare_grid_type_text_target(app, parsed.widget_id, focus_copy.as_deref(), modal_active, egui_wants_kb)?;
    let explicit_y_cell = parsed.widget_id.is_some_and(|w| parse_grid_cell_widget_id(w).is_some());
    if explicit_y_cell {
        let _ = step_headless_batches(app, parsed.metrics, Vec::new())?;
    }

    let clear_first = parsed.clear_param.unwrap_or_else(|| parsed.widget_id.is_some() && !(is_x_cell_target || is_y_cell_target));
    let inline_open = app.egui_views.column_grid.inline_edit.is_some();
    let batches = build_type_text_batches(parsed.text, clear_first, parsed.submit, is_x_cell_target, is_y_cell_target, inline_open);
    let (_, _, all_outcomes) =
        if batches.is_empty() { (FrameInspectMeta::default(), None, Vec::new()) } else { step_headless_batches(app, parsed.metrics, batches)? };
    Ok(format_type_text_result(app, parsed.text, parsed.widget_id, parsed.submit, &all_outcomes))
}

/// Parsed and validated `ui.press_key` parameters.
pub struct ParsedPressKeyParams<'a> {
    pub key_raw: &'a str,
    pub key: Key,
    pub modifiers: Modifiers,
    pub repeat: bool,
    pub metrics: ViewportMetrics,
}

/// Parse and validate `ui.press_key` request parameters.
///
/// # Errors
/// Returns [`EngineError`] on missing/unsupported key or invalid modifiers/viewport.
pub fn parse_press_key_params(params: &Value) -> Result<ParsedPressKeyParams<'_>, EngineError> {
    require_params_object_or_null(params, "ui.press_key").map_err(bad_input_err)?;
    let key_raw =
        opt_non_empty_str_param(params, &["key"]).map_err(bad_input_err)?.ok_or_else(|| bad_input_err("`ui.press_key` requires non-empty `key`"))?;
    let key = parse_egui_key(key_raw).ok_or_else(|| bad_input_err(format!("unsupported key `{key_raw}`")))?;
    let key_mods = parse_modifiers_from_params(params)?;
    let modifiers = to_egui_modifiers(key_mods);
    let repeat = opt_bool_param(params, &["repeat"]).map_err(bad_input_err)?.unwrap_or(false);
    let metrics = parse_viewport_metrics_from_params(params)?;
    Ok(ParsedPressKeyParams { key_raw, key, modifiers, repeat, metrics })
}

/// Build `egui::Event` batches for `ui.press_key`.
#[must_use]
pub fn build_press_key_batches(key: Key, modifiers: Modifiers, repeat: bool) -> Vec<Vec<egui::Event>> {
    let mut batches = vec![
        vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat, modifiers }],
        vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat, modifiers }],
    ];
    if !modifiers.is_none() {
        batches.push(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    }
    batches
}

/// Format the JSON response payload for `ui.press_key`.
#[must_use]
pub fn format_press_key_result(key_raw: &str, outcomes: &[UiIntentOutcome]) -> Value {
    let cmd_ids: Vec<String> = outcomes
        .iter()
        .filter_map(|o| match o {
            UiIntentOutcome::Engine { command_id, .. } | UiIntentOutcome::Ui { command_id, .. } => Some(command_id.clone()),
            UiIntentOutcome::ShellUpdated => None,
        })
        .collect();

    let mut out = json!({
        "handled": true,
        "key": key_raw,
        "commands": cmd_ids,
    });
    if let Some(out_obj) = out.as_object_mut() {
        for o in outcomes {
            let val_opt = match o {
                UiIntentOutcome::Engine { outcome, .. } => Some(&outcome.payload),
                UiIntentOutcome::Ui { result: Some(val), .. } => Some(val),
                UiIntentOutcome::Ui { result: None, .. } | UiIntentOutcome::ShellUpdated => None,
            };
            if let Some(map) = val_opt.and_then(Value::as_object) {
                for (k, v) in map {
                    if !matches!(k.as_str(), "handled" | "key" | "commands") {
                        out_obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }
    out
}

fn dispatch_gui_press_key(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let parsed = parse_press_key_params(params)?;
    let _ = ensure_egui_focus_if_needed(app, parsed.metrics)?;
    let batches = build_press_key_batches(parsed.key, parsed.modifiers, parsed.repeat);
    let (_, _, outcomes) = step_headless_batches(app, parsed.metrics, batches)?;
    Ok(format_press_key_result(parsed.key_raw, &outcomes))
}

/// Target parsed from `ui.switch_tab` parameters.
pub enum ParsedSwitchTabTarget<'a> {
    Sheet(&'a str),
    Tab(&'a str),
}

/// Parsed and validated `ui.switch_tab` parameters.
pub struct ParsedSwitchTabParams<'a> {
    pub target: ParsedSwitchTabTarget<'a>,
    pub metrics: ViewportMetrics,
}

const NUMERIC_TAB_INDICES: [&str; 64] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25",
    "26", "27", "28", "29", "30", "31", "32", "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46", "47", "48", "49",
    "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61", "62", "63",
];

fn parse_tab_target_str_or_index(params: &Value) -> Result<Option<&str>, EngineError> {
    let Some(obj) = params.as_object() else {
        return Ok(None);
    };
    for key in ["tab_id", "tab", "tab_index", "widget_id", "id"] {
        let Some(val) = obj.get(key) else {
            continue;
        };
        if val.is_null() {
            continue;
        }
        if let Some(s) = val.as_str() {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(bad_input_err(format!("`{key}` must be a non-empty string or non-negative integer")));
            }
            return Ok(Some(trimmed));
        }
        if let Some(idx_u64) = val.as_u64() {
            let idx = usize::try_from(idx_u64).map_err(|_| bad_input_err(format!("`{key}` index out of range")))?;
            let static_str = NUMERIC_TAB_INDICES
                .get(idx)
                .copied()
                .ok_or_else(|| bad_input_err(format!("`{key}` index {idx} exceeds supported tab range (0..{})", NUMERIC_TAB_INDICES.len())))?;
            return Ok(Some(static_str));
        }
        return Err(bad_input_err(format!("`{key}` must be a non-empty string or non-negative integer")));
    }
    Ok(None)
}

/// Parse and validate `ui.switch_tab` request parameters.
///
/// # Errors
/// Returns [`EngineError`] if neither `sheet_id` nor `tab_id`/`tab` is present or metrics are invalid.
pub fn parse_switch_tab_params(params: &Value) -> Result<ParsedSwitchTabParams<'_>, EngineError> {
    require_params_object_or_null(params, "ui.switch_tab").map_err(bad_input_err)?;
    let metrics = parse_viewport_metrics_from_params(params)?;
    if let Some(sheet_id) = opt_non_empty_str_param(params, &["sheet_id"]).map_err(bad_input_err)? {
        return Ok(ParsedSwitchTabParams { target: ParsedSwitchTabTarget::Sheet(sheet_id), metrics });
    }
    let tab_id =
        parse_tab_target_str_or_index(params)?.ok_or_else(|| bad_input_err("`ui.switch_tab` requires non-empty `tab_id`, `tab`, or `sheet_id`"))?;
    Ok(ParsedSwitchTabParams { target: ParsedSwitchTabTarget::Tab(tab_id), metrics })
}

/// Find the rendered widget node in `meta` corresponding to a `ui.switch_tab` target.
#[must_use]
pub fn find_switch_tab_widget_in_meta<'a>(meta: &'a FrameInspectMeta, target: &ParsedSwitchTabTarget<'_>) -> Option<&'a FrameInspectNodeMeta> {
    match target {
        ParsedSwitchTabTarget::Sheet(sheet_id) => {
            let nav_wid = format!("nav.sheet.{sheet_id}");
            meta.find_by_widget_id(&nav_wid)
        }
        ParsedSwitchTabTarget::Tab(tab_id) => {
            let candidates = [(*tab_id).to_owned(), format!("result.tab.{tab_id}"), format!("edit.tab.{tab_id}")];
            candidates
                .iter()
                .find_map(|cand| meta.find_by_widget_id(cand))
                .or_else(|| {
                    meta.entries.iter().find(|e| {
                        entry_belongs_to_scope(e, "tabs") && (e.widget_id.ends_with(&format!(".{tab_id}")) || e.label.eq_ignore_ascii_case(tab_id))
                    })
                })
                .or_else(|| {
                    let idx = tab_id.parse::<usize>().ok()?;
                    meta.entries
                        .iter()
                        .filter(|e| e.role == "tab" && entry_belongs_to_scope(e, "tabs"))
                        .nth(idx)
                        .or_else(|| meta.entries.iter().filter(|e| entry_belongs_to_scope(e, "tabs")).nth(idx))
                })
        }
    }
}

const POWER_STAGE_ALIASES: &[(&[&str], usize)] = &[
    (&["0", "design", "experiment_design", "power.stage.1", "power.tab.design"], 0),
    (&["1", "effect", "effect_size", "power.stage.2", "power.tab.effect"], 1),
    (&["2", "alpha", "power", "alpha_power", "significance", "significance_and_solve", "power.stage.3", "power.tab.alpha"], 2),
    (&["3", "results", "sensitivity", "power.stage.4", "power.tab.results"], 3),
];

/// Execute `PowerAndSampleSize` stage switching directly on `app` when `tab_id` names a stage.
///
/// # Errors
/// Returns [`EngineError`] if advancing or stepping back fails.
pub fn try_switch_power_wizard_stage(app: &mut UiApp, tab_id: &str) -> Result<Option<Value>, EngineError> {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize) {
        return Ok(None);
    }
    let norm = tab_id.trim().to_ascii_lowercase();
    let Some(target_idx) = POWER_STAGE_ALIASES.iter().find_map(|(aliases, idx)| aliases.contains(&norm.as_str()).then_some(*idx)) else {
        return Ok(None);
    };
    while app.canvas_dialogs.power_wizard.stage.index() < target_idx {
        let _ = crate::egui_shell::dialog_actions::execute_dialog_button_action(app, "next")?;
    }
    while app.canvas_dialogs.power_wizard.stage.index() > target_idx {
        let _ = crate::egui_shell::dialog_actions::execute_dialog_button_action(app, "back")?;
    }
    Ok(Some(json!({
        "switched": true,
        "tab_id": tab_id,
        "stage_index": app.canvas_dialogs.power_wizard.stage.index(),
        "stage": app.canvas_dialogs.power_wizard.stage.label(),
    })))
}

/// Switch to an off-screen `ResultSheet` tab or select a sheet by ID when not visible in `meta`.
///
/// # Errors
/// Returns [`EngineError`] with `ErrorCode::NotFound` if the sheet or tab does not exist.
pub fn execute_switch_tab_fallback(app: &mut UiApp, target: &ParsedSwitchTabTarget<'_>) -> Result<Value, EngineError> {
    match target {
        ParsedSwitchTabTarget::Sheet(sheet_id) => {
            let _ = app.execute_session_ui_cmd("sheet.select", json!({ "sheet_id": sheet_id }))?;
            let doc = app.document().clone();
            if let Some(active_id) = doc.active_sheet_id.clone() {
                app.split.sync_pin_invariants(&doc);
                app.canvas_dialogs.on_sheet_selected_with_auto_modal(&doc, &active_id, false);
            }
            Ok(json!({ "switched": true, "sheet_id": sheet_id, "widget_id": format!("nav.sheet.{sheet_id}") }))
        }
        ParsedSwitchTabTarget::Tab(tab_id) => {
            let active_rs_id = app.document().active_sheet_id.clone();
            if let Some(rs_id) = active_rs_id
                && let Some(rs) = app.document().result_sheet(&rs_id).cloned()
            {
                if rs.tabs.iter().any(|t| t.id == *tab_id || t.title.eq_ignore_ascii_case(tab_id)) {
                    if !app.canvas_dialogs.result_view.select_tab_by_id(&rs, tab_id)
                        && let Some(pos) = rs.tabs.iter().position(|t| t.title.eq_ignore_ascii_case(tab_id))
                    {
                        app.canvas_dialogs.result_view.active_tab_index = pos;
                    }
                    return Ok(json!({ "switched": true, "tab_id": tab_id, "widget_id": format!("result.tab.{tab_id}") }));
                }
                if let Ok(idx) = tab_id.parse::<usize>()
                    && idx < rs.tabs.len()
                {
                    app.canvas_dialogs.result_view.select_tab(idx, rs.tabs.len());
                    let resolved_id = rs.tabs[idx].id.clone();
                    return Ok(json!({
                        "switched": true,
                        "tab_id": resolved_id,
                        "tab_index": idx,
                        "widget_id": format!("result.tab.{resolved_id}"),
                    }));
                }
            }
            Err(EngineError::with_code(ErrorCode::NotFound, format!("tab `{tab_id}` not found in current UI tree")))
        }
    }
}

fn dispatch_gui_switch_tab(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    if app.is_human_path_only() {
        return Err(human_path_only_err("direct `ui.switch_tab` is blocked in human-path-only mode"));
    }
    let parsed = parse_switch_tab_params(params)?;
    if matches!(parsed.target, ParsedSwitchTabTarget::Sheet(_)) {
        let _ = crate::interaction::commit_inline_edit_if_active(app);
    }
    if let ParsedSwitchTabTarget::Tab(tab_id) = parsed.target
        && let Some(res) = try_switch_power_wizard_stage(app, tab_id)?
    {
        let _ = step_headless_batches(app, parsed.metrics, Vec::new())?;
        return Ok(res);
    }
    let (meta, _, _) = step_headless_batches(app, parsed.metrics, Vec::new())?;
    if let Some(entry) = find_switch_tab_widget_in_meta(&meta, &parsed.target) {
        if !entry.enabled {
            let tab_label = match parsed.target {
                ParsedSwitchTabTarget::Sheet(s) => s,
                ParsedSwitchTabTarget::Tab(t) => t,
            };
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                entry.disabled_reason.clone().unwrap_or_else(|| format!("tab `{tab_label}` is disabled")),
            ));
        }
        let pos = entry.clipped_rect.center();
        let wid = entry.widget_id.clone();
        let _ = step_headless_batches(app, parsed.metrics, click_pos_batches(pos, egui::PointerButton::Primary, Modifiers::NONE))?;
        return Ok(match parsed.target {
            ParsedSwitchTabTarget::Sheet(sheet_id) => json!({ "switched": true, "sheet_id": sheet_id, "widget_id": wid }),
            ParsedSwitchTabTarget::Tab(tab_id) => json!({ "switched": true, "tab_id": tab_id, "widget_id": wid }),
        });
    }
    let out = execute_switch_tab_fallback(app, &parsed.target)?;
    let _ = step_headless_batches(app, parsed.metrics, Vec::new())?;
    Ok(out)
}

/// Render a minimal `egui_shell` workspace + `About` modal view (`R15.1` b4/b5) using canonical
/// [`Tokens`], non-overlapping solid scrollbars, `100%` `ZLayer::ModalBackdrop` outside input
/// capture, [`FrameInspectMeta`], and [`PendingUiIntents`].
#[cfg(any(test, feature = "test-support"))]
pub fn render_minimal_shell_and_about(
    ui: &mut egui::Ui,
    app: &UiApp,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ShellAboutFrameReport {
    let ctx = ui.ctx().clone();
    ctx.enable_accesskit();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    tokens.apply_to_ctx(&ctx);
    // Synchronize the active pass's `Ui` style immediately so pass-0 widgets do not inherit stale context visuals.
    tokens.apply_to_ui(ui);
    meta.begin_pass(&ctx);

    let mut report = ShellAboutFrameReport::default();
    let modal_open = app.canvas_dialogs.active_dialog != ActiveDialog::None;

    render_top_about_only_bar(ui, app, modal_open, meta, intents);

    egui::CentralPanel::default().show(ui, |ui| {
        let panel_rect = ui.available_rect_before_wrap();
        report.regions.canvas_rect_pt = Some(panel_rect);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        let scroll_out = egui::ScrollArea::vertical()
            .id_salt("egui_shell_workspace_scroll")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 24.0).max(80.0));
                for idx in 0..40 {
                    let row_title = if idx == 0 { "CurveBook Workspace Shell" } else { "Sheet Overview Entry" };
                    let row_label = format!("{row_title} #{idx}");
                    let row_resp = ui.add_enabled(!modal_open, egui::Button::new(&row_label));
                    let wid = format!("toolbar.shell.row_{idx}");
                    meta.record_response(
                        ui,
                        &row_resp,
                        FrameInspectEntryInput {
                            widget_id: &wid,
                            role: "button",
                            label: &row_label,
                            command_id: None,
                            value: None,
                            enabled: !modal_open,
                            disabled_reason: modal_open.then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                }
            });
        let bar_w = ui.spacing().scroll.bar_width.max(10.0);
        if scroll_out.inner_rect.is_positive() {
            report.scrollbar_rect_pt =
                Some(egui::Rect::from_min_max(egui::pos2((panel_rect.right() - bar_w).max(panel_rect.left()), panel_rect.top()), panel_rect.max));
        }
    });

    if app.canvas_dialogs.active_dialog == ActiveDialog::About {
        render_about_modal(&ctx, app, &tokens, meta, intents, &mut report);
    }

    report
}

/// Top-level `egui_shell` workspace/dialog frame entry point routed by headless `UiApp` inspection
/// & software screenshots (`lib.rs`).
pub fn render_egui_shell_frame(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ShellAboutFrameReport {
    render_egui_shell_frame_with_host(ui, app, views, meta, intents, false)
}

/// Full native `egui` workspace/dialog frame entry point supporting optional host close modal
/// arbitration from `CurveBookNativeApp` (`R15.5`).
pub fn render_egui_shell_frame_with_host(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    host_close_dialog_open: bool,
) -> ShellAboutFrameReport {
    let ctx = ui.ctx().clone();
    ctx.enable_accesskit();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    tokens.apply_to_ctx(&ctx);
    // Synchronize the active pass's `Ui` style immediately so pass-0 widgets do not inherit stale context visuals.
    tokens.apply_to_ui(ui);
    meta.begin_pass(&ctx);

    let decision = HybridCoexistenceArbiter::evaluate_with_views(app, views, host_close_dialog_open);
    let mut report = ShellAboutFrameReport { decision: Some(decision), ..ShellAboutFrameReport::default() };

    views.sync_history_with_document(app.document());
    handle_shell_global_shortcuts(&ctx, app, views, &decision, intents);

    let content_rect = ctx.content_rect();
    let layout = tokens.metrics.solve_split_shell_layout(
        ShellLayoutInput {
            viewport_w_pt: content_rect.width(),
            viewport_h_pt: content_rect.height(),
            empty_workspace: app.document().total_sheet_count() == 0,
            navigator_open: app.nav.navigator_open,
            inspector_open: app.nav.inspector_open,
            family_open: app.family_open,
            leading_user_collapsed: views.leading_collapsed_rail,
            trailing_user_collapsed: views.trailing_collapsed_rail,
            leading_width_override_pt: views.leading_width_pt,
            trailing_width_override_pt: views.trailing_width_pt,
            host_draws_menu_bar: app.host_draws_menu_bar,
            is_rtl: crate::i18n::is_rtl_locale(&app.locale),
        },
        app.split.enabled,
    );
    views.last_shell_layout = Some(layout);

    let enabled = !decision.modal_backdrop_active;
    let project_title = app.session().file_path().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("CurveBook Project");
    let top_rep = render_top_chrome(
        ui,
        app,
        &mut views.top_chrome,
        TopChromeContext {
            tokens: &tokens,
            decision: &decision,
            layout,
            workspace_preset: views.workspace_preset,
            launcher_open: views.launcher_open,
            host_draws_menu_bar: app.host_draws_menu_bar,
            project_title,
            dirty: app.session().is_dirty(),
            search_recents: &views.search_recents,
        },
        meta,
        intents,
    );
    report.tier1_rect_pt = top_rep.tier1_rect_pt;
    report.tier2_rect_pt = top_rep.tier2_rect_pt;
    report.menu_popup_open = top_rep.menu_popup_open;

    let history_can_back = views.can_go_back();
    let history_can_forward = views.can_go_forward();
    let focused_secondary = views.center_stage.focused_split_pane == crate::egui_shell::center_stage::SplitPaneSlot::Secondary;
    let hover_readout = if app.split.enabled && focused_secondary {
        views.center_stage.secondary_canvas_view.hover_readout.clone().or_else(|| views.canvas_view.hover_readout.clone())
    } else if app.split.enabled {
        views.canvas_view.hover_readout.clone().or_else(|| views.center_stage.secondary_canvas_view.hover_readout.clone())
    } else {
        views.canvas_view.hover_readout.clone()
    };
    let foot_rep = render_status_footer(
        ui,
        app,
        views,
        StatusFooterContext { tokens: &tokens, layout, history_can_back, history_can_forward, hover_readout: hover_readout.as_ref(), enabled },
        meta,
        intents,
    );
    report.footer_rect_pt = foot_rep.footer_rect_pt;

    let lead_rep = render_leading_dock(
        ui,
        app,
        &mut views.leading_dock,
        LeadingDockContext {
            tokens: &tokens,
            layout,
            workspace_preset: views.workspace_preset,
            enabled,
            focused: app.nav.active_pane == ActiveWorkspacePane::Navigator,
        },
        meta,
        intents,
    );
    report.leading_dock_rect_pt = lead_rep.dock_rect_pt;

    let inspector_target_sheet = app.split.effective_inspector_target(app.document());
    let inspector_canvas_view = if app.split.inspector_targets_secondary_pane(app.document(), focused_secondary) {
        &views.center_stage.secondary_canvas_view
    } else {
        &views.canvas_view
    };
    let (selected_panel_index, selected_graph_annotation) = inspector_canvas_view.selection_for_sheet(inspector_target_sheet);

    let trail_rep = render_trailing_studio(
        ui,
        app,
        &mut views.trailing_studio,
        TrailingStudioContext {
            tokens: &tokens,
            layout,
            enabled,
            focused: app.nav.active_pane == ActiveWorkspacePane::Inspector,
            selected_panel_index,
            selected_graph_annotation,
        },
        meta,
        intents,
    );
    report.trailing_dock_rect_pt = trail_rep.dock_rect_pt;

    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tokens.surface_bg)).show(ui, |ui| {
        let canvas_rect = ui.available_rect_before_wrap();
        report.regions.canvas_rect_pt = Some(canvas_rect);
        let stage_rep = render_center_stage(
            ui,
            app,
            views,
            CenterStageContext { tokens: &tokens, decision: &decision, layout, focused_pane: app.nav.active_pane, enabled },
            meta,
            intents,
        );
        if let Some(primary) = stage_rep.primary_rect_pt {
            report.regions.canvas_rect_pt = Some(primary);
        }
        report.scrollbar_rect_pt = stage_rep.scrollbar_rect_pt;
        report.center_stage_handled_natively = stage_rep.handled_natively;
        report.grid_overlay_shown = stage_rep.grid_overlay_shown;
    });

    if !matches!(decision.modal_surface, HybridModalSurface::EguiStatParams | HybridModalSurface::EguiFitModal) {
        views.stats_modals.open_session_sheet_id = None;
        views.fit_modal.open_session_sheet_id = None;
    }

    macro_rules! record_modal {
        ($rep:expr) => {{
            let r = $rep;
            report.apply_modal_capture(r.dialog_rect_pt, r.outside_click_captured, r.outside_wheel_captured);
            r
        }};
    }

    match decision.modal_surface {
        HybridModalSurface::EguiAbout
        | HybridModalSurface::EguiAppModal
        | HybridModalSurface::HostUnsavedClose
        | HybridModalSurface::EguiPreferences
        | HybridModalSurface::EguiFindReplace => {
            record_modal!(render_app_modal(&ctx, app, &mut views.app_modals, host_close_dialog_open, &tokens, meta, intents));
        }
        HybridModalSurface::EguiOneWayAnova => {
            let anova_rep = record_modal!(render_one_way_anova_modal(&ctx, app, &mut views.anova_modal, &tokens, meta, intents));
            report.resolved_test_banner = anova_rep.resolved_test_banner;
        }
        HybridModalSurface::EguiCommandPalette => {
            record_modal!(render_command_palette(&ctx, app, &mut views.command_palette, &views.search_recents, &tokens, meta, intents));
        }
        HybridModalSurface::EguiManageVariables => {
            record_modal!(render_manage_vars_modal(&ctx, app, &mut views.manage_vars_modal, &tokens, meta, intents));
        }
        HybridModalSurface::EguiImportWizard => {
            record_modal!(render_import_or_convert_modal(&ctx, app, &mut views.import_wizard, &tokens, meta, intents));
        }
        HybridModalSurface::EguiTransformModal => {
            if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }) {
                record_modal!(render_exclude_with_reason_modal(&ctx, app, &mut views.transform_modals, &tokens, meta, intents));
            } else {
                record_modal!(render_transform_modal(&ctx, app, &mut views.transform_modals, &tokens, meta, intents));
            }
        }
        HybridModalSurface::EguiAnalyzeSheet => {
            record_modal!(render_analyze_sheet_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents));
        }
        HybridModalSurface::EguiDuplicateAnalysis => {
            record_modal!(render_duplicate_analysis_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents));
        }
        HybridModalSurface::EguiChangeDataAnalyzed => {
            record_modal!(render_change_data_analyzed_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents));
        }
        HybridModalSurface::EguiStatParams => {
            let st_rep = match &app.canvas_dialogs.active_dialog {
                ActiveDialog::RandomizeGroups | ActiveDialog::RandomSampleRows { .. } | ActiveDialog::SimulateDistributions => {
                    record_modal!(render_dialogs_r9_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents))
                }
                ActiveDialog::ResultViewer { .. } => record_modal!(render_result_viewer_modal(&ctx, app, &tokens, meta, intents)),
                _ => record_modal!(render_stat_params_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents)),
            };
            if !st_rep.resolved_test_banner.is_empty() {
                report.resolved_test_banner = st_rep.resolved_test_banner;
            }
        }
        HybridModalSurface::EguiFitModal => {
            record_modal!(render_fit_modal(&ctx, app, &mut views.fit_modal, &tokens, meta, intents));
        }
        HybridModalSurface::EguiHookPicker => {
            record_modal!(render_hook_constant_modal(&ctx, app, &mut views.fit_modal, &tokens, meta, intents));
        }
        HybridModalSurface::EguiPowerStudio => {
            record_modal!(render_power_studio_modal(&ctx, app, &mut views.power_studio, &tokens, meta, intents));
        }
        HybridModalSurface::EguiGraphModal => {
            record_modal!(render_graph_modal(&ctx, app, &mut views.graph_modals, &tokens, meta, intents));
        }
        HybridModalSurface::EguiNavDeleteConfirm => {
            record_modal!(crate::egui_shell::leading_dock::render_navigator_delete_confirm_modal(
                &ctx,
                &app.locale,
                &mut views.leading_dock,
                &tokens,
                meta,
                intents,
            ));
        }
        HybridModalSurface::EguiProgressModal | HybridModalSurface::None => {}
    }

    report
}

pub(crate) fn map_egui_key_to_shortcut_name(key: Key) -> Option<&'static str> {
    ALPHA_KEYS
        .iter()
        .chain(DIGIT_KEYS.iter())
        .find_map(|(k, name)| (*k == key).then_some(*name))
        .or_else(|| FUNCTION_KEYS.iter().find_map(|(k, name, _)| (*k == key).then_some(*name)))
        .or_else(|| PUNCT_KEYS.iter().find_map(|(k, name, _)| (*k == key).then_some(*name)))
}

const fn egui_mods_to_key_mods(m: Modifiers) -> KeyModifiers {
    KeyModifiers { shift: m.shift, ctrl: m.ctrl, alt: m.alt, meta: m.mac_cmd || (m.command && !m.ctrl) }
}

pub(crate) fn has_registered_shortcut(app: &UiApp, on_result_sheet: bool, key: Key, mods: Modifiers) -> bool {
    let Some(key_name) = map_egui_key_to_shortcut_name(key) else {
        return false;
    };
    let km = egui_mods_to_key_mods(mods);
    crate::interaction::build_shortcut_candidates(key_name, km)
        .into_iter()
        .any(|cand| app.shortcuts.handle_key_combo_for_context(&cand, on_result_sheet).is_some())
}

pub(crate) fn consume_matching_shortcut<F>(
    ctx: &egui::Context,
    app: &UiApp,
    on_result_sheet: bool,
    mut handles_command: F,
) -> Option<crate::menus::ShortcutMatch>
where
    F: FnMut(&str) -> bool,
{
    ctx.input_mut(|i| {
        let mut matched: Option<(usize, crate::menus::ShortcutMatch)> = None;
        for (idx, ev) in i.events.iter().enumerate() {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev
                && let Some(key_name) = map_egui_key_to_shortcut_name(*key)
            {
                let km = egui_mods_to_key_mods(*modifiers);
                for candidate in crate::interaction::build_shortcut_candidates(key_name, km) {
                    if let Some(m) = app.shortcuts.handle_key_combo_for_context(&candidate, on_result_sheet)
                        && handles_command(&m.command_id)
                    {
                        matched = Some((idx, m));
                        break;
                    }
                }
            }
            if matched.is_some() {
                break;
            }
        }
        if let Some((idx, m)) = matched {
            i.events.remove(idx);
            Some(m)
        } else {
            None
        }
    })
}

fn handle_shell_global_shortcuts(
    ctx: &egui::Context,
    app: &UiApp,
    views: &EguiShellViewState,
    decision: &HybridCoexistenceDecision,
    intents: &mut PendingUiIntents,
) {
    let egui_wants_kb = ctx.egui_wants_keyboard_input() || views.column_grid.inline_edit.is_some();
    if decision.modal_backdrop_active || egui_wants_kb {
        return;
    }

    let on_result_sheet = crate::interaction::resolve_result_sheet_for_shortcut(app, true).is_some();
    let on_searchable_sheet = !views.launcher_open
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && app
            .document()
            .active_sheet_id
            .as_ref()
            .is_some_and(|id| app.document().data_sheet(id).is_some() || app.document().result_sheet(id).is_some());
    let active_canvas_target = if !views.launcher_open && !app.nav.welcome.open && !app.nav.gallery.open {
        let doc = app.document();
        doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).cloned()
    } else {
        None
    };

    let has_renderable_figure = !views.launcher_open
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && (!app.document().graph_sheets.is_empty() || !app.document().layout_sheets.is_empty());
    let mut slide_copy_triggered = false;
    let mut copy_handled = false;
    let mut paste_handled = false;

    while let Some(m) = consume_matching_shortcut(ctx, app, on_result_sheet, |cmd| {
        if matches!(
            cmd,
            "project.save"
                | "project.saveAs"
                | "project.open"
                | "file.open"
                | "file.save"
                | "file.saveAs"
                | "window.new"
                | "window.close"
                | "app.quit"
        ) {
            return false;
        }
        if app.host_draws_menu_bar
            && !matches!(cmd, "edit.copy" | "edit.paste" | "edit.pasteImage" | "table.paste" | "ui.clipboard.copyForSlides")
            && (is_desktop_host_routed_command(cmd) || cmd == "app.preferences")
        {
            return false;
        }
        match cmd {
            "ui.editParams.open" => on_result_sheet,
            "ui.sheet.find" => on_searchable_sheet,
            "ui.clipboard.copyForSlides" => has_renderable_figure,
            "edit.paste" | "edit.pasteImage" | "table.paste" => active_canvas_target.is_some(),
            // Let LeadingDock / TrailingStudio / DataGrid own pane toggles, inline rename, and cell exclude/clear.
            "ui.navigator.toggle"
            | "ui.inspector.toggle"
            | "ui.family.toggle"
            | "sheet.rename"
            | "table.excludeCells"
            | "ui.grid.excludeWithReason"
            | "table.clearCells" => false,
            "ui.zoom.in" => {
                let custom = app.shortcuts.active_combo("ui.zoom.in").is_some_and(|c| c != "Cmd+=");
                active_canvas_target.is_some() || custom
            }
            "ui.zoom.out" => {
                let custom = app.shortcuts.active_combo("ui.zoom.out").is_some_and(|c| c != "Cmd+-");
                active_canvas_target.is_some() || custom
            }
            "ui.zoom.fitSheet" => {
                let doc = app.document();
                let canvas_moved = (app.canvas_dialogs.canvas.zoom - 1.0).abs() > 0.001
                    || app.canvas_dialogs.canvas.pan_pt.x.abs() > 0.001
                    || app.canvas_dialogs.canvas.pan_pt.y.abs() > 0.001
                    || views.canvas_view.pan_pt[0].abs() > 0.001
                    || views.canvas_view.pan_pt[1].abs() > 0.001;
                let ui_zoomed = (app.ui_scale() - 1.0).abs() > 0.001;
                doc.total_sheet_count() > 0 && (canvas_moved || !ui_zoomed)
            }
            _ => true,
        }
    }) {
        match m.command_id.as_str() {
            "ui.editParams.open" => intents.push_ui("ui.editParams.open", json!({})),
            "app.preferences" => intents.push(UiIntent::OpenPreferences { category: None }),
            "ui.sheet.find" => intents.push(UiIntent::OpenSheetFind),
            "ui.palette.open" => intents.push(UiIntent::SetPaletteOpen { open: true }),
            "sheet.createData" => intents.push(UiIntent::SetLauncherOpen { open: true }),
            "edit.redo" => {
                if app.session().redo_depth() > 0 {
                    intents.push_engine("edit.redo", json!({}));
                }
            }
            "edit.undo" => {
                if app.session().undo_depth() > 0 {
                    intents.push_engine("edit.undo", json!({}));
                }
            }
            "edit.copy" => {
                copy_handled = true;
                let dpi = ViewportMetrics::new(1.0, 1.0, f64::from(ctx.pixels_per_point())).dpi_for_base(144);
                intents.push(UiIntent::WorkspaceCopy { dpi });
            }
            "ui.clipboard.copyForSlides" => {
                slide_copy_triggered = true;
                intents.push(UiIntent::WorkspaceCopyForSlides { dpi: 300 });
            }
            "edit.paste" | "edit.pasteImage" | "table.paste" => {
                paste_handled = true;
                if let Some(ref target_id) = active_canvas_target {
                    intents.push(UiIntent::CanvasPaste { target_sheet_id: target_id.clone(), anchor_pt: None });
                }
            }
            "ui.split.toggle" => intents.push_ui("ui.split.toggle", json!({})),
            "ui.pane.cycle" => intents.push_ui("ui.pane.cycle", m.params),
            other => {
                let resolved_params = if m.params.is_null() { contextual_command_params(app, other) } else { m.params };
                if UI_COMMANDS.iter().any(|spec| spec.id == other) || other.starts_with("ui.") || other.starts_with("app.") {
                    intents.push_ui(other, resolved_params);
                } else if cmd::registry().get(other).is_some() {
                    intents.push_engine(other, resolved_params);
                }
            }
        }
    }

    // Standard Windows/Linux Ctrl+Y Redo convenience fallback when `Ctrl+Y`/`Cmd+Y` is not bound
    // to another command and `edit.redo` remains on its default binding (`Cmd+Shift+Z`).
    let redo_y_allowed = !cfg!(target_os = "macos")
        && app.shortcuts.active_combo("edit.redo") == Some("Cmd+Shift+Z")
        && !has_registered_shortcut(app, on_result_sheet, Key::Y, Modifiers::COMMAND)
        && !has_registered_shortcut(app, on_result_sheet, Key::Y, Modifiers::CTRL);
    if redo_y_allowed && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Y)) && app.session().redo_depth() > 0 {
        intents.push_engine("edit.redo", json!({}));
    }

    let copy_key_unbound = !slide_copy_triggered
        && !copy_handled
        && app.shortcuts.active_combo("edit.copy").is_none_or(|c| c == "Cmd+C")
        && !has_registered_shortcut(app, on_result_sheet, Key::C, Modifiers::COMMAND)
        && !has_registered_shortcut(app, on_result_sheet, Key::C, Modifiers::CTRL);
    let copy_default_bound = copy_key_unbound;
    let copy_triggered = ctx.input_mut(|i| {
        let mut hit = false;
        i.events.retain(|ev| {
            if matches!(ev, egui::Event::Copy) {
                if copy_default_bound {
                    hit = true;
                    false
                } else {
                    !(copy_handled || slide_copy_triggered)
                }
            } else {
                true
            }
        });
        if copy_key_unbound
            && (i.consume_key(Modifiers::COMMAND, Key::C) || i.consume_key(Modifiers::CTRL, Key::C) || i.consume_key(Modifiers::MAC_CMD, Key::C))
        {
            hit = true;
        }
        hit
    });
    if copy_triggered {
        let dpi = ViewportMetrics::new(1.0, 1.0, f64::from(ctx.pixels_per_point())).dpi_for_base(144);
        intents.push(UiIntent::WorkspaceCopy { dpi });
    }

    if let Some(target_id) = active_canvas_target {
        let paste_default_bound = !paste_handled
            && (app.shortcuts.active_combo("table.paste") == Some("Cmd+V") || app.shortcuts.active_combo("edit.paste") == Some("Cmd+V"));
        let paste_key_unbound = paste_default_bound
            && !has_registered_shortcut(app, on_result_sheet, Key::V, Modifiers::COMMAND)
            && !has_registered_shortcut(app, on_result_sheet, Key::V, Modifiers::CTRL);
        let paste_triggered = ctx.input_mut(|i| {
            let mut hit = false;
            i.events.retain(|ev| {
                if matches!(ev, egui::Event::Paste(_)) {
                    if paste_default_bound {
                        hit = true;
                        false
                    } else {
                        !paste_handled
                    }
                } else {
                    true
                }
            });
            if paste_key_unbound
                && (i.consume_key(Modifiers::COMMAND, Key::V) || i.consume_key(Modifiers::CTRL, Key::V) || i.consume_key(Modifiers::MAC_CMD, Key::V))
            {
                hit = true;
            }
            hit
        });
        if paste_triggered {
            intents.push(UiIntent::CanvasPaste { target_sheet_id: target_id, anchor_pt: None });
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn render_top_about_only_bar(ui: &mut egui::Ui, app: &UiApp, modal_open: bool, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let about_label = tr(&app.locale, "menu.help.about");
    egui::Panel::top("egui_shell_minimal_top_bar").show(ui, |ui| {
        ui.horizontal(|ui| {
            let resp = ui.add_enabled(!modal_open, egui::Button::new(about_label));
            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: "toolbar.help.about",
                    role: "button",
                    label: about_label,
                    command_id: Some("app.about"),
                    value: None,
                    enabled: !modal_open,
                    disabled_reason: modal_open.then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() && !modal_open {
                intents.push_ui("app.about", json!({}));
            }
        });
    });
}

#[cfg(any(test, feature = "test-support"))]
fn render_about_modal(
    ctx: &egui::Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut ShellAboutFrameReport,
) {
    let app_state = crate::egui_shell::app_modals::AppModalsEguiState::default();
    let app_rep = crate::egui_shell::app_modals::render_about_modal(ctx, app, &app_state, tokens, meta, intents);
    report.apply_modal_capture(app_rep.dialog_rect_pt, app_rep.outside_click_captured, app_rep.outside_wheel_captured);
}

#[cfg(any(test, feature = "test-support"))]
fn rects_overlap_significantly(a: egui::accesskit::Rect, b: egui::accesskit::Rect, tol: f64) -> bool {
    let x_overlap = a.x1.min(b.x1) - a.x0.max(b.x0);
    let y_overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    x_overlap > tol && y_overlap > tol
}

#[cfg(any(test, feature = "test-support"))]
fn is_interactive_child_role(role: egui::accesskit::Role) -> bool {
    use egui::accesskit::Role;
    matches!(
        role,
        Role::Button
            | Role::DefaultButton
            | Role::CheckBox
            | Role::RadioButton
            | Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::ComboBox
            | Role::Slider
            | Role::Tab
            | Role::MenuItem
            | Role::TreeItem
    )
}

/// Verify that no AccessKit `Role::ScrollBar` node overlaps any interactive child widget bounds
/// (`> 0.5 pt` in both X and Y).
///
/// # Errors
/// Returns a descriptive error naming the overlapping `ScrollBar` and child node on overlap.
#[cfg(any(test, feature = "test-support"))]
pub fn check_accesskit_scrollbar_non_overlap(update: &egui::accesskit::TreeUpdate) -> Result<(), String> {
    let mut scrollbars = Vec::new();
    let mut interactive = Vec::new();
    for (id, node) in &update.nodes {
        let Some(bounds) = node.bounds() else {
            continue;
        };
        if bounds.x1 <= bounds.x0 || bounds.y1 <= bounds.y0 {
            continue;
        }
        if node.role() == egui::accesskit::Role::ScrollBar {
            scrollbars.push((id, bounds, node.label().unwrap_or("scrollbar")));
        } else if is_interactive_child_role(node.role()) && !node.is_hidden() {
            interactive.push((id, bounds, node.label().unwrap_or("widget")));
        }
    }

    for (sb_id, sb_rect, sb_label) in &scrollbars {
        for (child_id, child_rect, child_label) in &interactive {
            if rects_overlap_significantly(*sb_rect, *child_rect, 0.5) {
                return Err(format!(
                    "AccessKit Role::ScrollBar ({sb_id:?}, `{sb_label}`) overlaps `{child_label}` ({child_id:?}): sb=({:.1},{:.1},{:.1},{:.1}) vs child=({:.1},{:.1},{:.1},{:.1})",
                    sb_rect.x0, sb_rect.y0, sb_rect.x1, sb_rect.y1, child_rect.x0, child_rect.y0, child_rect.x1, child_rect.y1,
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_logical_physical_roundtrip_across_1x_1_5x_2x() {
        for scale in [1.0_f32, 1.5, 2.0] {
            let logical = [123.5_f64, 456.25_f64];
            let phys = logical_pt_to_physical_px(logical, scale);
            let round = physical_px_to_logical_pt(phys, scale);
            assert!((round[0] - logical[0]).abs() < 1e-6);
            assert!((round[1] - logical[1]).abs() < 1e-6);
        }
    }

    fn build_xy_app_with_graphs() -> Result<UiApp, String> {
        let mut app = UiApp::new();
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setXCells", json!({ "sheet_id": "ds-1", "values": [1.0, 2.0, 3.0] })).map_err(|e| e.to_string())?;
        app.execute_command("table.setCells", json!({ "sheet_id": "ds-1", "rows": [[10.0], [20.0], [30.0]] })).map_err(|e| e.to_string())?;
        app.execute_command("graph.create", json!({ "sheet_id": "gs-a", "data_sheet_id": "ds-1", "title": "Graph A" })).map_err(|e| e.to_string())?;
        app.execute_command("graph.create", json!({ "sheet_id": "gs-b", "data_sheet_id": "ds-1", "title": "Graph B" })).map_err(|e| e.to_string())?;
        app.execute_command("sheet.select", json!({ "sheet_id": "gs-a" })).map_err(|e| e.to_string())?;
        app.sync_views();
        Ok(app)
    }

    #[test]
    fn m2_inspect_canvas_points_have_bounds_immediately_and_after_sheet_switch() -> Result<(), String> {
        let mut app = build_xy_app_with_graphs()?;
        let metrics = ViewportMetrics::default();

        // First ui.inspect without any prior pointer_gesture must populate bounds on canvas.point.*.
        let snap_a = inspect_live_egui_snapshot(&app, metrics, Some("canvas"), true);
        let pts_a: Vec<&UiWidgetNode> =
            snap_a.canvas_elements.as_deref().unwrap_or_default().iter().filter(|n| n.widget_id.starts_with("canvas.point.")).collect();
        assert_eq!(pts_a.len(), 3, "expected 3 plotted points on Graph A");
        assert!(pts_a.iter().all(|n| n.bounds.is_some()), "initial inspect missing canvas.point bounds: {pts_a:?}");

        // Immediate inspect after sheet.select to Graph B must also populate bounds.
        app.execute_session_ui_cmd("sheet.select", json!({ "sheet_id": "gs-b" })).map_err(|e| e.to_string())?;
        let snap_b = inspect_live_egui_snapshot(&app, metrics, Some("canvas"), true);
        let pts_b: Vec<&UiWidgetNode> =
            snap_b.canvas_elements.as_deref().unwrap_or_default().iter().filter(|n| n.widget_id.starts_with("canvas.point.")).collect();
        assert_eq!(pts_b.len(), 3, "expected 3 plotted points on Graph B");
        assert!(pts_b.iter().all(|n| n.bounds.is_some()), "post-switch inspect missing canvas.point bounds: {pts_b:?}");
        Ok(())
    }

    #[test]
    fn m2_inspect_scope_tabs_includes_modal_tabs() -> Result<(), String> {
        let mut app = build_xy_app_with_graphs()?;
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open" })).map_err(|e| e.to_string())?;
        let metrics = ViewportMetrics::default();

        let snap_all = inspect_live_egui_snapshot(&app, metrics, Some("all"), true);
        let snap_tabs = inspect_live_egui_snapshot(&app, metrics, Some("tabs"), true);
        let all_edit_tabs: Vec<&str> =
            snap_all.tabs.as_deref().unwrap_or_default().iter().map(|t| t.widget_id.as_str()).filter(|w| w.starts_with("edit.tab.")).collect();
        let scope_edit_tabs: Vec<&str> =
            snap_tabs.tabs.as_deref().unwrap_or_default().iter().map(|t| t.widget_id.as_str()).filter(|w| w.starts_with("edit.tab.")).collect();
        assert!(!all_edit_tabs.is_empty(), "expected edit.tab.* in scope=all");
        assert_eq!(scope_edit_tabs, all_edit_tabs, "scope=tabs must preserve modal dialog.tabs");
        Ok(())
    }

    #[test]
    fn m2_press_key_enter_commits_modals_without_breaking_multiline_or_wizards() -> Result<(), String> {
        // 1. SimulateDistributions commits on Enter via egui::Context::run.
        let mut app = build_xy_app_with_graphs()?;
        app.canvas_dialogs.open_simulate_distributions();
        let before_sheets = app.document().data_sheets.len();
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "SimulateDistributions must close on Enter");
        assert_eq!(app.document().data_sheets.len(), before_sheets + 1, "SimulateDistributions must create DataSheet on Enter");

        // 2. FormatAxes (graph_modals) commits and closes on Enter.
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "FormatAxes must close on Enter");

        // 3. Preferences (app_modals) saves and closes on Enter.
        app.egui_views.app_modals.preferences_open = true;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(!app.egui_views.app_modals.preferences_open, "Preferences must close on Enter");

        // 4. ExcludeWithReason multiline textbox keeps Enter as newline without submitting, and Cmd+Enter submits.
        app.canvas_dialogs.open_exclude_with_reason(curvebook_doc::SheetId::from("ds-1"), vec![curvebook_tools::GridCellCoord::new(0, 0, 0)]);
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "exclude.note" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "Line1" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }),
            "plain Enter in multiline exclude.note must keep modal open"
        );
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter", "modifiers": { "meta": true } })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "Cmd+Enter must commit ExcludeWithReason");

        // 5. PowerWizard step-by-step wizard advances from Stage 1 -> Stage 2 on Enter instead of skipping/erroring.
        app.canvas_dialogs.open_power_wizard();
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.power_wizard.stage, crate::power_wizard::PowerWizardStage::EffectSize);
        Ok(())
    }

    #[test]
    fn m2_power_stage_1_based_switch_tab_and_disabled_hover_and_retired_answer_dialog() -> Result<(), String> {
        let mut app = build_xy_app_with_graphs()?;
        app.canvas_dialogs.open_power_wizard();

        // Disabled hover/move succeeds while click fails with Conflict.
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "hover", "widget_id": "dialog.button.back" }))
            .map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "move", "widget_id": "dialog.button.back" }))
            .map_err(|e| e.to_string())?;
        let click_err = dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.back" }))
            .err()
            .ok_or_else(|| String::from("expected Conflict clicking disabled back button"))?;
        assert_eq!(click_err.code, ErrorCode::Conflict);

        // 1-based power.stage.* tabs align with ui.inspect.
        let s3 = dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab_id": "power.stage.3" })).map_err(|e| e.to_string())?;
        assert_eq!(s3.get("stage_index").and_then(Value::as_u64), Some(2));
        let s4 = dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab_id": "power.stage.4" })).map_err(|e| e.to_string())?;
        assert_eq!(s4.get("stage_index").and_then(Value::as_u64), Some(3));
        let s1 = dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab_id": "power.stage.1" })).map_err(|e| e.to_string())?;
        assert_eq!(s1.get("stage_index").and_then(Value::as_u64), Some(0));

        // Retired ui.answer_dialog returns BadInput in normal mode.
        let retired_err = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))
            .err()
            .ok_or_else(|| String::from("expected BadInput for retired ui.answer_dialog"))?;
        assert_eq!(retired_err.code, ErrorCode::BadInput, "got {retired_err:?}");
        app.canvas_dialogs.close_dialog();
        Ok(())
    }

    /// `R0.36`: `ui.type_text` aimed at an X cell of the active sheet rebinds the selection to that
    /// sheet even when the previous selection belonged to another sheet (split view).
    #[test]
    fn r0_36_type_text_x_cell_binds_selection_to_active_sheet() -> Result<(), String> {
        let mut app = UiApp::new();
        for sid in ["xy-a", "xy-b"] {
            app.execute_command("sheet.createData", json!({ "sheet_id": sid, "kind": "xy", "cols": 1, "rows": 4, "auto_graph": false }))
                .map_err(|e| e.to_string())?;
        }
        app.execute_command("sheet.select", json!({ "sheet_id": "xy-b" })).map_err(|e| e.to_string())?;
        let sheet_a = curvebook_doc::SheetId::from("xy-a");
        let sheet_b = curvebook_doc::SheetId::from("xy-b");
        app.grid.select_x_cell_for_sheet(&sheet_a, 0, 0);
        assert_eq!(app.grid.active_x_cell_for_sheet(&sheet_b), None);

        let (is_x, _) = super::prepare_grid_type_text_target(&mut app, Some("grid.x.2.0"), None, false, false).map_err(|e| e.to_string())?;
        assert!(is_x);
        assert_eq!(app.grid.active_x_cell_for_sheet(&sheet_b), Some((2, 0)), "X-cell selection must bind to the active sheet");
        assert_eq!(app.grid.active_x_cell_for_sheet(&sheet_a), None, "the other sheet must no longer own the selection");
        Ok(())
    }

    #[test]
    fn m2_type_text_on_linked_data_region_returns_conflict() -> Result<(), String> {
        let mut app = UiApp::new();
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-lnk", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setCells", json!({ "sheet_id": "ds-lnk", "rows": [[250.0], [350.0]] })).map_err(|e| e.to_string())?;
        let mut doc = app.document().clone();
        doc.data_sheet_mut(&curvebook_doc::SheetId::from("ds-lnk"))
            .ok_or_else(|| String::from("missing ds-lnk"))?
            .add_linked_region(curvebook_doc::LinkedDataRegion {
                id: String::from("link-1"),
                spec: curvebook_doc::ImportLinkSpec {
                    source: curvebook_doc::LinkSource::Clipboard,
                    filter: Default::default(),
                    placement: Default::default(),
                },
                target_range: curvebook_doc::SheetCellRange::new(curvebook_doc::CellCoord::new(0, 0, 0), curvebook_doc::CellCoord::new(1, 0, 0)),
                provenance_summary: String::from("Linked"),
            })
            .map_err(|e| e.to_string())?;
        doc.active_sheet_id = Some(curvebook_doc::SheetId::from("ds-lnk"));
        app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        app.sync_views();

        let err = dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.0.0.0", "text": "999", "submit": true }))
            .err()
            .ok_or_else(|| String::from("expected Conflict typing into LinkedDataRegion cell"))?;
        assert_eq!(err.code, ErrorCode::Conflict, "got {err:?}");

        // Sequential focus (click then type_text without widget_id) must also return Conflict.
        let seq_err = dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "999", "submit": true }))
            .err()
            .ok_or_else(|| String::from("expected Conflict on focused LinkedDataRegion cell"))?;
        assert_eq!(seq_err.code, ErrorCode::Conflict, "got {seq_err:?}");

        let cell_val = app
            .document()
            .data_sheet(&curvebook_doc::SheetId::from("ds-lnk"))
            .and_then(|ds| ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)))
            .and_then(|c| c.active_number());
        assert_eq!(cell_val, Some(250.0), "LinkedDataRegion cell must remain unmodified");
        Ok(())
    }

    #[test]
    fn m2_iter3_pinch_short_drag_human_path_import_wizard_and_format_points() -> Result<(), String> {
        let mut app = build_xy_app_with_graphs()?;

        // 1. Pinch rejects scale_delta <= 0 / NaN with BadInput and accepts positive finite values.
        for bad_delta in [json!(0.0), json!(-1.5), json!("bad")] {
            let err =
                dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "pinch", "x": 400.0, "y": 300.0, "scale_delta": bad_delta }))
                    .err()
                    .ok_or_else(|| format!("expected BadInput for scale_delta={bad_delta}"))?;
            assert_eq!(err.code, ErrorCode::BadInput, "got {err:?}");
        }
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "pinch", "x": 400.0, "y": 300.0, "scale_delta": 1.25 }))
            .map_err(|e| e.to_string())?;

        // 2. human_path_only blocks to_widget_id / end_widget_id as well as widget_id.
        app.set_human_path_only(true);
        let to_wid_err = dispatch_gui_action(
            &mut app,
            "ui.pointer_gesture",
            json!({ "action": "drag", "x": 100.0, "y": 100.0, "to_widget_id": "nav.sheet.ds-1" }),
        )
        .err()
        .ok_or_else(|| String::from("expected Conflict for to_widget_id under human_path_only"))?;
        assert_eq!(to_wid_err.code, ErrorCode::Conflict, "got {to_wid_err:?}");
        assert_eq!(to_wid_err.details, Some(json!({ "code": "human_path_only_policy" })));
        app.set_human_path_only(false);

        // 3. Short drags (7.5pt and 9.0pt) on canvas.layout_panel.0 genuinely translate bounds_pt.x.
        app.execute_command("layout.create", json!({ "sheet_id": "ly-m2", "title": "Layout M2" })).map_err(|e| e.to_string())?;
        let item = curvebook_doc::LayoutItem::LiveGraph {
            id: String::from("item-1"),
            graph_sheet_id: curvebook_doc::SheetId::from("gs-a"),
            bounds_pt: curvebook_doc::RectPt::new(40.0, 40.0, 200.0, 150.0),
            scale_percent: 100.0,
            badge_override: None,
        };
        app.execute_command("layout.setSpec", json!({ "sheet_id": "ly-m2", "spec": { "items": [item] } })).map_err(|e| e.to_string())?;
        app.execute_command("sheet.select", json!({ "sheet_id": "ly-m2" })).map_err(|e| e.to_string())?;
        app.sync_views();

        let metrics = ViewportMetrics::default();
        let read_x = |a: &UiApp| -> Result<f64, String> {
            a.document()
                .layout_sheet(&curvebook_doc::SheetId::from("ly-m2"))
                .and_then(|ls| ls.spec.items.first())
                .and_then(|it| match it {
                    curvebook_doc::LayoutItem::LiveGraph { bounds_pt, .. } => Some(bounds_pt.x),
                    _ => None,
                })
                .ok_or_else(|| String::from("missing layout item 0"))
        };
        for dist_pt in [7.5_f64, 9.0_f64] {
            let before_x = read_x(&app)?;
            let snap = inspect_live_egui_snapshot(&app, metrics, Some("canvas"), true);
            let panel = snap
                .canvas_elements
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|n| n.widget_id == "canvas.layout_panel.0")
                .and_then(|n| n.bounds.as_ref())
                .ok_or_else(|| String::from("missing canvas.layout_panel.0 bounds"))?;
            let cx = panel[0] + panel[2] * 0.5;
            let cy = panel[1] + panel[3] * 0.5;
            dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "drag", "x": cx, "y": cy, "to_x": cx + dist_pt, "to_y": cy }))
                .map_err(|e| e.to_string())?;
            let after_x = read_x(&app)?;
            assert!(after_x > before_x, "short {dist_pt}pt drag produced zero delta_x ({before_x} -> {after_x})");
        }
        Ok(())
    }

    #[test]
    fn m2_iter3_singleline_textbox_enter_does_not_dismiss_fit_stat_or_anova_modals() -> Result<(), String> {
        let mut app = build_xy_app_with_graphs()?;
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-fit", "kind": "xy", "cols": 1, "rows": 6, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setXCells", json!({ "sheet_id": "ds-fit", "values": [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0] }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setCells", json!({ "sheet_id": "ds-fit", "rows": [[10.0], [15.0], [35.0], [65.0], [88.0], [95.0]] }))
            .map_err(|e| e.to_string())?;

        let open_edit_for_entry = |a: &mut UiApp, sheet_id: &str, entry_id: &str| -> Result<curvebook_doc::SheetId, String> {
            let doc = a.document().clone();
            assert!(a.canvas_dialogs.open_analyze_dialog(&doc, &curvebook_doc::SheetId::from(sheet_id)));
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "analyze.search" })).map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.type_text", json!({ "text": entry_id })).map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": format!("analyze.entry.{entry_id}") }))
                .map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "modal.ok" })).map_err(|e| e.to_string())?;
            if let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &a.canvas_dialogs.active_dialog {
                return Ok(result_sheet_id.clone());
            }
            let rs_id = a.document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("missing result sheet"))?;
            let doc2 = a.document().clone();
            assert!(a.canvas_dialogs.open_edit_parameters(&doc2, &rs_id), "failed to open edit parameters for {rs_id}");
            Ok(rs_id)
        };

        // 1. FitModal (`fit.eq.search`): Enter inside singleline textbox keeps modal open; unfocused Enter commits.
        let nl_rs_id = open_edit_for_entry(&mut app, "ds-fit", "fit.nonlinear")?;
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "fit.eq.search" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "4pl" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
            "Enter inside fit.eq.search must keep FitModal open"
        );
        // After TextEdit released focus on Enter, unfocused Enter commits the FitModal.
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "unfocused Enter must commit FitModal");

        // 2. ConstantHookPicker (`hook.search_input`): Enter inside search textbox keeps modal open.
        let doc = app.document().clone();
        app.canvas_dialogs.open_hook_picker(&doc, &nl_rs_id, Some(String::from("Top")));
        if app.canvas_dialogs.hook_picker.options.is_empty() {
            app.canvas_dialogs.hook_picker.options.push(crate::dialogs_canvas::AvailableHookOption {
                label: String::from("Mock Info"),
                preview_value: String::from("1.0"),
                template_token: String::from("{{info.mock}}"),
                source: curvebook_doc::HookSource::InfoConstant {
                    info_sheet_id: Some(curvebook_doc::SheetId::from("info-1")),
                    constant_name: String::from("mock"),
                },
            });
        }
        app.canvas_dialogs.hook_picker.selected_index = Some(0);
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "hook.search_input" }))
            .map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "Mock" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ConstantHookPicker { .. }),
            "Enter inside hook.search_input must keep ConstantHookPicker open"
        );
        app.canvas_dialogs.close_dialog();

        // 3. stat_params (`edit.custom_title` and `edit.two_group.hypothetical`): Enter inside textbox keeps modal open; unfocused Enter commits.
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-col", "kind": "column", "cols": 3, "rows": 4, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command(
            "table.setCells",
            json!({ "sheet_id": "ds-col", "rows": [[1.0, 2.0, 5.0], [3.0, 4.0, 6.0], [5.0, 6.0, 7.0], [7.0, 8.0, 9.0]] }),
        )
        .map_err(|e| e.to_string())?;
        let _tg_rs_id = open_edit_for_entry(&mut app, "ds-col", "stats.two_group")?;
        if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
            && let curvebook_doc::AnalysisKind::TwoGroupComparison(mut tg) = spec.kind.clone()
        {
            tg.method = curvebook_doc::TwoGroupMethod::OneSampleT { hypothetical: 0.0 };
            spec.update_kind(curvebook_doc::AnalysisKind::TwoGroupComparison(tg));
            app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
        }

        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.two_group.hypothetical" }))
            .map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "42" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
            "Enter inside edit.two_group.hypothetical must keep stat_params open"
        );

        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.custom_title" }))
            .map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "Custom" })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
            "Enter inside edit.custom_title must keep stat_params open"
        );
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "unfocused Enter must commit stat_params");

        // 4. stat_anova_modal (`OneWayAnova`): unfocused Enter commits cleanly.
        let _av_rs_id = open_edit_for_entry(&mut app, "ds-col", "stats.one_way_anova")?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "unfocused Enter must commit OneWayAnova");
        Ok(())
    }

    #[test]
    fn m2_iter4_render_modal_textbox_row_enter_commits_without_refocus_trap() -> Result<(), String> {
        let mut app = build_xy_app_with_graphs()?;
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-fit", "kind": "xy", "cols": 1, "rows": 6, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setXCells", json!({ "sheet_id": "ds-fit", "values": [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0] }))
            .map_err(|e| e.to_string())?;
        app.execute_command("table.setCells", json!({ "sheet_id": "ds-fit", "rows": [[10.0], [15.0], [35.0], [65.0], [88.0], [95.0]] }))
            .map_err(|e| e.to_string())?;
        app.execute_command("sheet.createData", json!({ "sheet_id": "ds-col", "kind": "column", "cols": 3, "rows": 4, "auto_graph": false }))
            .map_err(|e| e.to_string())?;
        app.execute_command(
            "table.setCells",
            json!({ "sheet_id": "ds-col", "rows": [[1.0, 2.0, 5.0], [3.0, 4.0, 6.0], [5.0, 6.0, 7.0], [7.0, 8.0, 9.0]] }),
        )
        .map_err(|e| e.to_string())?;

        let open_edit = |a: &mut UiApp, sheet_id: &str, entry_id: &str| -> Result<curvebook_doc::SheetId, String> {
            let doc = a.document().clone();
            assert!(a.canvas_dialogs.open_analyze_dialog(&doc, &curvebook_doc::SheetId::from(sheet_id)));
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "analyze.search" })).map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.type_text", json!({ "text": entry_id })).map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": format!("analyze.entry.{entry_id}") }))
                .map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "modal.ok" })).map_err(|e| e.to_string())?;
            if let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &a.canvas_dialogs.active_dialog {
                return Ok(result_sheet_id.clone());
            }
            let rs_id = a.document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("missing result sheet"))?;
            let doc2 = a.document().clone();
            assert!(a.canvas_dialogs.open_edit_parameters(&doc2, &rs_id), "open edit params {rs_id}");
            Ok(rs_id)
        };

        let verify_textbox_enter_commit = |a: &mut UiApp, tab_id: Option<&str>, widget_id: &str, text: &str| -> Result<(), String> {
            if let Some(tab) = tab_id {
                dispatch_gui_action(a, "ui.switch_tab", json!({ "tab_id": tab })).map_err(|e| e.to_string())?;
            }
            dispatch_gui_action(a, "ui.pointer_gesture", json!({ "action": "click", "widget_id": widget_id })).map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.type_text", json!({ "text": text })).map_err(|e| e.to_string())?;
            dispatch_gui_action(a, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
            assert!(
                matches!(a.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
                "1st Enter inside {widget_id} must keep modal open"
            );
            let snap = inspect_live_egui_snapshot(a, ViewportMetrics::default(), Some("dialog"), false);
            assert_eq!(snap.focus_widget_id, None, "1st Enter must clear textbox focus on {widget_id}");
            dispatch_gui_action(a, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
            assert_eq!(a.canvas_dialogs.active_dialog, ActiveDialog::None, "2nd Enter must commit modal after {widget_id}");
            Ok(())
        };

        // 1. FitModal (`fit.nonlinear` -> `fit.tab.output` -> `fit.diagnostics.r2.input`)
        let pending_nl_id = open_edit(&mut app, "ds-fit", "fit.nonlinear")?;
        assert!(app.document().result_sheet(&pending_nl_id).is_none(), "R20.20 defers creation until FitModal commit");
        verify_textbox_enter_commit(&mut app, Some("fit.tab.output"), "fit.diagnostics.r2.input", "0.95")?;
        let nl_rs_id = app.document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("missing committed fit sheet"))?;
        let committed_r2 = app.document().result_sheet(&nl_rs_id).and_then(|rs| match &rs.analysis.kind {
            curvebook_doc::AnalysisKind::NonlinearRegression(fit) => fit.flag_r_squared_below,
            _ => None,
        });
        assert!(committed_r2.is_some_and(|v| (v - 0.95).abs() < 1e-9), "expected flag_r_squared_below == 0.95, got {committed_r2:?}");

        // 2. stat_params (`stats.two_group` -> `edit.custom_title`)
        let tg_rs_id = open_edit(&mut app, "ds-col", "stats.two_group")?;
        verify_textbox_enter_commit(&mut app, None, "edit.custom_title", "Custom")?;
        assert_eq!(app.document().result_sheet(&tg_rs_id).and_then(|rs| rs.analysis.custom_title.as_deref()), Some("Custom"));

        // 3. stat_params (`fit.linear` -> `edit.tab.range` -> `fit.lr.range.input.x_min`)
        let lr_rs_id = open_edit(&mut app, "ds-fit", "fit.linear")?;
        verify_textbox_enter_commit(&mut app, Some("edit.tab.range"), "fit.lr.range.input.x_min", "-1.5")?;
        let committed_range = app.document().result_sheet(&lr_rs_id).and_then(|rs| match &rs.analysis.kind {
            curvebook_doc::AnalysisKind::LinearRegression(lr) => lr.x_range_filter,
            _ => None,
        });
        assert!(
            matches!(committed_range, Some((Some(lo), None)) if (lo - (-1.5)).abs() < 1e-9),
            "expected x_range_filter == Some((Some(-1.5), None)), got {committed_range:?}"
        );

        // 4. Even when `app.focus_widget_id` is explicitly pre-seeded before typing, 1st Enter clears stale focus
        // so 2nd Enter commits without looping in `ensure_egui_focus_if_needed`.
        let doc_tg = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc_tg, &tg_rs_id), "reopen two-group stat_params");
        app.focus_widget_id = Some(String::from("edit.custom_title"));
        dispatch_gui_action(&mut app, "ui.type_text", json!({ "text": "Preseeded", "clear_first": true })).map_err(|e| e.to_string())?;
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
        assert_eq!(app.focus_widget_id, None, "pre-seeded focus_widget_id must clear when TextEdit releases focus on Enter");
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
        assert_eq!(app.document().result_sheet(&tg_rs_id).and_then(|rs| rs.analysis.custom_title.as_deref()), Some("Preseeded"));

        // 5. Parity with gui_window: passing both `widget_id` and `x`/`y` to `ui.pointer_gesture` returns BadInput.
        let ambig_err =
            dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "toolbar.analyze", "x": 10.0, "y": 10.0 }))
                .err()
                .ok_or_else(|| String::from("expected BadInput for ambiguous widget_id + x/y"))?;
        assert_eq!(ambig_err.code, ErrorCode::BadInput, "got {ambig_err:?}");
        Ok(())
    }
}
