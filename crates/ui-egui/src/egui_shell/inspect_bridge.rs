//! AccessKit + `FrameInspectMeta` -> `UiTreeSnapshot` bridge & Workspace Coexistence Arbiter (`R15.1`–`R15.5`).
//!
//! Coordinates native `egui` workspace surfaces and modal focus ownership, building settled
//! `ui.inspect` [`UiTreeSnapshot`] output, enforcing `100%` `ZLayer::ModalBackdrop` outside
//! capture, and verifying AccessKit `Role::ScrollBar` non-overlap.

use std::collections::BTreeMap;

use curvebook_doc::TableArchetype;
use curvebook_engine::EngineError;
use egui::{Color32, Key, Modifiers, Pos2, Rect, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
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
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, ShellLayoutInput, Tokens, UiIntentOutcome,
};
use crate::inspect::{
    GridSelectionSnapshot, UiDialogSnapshot, UiMenuSnapshot, UiNavigatorSheetNode, UiTreeSnapshot, UiWidgetNode, ZLayer, active_dialog_slug,
    inspect_semantic_snapshot,
};
use crate::interaction::{KeyModifiers, UiEventOutcome, ViewportMetrics};
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
    /// Native `egui` 5-category `Preferences` modal (`R15.4`).
    EguiPreferences,
    /// Native `egui` `Recovery` session restore modal (`R15.4`).
    EguiRecoveryDialog,
    /// Native `egui` `Find & Replace` modal (`R15.4`).
    EguiFindReplace,
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
        clipped_rect: viewport,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
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
        clipped_rect: meta.viewport_rect.intersect(dlg_rect),
        z_layer: spec.z_dialog,
        ellipsized: false,
        full_label: None,
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
        } else if views.app_modals.recovery_open {
            HybridModalSurface::EguiRecoveryDialog
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
    #[must_use]
    pub fn allows_cmd_t_edit_params(app: &UiApp, decision: &HybridCoexistenceDecision, egui_wants_keyboard: bool) -> bool {
        !decision.modal_backdrop_active && !egui_wants_keyboard && crate::interaction::resolve_result_sheet_for_shortcut(app, true).is_some()
    }
}

fn classify_active_dialog_surface(app: &UiApp) -> HybridModalSurface {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::None => HybridModalSurface::None,
        ActiveDialog::About => HybridModalSurface::EguiAbout,
        ActiveDialog::ExportFigure { .. } | ActiveDialog::Print { .. } => HybridModalSurface::EguiAppModal,
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

/// Result emitted by [`render_minimal_shell_and_about`] and [`render_egui_shell_frame`] for
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
}

/// Convert a logical coordinate `[x_pt, y_pt]` to physical pixels using `pixels_per_point`.
#[must_use]
pub fn logical_pt_to_physical_px(pt: [f64; 2], pixels_per_point: f32) -> [f64; 2] {
    let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { f64::from(pixels_per_point) } else { 1.0 };
    [pt[0] * scale, pt[1] * scale]
}

/// Convert a physical pixel coordinate `[x_px, y_px]` to logical points using `pixels_per_point`.
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
        "tabs" => wid.starts_with("tab.") || wid.starts_with("sheet.tab.") || wid.starts_with("result.tab."),
        "canvas" => {
            wid.starts_with("canvas.")
                || wid.starts_with("center.pane.")
                || wid == "split.divider"
                || wid.starts_with("result.diagnostic_banner")
                || wid.starts_with("info.")
                || wid.starts_with("welcome.")
        }
        "grid" => wid.starts_with("grid."),
        "dialog" => {
            entry.z_layer == ZLayer::ModalDialog
                || entry.z_layer == ZLayer::ModalBackdrop
                || wid.starts_with("dialog.")
                || wid.starts_with("modal.")
                || wid.starts_with("edit.")
                || wid.starts_with("palette.")
        }
        "navigator" => wid.starts_with("nav."),
        "inspector" => wid.starts_with("inspector."),
        "status" => wid.starts_with("status."),
        _ => false,
    }
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

fn build_dialog_snapshot_from_meta(
    app: &UiApp,
    meta: &FrameInspectMeta,
    ak: &AccessKitLookup<'_>,
    _metrics: ViewportMetrics,
    include_bounds: bool,
    baseline_dialog: Option<&UiDialogSnapshot>,
) -> Option<UiDialogSnapshot> {
    let dialog_entry = meta
        .find_by_widget_id("modal.dialog")
        .or_else(|| meta.find_by_widget_id("dialog.about"))
        .or_else(|| meta.entries.iter().find(|e| (e.widget_id.starts_with("dialog.") || e.widget_id.starts_with("modal.")) && e.role == "dialog"));
    let has_shell_modal = dialog_entry.is_some() || app.grid.variables_dialog.open || app.grid.import_wizard.open;
    if app.canvas_dialogs.active_dialog == ActiveDialog::None && !has_shell_modal {
        return None;
    }

    let dialog_kind = if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        active_dialog_slug(&app.canvas_dialogs.active_dialog)
    } else if app.grid.variables_dialog.open {
        String::from("manage_variables")
    } else if app.grid.import_wizard.open {
        String::from("import_paste_special")
    } else {
        dialog_entry.map(|e| e.widget_id.trim_start_matches("dialog.").to_owned()).unwrap_or_else(|| String::from("shell_modal"))
    };

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
        .or_else(|| baseline_dialog.map(|d| d.title.clone()))
        .unwrap_or_else(|| tr(&app.locale, "menu.help.about").to_owned());

    let bounds = if include_bounds {
        dialog_entry.and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor)).or_else(|| baseline_dialog.and_then(|d| d.bounds))
    } else {
        None
    };

    let mut tabs = Vec::new();
    let mut controls = Vec::new();
    let mut buttons = Vec::new();
    for entry in &meta.entries {
        let wid = entry.widget_id.as_str();
        if matches!(wid, "modal.backdrop" | "modal.dialog" | "dialog.about" | "modal.title") || entry.role == "dialog" {
            continue;
        }
        if !(entry.z_layer == ZLayer::ModalDialog || wid.starts_with("dialog.") || wid.starts_with("modal.") || wid.starts_with("edit.")) {
            continue;
        }
        let node = widget_node_from_meta(entry, ak, meta.scale_factor, include_bounds);
        if wid.starts_with("edit.tab.") || wid.contains(".tab.") || node.role == "tab" {
            tabs.push(node);
        } else if wid.starts_with("dialog.button.")
            || matches!(
                wid,
                "modal.ok"
                    | "modal.cancel"
                    | "modal.apply"
                    | "modal.close"
                    | "modal.next"
                    | "modal.back"
                    | "modal.create"
                    | "modal.restore"
                    | "modal.discard"
            )
        {
            buttons.push(node);
        } else {
            controls.push(node);
        }
    }

    if let Some(base) = baseline_dialog {
        for tab in &base.tabs {
            if !tabs.iter().any(|n| n.widget_id == tab.widget_id) {
                tabs.push(tab.clone());
            }
        }
        for ctrl in &base.controls {
            if !controls.iter().any(|n| n.widget_id == ctrl.widget_id) {
                controls.push(ctrl.clone());
            }
        }
        for btn in &base.buttons {
            if !buttons.iter().any(|n| n.widget_id == btn.widget_id) {
                buttons.push(btn.clone());
            }
        }
    }

    Some(UiDialogSnapshot {
        dialog_kind,
        title,
        target_sheet_id: baseline_dialog.and_then(|d| d.target_sheet_id.clone()),
        stage: baseline_dialog.and_then(|d| d.stage.clone()),
        bounds,
        tabs,
        controls,
        buttons,
    })
}

fn configure_offscreen_egui_ctx(app: &UiApp, metrics: ViewportMetrics) -> (egui::Context, f32, f32, f32) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);

    #[allow(clippy::cast_possible_truncation)]
    let w_pt = metrics.logical_width.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let h_pt = metrics.logical_height.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    let zoom = app.ui_scale();
    let native_ppp = (ppp / zoom).max(0.05);
    ctx.options_mut(|o| {
        o.zoom_with_keyboard = false;
        o.zoom_factor = zoom;
    });
    (ctx, w_pt, h_pt, native_ppp)
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
    let (w_pt, h_pt, native_ppp) = geom;
    let mut pending_events = initial_events;
    for _pass in 0..6_u32 {
        let events = std::mem::take(&mut pending_events);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(w_pt, h_pt))),
            time: Some(time_sec),
            predicted_dt: 1.0 / 60.0,
            focused: true,
            events,
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(native_ppp), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut intents = PendingUiIntents::default();
        let mut settled = false;
        let mut output = ctx.run_ui(raw_input, |ui| {
            let _ = render_egui_shell_frame(ui, app, views, meta, &mut intents);
            settled = !ui.ctx().will_discard();
        });
        output.textures_delta.clear();
        if settled && !output.platform_output.requested_discard() {
            app.egui_views = views.clone();
            let outcomes = intents.commit_if_settled(app, false)?;
            *views = app.egui_views.clone();
            return Ok(outcomes);
        }
    }
    Ok(Vec::new())
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
    let egui_mods = Modifiers {
        alt: modifiers.alt,
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        mac_cmd: modifiers.meta,
        command: modifiers.meta || modifiers.ctrl,
    };

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
    if dist > 8.0 {
        t += 0.02;
        let trigger = start + delta.normalized() * 6.0;
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

/// Run a settled offscreen `egui` frame (reusing cached font definitions) and construct a canonical
/// [`UiTreeSnapshot`] (`ui.inspect`) directly from settled `FrameInspectMeta` and AccessKit state.
#[must_use]
pub fn inspect_live_egui_snapshot(app: &UiApp, metrics: ViewportMetrics, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    let (ctx, w_pt, h_pt, native_ppp) = configure_offscreen_egui_ctx(app, metrics);

    let mut views = app.egui_views.clone();
    let mut meta = FrameInspectMeta::default();
    for _pass in 0..8_u32 {
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(w_pt, h_pt))),
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
        if settled {
            return build_ui_tree_snapshot_from_egui(app, &meta, output.platform_output.accesskit_update.as_ref(), scope, include_bounds, metrics);
        }
    }
    inspect_semantic_snapshot(app, metrics, scope, include_bounds)
}

/// Build a [`UiTreeSnapshot`] (`ui.inspect`) directly from settled `egui` AccessKit output and
/// [`FrameInspectMeta`].
///
/// Prefers settled `egui` `FrameInspectMeta` + AccessKit entries when non-empty for rendered scopes
/// (`toolbar`, `tabs`, `grid_cells`, `dialog`), complementing them with semantic baseline context.
#[must_use]
pub fn build_ui_tree_snapshot_from_egui(
    app: &UiApp,
    meta: &FrameInspectMeta,
    accesskit_update: Option<&egui::accesskit::TreeUpdate>,
    scope: Option<&str>,
    include_bounds: bool,
    metrics: ViewportMetrics,
) -> UiTreeSnapshot {
    let baseline = inspect_semantic_snapshot(app, metrics, scope, include_bounds);
    let ak = AccessKitLookup::from_update(accesskit_update);

    let mut focus_widget_id = baseline.focus_widget_id.clone();
    if let Some(focused_ak_id) = ak.focus
        && let Some(entry) = meta.find_by_accesskit_id(focused_ak_id)
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

    let toolbar = if scope_wants(scope, "toolbar") {
        let egui_nodes = collect_scope("toolbar");
        if egui_nodes.is_empty() { baseline.toolbar.clone() } else { Some(egui_nodes) }
    } else {
        None
    };

    let dialog = if scope_wants(scope, "dialog") || scope_wants(scope, "tabs") {
        build_dialog_snapshot_from_meta(app, meta, &ak, metrics, include_bounds, baseline.dialog.as_ref())
    } else {
        None
    };

    let tabs = if scope_wants(scope, "tabs") {
        let mut combined = collect_scope("tabs");
        if let Some(ref base_tabs) = baseline.tabs {
            for bt in base_tabs {
                if bt.widget_id.starts_with("result.tab.") && !combined.iter().any(|n| n.widget_id == bt.widget_id) {
                    combined.push(bt.clone());
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
        if combined.is_empty() { baseline.tabs.clone() } else { Some(combined) }
    } else {
        None
    };

    let grid_cells = scope_wants(scope, "grid")
        .then(|| build_egui_grid_cells_snapshot(meta, include_bounds, baseline.grid_cells, collect_scope("grid")))
        .flatten();

    let menus: Option<Vec<UiMenuSnapshot>> = if scope_wants(scope, "menus") { baseline.menus.clone() } else { None };
    let navigator_sheets: Option<Vec<UiNavigatorSheetNode>> = if scope_wants(scope, "navigator") {
        let egui_nav = collect_scope("navigator");
        match baseline.navigator_sheets.clone() {
            Some(mut nodes) if include_bounds && !egui_nav.is_empty() => {
                for node in &mut nodes {
                    if let Some(hit) = egui_nav.iter().find(|n| n.widget_id == node.widget_id)
                        && hit.bounds.is_some()
                    {
                        node.bounds = hit.bounds;
                    }
                }
                Some(nodes)
            }
            other => other,
        }
    } else {
        None
    };
    let grid_selection: GridSelectionSnapshot = baseline.grid_selection;
    let dialog_out = if scope_wants(scope, "dialog") { dialog } else { None };

    let active_dialog = dialog_out.as_ref().map_or(baseline.active_dialog, |d| d.dialog_kind.clone());

    let canvas_elements = if scope_wants(scope, "canvas") {
        let mut combined = collect_scope("canvas");
        if let Some(base_canvas) = baseline.canvas_elements {
            for elem in base_canvas {
                if !combined.iter().any(|n| n.widget_id == elem.widget_id) {
                    combined.push(elem);
                }
            }
        }
        if combined.is_empty() { None } else { Some(combined) }
    } else {
        None
    };

    UiTreeSnapshot {
        active_sheet_id: baseline.active_sheet_id,
        active_sheet_kind: baseline.active_sheet_kind,
        active_dialog,
        focus_widget_id,
        zoom: baseline.zoom,
        gui_only: baseline.gui_only,
        undo_depth: baseline.undo_depth,
        redo_depth: baseline.redo_depth,
        dirty: baseline.dirty,
        cursor_readout: baseline.cursor_readout,
        grid_selection,
        brushed_points: baseline.brushed_points,
        status_banner: baseline.status_banner,
        context_menu_target: baseline.context_menu_target,
        result_diagnostic_banner: baseline.result_diagnostic_banner,
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
    meta: &FrameInspectMeta,
    include_bounds: bool,
    baseline_grid: Option<Vec<UiWidgetNode>>,
    mut egui_grid: Vec<UiWidgetNode>,
) -> Option<Vec<UiWidgetNode>> {
    let has_base_grid = baseline_grid.is_some();
    if let Some(base_grid) = baseline_grid {
        for mut node in base_grid {
            if !egui_grid.iter().any(|n| n.widget_id == node.widget_id) {
                if include_bounds
                    && let Some(col_str) = node.widget_id.strip_prefix("formula_bar.open_col.")
                    && let Ok(col_idx) = col_str.parse::<usize>()
                {
                    let badge_id = format!("mv_formula.role_badge.{col_idx}");
                    if let Some(badge) = meta.find_by_widget_id(&badge_id) {
                        node.bounds = clipped_rect_to_bounds_px(badge.clipped_rect, meta.scale_factor);
                    }
                }
                egui_grid.push(node);
            }
        }
    }
    if egui_grid.is_empty() { has_base_grid.then(Vec::new) } else { Some(egui_grid) }
}

/// Render a minimal `egui_shell` workspace + `About` modal view (`R15.1` b4/b5) using canonical
/// [`Tokens`], non-overlapping solid scrollbars, `100%` `ZLayer::ModalBackdrop` outside input
/// capture, [`FrameInspectMeta`], and [`PendingUiIntents`].
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
#[allow(clippy::too_many_arguments)]
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
    meta.begin_pass(&ctx);

    let decision = HybridCoexistenceArbiter::evaluate_with_views(app, views, host_close_dialog_open);
    let mut report = ShellAboutFrameReport { decision: Some(decision), ..ShellAboutFrameReport::default() };

    views.sync_history_with_document(app.document());
    handle_shell_global_shortcuts(&ctx, app, views, &decision, intents);

    let content_rect = ctx.content_rect();
    let layout = tokens.metrics.solve_shell_layout(ShellLayoutInput {
        viewport_w_pt: content_rect.width(),
        viewport_h_pt: content_rect.height(),
        presentation_mode: app.presentation_mode,
        empty_workspace: app.document().total_sheet_count() == 0,
        navigator_open: app.nav.navigator_open,
        inspector_open: app.nav.inspector_open,
        family_open: app.family_open,
        leading_user_collapsed: views.leading_collapsed_rail,
        trailing_user_collapsed: views.trailing_collapsed_rail,
        leading_width_override_pt: views.leading_width_pt,
        trailing_width_override_pt: views.trailing_width_pt,
        host_draws_menu_bar: app.host_draws_menu_bar,
    });
    views.last_shell_layout = Some(layout);

    let enabled = !decision.modal_backdrop_active;
    if !app.presentation_mode {
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
            },
            meta,
            intents,
        );
        report.tier1_rect_pt = top_rep.tier1_rect_pt;
        report.tier2_rect_pt = top_rep.tier2_rect_pt;
        report.menu_popup_open = top_rep.menu_popup_open;

        let history_can_back = views.can_go_back();
        let history_can_forward = views.can_go_forward();
        let hover_readout = views.canvas_view.hover_readout.clone();
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
            LeadingDockContext { tokens: &tokens, layout, enabled, focused: app.nav.active_pane == ActiveWorkspacePane::Navigator },
            meta,
            intents,
        );
        report.leading_dock_rect_pt = lead_rep.dock_rect_pt;

        let trail_rep = render_trailing_studio(
            ui,
            app,
            &mut views.trailing_studio,
            TrailingStudioContext { tokens: &tokens, layout, enabled, focused: app.nav.active_pane == ActiveWorkspacePane::Inspector },
            meta,
            intents,
        );
        report.trailing_dock_rect_pt = trail_rep.dock_rect_pt;
    }

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
    });

    if app.presentation_mode && meta.find_by_widget_id("toolbar.presentation.exit").is_none() {
        let _ = crate::egui_shell::top_chrome::render_presentation_exit_chip(&ctx, app, &tokens, enabled, meta, intents);
    }

    if !matches!(decision.modal_surface, HybridModalSurface::EguiStatParams | HybridModalSurface::EguiFitModal) {
        views.stats_modals.open_session_sheet_id = None;
        views.fit_modal.open_session_sheet_id = None;
    }

    match decision.modal_surface {
        HybridModalSurface::EguiAbout
        | HybridModalSurface::EguiAppModal
        | HybridModalSurface::HostUnsavedClose
        | HybridModalSurface::EguiPreferences
        | HybridModalSurface::EguiRecoveryDialog
        | HybridModalSurface::EguiFindReplace => {
            let app_rep = render_app_modal(&ctx, app, &mut views.app_modals, host_close_dialog_open, &tokens, meta, intents);
            report.regions.dialog_rect_pt = app_rep.dialog_rect_pt;
            report.outside_click_captured = app_rep.outside_click_captured;
            report.outside_wheel_captured = app_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiOneWayAnova => {
            let anova_rep = render_one_way_anova_modal(&ctx, app, &mut views.anova_modal, &tokens, meta, intents);
            report.regions.dialog_rect_pt = anova_rep.dialog_rect_pt;
            report.outside_click_captured = anova_rep.outside_click_captured;
            report.outside_wheel_captured = anova_rep.outside_wheel_captured;
            report.resolved_test_banner = anova_rep.resolved_test_banner;
        }
        HybridModalSurface::EguiCommandPalette => {
            let pal_rep = render_command_palette(&ctx, app, &mut views.command_palette, &tokens, meta, intents);
            report.regions.dialog_rect_pt = pal_rep.dialog_rect_pt;
            report.outside_click_captured = pal_rep.outside_click_captured;
            report.outside_wheel_captured = pal_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiManageVariables => {
            let mv_rep = render_manage_vars_modal(&ctx, app, &mut views.manage_vars_modal, &tokens, meta, intents);
            report.regions.dialog_rect_pt = mv_rep.dialog_rect_pt;
            report.outside_click_captured = mv_rep.outside_click_captured;
            report.outside_wheel_captured = mv_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiImportWizard => {
            let imp_rep = render_import_or_convert_modal(&ctx, app, &mut views.import_wizard, &tokens, meta, intents);
            report.regions.dialog_rect_pt = imp_rep.dialog_rect_pt;
            report.outside_click_captured = imp_rep.outside_click_captured;
            report.outside_wheel_captured = imp_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiTransformModal => {
            let tr_rep = if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }) {
                render_exclude_with_reason_modal(&ctx, app, &mut views.transform_modals, &tokens, meta, intents)
            } else {
                render_transform_modal(&ctx, app, &mut views.transform_modals, &tokens, meta, intents)
            };
            report.regions.dialog_rect_pt = tr_rep.dialog_rect_pt;
            report.outside_click_captured = tr_rep.outside_click_captured;
            report.outside_wheel_captured = tr_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiAnalyzeSheet => {
            let st_rep = render_analyze_sheet_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents);
            report.regions.dialog_rect_pt = st_rep.dialog_rect_pt;
            report.outside_click_captured = st_rep.outside_click_captured;
            report.outside_wheel_captured = st_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiDuplicateAnalysis => {
            let st_rep = render_duplicate_analysis_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents);
            report.regions.dialog_rect_pt = st_rep.dialog_rect_pt;
            report.outside_click_captured = st_rep.outside_click_captured;
            report.outside_wheel_captured = st_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiChangeDataAnalyzed => {
            let st_rep = render_change_data_analyzed_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents);
            report.regions.dialog_rect_pt = st_rep.dialog_rect_pt;
            report.outside_click_captured = st_rep.outside_click_captured;
            report.outside_wheel_captured = st_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiStatParams => {
            let st_rep = match &app.canvas_dialogs.active_dialog {
                ActiveDialog::RandomizeGroups | ActiveDialog::RandomSampleRows { .. } | ActiveDialog::SimulateDistributions => {
                    render_dialogs_r9_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents)
                }
                ActiveDialog::ResultViewer { .. } => render_result_viewer_modal(&ctx, app, &tokens, meta, intents),
                _ => render_stat_params_modal(&ctx, app, &mut views.stats_modals, &tokens, meta, intents),
            };
            report.regions.dialog_rect_pt = st_rep.dialog_rect_pt;
            report.outside_click_captured = st_rep.outside_click_captured;
            report.outside_wheel_captured = st_rep.outside_wheel_captured;
            if !st_rep.resolved_test_banner.is_empty() {
                report.resolved_test_banner = st_rep.resolved_test_banner;
            }
        }
        HybridModalSurface::EguiFitModal => {
            let fit_rep = render_fit_modal(&ctx, app, &mut views.fit_modal, &tokens, meta, intents);
            report.regions.dialog_rect_pt = fit_rep.dialog_rect_pt;
            report.outside_click_captured = fit_rep.outside_click_captured;
            report.outside_wheel_captured = fit_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiHookPicker => {
            let fit_rep = render_hook_constant_modal(&ctx, app, &mut views.fit_modal, &tokens, meta, intents);
            report.regions.dialog_rect_pt = fit_rep.dialog_rect_pt;
            report.outside_click_captured = fit_rep.outside_click_captured;
            report.outside_wheel_captured = fit_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiPowerStudio => {
            let pow_rep = render_power_studio_modal(&ctx, app, &mut views.power_studio, &tokens, meta, intents);
            report.regions.dialog_rect_pt = pow_rep.dialog_rect_pt;
            report.outside_click_captured = pow_rep.outside_click_captured;
            report.outside_wheel_captured = pow_rep.outside_wheel_captured;
        }
        HybridModalSurface::EguiGraphModal => {
            let gr_rep = render_graph_modal(&ctx, app, &mut views.graph_modals, &tokens, meta, intents);
            report.regions.dialog_rect_pt = gr_rep.dialog_rect_pt;
            report.outside_click_captured = gr_rep.outside_click_captured;
            report.outside_wheel_captured = gr_rep.outside_wheel_captured;
        }
        HybridModalSurface::None => {}
    }

    report
}

fn handle_shell_global_shortcuts(
    ctx: &egui::Context,
    app: &UiApp,
    views: &EguiShellViewState,
    decision: &HybridCoexistenceDecision,
    intents: &mut PendingUiIntents,
) {
    let egui_wants_kb = ctx.egui_wants_keyboard_input() || views.column_grid.inline_edit.is_some();
    if HybridCoexistenceArbiter::allows_cmd_t_edit_params(app, decision, egui_wants_kb)
        && ctx.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND, Key::T) || i.consume_key(Modifiers::CTRL, Key::T) || i.consume_key(Modifiers::MAC_CMD, Key::T)
        })
    {
        intents.push_ui("ui.editParams.open", json!({}));
    }
    if decision.modal_backdrop_active || egui_wants_kb {
        return;
    }
    if ctx.input_mut(|i| {
        i.consume_key(Modifiers::COMMAND, Key::K) || i.consume_key(Modifiers::CTRL, Key::K) || i.consume_key(Modifiers::MAC_CMD, Key::K)
    }) {
        intents.push_set_palette_open(true);
    }
    let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
    let shift_ctrl = Modifiers::CTRL | Modifiers::SHIFT;
    let shift_mac = Modifiers::MAC_CMD | Modifiers::SHIFT;
    if ctx.input_mut(|i| {
        i.consume_key(shift_cmd, Key::Z)
            || i.consume_key(shift_ctrl, Key::Z)
            || i.consume_key(shift_mac, Key::Z)
            || i.consume_key(Modifiers::COMMAND, Key::Y)
            || i.consume_key(Modifiers::CTRL, Key::Y)
            || i.consume_key(Modifiers::MAC_CMD, Key::Y)
    }) {
        if app.session().redo_depth() > 0 {
            intents.push_engine("edit.redo", json!({}));
        }
    } else if ctx.input_mut(|i| {
        i.consume_key(Modifiers::COMMAND, Key::Z) || i.consume_key(Modifiers::CTRL, Key::Z) || i.consume_key(Modifiers::MAC_CMD, Key::Z)
    }) && app.session().undo_depth() > 0
    {
        intents.push_engine("edit.undo", json!({}));
    }
    let copy_triggered = ctx.input_mut(|i| {
        let mut hit = false;
        i.events.retain(|ev| {
            if matches!(ev, egui::Event::Copy) {
                hit = true;
                false
            } else {
                true
            }
        });
        if i.consume_key(Modifiers::COMMAND, Key::C) || i.consume_key(Modifiers::CTRL, Key::C) || i.consume_key(Modifiers::MAC_CMD, Key::C) {
            hit = true;
        }
        hit
    });
    if copy_triggered {
        let dpi = ViewportMetrics::new(1.0, 1.0, f64::from(ctx.pixels_per_point())).dpi_for_base(144);
        intents.push_workspace_copy(dpi);
    }
    if ctx.input_mut(|i| {
        i.consume_key(Modifiers::COMMAND, Key::Backslash)
            || i.consume_key(Modifiers::CTRL, Key::Backslash)
            || i.consume_key(Modifiers::MAC_CMD, Key::Backslash)
    }) {
        intents.push_ui("ui.split.toggle", json!({}));
    }
    let toggle_presentation = ctx.input_mut(|i| {
        i.consume_key(Modifiers::NONE, Key::F11)
            || i.consume_key(shift_cmd, Key::P)
            || i.consume_key(shift_ctrl, Key::P)
            || i.consume_key(shift_mac, Key::P)
    }) || (app.presentation_mode && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)));
    if toggle_presentation {
        intents.push_ui("ui.presentation.toggle", json!({}));
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F6)) {
        intents.push_ui("ui.pane.cycle", json!({ "reverse": true }));
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F6)) {
        intents.push_ui("ui.pane.cycle", json!({ "reverse": false }));
    }
}

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

fn render_about_modal(
    ctx: &egui::Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut ShellAboutFrameReport,
) {
    let about_label = tr(&app.locale, "menu.help.about");
    let viewport = ctx.content_rect();
    let backdrop_scrim = if tokens.dark { Color32::from_black_alpha(150) } else { Color32::from_black_alpha(110) };

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_about_dialog")).backdrop_color(backdrop_scrim).show(ctx, |ui| {
        ui.heading(about_label);
        ui.label(format!("CurveBook {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ok_label = tr(&app.locale, "btn.ok");
            let ok_resp = ui.button(ok_label);
            meta.record_response(
                ui,
                &ok_resp,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.ok",
                    role: "button",
                    label: ok_label,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if ok_resp.clicked() {
                intents.push_ui("dialog.ok", json!({}));
            }

            let cancel_label = tr(&app.locale, "btn.cancel");
            let cancel_resp = ui.button(cancel_label);
            meta.record_response(
                ui,
                &cancel_resp,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.cancel",
                    role: "button",
                    label: cancel_label,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if cancel_resp.clicked() {
                intents.push_ui("dialog.cancel", json!({}));
            }
        });
    });

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
        clipped_rect: viewport,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
    });

    let has_wheel = ctx.input(|i| i.smooth_scroll_delta.length_sq() > 0.0 || i.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })));
    if modal_resp.backdrop_response.clicked() {
        report.outside_click_captured = true;
    }
    if has_wheel {
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
            i.events.retain(|ev| !matches!(ev, egui::Event::MouseWheel { .. }));
        });
        report.outside_wheel_captured = true;
    }

    let dlg_rect = modal_resp.response.rect;
    report.regions.dialog_rect_pt = Some(dlg_rect);
    meta.record(FrameInspectNodeMeta {
        egui_id: modal_resp.response.id,
        accesskit_id: modal_resp.response.id.accesskit_id(),
        widget_id: String::from("dialog.about"),
        role: String::from("dialog"),
        label: String::from(about_label),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        clipped_rect: meta.viewport_rect.intersect(dlg_rect),
        z_layer: ZLayer::ModalDialog,
        ellipsized: false,
        full_label: None,
    });
}

fn rects_overlap_significantly(a: egui::accesskit::Rect, b: egui::accesskit::Rect, tol: f64) -> bool {
    let x_overlap = a.x1.min(b.x1) - a.x0.max(b.x0);
    let y_overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    x_overlap > tol && y_overlap > tol
}

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
}
