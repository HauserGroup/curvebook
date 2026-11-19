//! AccessKit + `FrameInspectMeta` -> `UiTreeSnapshot` bridge (`R15.1` b4/b5).
//!
//! Builds deterministic `ui.inspect` (`UiTreeSnapshot`) output directly from settled (`!ctx.will_discard()`)
//! `egui` AccessKit tree updates and [`FrameInspectMeta`] sidecar metadata, plus reusable modal
//! backdrop absorption and AccessKit `Role::ScrollBar` non-overlap checks.

use std::collections::BTreeMap;

use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::egui_shell::rasterizer::EguiScreenshotRegions;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, Tokens};
use crate::inspect::{
    GridSelectionSnapshot, UiDialogSnapshot, UiMenuSnapshot, UiNavigatorSheetNode, UiTreeSnapshot, UiWidgetNode, active_dialog_slug,
    inspect_ui_tree_for_metrics,
};
use crate::interaction::ViewportMetrics;
use crate::layout::ZLayer;
use crate::tr;

/// Result emitted by [`render_minimal_shell_and_about`] for testing, `ui.inspect`, and `ui.screenshot`.
#[derive(Debug, Clone, Default)]
pub struct ShellAboutFrameReport {
    pub regions: EguiScreenshotRegions,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub scrollbar_rect_pt: Option<egui::Rect>,
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
        "tabs" => wid.starts_with("tab."),
        "canvas" => wid.starts_with("canvas."),
        "grid" => wid.starts_with("grid."),
        "dialog" => wid.starts_with("dialog.") || wid.starts_with("modal."),
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
    include_bounds: bool,
    baseline_dialog: Option<&UiDialogSnapshot>,
) -> Option<UiDialogSnapshot> {
    if app.canvas_dialogs.active_dialog == ActiveDialog::None {
        return None;
    }

    let dialog_kind = active_dialog_slug(&app.canvas_dialogs.active_dialog);
    let dialog_entry =
        meta.find_by_widget_id("dialog.about").or_else(|| meta.entries.iter().find(|e| e.widget_id.starts_with("dialog.") && e.role == "dialog"));

    let title = dialog_entry
        .and_then(|e| ak.nodes.get(&e.accesskit_id).copied())
        .and_then(|n| n.label())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| baseline_dialog.map(|d| d.title.clone()))
        .unwrap_or_else(|| tr(&app.locale, "menu.help.about").to_owned());

    let bounds = if include_bounds { dialog_entry.and_then(|e| clipped_rect_to_bounds_px(e.clipped_rect, meta.scale_factor)) } else { None };

    let mut tabs = Vec::new();
    let mut controls = Vec::new();
    let mut buttons = Vec::new();
    for entry in &meta.entries {
        if !entry.widget_id.starts_with("dialog.") || entry.widget_id == "dialog.about" {
            continue;
        }
        let node = widget_node_from_meta(entry, ak, meta.scale_factor, include_bounds);
        match node.role.as_str() {
            "tab" => tabs.push(node),
            "button" => buttons.push(node),
            _ => controls.push(node),
        }
    }

    if let Some(base) = baseline_dialog {
        if tabs.is_empty() {
            tabs.clone_from(&base.tabs);
        }
        if controls.is_empty() {
            controls.clone_from(&base.controls);
        }
        if buttons.is_empty() {
            buttons.clone_from(&base.buttons);
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
/// Requires metadata captured from a settled pass (`!ctx.will_discard()`). Non-rendered workspace
/// document metadata (`active_sheet_id`, `grid_selection`, `menus`, `navigator_sheets`, etc.)
/// matches [`inspect_ui_tree_for_metrics`] so `legacy` and `egui` backends agree structurally.
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

    let tabs = if scope_wants(scope, "tabs") {
        let egui_tabs = collect_scope("tabs");
        if egui_tabs.is_empty() { baseline.tabs.clone() } else { Some(egui_tabs) }
    } else {
        None
    };

    let dialog =
        if scope_wants(scope, "dialog") { build_dialog_snapshot_from_meta(app, meta, &ak, include_bounds, baseline.dialog.as_ref()) } else { None };

    let menus: Option<Vec<UiMenuSnapshot>> = if scope_wants(scope, "menus") { baseline.menus.clone() } else { None };
    let navigator_sheets: Option<Vec<UiNavigatorSheetNode>> = if scope_wants(scope, "navigator") { baseline.navigator_sheets.clone() } else { None };
    let grid_selection: GridSelectionSnapshot = baseline.grid_selection;

    UiTreeSnapshot {
        active_sheet_id: baseline.active_sheet_id,
        active_sheet_kind: baseline.active_sheet_kind,
        active_dialog: baseline.active_dialog,
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
        grid_cells: if scope_wants(scope, "grid") { baseline.grid_cells } else { None },
        dialog,
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
    let backdrop_scrim = if tokens.dark { egui::Color32::from_black_alpha(150) } else { egui::Color32::from_black_alpha(110) };

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
        let viewport = ctx.content_rect();
        let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_about_dialog")).backdrop_color(backdrop_scrim).show(&ctx, |ui| {
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

        let has_wheel =
            ctx.input(|i| i.smooth_scroll_delta.length_sq() > 0.0 || i.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })));
        if modal_resp.backdrop_response.clicked() {
            report.outside_click_captured = true;
        }
        if has_wheel {
            ctx.input_mut(|i| {
                i.smooth_scroll_delta = egui::Vec2::ZERO;
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

    report
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
