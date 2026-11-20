//! AccessKit + `FrameInspectMeta` -> `UiTreeSnapshot` bridge & Hybrid Coexistence Arbiter (`R15.1`–`R15.2`).
//!
//! Coordinates hybrid desktop coexistence across `egui`-native surfaces (`Column` grid, top chrome,
//! standardized `Parameters: One-Way ANOVA` modal) and unported legacy presenter surfaces
//! (`ResultSheet`, `ChartScene`, `AnalyzeSheet` picker), building settled `ui.inspect`
//! [`UiTreeSnapshot`] output, enforcing `100%` `ZLayer::ModalBackdrop` outside capture, and
//! verifying AccessKit `Role::ScrollBar` non-overlap.

use std::collections::BTreeMap;

use curvebook_doc::TableArchetype;
use curvebook_render::{RasterImage, RenderTargetSpec};
use egui::{Color32, CornerRadius, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::compositor::{WorkspacePixelRect, compute_workspace_layout_for_dpi, render_workspace_rgba};
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
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, ShellLayoutInput, Tokens,
};
use crate::inspect::{
    GridSelectionSnapshot, UiDialogSnapshot, UiMenuSnapshot, UiNavigatorSheetNode, UiTreeSnapshot, UiWidgetNode, active_dialog_slug,
    inspect_ui_tree_for_metrics,
};
use crate::interaction::ViewportMetrics;
use crate::layout::ZLayer;
use crate::tr;
use crate::workspace_nav::ActiveWorkspacePane;

/// Which center-stage sheet backend renders the current frame in `--ui-backend=egui` (`R15.2+`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HybridSheetSurface {
    /// Ported `egui 0.37` virtualized `Column` `DataSheet` grid.
    EguiColumnGrid,
    /// Native `egui 0.37` virtualized grid for the remaining 7 `DataSheet` archetypes (`R15.4`).
    EguiDataGrid,
    /// Unported legacy presenter (or Wave 2 compatibility marker on non-grid sheets).
    LegacyPresenter,
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
    /// Native `egui` `Analyze...` picker modal (`R15.4`, gated by `native_analyze_sheet`).
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
    /// Unported legacy modal dialog (e.g. `AnalyzeSheet` while `native_analyze_sheet == false`).
    LegacyModal,
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
    pub blocks_legacy_background: bool,
    pub renders_egui_column_grid: bool,
    pub renders_legacy_canvas: bool,
    pub renders_legacy_modal_overlay: bool,
    pub renders_egui_anova_modal: bool,
    pub renders_egui_about_modal: bool,
}

/// Pure, stateless arbiter controlling focus ownership, Z-order, and raw-input routing
/// across coexisting `egui` and legacy presenter surfaces (`R15.2+`).
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
            None => HybridSheetSurface::LegacyPresenter,
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
            classify_active_dialog_surface(app, views)
        };

        let modal_backdrop_active = modal_surface != HybridModalSurface::None;
        let blocks_egui_grid = modal_backdrop_active;
        let blocks_legacy_background = modal_backdrop_active && modal_surface != HybridModalSurface::LegacyModal;

        HybridCoexistenceDecision {
            sheet_surface,
            modal_surface,
            modal_backdrop_active,
            blocks_egui_grid,
            blocks_legacy_background,
            renders_egui_column_grid: matches!(sheet_surface, HybridSheetSurface::EguiColumnGrid | HybridSheetSurface::EguiDataGrid),
            renders_legacy_canvas: sheet_surface == HybridSheetSurface::LegacyPresenter,
            renders_legacy_modal_overlay: modal_surface == HybridModalSurface::LegacyModal,
            renders_egui_anova_modal: modal_surface == HybridModalSurface::EguiOneWayAnova,
            renders_egui_about_modal: modal_surface == HybridModalSurface::EguiAbout,
        }
    }

    /// Return `true` when pointer/wheel events at `pos` should be forwarded to the legacy presenter
    /// (either `Case B` clicks inside an unported legacy modal card, or direct interaction on a
    /// legacy sheet when no modal blocks it).
    #[must_use]
    pub fn pointer_routes_to_legacy(
        decision: &HybridCoexistenceDecision,
        canvas_rect: Rect,
        pos: Pos2,
        menu_popup_open: bool,
        legacy_modal_rect: Option<Rect>,
    ) -> bool {
        if menu_popup_open {
            return false;
        }
        match decision.modal_surface {
            HybridModalSurface::LegacyModal => legacy_modal_rect.is_some_and(|r| r.contains(pos)),
            HybridModalSurface::None => decision.renders_legacy_canvas && canvas_rect.contains(pos),
            _ => false,
        }
    }

    /// Return `true` when `Cmd/Ctrl+T` is eligible to open `EditAnalysisParameters` on an active `ResultSheet`.
    #[must_use]
    pub fn allows_cmd_t_edit_params(app: &UiApp, decision: &HybridCoexistenceDecision, egui_wants_keyboard: bool) -> bool {
        !decision.modal_backdrop_active && !egui_wants_keyboard && crate::interaction::resolve_result_sheet_for_shortcut(app, true).is_some()
    }
}

fn classify_active_dialog_surface(app: &UiApp, views: &EguiShellViewState) -> HybridModalSurface {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::None => HybridModalSurface::None,
        ActiveDialog::About => HybridModalSurface::EguiAbout,
        ActiveDialog::ExportFigure { .. } | ActiveDialog::Print { .. } => HybridModalSurface::EguiAppModal,
        ActiveDialog::CommandPalette => HybridModalSurface::EguiCommandPalette,
        ActiveDialog::AnalyzeSheet { .. } => {
            if views.native_analyze_sheet {
                HybridModalSurface::EguiAnalyzeSheet
            } else {
                HybridModalSurface::LegacyModal
            }
        }
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
        _ => HybridModalSurface::LegacyModal,
    }
}

/// Optional host-provided legacy texture overlay passed from `CurveBookNativeApp` so 60 FPS desktop
/// frames reuse GPU textures without re-running software rasterization inside `inspect_bridge`.
#[derive(Debug)]
pub struct HybridHostTextureFrame {
    /// Pre-rendered legacy canvas texture (`ResultSheet`, `ChartScene`, non-`Column` table) scaled for `canvas_rect`.
    pub canvas_texture: Option<egui::TextureId>,
    /// Pre-rendered cropped legacy modal overlay texture and its exact screen-space rect in logical `pt`.
    pub legacy_modal_texture: Option<(egui::TextureId, Rect)>,
    /// Optional callback invoked inside `CentralPanel` so `CurveBookNativeApp` can sync AccessKit for the legacy canvas.
    pub accesskit_canvas_id: Option<egui::Id>,
}

/// Result emitted by [`render_minimal_shell_and_about`] and [`render_egui_shell_frame`] for
/// testing, `ui.inspect`, `ui.screenshot`, and `apps/curvebook` hybrid viewport sync.
#[derive(Debug, Clone, Default)]
pub struct ShellAboutFrameReport {
    pub regions: EguiScreenshotRegions,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub scrollbar_rect_pt: Option<egui::Rect>,
    pub legacy_canvas_texture_rect_pt: Option<egui::Rect>,
    pub legacy_modal_rect_pt: Option<egui::Rect>,
    pub legacy_canvas_widget_id: Option<egui::Id>,
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
            wid.starts_with("canvas.") || wid.starts_with("result.diagnostic_banner") || wid.starts_with("info.") || wid.starts_with("welcome.")
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
    metrics: ViewportMetrics,
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
        let egui_rendered_dialog = !(tabs.is_empty() && controls.is_empty() && buttons.is_empty());
        if tabs.is_empty() {
            tabs.clone_from(&base.tabs);
        }
        if !egui_rendered_dialog {
            controls.clone_from(&base.controls);
        }
        if buttons.is_empty() {
            buttons.clone_from(&base.buttons);
        }
        if !egui_rendered_dialog {
            let tree = app.layout_tree_cow(metrics);
            let scale = tree.metrics.clamped_scale_factor();
            for node in &tree.nodes {
                if !node.drawn_in_frame || node.z_layer != ZLayer::ModalDialog {
                    continue;
                }
                let wid = node.id.as_str();
                if matches!(wid, "modal.dialog" | "modal.backdrop" | "modal.title") {
                    continue;
                }
                let node_bounds = (include_bounds && node.bounds.visible_rect_px.width > 0 && node.bounds.visible_rect_px.height > 0).then(|| {
                    let v = node.bounds.visible_rect_px;
                    [f64::from(v.x) / scale, f64::from(v.y) / scale, f64::from(v.width) / scale, f64::from(v.height) / scale]
                });
                let ui_node = UiWidgetNode {
                    widget_id: wid.to_owned(),
                    role: String::from(node.a11y.role.as_str()),
                    label: node.a11y.label.clone(),
                    command_id: None,
                    value: node.a11y.value_json.clone().or_else(|| node.a11y.selected.map(|s| json!(s))),
                    enabled: node.state.enablement.enabled,
                    disabled_reason: node.state.enablement.disabled_reason.clone(),
                    bounds: node_bounds,
                };
                if matches!(wid, "modal.ok" | "modal.cancel" | "modal.apply" | "modal.close") {
                    if !buttons.iter().any(|b| b.widget_id == wid) {
                        buttons.push(ui_node);
                    }
                } else if (wid == "analyze.entries_list" || wid.starts_with("analyze.entry.")) && !controls.iter().any(|c| c.widget_id == wid) {
                    controls.push(ui_node);
                }
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

/// Build a [`UiTreeSnapshot`] (`ui.inspect`) directly from settled `egui` AccessKit output and
/// [`FrameInspectMeta`].
///
/// Prefers settled `egui` `FrameInspectMeta` + AccessKit entries when non-empty for rendered scopes
/// (`toolbar`, `tabs`, `grid_cells`, `dialog`), falling back seamlessly to `baseline` for unported
/// legacy surfaces (`ResultSheet` sub-tabs, `AnalyzeSheet` picker modal, navigator, menus).
#[must_use]
pub fn build_ui_tree_snapshot_from_egui(
    app: &UiApp,
    meta: &FrameInspectMeta,
    accesskit_update: Option<&egui::accesskit::TreeUpdate>,
    scope: Option<&str>,
    include_bounds: bool,
    metrics: ViewportMetrics,
) -> UiTreeSnapshot {
    let baseline = inspect_ui_tree_for_metrics(app, metrics, scope, include_bounds);
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

    let grid_cells = if scope_wants(scope, "grid") {
        let egui_grid = collect_scope("grid");
        if egui_grid.is_empty() { baseline.grid_cells.clone() } else { Some(egui_grid) }
    } else {
        None
    };

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
        canvas_elements: if scope_wants(scope, "canvas") { baseline.canvas_elements } else { None },
        grid_cells,
        dialog: dialog_out,
    }
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
    render_egui_shell_frame_with_host(ui, app, views, meta, intents, false, None)
}

/// Full hybrid coexistence workspace/dialog frame entry point supporting optional pre-rendered GPU
/// textures from `CurveBookNativeApp` (`R15.2+`).
#[allow(clippy::too_many_arguments)]
pub fn render_egui_shell_frame_with_host(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    host_close_dialog_open: bool,
    host_textures: Option<&HybridHostTextureFrame>,
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
            CenterStageContext { tokens: &tokens, decision: &decision, layout, host_textures, focused_pane: app.nav.active_pane, enabled },
            meta,
            intents,
        );
        if let Some(primary) = stage_rep.primary_rect_pt {
            report.regions.canvas_rect_pt = Some(primary);
        }
        report.scrollbar_rect_pt = stage_rep.scrollbar_rect_pt;
        report.legacy_canvas_texture_rect_pt = stage_rep.legacy_canvas_texture_rect_pt;
        report.legacy_canvas_widget_id = stage_rep.legacy_canvas_widget_id;
        report.center_stage_handled_natively = stage_rep.handled_natively;
    });

    if app.presentation_mode && meta.find_by_widget_id("toolbar.presentation.exit").is_none() {
        let _ = crate::egui_shell::top_chrome::render_presentation_exit_chip(&ctx, app, &tokens, enabled, meta, intents);
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
        HybridModalSurface::LegacyModal => {
            render_legacy_modal_overlay_with_backdrop(&ctx, app, &tokens, host_textures, meta, &mut report);
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

/// Render the legacy `UiApp` workspace/scene into `canvas_rect` via optional pre-uploaded host
/// textures or on-demand software rasterization (used by deferred `R15.4` sheets and `canvas_view`).
pub fn render_legacy_canvas_in_rect(
    ui: &mut egui::Ui,
    app: &UiApp,
    canvas_rect: Rect,
    host_textures: Option<&HybridHostTextureFrame>,
) -> egui::Response {
    let resp = ui.allocate_rect(canvas_rect, Sense::click_and_drag());
    if !canvas_rect.is_positive() {
        return resp;
    }

    if let Some(tex_id) = host_textures.and_then(|h| h.canvas_texture) {
        ui.painter().image(tex_id, canvas_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        return resp;
    }

    let ppp = ui.ctx().pixels_per_point().max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width_px = (canvas_rect.width() * ppp).round().max(1.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height_px = (canvas_rect.height() * ppp).round().max(1.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let dpi = (f64::from(ppp) * 96.0).round().clamp(72.0, 600.0) as u16;

    // Render legacy presenter without active dialog so background canvas stays clean when Case A overlays it.
    let mut canvas_app = app.clone();
    canvas_app.canvas_dialogs.active_dialog = ActiveDialog::None;
    canvas_app.host_draws_menu_bar = true;
    if let Ok(raster) = render_workspace_rgba(&canvas_app, RenderTargetSpec { width_px, height_px, dpi }) {
        let color_image = egui::ColorImage::from_rgba_unmultiplied([raster.width_px as usize, raster.height_px as usize], &raster.rgba);
        let tex_id = upload_cached_texture(ui.ctx(), Id::unique("hybrid.legacy_canvas_tex"), "hybrid_legacy_canvas", color_image);
        ui.painter().image(tex_id, canvas_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    resp
}

fn render_legacy_modal_overlay_with_backdrop(
    ctx: &egui::Context,
    app: &UiApp,
    tokens: &Tokens,
    host_textures: Option<&HybridHostTextureFrame>,
    meta: &mut FrameInspectMeta,
    report: &mut ShellAboutFrameReport,
) {
    let screen_rect = ctx.content_rect();
    let backdrop_scrim = if tokens.dark { Color32::from_black_alpha(150) } else { Color32::from_rgba_unmultiplied(15, 23, 42, 118) };

    let backdrop_resp = egui::Area::new(Id::unique("hybrid.legacy_modal.backdrop"))
        .fixed_pos(screen_rect.min)
        .order(Order::Middle)
        .interactable(true)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen_rect.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, backdrop_scrim);
            resp
        })
        .inner;

    meta.record(FrameInspectNodeMeta {
        egui_id: backdrop_resp.id,
        accesskit_id: backdrop_resp.id.accesskit_id(),
        widget_id: String::from("modal.backdrop"),
        role: String::from("backdrop"),
        label: String::from("Modal Backdrop"),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        clipped_rect: screen_rect,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
    });

    let mut drawn_modal: Option<(egui::TextureId, Rect)> = host_textures.and_then(|h| h.legacy_modal_texture);
    if drawn_modal.is_none() {
        let ppp = ctx.pixels_per_point().max(1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let full_w = (screen_rect.width() * ppp).round().max(1.0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let full_h = (screen_rect.height() * ppp).round().max(1.0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let dpi = (f64::from(ppp) * 96.0).round().clamp(72.0, 600.0) as u16;
        let layout = compute_workspace_layout_for_dpi(app, full_w, full_h, dpi);
        if let Some(modal_px) = layout.modal_overlay
            && modal_px.width > 0
            && modal_px.height > 0
            && let Ok(full_img) = render_workspace_rgba(app, RenderTargetSpec { width_px: full_w, height_px: full_h, dpi })
            && let Some(modal_color_img) = crop_workspace_modal_to_color_image(&full_img, modal_px)
        {
            let tex_id = upload_cached_texture(ctx, Id::unique("hybrid.legacy_modal_tex"), "hybrid_legacy_modal", modal_color_img);
            #[allow(clippy::cast_precision_loss)]
            let modal_rect = Rect::from_min_size(
                Pos2::new(screen_rect.min.x + (modal_px.x as f32) / ppp, screen_rect.min.y + (modal_px.y as f32) / ppp),
                Vec2::new((modal_px.width as f32) / ppp, (modal_px.height as f32) / ppp),
            );
            drawn_modal = Some((tex_id, modal_rect));
        }
    }

    if let Some((tex_id, modal_rect)) = drawn_modal {
        report.legacy_modal_rect_pt = Some(modal_rect);
        report.regions.dialog_rect_pt = Some(modal_rect);
        egui::Area::new(Id::unique("hybrid.legacy_modal.card")).fixed_pos(modal_rect.min).order(Order::Foreground).interactable(true).show(
            ctx,
            |ui| {
                let (_r, _resp) = ui.allocate_exact_size(modal_rect.size(), Sense::click_and_drag());
                ui.painter().image(tex_id, modal_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            },
        );
    }

    let pointer_pos = ctx.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()));
    let is_outside = |p: Pos2| report.legacy_modal_rect_pt.is_none_or(|mr| !mr.contains(p));
    let outside = pointer_pos.is_some_and(is_outside);
    let any_click_outside = outside
        && (backdrop_resp.clicked()
            || backdrop_resp.secondary_clicked()
            || ctx.input(|i| {
                i.pointer.primary_clicked()
                    || i.pointer.secondary_clicked()
                    || i.events.iter().any(|ev| matches!(ev, egui::Event::PointerButton { pos, pressed: false, .. } if is_outside(*pos)))
            }));
    if any_click_outside {
        report.outside_click_captured = true;
    }
    let has_wheel = ctx.input(|i| i.smooth_scroll_delta.length_sq() > 0.0 || i.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })));
    if has_wheel && outside {
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
            i.events.retain(|ev| !matches!(ev, egui::Event::MouseWheel { .. }));
        });
        report.outside_wheel_captured = true;
    }
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

/// Crop `rect` from a straight-RGBA [`RasterImage`] into an `egui::ColorImage`.
#[must_use]
pub fn crop_workspace_modal_to_color_image(src: &RasterImage, rect: WorkspacePixelRect) -> Option<egui::ColorImage> {
    if src.width_px == 0 || src.height_px == 0 || rect.width == 0 || rect.height == 0 {
        return None;
    }
    let x0 = rect.x.min(src.width_px.saturating_sub(1));
    let y0 = rect.y.min(src.height_px.saturating_sub(1));
    let w = rect.width.min(src.width_px.saturating_sub(x0));
    let h = rect.height.min(src.height_px.saturating_sub(y0));
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    for row in 0..h {
        let src_start = (((y0 + row) as usize) * (src.width_px as usize) + (x0 as usize)) * 4;
        let dst_start = (row as usize) * (w as usize) * 4;
        let bytes = (w as usize) * 4;
        if let (Some(src_slice), Some(dst_slice)) = (src.rgba.get(src_start..src_start + bytes), rgba.get_mut(dst_start..dst_start + bytes)) {
            dst_slice.copy_from_slice(src_slice);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba))
}

fn upload_cached_texture(ctx: &egui::Context, slot_id: Id, name: &str, color_image: egui::ColorImage) -> egui::TextureId {
    let mut handle: Option<egui::TextureHandle> = ctx.data_mut(|d| d.get_temp(slot_id));
    match handle.as_mut() {
        Some(tex) => {
            tex.set(color_image, egui::TextureOptions::NEAREST);
            let id = tex.id();
            ctx.data_mut(|d| d.insert_temp(slot_id, tex.clone()));
            id
        }
        None => {
            let tex = ctx.load_texture(name, color_image, egui::TextureOptions::NEAREST);
            let id = tex.id();
            ctx.data_mut(|d| d.insert_temp(slot_id, tex));
            id
        }
    }
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
