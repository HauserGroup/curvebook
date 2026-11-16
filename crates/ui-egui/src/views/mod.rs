//! View-slot orchestrator & default interactive view builders (`R10.1`, `R10.15`, `R10.16`, `R10.22`).
//!
//! Keeps shell chrome (`toolbar`, `navigator`, `data_grid`, `result_viewer`, `info_sheet`,
//! `canvas_viewport`, `inspector`, `status_bar`, `modal_dialog`) cleanly modularized behind one
//! [`build_workspace_layout_tree`] entry point so Wave 5 workers implement dedicated `views/*.rs`
//! submodules without touching `compositor.rs` or colliding across shards.

pub mod a11y;
pub mod analyze_picker;
pub mod dialogs_graph_misc;
pub mod fit_params;
pub mod grid_core;
pub mod import_paste;
pub mod inspector;
pub mod mv_formula;
pub mod nav_notes;
pub mod power_wizard;
pub mod results;
pub mod stat_params;
pub mod toolbar_menus;
pub mod welcome;

use curvebook_doc::SheetId;
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::compositor::{FramePalette, WorkspaceFrameLayout, WorkspacePixelRect};
use crate::dialogs_canvas::ActiveDialog;
use crate::interaction::{UiEventOutcome, ViewportMetrics};
use crate::layout::a11y::{FocusScope, WidgetRole};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, emit_active_tooltip, emit_button, emit_open_dropdown_popup, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, LayoutTree, NodeSpec, WidgetAction, WidgetId, ZLayer};
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_button};
use crate::text_bridge::{DesktopTextMeasurer, UiTextStyle};
use crate::workspace_nav::ActiveWorkspacePane;

use a11y::labeled_meta;
use analyze_picker::{AnalyzePickerViewState, build_analyze_modal_body};
use dialogs_graph_misc::{
    DialogsGraphMiscViewState, build_canvas_or_layout_view, build_command_palette_body, build_descriptor_modal_body, draw_rulers_if_visible,
    modal_title,
};
use fit_params::FitParamsViewState;
use grid_core::{GridCoreViewState, build_data_grid_view};
use import_paste::{ImportPasteViewState, build_import_or_convert_modal_body};
use inspector::{InspectorViewState, build_inspector_slot};
use mv_formula::MvFormulaViewState;
use nav_notes::{NavNotesViewState, build_info_sheet_view, build_navigator_slot};
use power_wizard::{PowerWizardViewState, build_power_wizard_modal_body};
use results::{ResultsViewState, build_result_viewer_view};
use stat_params::{StatParamsViewState, build_edit_params_modal_body};
use toolbar_menus::{ToolbarMenusViewState, build_open_menu_popup, build_status_bar_slot, build_toolbar_slot};
use welcome::{WelcomeViewState, build_portfolio_gallery};

/// Aggregated transient UI state for all interactive view shards wired onto [`UiApp`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ViewShardStates {
    pub a11y: a11y::A11yViewState,
    pub welcome: WelcomeViewState,
    pub nav_notes: NavNotesViewState,
    pub toolbar_menus: ToolbarMenusViewState,
    pub grid_core: GridCoreViewState,
    pub mv_formula: MvFormulaViewState,
    pub results: ResultsViewState,
    pub analyze_picker: AnalyzePickerViewState,
    pub stat_params: StatParamsViewState,
    pub fit_params: FitParamsViewState,
    pub power_wizard: PowerWizardViewState,
    pub dialogs_graph_misc: DialogsGraphMiscViewState,
    pub import_paste: ImportPasteViewState,
    pub inspector: InspectorViewState,
}

/// Dispatch a `WidgetAction::Custom` event to the owning view shard by `domain`.
///
/// # Errors
/// Returns an error if `domain` is unknown or the shard handler fails.
pub fn dispatch_custom_view_action(
    app: &mut UiApp,
    shard_states: &mut ViewShardStates,
    domain: &str,
    payload: &str,
) -> Result<UiEventOutcome, String> {
    match domain {
        "a11y" => a11y::handle_custom_action(app, &mut shard_states.a11y, payload),
        "welcome" => welcome::handle_custom_action(app, &mut shard_states.welcome, payload),
        "nav_notes" => nav_notes::handle_custom_action(app, &mut shard_states.nav_notes, payload),
        "toolbar_menus" => toolbar_menus::handle_custom_action(app, &mut shard_states.toolbar_menus, payload),
        "grid_core" => grid_core::handle_custom_action(app, &mut shard_states.grid_core, payload),
        "mv_formula" => mv_formula::handle_custom_action(app, &mut shard_states.mv_formula, payload),
        "results" => results::handle_custom_action(app, &mut shard_states.results, payload),
        "analyze_picker" => analyze_picker::handle_custom_action(app, &mut shard_states.analyze_picker, payload),
        "stat_params" => stat_params::handle_custom_action(app, &mut shard_states.stat_params, payload),
        "fit_params" => fit_params::handle_custom_action(app, &mut shard_states.fit_params, payload),
        "power_wizard" => power_wizard::handle_custom_action(app, &mut shard_states.power_wizard, payload),
        "dialogs_graph_misc" => dialogs_graph_misc::handle_custom_action(app, &mut shard_states.dialogs_graph_misc, payload),
        "import_paste" => import_paste::handle_custom_action(app, &mut shard_states.import_paste, payload),
        "inspector" => inspector::handle_custom_action(app, &mut shard_states.inspector, payload),
        _ => Err(format!("unknown custom view action domain `{domain}`")),
    }
}

/// Build the complete finalized [`LayoutTree`] for `(app, metrics)`.
#[must_use]
pub fn build_workspace_layout_tree(app: &UiApp, metrics: ViewportMetrics, measurer: &DesktopTextMeasurer, palette: &FramePalette) -> LayoutTree {
    let w_px = metrics.physical_width_px().max(1);
    let h_px = metrics.physical_height_px().max(1);
    let frame = compute_frame_geometry(app, w_px, h_px);

    let mut builder = LayoutBuilder::new(app, metrics, measurer, palette);
    builder.tree.frame = frame;
    builder.tree.active_focus_scope = if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        FocusScope::ModalDialog
    } else if app.interaction.open_menu.is_some() || app.interaction.open_dropdown.is_some() {
        FocusScope::PopupMenu
    } else {
        FocusScope::Workspace
    };

    let full_rect = LayoutRectPx::new(0, 0, w_px, h_px);
    builder.paint_fill(ZLayer::Workspace, None, full_rect, palette.surface);

    if !app.presentation_mode {
        build_toolbar_slot(&mut builder, LayoutRectPx::from_workspace_rect(frame.toolbar));
        if frame.navigator.width > 0 {
            build_navigator_slot(&mut builder, LayoutRectPx::from_workspace_rect(frame.navigator));
        }
    }

    let center_rect = draw_rulers_if_visible(&mut builder, frame.canvas);
    build_center_slot(&mut builder, center_rect);

    if !app.presentation_mode {
        if frame.inspector.width > 0 {
            build_inspector_slot(&mut builder, LayoutRectPx::from_workspace_rect(frame.inspector));
        }
        build_status_bar_slot(&mut builder, LayoutRectPx::from_workspace_rect(frame.status_bar));
        draw_active_pane_focus(&mut builder, &frame);
        if let Some(modal_rect) = frame.modal_overlay {
            build_modal_dialog_slot(&mut builder, full_rect, LayoutRectPx::from_workspace_rect(modal_rect));
        }
        if let Some(open_menu) = app.interaction.open_menu.as_deref() {
            build_open_menu_popup(&mut builder, open_menu, LayoutRectPx::from_workspace_rect(frame.toolbar));
        }
        if let Some(open_dd) = &app.interaction.open_dropdown {
            emit_open_dropdown_popup(&mut builder, open_dd);
        }
        emit_active_tooltip(&mut builder, full_rect);
    }

    builder.finish()
}

/// Pure frame partitioning preserving [`crate::compositor::WorkspaceFrameLayout`].
#[must_use]
pub fn compute_frame_geometry(app: &UiApp, width_px: u32, height_px: u32) -> WorkspaceFrameLayout {
    if app.presentation_mode {
        return WorkspaceFrameLayout {
            total_width: width_px,
            total_height: height_px,
            toolbar: WorkspacePixelRect::new(0, 0, width_px, 0),
            navigator: WorkspacePixelRect::new(0, 0, 0, height_px),
            canvas: WorkspacePixelRect::new(0, 0, width_px, height_px),
            inspector: WorkspacePixelRect::new(width_px, 0, 0, height_px),
            status_bar: WorkspacePixelRect::new(0, height_px, width_px, 0),
            modal_overlay: None,
        };
    }

    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let toolbar_h = if height_px >= 240 { clamp_u32_safe(height_px / 11, 46, 52) } else { clamp_u32_safe(height_px / 14, 24, 38).min(height_px) };
    let status_h = clamp_u32_safe(height_px / 22, 18, 26).min(height_px.saturating_sub(toolbar_h));
    let mid_y = toolbar_h;
    let mid_h = height_px.saturating_sub(toolbar_h).saturating_sub(status_h);

    let nav_w = if app.nav.navigator_open && width_px >= 320 { clamp_u32_safe(width_px / 5, 140, 244) } else { 0 };
    let insp_w = if app.nav.inspector_open && width_px >= 420 { clamp_u32_safe(width_px / 4, 180, 288) } else { 0 };
    let canvas_w = width_px.saturating_sub(nav_w).saturating_sub(insp_w);

    let nav_x = crate::i18n::mirror_x_for_rtl(0, nav_w, width_px, is_rtl);
    let canvas_x = crate::i18n::mirror_x_for_rtl(nav_w, canvas_w, width_px, is_rtl);
    let insp_x = crate::i18n::mirror_x_for_rtl(nav_w.saturating_add(canvas_w), insp_w, width_px, is_rtl);

    let modal_overlay = if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        let max_mw = width_px.saturating_sub(24).max(1).min(width_px);
        let max_mh = height_px.saturating_sub(24).max(1).min(height_px);
        let mw = clamp_u32_safe(width_px.saturating_mul(16) / 25, 180, max_mw);
        let mh = clamp_u32_safe(height_px.saturating_mul(17) / 25, 120, max_mh);
        let mx = crate::i18n::mirror_x_for_rtl((width_px.saturating_sub(mw)) / 2, mw, width_px, is_rtl);
        Some(WorkspacePixelRect::new(mx, (height_px.saturating_sub(mh)) / 2, mw, mh))
    } else {
        None
    };

    WorkspaceFrameLayout {
        total_width: width_px,
        total_height: height_px,
        toolbar: WorkspacePixelRect::new(0, 0, width_px, toolbar_h),
        navigator: WorkspacePixelRect::new(nav_x, mid_y, nav_w, mid_h),
        canvas: WorkspacePixelRect::new(canvas_x, mid_y, canvas_w, mid_h),
        inspector: WorkspacePixelRect::new(insp_x, mid_y, insp_w, mid_h),
        status_bar: WorkspacePixelRect::new(0, mid_y.saturating_add(mid_h), width_px, status_h),
        modal_overlay,
    }
}

const fn clamp_u32_safe(value: u32, min: u32, max: u32) -> u32 {
    let lo = if min < max { min } else { max };
    if value < lo {
        lo
    } else if value > max {
        max
    } else {
        value
    }
}

fn build_center_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    if builder.app.nav.gallery.open {
        build_portfolio_gallery(builder, rect);
        return;
    }
    if builder.app.split.enabled && rect.width >= 120 && rect.height >= 24 {
        let div_w = 4u32;
        let avail = rect.width.saturating_sub(div_w);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let primary_w = ((f64::from(avail) * builder.app.split.ratio.clamp(0.15, 0.85)).round() as u32).clamp(24, avail.saturating_sub(24).max(24));
        let secondary_w = avail.saturating_sub(primary_w);

        let (p_rect, d_rect, s_rect) = if builder.tree.is_rtl {
            (
                LayoutRectPx::new(rect.x + i32::try_from(secondary_w + div_w).unwrap_or(0), rect.y, primary_w, rect.height),
                LayoutRectPx::new(rect.x + i32::try_from(secondary_w).unwrap_or(0), rect.y, div_w, rect.height),
                LayoutRectPx::new(rect.x, rect.y, secondary_w, rect.height),
            )
        } else {
            (
                LayoutRectPx::new(rect.x, rect.y, primary_w, rect.height),
                LayoutRectPx::new(rect.x + i32::try_from(primary_w).unwrap_or(0), rect.y, div_w, rect.height),
                LayoutRectPx::new(rect.x + i32::try_from(primary_w + div_w).unwrap_or(0), rect.y, secondary_w, rect.height),
            )
        };

        builder.tree.split_primary = Some(p_rect.to_workspace_rect());
        builder.tree.split_divider = Some(d_rect.to_workspace_rect());
        builder.tree.split_secondary = Some(s_rect.to_workspace_rect());

        let (prim_id, sec_id) = builder.app.split.resolve_sheets(builder.app.document());
        if let Some(pid) = &prim_id {
            build_sheet_pane(builder, pid, p_rect, "primary");
        }
        if let Some(sid) = &sec_id {
            build_sheet_pane(builder, sid, s_rect, "secondary");
        }
        let div_wid = builder.push_node(NodeSpec {
            id: WidgetId::from("split.divider"),
            parent: None,
            z_layer: ZLayer::SplitHandle,
            rect_px: d_rect,
            a11y: labeled_meta(WidgetRole::Splitter, "Split Pane Divider", FocusScope::Workspace, "Cmd+Shift+\\"),
            state: widget_state(builder, "split.divider", ControlEnablement::enabled()),
            action: WidgetAction::SwapSplitPanes,
        });
        builder.paint_fill(ZLayer::SplitHandle, Some(&div_wid), d_rect, builder.palette.border);
        return;
    }

    if let Some(active_id) = &builder.app.document().active_sheet_id {
        let id_owned = active_id.clone();
        build_sheet_pane(builder, &id_owned, rect, "main");
    }
}

fn build_sheet_pane(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, rect: LayoutRectPx, slot_tag: &str) {
    let doc = builder.app.document();
    if doc.graph_sheet(sheet_id).is_some() || doc.layout_sheet(sheet_id).is_some() {
        build_canvas_or_layout_view(builder, sheet_id, rect, slot_tag);
    } else if let Some(ds) = doc.data_sheet(sheet_id) {
        let ds_cloned = ds.clone();
        build_data_grid_view(builder, &ds_cloned, rect, slot_tag);
    } else if let Some(rs) = doc.result_sheet(sheet_id) {
        let rs_cloned = rs.clone();
        build_result_viewer_view(builder, &rs_cloned, rect, slot_tag);
    } else if let Some(info) = doc.info_sheet(sheet_id) {
        let info_cloned = info.clone();
        build_info_sheet_view(builder, &info_cloned, rect, slot_tag);
    }
}

fn draw_active_pane_focus(builder: &mut LayoutBuilder<'_>, layout: &WorkspaceFrameLayout) {
    let nav = layout.navigator;
    let nav_split = (nav.height * 2) / 3;
    let target = match builder.app.nav.active_pane {
        ActiveWorkspacePane::Navigator => WorkspacePixelRect::new(nav.x, nav.y, nav.width, nav_split.max(8)),
        ActiveWorkspacePane::FamilyStrip => WorkspacePixelRect::new(nav.x, nav.y + nav_split, nav.width, nav.height.saturating_sub(nav_split).max(8)),
        ActiveWorkspacePane::MainCanvasOrGrid => layout.canvas,
        ActiveWorkspacePane::Inspector => layout.inspector,
        ActiveWorkspacePane::FloatingNotes => {
            let w = layout.canvas.width.clamp(24, 130);
            let h = layout.canvas.height.clamp(12, 26);
            WorkspacePixelRect::new(
                layout.canvas.x + layout.canvas.width.saturating_sub(w + 8),
                layout.canvas.y + layout.canvas.height.saturating_sub(h + 8),
                w,
                h,
            )
        }
    };
    if target.width >= 8 && target.height >= 8 {
        let r = LayoutRectPx::from_workspace_rect(target);
        if builder.app.nav.active_pane == ActiveWorkspacePane::FloatingNotes {
            builder.paint_fill(ZLayer::FloatingOverlay, None, r, builder.palette.panel);
        }
        builder.paint_stroke(ZLayer::FloatingOverlay, None, r, builder.palette.accent, 2);
    }
}

fn build_modal_dialog_slot(builder: &mut LayoutBuilder<'_>, full_rect: LayoutRectPx, modal_rect: LayoutRectPx) {
    // 1. ZLayer::ModalBackdrop captures ALL pointer hits outside the modal surface (zero click-through!).
    let scrim_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.backdrop"),
        parent: None,
        z_layer: ZLayer::ModalBackdrop,
        rect_px: full_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Modal Backdrop", FocusScope::ModalDialog, "Escape"),
        state: Default::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalBackdrop, Some(&scrim_wid), full_rect, Rgba8::rgba(15, 23, 42, 128));

    let title = modal_title(&builder.app.canvas_dialogs.active_dialog);
    let modal_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.dialog"),
        parent: None,
        z_layer: ZLayer::ModalDialog,
        rect_px: modal_rect,
        a11y: labeled_meta(WidgetRole::Dialog, title.clone(), FocusScope::ModalDialog, "Escape"),
        state: Default::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&modal_wid), modal_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&modal_wid), modal_rect, builder.palette.accent, 2);

    builder.push_clip(modal_rect);
    let hdr_h = 28u32.min(modal_rect.height);
    let hdr_rect = LayoutRectPx::new(modal_rect.x, modal_rect.y, modal_rect.width, hdr_h);
    builder.paint_fill(ZLayer::ModalDialog, Some(&modal_wid), hdr_rect, builder.palette.header);
    let size_px = (13.0 * builder.scale()).clamp(10.0, 24.0);
    let title_area = LayoutRectPx::new(hdr_rect.x + 8, hdr_rect.y, hdr_rect.width.saturating_sub(38), hdr_h);
    paint_text_fitted(
        builder,
        &modal_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: title_area,
            pad_x: 2,
            text: &title,
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    if modal_rect.width >= 60 && hdr_h >= 16 {
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "modal.close",
                parent: Some(modal_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(modal_rect.right() - 28, modal_rect.y + 3, 24, hdr_h.saturating_sub(6).max(12)),
                label: "X",
                keyboard_path: "Escape",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(900),
                primary: false,
                selected: false,
                shortcut: Some("Escape"),
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::DialogCancel,
            },
        );
    }

    let footer_h = 34u32.min(modal_rect.height.saturating_sub(hdr_h));
    let body_h = modal_rect.height.saturating_sub(hdr_h + footer_h);
    let body_rect = LayoutRectPx::new(
        modal_rect.x + 8,
        modal_rect.y + i32::try_from(hdr_h).unwrap_or(28) + 4,
        modal_rect.width.saturating_sub(16),
        body_h.saturating_sub(8),
    );

    if body_rect.width >= 40 && body_rect.height >= 24 {
        build_modal_body(builder, modal_wid.clone(), body_rect);
    }

    if footer_h >= 20 && modal_rect.width >= 140 {
        let fy = modal_rect.bottom() - i32::try_from(footer_h).unwrap_or(34);
        let ok_enable = evaluate_dialog_button(&builder.app.canvas_dialogs, builder.app.document(), "ok").unwrap_or_else(ControlEnablement::enabled);
        let btn_h = footer_h.saturating_sub(8).max(14);
        let btn_w = (modal_rect.width / 5).clamp(58, 96);
        let cancel_rect = LayoutRectPx::new(modal_rect.right() - i32::try_from(btn_w * 2 + 16).unwrap_or(140), fy + 4, btn_w, btn_h);
        let ok_rect = LayoutRectPx::new(modal_rect.right() - i32::try_from(btn_w + 8).unwrap_or(72), fy + 4, btn_w, btn_h);

        if let Some((_, _, _, _, _, buttons_list)) = crate::inspect::dialog_widget_descriptors(builder.app) {
            let mut ex = modal_rect.x + 8;
            for (idx, (id, lbl)) in buttons_list.iter().enumerate() {
                if matches!(*id, "ok" | "create" | "cancel" | "close") {
                    continue;
                }
                if ex + i32::try_from(btn_w).unwrap_or(64) >= cancel_rect.x - 4 {
                    break;
                }
                let en = evaluate_dialog_button(&builder.app.canvas_dialogs, builder.app.document(), id).unwrap_or_else(ControlEnablement::enabled);
                emit_button(
                    builder,
                    ButtonWidgetSpec {
                        id: &format!("dialog.button.{id}"),
                        parent: Some(modal_wid.clone()),
                        z_layer: ZLayer::ModalDialog,
                        rect: LayoutRectPx::new(ex, fy + 4, btn_w, btn_h),
                        label: lbl,
                        keyboard_path: &format!("dialog.button.{id}"),
                        focus_scope: FocusScope::ModalDialog,
                        tab_order: Some(780 + idx as u32),
                        primary: false,
                        selected: false,
                        shortcut: None,
                        enablement: en,
                        action: WidgetAction::InvokeDialogButton((*id).to_owned()),
                    },
                );
                ex += i32::try_from(btn_w + 6).unwrap_or(70);
            }
        }

        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "modal.cancel",
                parent: Some(modal_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: cancel_rect,
                label: "Cancel",
                keyboard_path: "Escape",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(800),
                primary: false,
                selected: false,
                shortcut: Some("Escape"),
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::DialogCancel,
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "modal.ok",
                parent: Some(modal_wid),
                z_layer: ZLayer::ModalDialog,
                rect: ok_rect,
                label: if matches!(builder.app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize) { "Create" } else { "OK" },
                keyboard_path: "Enter",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(801),
                primary: true,
                selected: false,
                shortcut: Some("Enter"),
                enablement: ok_enable,
                action: WidgetAction::DialogConfirm,
            },
        );
    }
    builder.pop_clip();
}

fn build_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    match &builder.app.canvas_dialogs.active_dialog {
        ActiveDialog::AnalyzeSheet { .. } => build_analyze_modal_body(builder, parent, body_rect),
        ActiveDialog::EditAnalysisParameters { .. } => build_edit_params_modal_body(builder, parent, body_rect),
        ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } => {
            let msg = format!("{} existing analysis sheet(s) match this analysis.", existing_result_ids.len());
            paint_text_fitted(
                builder,
                &parent,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: body_rect,
                    pad_x: 8,
                    text: &msg,
                    style: &UiTextStyle::ui_sans(13.0),
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
            build_descriptor_modal_body(
                builder,
                parent,
                LayoutRectPx::new(body_rect.x, body_rect.y + 24, body_rect.width, body_rect.height.saturating_sub(24)),
            );
        }
        ActiveDialog::ImportReshape | ActiveDialog::ConvertArchetype { .. } => build_import_or_convert_modal_body(builder, parent, body_rect),
        ActiveDialog::PowerAndSampleSize => build_power_wizard_modal_body(builder, parent, body_rect),
        ActiveDialog::CommandPalette => build_command_palette_body(builder, parent, body_rect),
        _ => build_descriptor_modal_body(builder, parent, body_rect),
    }
}
