//! View-slot orchestrator & default interactive view builders (`R10.1`, `R10.15`, `R10.16`, `R10.22`).
//!
//! Keeps shell chrome (`toolbar`, `navigator`, `data_grid`, `result_viewer`, `info_sheet`,
//! `canvas_viewport`, `inspector`, `status_bar`, `modal_dialog`) cleanly modularized behind one
//! [`build_workspace_layout_tree`] entry point so Wave 5 workers implement dedicated `views/*.rs`
//! submodules without touching `compositor.rs`.

use std::sync::Arc;

use lumencraft_chart::{ChartScene, PointPt, preflight_check_scene};
use lumencraft_doc::{AnalysisKind, DataSheet, InfoSheet, LayoutItem, ResultSheet, SheetFolderKind, SheetId, SheetSection, TableArchetype};
use lumencraft_palette::{CanvasPreviewFilter, Rgba8};
use lumencraft_render::{RasterImage, RenderTargetSpec, apply_canvas_preview_filter, render_scene};
use lumencraft_tools::canvas::{SnapOrientation, SnapSource};
use lumencraft_tools::{GridCellCoord, PhysicalUnit, build_ruler_ticks};
use serde_json::json;

use crate::UiApp;
use crate::compositor::{FramePalette, WorkspaceFrameLayout, WorkspacePixelRect};
use crate::dialogs_canvas::{
    ANALYSIS_CATALOG, ActiveDialog, ActiveInspector, AnalysisCategory, AssumptionDiagnosticBanner, ResultSheetViewState, data_column_title,
    pairwise_display_mode_str, variable_role_color,
};
use crate::grid_view::resolve_flat_coord;
use crate::interaction::{CanvasDragTarget, GridHitGeometry, InfoRowHitRect, NavigatorEntryHitRect, ScenePaperMapping, ViewportMetrics};
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::canvas_viewport::{
    CanvasViewportGeometry, compute_aspect_ratio_paper_mapping, should_show_graph_series_chip, should_show_layout_panel_chrome,
};
use crate::layout::paint::{PaintOp, PaintOpKind};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, DropdownSpec, FittedTextSpec, SliderWidgetSpec, SpinnerSpec, TextFieldSpec, ToggleWidgetSpec, emit_active_tooltip, emit_button,
    emit_dropdown, emit_numeric_spinner, emit_open_dropdown_popup, emit_scrollbars_for_meta, emit_slider, emit_tab, emit_text_field, emit_toggle,
    paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, LayoutTree, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_button, evaluate_dialog_widget, evaluate_edit_params_tab};
use crate::text_bridge::{DesktopTextMeasurer, TextMeasurer, UiTextStyle};
use crate::workspace_nav::ActiveWorkspacePane;

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

fn build_toolbar_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.bottom() - 1], [rect.right(), rect.bottom() - 1], builder.palette.border, 1);

    if rect.width < 120 || rect.height < 14 {
        return;
    }

    builder.push_clip(rect);
    let two_rows = rect.height >= 40;
    let menu_h = if two_rows { (rect.height / 2).saturating_sub(3).max(14) } else { rect.height.saturating_sub(8).clamp(14, 28) };
    let menu_y = if two_rows { rect.y + 2 } else { rect.y + i32::try_from((rect.height.saturating_sub(menu_h)) / 2).unwrap_or(0) };
    let mut cur_x = rect.x + 8;

    let size_px = (11.5 * builder.scale()).clamp(9.0, 22.0);
    let menu_style = UiTextStyle::ui_sans(size_px);
    let menus = crate::menus::menu_bar_tooltips(builder.app);
    for (idx, sec) in menus.iter().enumerate() {
        let measured = builder.measurer.measure(sec.title, &menu_style, &builder.tree.locale).width_px.ceil() as u32;
        let bw = (measured + 14).clamp(38, 78);
        if cur_x + i32::try_from(bw).unwrap_or(44) > rect.right() - 8 {
            break;
        }
        let selected = builder.app.interaction.open_menu.as_deref() == Some(sec.menu_key);
        let kpath = format!("Alt+{}", idx + 1);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: sec.menu_key,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(cur_x, menu_y, bw, menu_h),
                label: sec.title,
                keyboard_path: &kpath,
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::MenuOpen(sec.menu_key.to_owned()),
            },
        );
        cur_x += i32::try_from(bw + 3).unwrap_or(46);
    }

    let tb_y = if two_rows { rect.y + i32::try_from(rect.height / 2 + 1).unwrap_or(23) } else { menu_y };
    let tb_h = if two_rows { rect.height.saturating_sub(rect.height / 2 + 3).max(14) } else { menu_h };
    let mut tb_x = if two_rows { rect.x + 8 } else { cur_x + 6 };
    let tb_entries = crate::menus::toolbar_tooltips(builder.app);
    for entry in &tb_entries {
        let measured = builder.measurer.measure(entry.caption, &menu_style, &builder.tree.locale).width_px.ceil() as u32;
        let bw = (measured + 12).clamp(40, 96);
        if tb_x + i32::try_from(bw).unwrap_or(44) > rect.right() - 6 {
            break;
        }
        let en = if entry.tooltip.enabled {
            ControlEnablement::enabled()
        } else {
            ControlEnablement::disabled(format!("{} is currently unavailable.", entry.caption))
        };
        let shortcut = (!entry.tooltip.shortcut.is_empty()).then_some(entry.tooltip.shortcut.as_str());
        let kpath = shortcut.unwrap_or(entry.button.command_id);
        let cmd_id = if entry.button.id == "toolbar.axes" { "ui.graph.axes.open" } else { entry.button.command_id };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: entry.button.id,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(tb_x, tb_y, bw, tb_h),
                label: entry.caption,
                keyboard_path: kpath,
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: entry.button.command_id == "ui.analyze.open",
                selected: false,
                shortcut,
                enablement: en,
                action: WidgetAction::InvokeCommand { command_id: cmd_id.to_owned(), params: json!({}) },
            },
        );
        tb_x += i32::try_from(bw + 4).unwrap_or(48);
    }
    builder.pop_clip();
}

fn build_open_menu_popup(builder: &mut LayoutBuilder<'_>, menu_key: &str, toolbar_rect: LayoutRectPx) {
    let sections = crate::menus::menu_bar_tooltips(builder.app);
    let Some(section) = sections.into_iter().find(|s| s.menu_key == menu_key) else {
        return;
    };
    let anchor_x = builder.tree.node(menu_key).map_or(toolbar_rect.x + 8, |n| n.bounds.rect_px.x);
    let pop_w = 248u32;
    let row_h = 22u32;
    let max_h = builder.tree.metrics.physical_height_px().saturating_sub(toolbar_rect.bottom() as u32 + 12).clamp(60, 320);
    let content_h = (section.items.len() as u32).saturating_mul(row_h) + 6;
    let pop_h = content_h.min(max_h);
    let pop_rect = LayoutRectPx::new(anchor_x, toolbar_rect.bottom(), pop_w, pop_h);

    let scroll_id = format!("scroll.menu.{menu_key}");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), pop_rect, [pop_w, content_h], scroll_off, ScrollAxes::Vertical);

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("popup.{menu_key}")),
        parent: None,
        z_layer: ZLayer::DropdownMenu,
        rect_px: pop_rect,
        a11y: AccessibilityMeta::new(WidgetRole::ContextMenu, format!("{} Menu", section.title), FocusScope::PopupMenu).with_keyboard_path(menu_key),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::DropdownMenu, Some(&wid), pop_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::DropdownMenu, Some(&wid), pop_rect, builder.palette.accent, 1);

    let clip = LayoutRectPx::new(pop_rect.x + 2, pop_rect.y + 2, pop_w.saturating_sub(14), pop_h.saturating_sub(4));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    for (idx, tip) in section.items.iter().enumerate() {
        let ry = clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
        if ry + i32::try_from(row_h).unwrap_or(22) <= clip.y || ry >= clip.bottom() {
            continue;
        }
        let item_rect = LayoutRectPx::new(clip.x, ry, clip.width, row_h);
        let item_id = format!("menu.{}", tip.command_id);
        let en = if tip.enabled {
            ControlEnablement::enabled()
        } else {
            ControlEnablement::disabled(format!("{} is unavailable in this context.", tip.localized_label))
        };
        let st = widget_state(builder, &item_id, en.clone());
        let mut a11y = AccessibilityMeta::new(WidgetRole::MenuItem, tip.localized_label, FocusScope::PopupMenu)
            .with_keyboard_path(if tip.shortcut.is_empty() { tip.command_id } else { tip.shortcut.as_str() });
        a11y.shortcut = (!tip.shortcut.is_empty()).then(|| tip.shortcut.clone());
        let item_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item_id),
            parent: Some(wid.clone()),
            z_layer: ZLayer::DropdownMenu,
            rect_px: item_rect,
            a11y,
            state: st.clone(),
            action: if en.enabled {
                WidgetAction::InvokeCommand { command_id: tip.command_id.to_owned(), params: json!({}) }
            } else {
                WidgetAction::NoOp
            },
        });
        let bg = if st.hovered && en.enabled { builder.palette.header } else { builder.palette.surface };
        let fg = if en.enabled { builder.palette.text } else { builder.palette.muted };
        builder.paint_fill(ZLayer::DropdownMenu, Some(&item_wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &item_wid,
            FittedTextSpec {
                z_layer: ZLayer::DropdownMenu,
                rect: item_rect,
                pad_x: 6,
                text: tip.localized_label,
                style: &UiTextStyle::ui_sans(size_px),
                color: fg,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::DropdownMenu, &scroll_meta, Some(wid));
}

fn build_navigator_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, None, rect, builder.palette.border, 1);

    let fam_h = if builder.app.family_open && rect.height >= 160 { rect.height / 3 } else { 24u32.min(rect.height) };
    let tree_h = rect.height.saturating_sub(fam_h);
    let tree_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, tree_h);
    let fam_rect = LayoutRectPx::new(rect.x, rect.y + i32::try_from(tree_h).unwrap_or(0), rect.width, fam_h);

    let row_h = 20u32;
    let total_items = 5usize + builder.app.nav.folders.iter().map(|f| f.entries.len()).sum::<usize>();
    let content_h = (total_items as u32) * row_h + 28;
    let scroll_id = String::from("scroll.navigator");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), tree_rect, [tree_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let mut tree_a11y = AccessibilityMeta::new(WidgetRole::TreeItem, "Project Navigator", FocusScope::Workspace).with_keyboard_path("Cmd+Shift+N");
    tree_a11y.expanded = Some(true);
    let tree_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("navigator.tree"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: tree_rect,
        a11y: tree_a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&tree_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&tree_wid), tree_rect, builder.palette.panel);

    let inner = tree_rect.inset(4);
    builder.push_clip(inner);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let mut cur_y = inner.y + 4 - y_scroll;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let bold_style = UiTextStyle::ui_bold(size_px);
    let item_style = UiTextStyle::ui_sans(size_px);

    for (f_idx, folder) in builder.app.nav.folders.iter().enumerate() {
        let collapsed = folder.collapsed || builder.app.nav.collapsed_sections.contains(&folder.section);
        let folder_kind = folder_kind_for_section(folder.section);
        if cur_y + i32::try_from(row_h).unwrap_or(20) > inner.y && cur_y < inner.bottom() {
            let f_rect = LayoutRectPx::new(inner.x, cur_y, inner.width.saturating_sub(12), row_h);
            let fid = format!("nav.folder.{f_idx}");
            let fst = widget_state(builder, &fid, ControlEnablement::enabled());
            let mut fa = AccessibilityMeta::new(WidgetRole::Disclosure, folder.label.clone(), FocusScope::Workspace)
                .with_keyboard_path(format!("nav.folder.{f_idx}"));
            fa.expanded = Some(!collapsed);
            let fwid = builder.push_node(NodeSpec {
                id: WidgetId::from(fid),
                parent: Some(tree_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect_px: f_rect,
                a11y: fa,
                state: fst.clone(),
                action: WidgetAction::ToggleFolder(folder.section),
            });
            let bg = if fst.hovered { builder.palette.header } else { builder.palette.panel };
            builder.paint_fill(ZLayer::Workspace, Some(&fwid), f_rect, bg);
            let marker = if collapsed { "+" } else { "-" };
            let txt = format!("{marker} {}", folder.label);
            paint_text_fitted(
                builder,
                &fwid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: f_rect,
                    pad_x: 4,
                    text: &txt,
                    style: &bold_style,
                    color: builder.palette.text,
                    scroll_container: Some(&scroll_id),
                },
            );
        }
        cur_y += i32::try_from(row_h).unwrap_or(20);
        if collapsed {
            continue;
        }
        for (e_idx, entry) in folder.entries.iter().enumerate() {
            if cur_y + i32::try_from(row_h).unwrap_or(20) > inner.y && cur_y < inner.bottom() {
                let row_rect = LayoutRectPx::new(inner.x + 8, cur_y, inner.width.saturating_sub(20), row_h);
                let section = folder.section;
                let eid = format!("nav.sheet.{}", entry.sheet_id.as_str());
                let selected = builder.app.document().active_sheet_id.as_ref() == Some(&entry.sheet_id);
                let est = widget_state(builder, &eid, ControlEnablement::enabled());
                let mut ea = AccessibilityMeta::new(WidgetRole::TreeItem, entry.title.clone(), FocusScope::Workspace)
                    .with_keyboard_path(format!("nav.{}.{}", f_idx, e_idx));
                ea.selected = Some(selected);
                let ewid = builder.push_node(NodeSpec {
                    id: WidgetId::from(eid),
                    parent: Some(tree_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    rect_px: row_rect,
                    a11y: ea,
                    state: est.clone(),
                    action: WidgetAction::SwitchSheet { sheet_id: entry.sheet_id.clone(), section },
                });
                let bg = if selected {
                    builder.palette.accent
                } else if est.hovered {
                    builder.palette.header
                } else {
                    builder.palette.panel
                };
                let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
                builder.paint_fill(ZLayer::Workspace, Some(&ewid), row_rect, bg);
                paint_text_fitted(
                    builder,
                    &ewid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: row_rect,
                        pad_x: 6,
                        text: &entry.title,
                        style: &item_style,
                        color: fg,
                        scroll_container: Some(&scroll_id),
                    },
                );
                builder.tree.navigator_entries.push(NavigatorEntryHitRect {
                    sheet_id: entry.sheet_id.clone(),
                    folder: folder_kind,
                    index_in_folder: e_idx,
                    rect: row_rect.to_workspace_rect(),
                });
            }
            cur_y += i32::try_from(row_h).unwrap_or(20);
        }
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(tree_wid));

    build_family_pane(builder, fam_rect);
}

fn build_family_pane(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height < 16 {
        return;
    }
    builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.y], [rect.right(), rect.y], builder.palette.border, 1);
    let hdr_h = 22u32.min(rect.height);
    let hdr = LayoutRectPx::new(rect.x + 2, rect.y + 1, rect.width.saturating_sub(4), hdr_h);
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "navigator.family_toggle",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: hdr,
            label: if builder.app.family_open { "Family (-)" } else { "Family (+)" },
            keyboard_path: "Cmd+Shift+F",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: builder.app.family_open,
            shortcut: Some("Cmd+Shift+F"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::ToggleFamilyPane,
        },
    );

    if builder.app.family_open && rect.height > hdr_h + 18 {
        builder.push_clip(rect);
        let mut cy = hdr.bottom() + 4;
        let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
        let st = UiTextStyle::ui_sans(size_px);
        for (idx, item) in builder.app.nav.family_nodes.iter().enumerate() {
            if cy + 18 >= rect.bottom() {
                break;
            }
            let r = LayoutRectPx::new(rect.x + 6, cy, rect.width.saturating_sub(12), 18);
            let id = format!("family.item.{idx}");
            let mut a11y =
                AccessibilityMeta::new(WidgetRole::TreeItem, item.title.clone(), FocusScope::Workspace).with_keyboard_path(format!("family.{idx}"));
            a11y.selected = Some(builder.app.document().active_sheet_id.as_ref() == Some(&item.sheet_id));
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(id),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: r,
                a11y,
                state: WidgetVisualState::default(),
                action: WidgetAction::SwitchSheet { sheet_id: item.sheet_id.clone(), section: item.section },
            });
            builder.paint_fill(ZLayer::Workspace, Some(&wid), r, builder.palette.panel);
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: r,
                    pad_x: 4,
                    text: &item.title,
                    style: &st,
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
            cy += 19;
        }
        builder.pop_clip();
    }
}

const fn folder_kind_for_section(section: SheetSection) -> SheetFolderKind {
    match section {
        SheetSection::Data => SheetFolderKind::Data,
        SheetSection::Info => SheetFolderKind::Info,
        SheetSection::Analyses => SheetFolderKind::Results,
        SheetSection::Graphs => SheetFolderKind::Graphs,
        SheetSection::Layouts => SheetFolderKind::Layouts,
    }
}

const fn section_for_folder_kind(folder: SheetFolderKind) -> SheetSection {
    match folder {
        SheetFolderKind::Data => SheetSection::Data,
        SheetFolderKind::Info => SheetSection::Info,
        SheetFolderKind::Results => SheetSection::Analyses,
        SheetFolderKind::Graphs => SheetSection::Graphs,
        SheetFolderKind::Layouts => SheetSection::Layouts,
    }
}

fn draw_rulers_if_visible(builder: &mut LayoutBuilder<'_>, canvas: WorkspacePixelRect) -> LayoutRectPx {
    if !builder.app.presentation_mode && builder.app.canvas_dialogs.rulers_visible && canvas.width >= 48 && canvas.height >= 48 {
        let ruler_h = 16u32;
        let ruler_w = 24u32;
        let top = LayoutRectPx::from_workspace_rect(WorkspacePixelRect::new(canvas.x + ruler_w, canvas.y, canvas.width - ruler_w, ruler_h));
        let left = LayoutRectPx::from_workspace_rect(WorkspacePixelRect::new(canvas.x, canvas.y + ruler_h, ruler_w, canvas.height - ruler_h));
        builder.paint_fill(ZLayer::Workspace, None, top, builder.palette.panel);
        builder.paint_fill(ZLayer::Workspace, None, left, builder.palette.panel);
        let ticks = build_ruler_ticks(PhysicalUnit::Millimeters, 420.0, builder.app.canvas_dialogs.canvas.zoom, 0.0, 1.0);
        let tick_step = (top.width / ticks.len().max(1) as u32).max(16);
        for i in 0..ticks.len().min(6) {
            let tx = top.x + i32::try_from(i as u32 * tick_step).unwrap_or(0);
            builder.paint_line(ZLayer::Workspace, None, [tx, top.bottom() - 4], [tx, top.bottom()], builder.palette.border, 1);
        }
        LayoutRectPx::from_workspace_rect(WorkspacePixelRect::new(
            canvas.x + ruler_w,
            canvas.y + ruler_h,
            canvas.width - ruler_w,
            canvas.height - ruler_h,
        ))
    } else {
        LayoutRectPx::from_workspace_rect(canvas)
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
            a11y: AccessibilityMeta::new(WidgetRole::Splitter, "Split Pane Divider", FocusScope::Workspace).with_keyboard_path("Cmd+Shift+\\"),
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

fn build_portfolio_gallery(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    let cards = &builder.app.nav.gallery.cards;
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.surface);
    builder.push_clip(rect);
    let cols = (rect.width / 190).max(1) as usize;
    let cw = (rect.width.saturating_sub(24)) / (cols as u32).max(1);
    let ch = 120u32;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    for (idx, card) in cards.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cx = rect.x + 8 + i32::try_from(col as u32 * cw).unwrap_or(0);
        let cy = rect.y + 8 + i32::try_from(row as u32 * (ch + 8)).unwrap_or(0);
        if cy >= rect.bottom() {
            break;
        }
        let card_rect = LayoutRectPx::new(cx, cy, cw.saturating_sub(8), ch);
        let cid = format!("gallery.card.{idx}");
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(cid),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: card_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Button, card.title.clone(), FocusScope::Workspace).with_keyboard_path(format!("gallery.{idx}")),
            state: WidgetVisualState::default(),
            action: WidgetAction::SwitchSheet { sheet_id: card.sheet_id.clone(), section: section_for_folder_kind(card.folder) },
        });
        let is_sel = builder.app.nav.gallery.selected_ids.contains(&card.sheet_id);
        let fill_col = if is_sel { builder.palette.header } else { builder.palette.panel };
        let stroke_col = if is_sel { builder.palette.accent } else { builder.palette.border };
        builder.paint_fill(ZLayer::Workspace, Some(&wid), card_rect, fill_col);
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), card_rect, stroke_col, if is_sel { 2 } else { 1 });
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: card_rect,
                pad_x: 8,
                text: &card.title,
                style: &UiTextStyle::ui_bold(size_px),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }
    builder.pop_clip();
}

fn build_sheet_pane(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, rect: LayoutRectPx, slot_tag: &str) {
    let doc = builder.app.document();
    if doc.graph_sheet(sheet_id).is_some() || doc.layout_sheet(sheet_id).is_some() {
        build_canvas_or_layout_view(builder, sheet_id, rect, slot_tag);
    } else if let Some(ds) = doc.data_sheet(sheet_id) {
        let ds_cloned = ds.clone();
        build_data_grid_view(builder, &ds_cloned, rect);
    } else if let Some(rs) = doc.result_sheet(sheet_id) {
        let rs_cloned = rs.clone();
        build_result_viewer_view(builder, &rs_cloned, rect);
    } else if let Some(info) = doc.info_sheet(sheet_id) {
        let info_cloned = info.clone();
        build_info_sheet_view(builder, &info_cloned, rect);
    }
}

fn build_data_grid_view(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, rect: LayoutRectPx) {
    let sheet_rect = LayoutRectPx::new(rect.x + 6, rect.y + 6, rect.width.saturating_sub(12), rect.height.saturating_sub(12));
    if sheet_rect.width < 40 || sheet_rect.height < 40 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, sheet_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, sheet_rect, builder.palette.border, 1);
    builder.push_clip(sheet_rect);

    let show_formula = matches!(ds.archetype, TableArchetype::MultipleVariables(_)) || builder.app.grid.formula_bar.active;
    let formula_h = if show_formula && sheet_rect.height > 80 { 24u32 } else { 0 };
    if formula_h > 0 {
        let btn_w = 54u32;
        let input_w = sheet_rect.width.saturating_sub(btn_w * 2 + 16).max(40);
        let fx_rect = LayoutRectPx::new(sheet_rect.x + 4, sheet_rect.y + 2, input_w, formula_h - 4);
        emit_text_field(
            builder,
            TextFieldSpec {
                id: "formula_bar.input",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: fx_rect,
                label: "Formula Bar",
                fallback_text: &builder.app.grid.formula_bar.input,
                keyboard_path: "F2",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        let fb = &builder.app.grid.formula_bar;
        let commit_ok = !ds.frozen && fb.active && fb.target_column.is_some() && fb.is_valid && fb.parse_error.is_none();
        let commit_en = if commit_ok { ControlEnablement::enabled() } else { ControlEnablement::disabled("Formula bar inactive or incomplete") };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "formula_bar.commit",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(fx_rect.right() + 4, sheet_rect.y + 2, btn_w, formula_h - 4),
                label: "Apply",
                keyboard_path: "Enter",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: commit_en,
                action: WidgetAction::InvokeCommand { command_id: String::from("table.setCalculatedVariable"), params: json!({}) },
            },
        );
        let cancel_en = if fb.active { ControlEnablement::enabled() } else { ControlEnablement::disabled("Formula bar inactive") };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "formula_bar.cancel",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(fx_rect.right() + i32::try_from(btn_w + 8).unwrap_or(62), sheet_rect.y + 2, btn_w, formula_h - 4),
                label: "Cancel",
                keyboard_path: "Escape",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: cancel_en,
                action: WidgetAction::FocusWidget(String::new()),
            },
        );
    }

    let frame = builder.app.grid.tool.format_visible_window(ds);
    let hdr_y = sheet_rect.y + 4 + i32::try_from(formula_h).unwrap_or(0);
    let row_h = 20u32;
    let body_y = hdr_y + i32::try_from(row_h + 2).unwrap_or(22);
    let gutter_w = 38u32;
    let (x_subcols, x_col_w) = match &ds.archetype {
        TableArchetype::Xy(xy) => (xy.x_mode.subcolumn_count().max(1), 64u32),
        TableArchetype::Survival(_) => (1, 64u32),
        _ => (0, 0),
    };
    let x_total_w = u32::try_from(x_subcols).unwrap_or(0).saturating_mul(x_col_w);
    let avail_cols_w = sheet_rect.width.saturating_sub(gutter_w + x_total_w + 14);
    let total_frame_cols = frame.window.end_col.saturating_sub(frame.window.start_col).max(1) as u32;
    let col_w = if total_frame_cols > 6 { (avail_cols_w / total_frame_cols).clamp(52, 76) } else { 76u32 };
    let max_cols = (avail_cols_w / col_w).max(1) as usize;
    let avail_body_h = u32::try_from((sheet_rect.bottom() - body_y - 12).max(0)).unwrap_or(0);
    let max_rows = (avail_body_h / row_h) as usize;
    let visible_rows = max_rows.min(frame.window.end_row.saturating_sub(frame.window.start_row));
    let end_row = frame.window.start_row.saturating_add(visible_rows);

    let mut visible_cols = Vec::new();
    for flat_col in (frame.window.start_col..frame.window.end_col).take(max_cols) {
        let (col, subcol) = resolve_flat_coord(&ds.archetype, flat_col).unwrap_or((flat_col, 0));
        visible_cols.push((flat_col, GridCellCoord::new(0, col, subcol)));
    }

    let geom = GridHitGeometry {
        sheet_id: ds.id.clone(),
        sheet_rect: sheet_rect.to_workspace_rect(),
        hdr_y: u32::try_from(hdr_y.max(0)).unwrap_or(0),
        body_y: u32::try_from(body_y.max(0)).unwrap_or(0),
        row_h,
        gutter_w,
        col_w,
        x_subcols,
        x_col_w,
        start_row: frame.window.start_row,
        end_row,
        start_col: frame.window.start_col,
        visible_cols: visible_cols.clone(),
        is_rtl: builder.tree.is_rtl,
    };

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let header_style = UiTextStyle::ui_bold(size_px);
    let cell_style = UiTextStyle::grid_tabular(size_px);

    // X/Time headers (if XY or Survival)
    for subcol in 0..x_subcols {
        if let Some(xr) = geom.x_header_rect(subcol) {
            let x_rect = LayoutRectPx::from_workspace_rect(xr);
            let x_lbl = if x_subcols == 1 { String::from("X") } else { format!("X{}", subcol + 1) };
            let x_wid = builder.push_node(NodeSpec {
                id: WidgetId::from(format!("grid.x_hdr.{subcol}")),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: x_rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, x_lbl.clone(), FocusScope::Workspace),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            builder.paint_fill(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.header);
            builder.paint_stroke(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.border, 1);
            paint_text_fitted(
                builder,
                &x_wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: x_rect,
                    pad_x: 4,
                    text: &x_lbl,
                    style: &header_style,
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
        }
    }

    // Y/Data Column headers
    let is_mv = matches!(&ds.archetype, TableArchetype::MultipleVariables(_));
    for (vis_idx, &(flat_col, coord)) in visible_cols.iter().enumerate() {
        let h_rect = LayoutRectPx::from_workspace_rect(geom.column_header_rect(vis_idx as u32));
        let title = data_column_title(ds, coord.col).map_or_else(|| format!("Col {}", coord.col + 1), str::to_owned);
        let fx_w = if is_mv && coord.subcol == 0 && h_rect.width >= 38 { 18u32 } else { 0 };
        let lbl_rect = LayoutRectPx::new(h_rect.x, h_rect.y, h_rect.width.saturating_sub(fx_w), h_rect.height);
        let h_id = format!("grid.col_hdr.{flat_col}");
        let h_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(h_id),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: lbl_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, title.clone(), FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.paint_fill(ZLayer::Workspace, Some(&h_wid), lbl_rect, builder.palette.header);
        builder.paint_stroke(ZLayer::Workspace, Some(&h_wid), lbl_rect, builder.palette.border, 1);
        paint_text_fitted(
            builder,
            &h_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: lbl_rect,
                pad_x: 4,
                text: &title,
                style: &header_style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        if fx_w > 0 {
            let fx_rect = LayoutRectPx::new(lbl_rect.right(), h_rect.y, fx_w, h_rect.height);
            let en = if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() };
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("formula_bar.open_col.{}", coord.col),
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect: fx_rect,
                    label: "fx",
                    keyboard_path: "F2",
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: builder.app.grid.formula_bar.active && builder.app.grid.formula_bar.target_column == Some(coord.col),
                    shortcut: None,
                    enablement: en,
                    action: WidgetAction::InvokeCommand {
                        command_id: String::from("ui.table.openFormulaBar"),
                        params: json!({ "column": coord.col }),
                    },
                },
            );
        }
    }

    // Virtualized visible cells only (`<= 16.67 ms` on 1M-cell grid!)
    for (r_idx, row_abs) in (frame.window.start_row..end_row).enumerate() {
        let g_rect = LayoutRectPx::from_workspace_rect(geom.row_gutter_rect(r_idx as u32));
        let row_label = format!("{}", row_abs + 1);
        builder.paint_fill(ZLayer::Workspace, None, g_rect, builder.palette.panel);
        let g_wid = WidgetId::from(format!("grid.gutter.{row_abs}"));
        let mut g_a11y = AccessibilityMeta::new(WidgetRole::Label, row_label.clone(), FocusScope::Workspace);
        g_a11y.value_text = Some(row_label.clone());
        builder.push_node(NodeSpec {
            id: g_wid.clone(),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: g_rect,
            a11y: g_a11y,
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        paint_text_fitted(
            builder,
            &g_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: g_rect,
                pad_x: 3,
                text: &row_label,
                style: &cell_style,
                color: builder.palette.muted,
                scroll_container: None,
            },
        );

        for subcol in 0..x_subcols {
            if let Some(xr) = geom.x_cell_rect(row_abs, subcol) {
                let x_rect = LayoutRectPx::from_workspace_rect(xr);
                let x_text = x_cell_display(ds, row_abs, subcol, &builder.app.locale_preset);
                let x_id = format!("grid.xcell.{row_abs}.{subcol}");
                let x_wid = builder.push_node(NodeSpec {
                    id: WidgetId::from(x_id),
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect_px: x_rect,
                    a11y: AccessibilityMeta::new(WidgetRole::GridCell, format!("R{}X{}", row_abs + 1, subcol + 1), FocusScope::Workspace)
                        .with_keyboard_path("ArrowKeys"),
                    state: WidgetVisualState::default(),
                    action: WidgetAction::SelectGridCell { row: row_abs as u32, col: u32::MAX, subcol: subcol as u16 },
                });
                builder.paint_fill(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.surface);
                builder.paint_stroke(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.border, 1);
                if !x_text.is_empty() {
                    paint_text_fitted(
                        builder,
                        &x_wid,
                        FittedTextSpec {
                            z_layer: ZLayer::Workspace,
                            rect: x_rect,
                            pad_x: 3,
                            text: &x_text,
                            style: &cell_style,
                            color: builder.palette.text,
                            scroll_container: None,
                        },
                    );
                }
            }
        }

        for &(flat_col, base_coord) in &visible_cols {
            let Some(cr) = geom.cell_rect(row_abs, flat_col) else {
                continue;
            };
            let cell_rect = LayoutRectPx::from_workspace_rect(cr);
            let coord = GridCellCoord::new(row_abs, base_coord.col, base_coord.subcol);
            let cell_view = frame.cells.iter().find(|c| c.coord.row == row_abs && c.flat_col == flat_col);
            let text = cell_view.map(|c| c.display_text.as_str()).unwrap_or("");
            let excluded = cell_view.is_some_and(|c| c.excluded);
            let selected = cell_view.is_some_and(|c| c.is_selected);

            let cid = format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol);
            let mut a11y = AccessibilityMeta::new(WidgetRole::GridCell, format!("R{}C{}", coord.row + 1, coord.col + 1), FocusScope::Workspace)
                .with_keyboard_path("ArrowKeys");
            a11y.selected = Some(selected);
            a11y.value_text = (!text.is_empty()).then(|| text.to_owned());

            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(cid.clone()),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: cell_rect,
                a11y,
                state: widget_state(builder, &cid, ControlEnablement::enabled()),
                action: WidgetAction::SelectGridCell { row: coord.row as u32, col: coord.col as u32, subcol: coord.subcol as u16 },
            });
            let bg = if selected { Rgba8::rgba(59, 130, 246, 48) } else { builder.palette.surface };
            builder.paint_fill(ZLayer::Workspace, Some(&wid), cell_rect, bg);
            builder.paint_stroke(ZLayer::Workspace, Some(&wid), cell_rect, if selected { builder.palette.accent } else { builder.palette.border }, 1);
            if !text.is_empty() {
                let ink = if excluded { builder.palette.excluded } else { builder.palette.text };
                let style = if excluded { cell_style.clone().italic() } else { cell_style.clone() };
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec { z_layer: ZLayer::Workspace, rect: cell_rect, pad_x: 4, text, style: &style, color: ink, scroll_container: None },
                );
                if excluded {
                    let mid_y = cell_rect.center()[1];
                    builder.paint_line(ZLayer::Workspace, Some(&wid), [cell_rect.x + 3, mid_y], [cell_rect.right() - 3, mid_y], ink, 1);
                }
            }
        }
    }

    builder.tree.grid_geometry = Some(geom);
    builder.pop_clip();
}

fn x_cell_display(ds: &DataSheet, row: usize, subcol: usize, preset: &crate::i18n::LocalePreset) -> String {
    ds.x_or_time_cell(row, subcol).map(|c| crate::grid_view::format_cell_display(c, preset)).unwrap_or_default()
}

fn build_result_viewer_view(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, rect: LayoutRectPx) {
    let box_rect = rect.inset(6);
    if box_rect.width < 60 || box_rect.height < 60 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, box_rect, builder.palette.border, 1);
    builder.push_clip(box_rect);

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let hdr_h = 24u32;
    let hdr_rect = LayoutRectPx::new(box_rect.x + 8, box_rect.y + 4, box_rect.width.saturating_sub(148), hdr_h);
    let title_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("result.title.{}", rs.id.as_str())),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: hdr_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, rs.title().to_owned(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &title_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr_rect,
            pad_x: 2,
            text: rs.title(),
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "result.edit_params_btn",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: LayoutRectPx::new(box_rect.right() - 134, box_rect.y + 4, 126, hdr_h),
            label: "Parameters (Cmd+T)",
            keyboard_path: "Cmd+T",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: Some("Cmd+T"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::InvokeCommand { command_id: String::from("ui.editParams.open"), params: json!({ "sheet_id": rs.id.as_str() }) },
        },
    );

    // Scrollable horizontal sub-tab strip (zero truncation cap!)
    let tab_y = box_rect.y + 32;
    let tab_strip_rect = LayoutRectPx::new(box_rect.x + 8, tab_y, box_rect.width.saturating_sub(16), 24);
    let active_tab_idx = builder.app.canvas_dialogs.result_view.active_tab_index;
    let mut tx = tab_strip_rect.x;
    for (idx, tab) in rs.tabs.iter().enumerate() {
        let tw =
            (builder.measurer.measure(&tab.title, &UiTextStyle::ui_sans(size_px), &builder.tree.locale).width_px.ceil() as u32 + 20).clamp(68, 200);
        if tx + i32::try_from(tw).unwrap_or(68) > tab_strip_rect.right() {
            break;
        }
        let selected = active_tab_idx == idx;
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("result.tab.{}", tab.id),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(tx, tab_y, tw, 22),
                label: &tab.title,
                keyboard_path: &format!("result.tab.{idx}"),
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::SelectResultTab(tab.id.clone()),
            },
        );
        tx += i32::try_from(tw + 4).unwrap_or(72);
    }

    let mut body_y = tab_y + 28;
    if let Some(banner) = AssumptionDiagnosticBanner::from_result(rs)
        && box_rect.width >= 100
        && box_rect.height >= 120
    {
        let links_w = (banner.detail_tab_ids.len() as u32).saturating_mul(78).min(box_rect.width / 2);
        let banner_rect = LayoutRectPx::new(box_rect.x + 8, body_y, box_rect.width.saturating_sub(16 + links_w), 22);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "result.diagnostic_banner",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: banner_rect,
                label: "Assumption Diagnostics Available",
                keyboard_path: "ui.result.inspectDiagnostics",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::InvokeCommand {
                    command_id: String::from("ui.result.inspectDiagnostics"),
                    params: json!({ "result_sheet_id": rs.id.as_str() }),
                },
            },
        );
        let mut lx = banner_rect.right() + 4;
        for tab_id in &banner.detail_tab_ids {
            if lx + 64 > box_rect.right() - 8 {
                break;
            }
            let r = LayoutRectPx::new(lx, body_y, 72, 22);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("result.diagnostic_banner.tab.{tab_id}"),
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect: r,
                    label: tab_id,
                    keyboard_path: "ui.result.inspectDiagnostics",
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::InvokeCommand {
                        command_id: String::from("ui.result.inspectDiagnostics"),
                        params: json!({ "result_sheet_id": rs.id.as_str(), "tab_id": tab_id }),
                    },
                },
            );
            lx += 76;
        }
        builder.tree.result_banner_rect = Some(banner_rect.to_workspace_rect());
        builder.tree.result_banner_sheet_id = Some(rs.id.clone());
        body_y += 26;
    }

    let active_tab = rs.tabs.get(active_tab_idx).or_else(|| rs.tabs.first());

    if let Some(tab) = active_tab
        && box_rect.bottom() > body_y + 20
    {
        let table_rect = LayoutRectPx::new(box_rect.x + 8, body_y, box_rect.width.saturating_sub(16), (box_rect.bottom() - body_y - 6) as u32);
        let row_h = 20u32;
        let content_h = (tab.rows.len() as u32 + 1).saturating_mul(row_h) + 8;
        let scroll_id = format!("scroll.result.{}", rs.id.as_str());
        let scroll_off = builder.scroll_offset(&scroll_id);
        let scroll_meta = ScrollMeta::new(scroll_id.clone(), table_rect, [table_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

        let tbl_wid = builder.push_node(NodeSpec {
            id: WidgetId::from("result.table"),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: table_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, tab.title.clone(), FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.attach_scroll(&tbl_wid, scroll_meta.clone());
        builder.paint_fill(ZLayer::Workspace, Some(&tbl_wid), table_rect, builder.palette.surface);

        builder.push_clip(table_rect);
        let col_count = tab.column_headers.len().max(1);
        let col_w = (table_rect.width.saturating_sub(12) / (col_count as u32 + 1)).max(56);
        let tab_style = UiTextStyle::grid_tabular(size_px);
        let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;

        for (r_idx, row) in tab.rows.iter().enumerate() {
            let ry = table_rect.y + 4 + i32::try_from(r_idx as u32 * row_h).unwrap_or(0) - y_scroll;
            if ry + i32::try_from(row_h).unwrap_or(20) <= table_rect.y || ry >= table_rect.bottom() {
                continue;
            }
            let r_label = tab.row_labels.get(r_idx).map(String::as_str).unwrap_or("");
            let lbl_rect = LayoutRectPx::new(table_rect.x + 2, ry, col_w.saturating_sub(4), row_h);
            paint_text_fitted(
                builder,
                &tbl_wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: lbl_rect,
                    pad_x: 2,
                    text: r_label,
                    style: &tab_style,
                    color: builder.palette.muted,
                    scroll_container: Some(&scroll_id),
                },
            );
            for (c_idx, cell) in row.iter().enumerate() {
                let cx = table_rect.x + i32::try_from((c_idx as u32 + 1) * col_w).unwrap_or(0);
                if cx >= table_rect.right() - 12 {
                    break;
                }
                let txt = ResultSheetViewState::format_result_cell(cell, rs.analysis.p_value_format);
                let cell_rect = LayoutRectPx::new(cx, ry, col_w.saturating_sub(4), row_h);
                paint_text_fitted(
                    builder,
                    &tbl_wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: cell_rect,
                        pad_x: 2,
                        text: &txt,
                        style: &tab_style,
                        color: builder.palette.text,
                        scroll_container: Some(&scroll_id),
                    },
                );
            }
        }
        builder.pop_clip();
        emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(tbl_wid));
    }

    builder.pop_clip();
}

fn build_info_sheet_view(builder: &mut LayoutBuilder<'_>, info: &InfoSheet, rect: LayoutRectPx) {
    let box_rect = rect.inset(8);
    if box_rect.width < 60 || box_rect.height < 60 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, box_rect, builder.palette.border, 1);

    let half_w = box_rect.width / 2;
    let const_x = box_rect.x + i32::try_from(crate::i18n::mirror_x_for_rtl(0, half_w, box_rect.width, builder.tree.is_rtl)).unwrap_or(0);
    let notes_x =
        box_rect.x + i32::try_from(crate::i18n::mirror_x_for_rtl(half_w, box_rect.width - half_w, box_rect.width, builder.tree.is_rtl)).unwrap_or(0);
    let notes_rect = LayoutRectPx::new(notes_x + 6, box_rect.y + 24, box_rect.width.saturating_sub(half_w + 12), box_rect.height.saturating_sub(30));

    builder.tree.info_notes_rect = Some(notes_rect.to_workspace_rect());
    builder.tree.info_sheet_id = Some(info.id.clone());

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let st = UiTextStyle::ui_sans(size_px);
    let mut y = box_rect.y + 24;
    for (idx, c) in info.constants.iter().enumerate() {
        if y + 20 >= box_rect.bottom() {
            break;
        }
        let r = LayoutRectPx::new(const_x + 6, y, half_w.saturating_sub(12), 18);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(format!("info.const.{idx}")),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: r,
            a11y: AccessibilityMeta::new(WidgetRole::TextBox, c.name.clone(), FocusScope::Workspace).with_keyboard_path(format!("info.{idx}")),
            state: WidgetVisualState::default(),
            action: WidgetAction::FocusWidget(format!("info.const.{idx}")),
        });
        builder.paint_fill(ZLayer::Workspace, Some(&wid), r, builder.palette.panel);
        let txt = format!("{}: {}", c.name, c.value.display_text());
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: r,
                pad_x: 4,
                text: &txt,
                style: &st,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        builder.tree.info_constant_rows.push(InfoRowHitRect {
            sheet_id: info.id.clone(),
            name: c.name.clone(),
            value_text: c.value.display_text(),
            rect: r.to_workspace_rect(),
        });
        y += 20;
    }

    let n_wid = emit_text_field(
        builder,
        TextFieldSpec {
            id: "info.notes",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: notes_rect,
            label: "Experiment Notes",
            fallback_text: &info.notes_markdown,
            keyboard_path: "info.notes",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            multiline: true,
            enablement: ControlEnablement::enabled(),
        },
    );
    let _ = n_wid;
}

fn build_canvas_or_layout_view(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, outer_rect: LayoutRectPx, slot_tag: &str) {
    let scene_res = builder.app.session().render_sheet_scene(sheet_id);
    let (scene_w_pt, scene_h_pt, scene_opt) = match scene_res {
        Ok(scene) => (scene.width_pt, scene.height_pt, Some(scene)),
        Err(_) => {
            if let Some(ls) = builder.app.document().layout_sheet(sheet_id) {
                let p = ls.spec.page_size_pt();
                (p.width, p.height, None)
            } else {
                (420.0, 297.0, None)
            }
        }
    };

    let geom = compute_aspect_ratio_paper_mapping(sheet_id.clone(), outer_rect, scene_w_pt, scene_h_pt, builder.app, builder.scale());
    builder.paint_fill(ZLayer::Workspace, None, geom.mat_rect_px, builder.palette.canvas_mat);

    let canvas_id = format!("canvas.surface.{slot_tag}.{}", sheet_id.as_str());
    let canvas_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(canvas_id),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: geom.mat_rect_px,
        a11y: AccessibilityMeta::new(WidgetRole::Canvas, format!("Canvas {}", sheet_id.as_str()), FocusScope::Workspace).with_keyboard_path("Cmd+0"),
        state: WidgetVisualState::default(),
        action: WidgetAction::FocusWidget(format!("canvas.surface.{slot_tag}.{}", sheet_id.as_str())),
    });

    builder.tree.scene_papers.push(geom.mapping.clone());
    if !builder.app.presentation_mode && geom.mat_rect_px.width >= 60 && geom.mat_rect_px.height >= 60 {
        let p = geom.paper_rect_px;
        builder.tree.canvas_title_rect = Some(WorkspacePixelRect::new(
            u32::try_from((geom.mat_rect_px.x + 8).max(0)).unwrap_or(0),
            u32::try_from((geom.mat_rect_px.y + 4).max(0)).unwrap_or(0),
            geom.mat_rect_px.width.saturating_sub(16),
            16,
        ));
        let xr = LayoutRectPx::new(p.x + i32::try_from(p.width / 4).unwrap_or(0), p.bottom() - 24, (p.width / 2).max(16), 20);
        builder.tree.x_axis_title_rect = Some(xr.to_workspace_rect());
        let y_local_x = crate::i18n::mirror_x_for_rtl(4, 24, p.width, builder.tree.is_rtl);
        let yr = LayoutRectPx::new(
            p.x + i32::try_from(y_local_x).unwrap_or(4),
            p.y + i32::try_from(p.height / 4).unwrap_or(0),
            24,
            (p.height / 2).max(16),
        );
        builder.tree.y_axis_title_rect = Some(yr.to_workspace_rect());
    }

    if let Some(scene) = scene_opt {
        let spec = RenderTargetSpec {
            width_px: geom.base_paper_rect_px.width.max(1),
            height_px: geom.base_paper_rect_px.height.max(1),
            dpi: (builder.tree.metrics.scale_factor * 96.0).round().clamp(72.0, 384.0) as u16,
        };
        if let Ok(mut img) = render_scene(&scene, spec) {
            if builder.app.preview_filter != CanvasPreviewFilter::Normal {
                apply_canvas_preview_filter(&mut img, builder.app.preview_filter);
            }
            draw_scene_overlays(&mut img, builder.app, sheet_id, &scene, &geom, builder.palette);

            builder.push_clip(geom.mat_rect_px);
            let order = builder.next_order();
            builder.tree.paint_ops.push(PaintOp {
                z_layer: ZLayer::Workspace,
                paint_order: order,
                clip: Some(builder.active_clip_id()),
                owner_widget_id: Some(canvas_wid),
                kind: PaintOpKind::BlitImage { dst: geom.paper_rect_px, image: Arc::new(img) },
            });
            builder.pop_clip();
        }
    }

    draw_canvas_zoom_controls(builder, &geom, slot_tag);
}

fn draw_canvas_zoom_controls(builder: &mut LayoutBuilder<'_>, geom: &CanvasViewportGeometry, slot_tag: &str) {
    if !builder.app.presentation_mode && geom.zoom_bar_rect_px.height > 0 {
        builder.paint_fill(ZLayer::Workspace, None, geom.zoom_bar_rect_px, builder.palette.panel);
        builder.paint_line(
            ZLayer::Workspace,
            None,
            [geom.zoom_bar_rect_px.x, geom.zoom_bar_rect_px.y],
            [geom.zoom_bar_rect_px.right(), geom.zoom_bar_rect_px.y],
            builder.palette.border,
            1,
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("canvas.{slot_tag}.zoom_fit"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: geom.zoom_fit_btn_px,
                label: "Fit Sheet",
                keyboard_path: "Cmd+0",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: (geom.mapping.zoom - 1.0).abs() < 0.02,
                shortcut: Some("Cmd+0"),
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::SetCanvasZoomFit,
            },
        );
        let pct = (geom.mapping.zoom * 100.0).round();
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("canvas.{slot_tag}.zoom_pct"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: geom.zoom_pct_btn_px,
                label: &format!("{pct:.0}%"),
                keyboard_path: "Cmd+=",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::CycleCanvasZoomPreset,
            },
        );
    }
    if let Some((track, thumb)) = geom.h_scrollbar {
        builder.paint_fill(ZLayer::Workspace, None, track, builder.palette.panel);
        builder.paint_fill(ZLayer::Workspace, None, thumb, builder.palette.muted);
    }
    if let Some((track, thumb)) = geom.v_scrollbar {
        builder.paint_fill(ZLayer::Workspace, None, track, builder.palette.panel);
        builder.paint_fill(ZLayer::Workspace, None, thumb, builder.palette.muted);
    }
}

fn draw_scene_overlays(
    img: &mut RasterImage,
    app: &UiApp,
    sheet_id: &SheetId,
    scene: &ChartScene,
    geom: &CanvasViewportGeometry,
    pal: &FramePalette,
) {
    // Draw brushed points & active drag/snap guides onto local paper pixels (`base_paper_rect_px` relative).
    let pw = img.width_px;
    let ph = img.height_px;
    let paper_local = WorkspacePixelRect::new(0, 0, pw, ph);
    let local_map = ScenePaperMapping {
        sheet_id: sheet_id.clone(),
        paper: paper_local,
        scene_width_pt: scene.width_pt.max(1.0),
        scene_height_pt: scene.height_pt.max(1.0),
        zoom: 1.0,
        pan_pt: [0.0, 0.0],
    };

    for guide in &app.canvas_dialogs.canvas.active_snap_guides {
        let color = if guide.source == SnapSource::PlotOrigin { Rgba8::rgb(220, 38, 38) } else { pal.accent };
        match guide.orientation {
            SnapOrientation::Vertical => {
                if let (Some([px, sy]), Some([_, ey])) = (
                    local_map.scene_pt_to_pixel(PointPt::new(guide.position_pt, guide.span_start_pt)),
                    local_map.scene_pt_to_pixel(PointPt::new(guide.position_pt, guide.span_end_pt)),
                ) {
                    let x = (px.round().max(0.0) as u32).min(pw.saturating_sub(1));
                    let y0 = (sy.min(ey).round().max(0.0) as u32).min(ph);
                    let y1 = (sy.max(ey).round().max(0.0) as u32).min(ph);
                    for py in y0..y1 {
                        img.blend_pixel(x, py, color);
                    }
                }
            }
            SnapOrientation::Horizontal => {
                if let (Some([sx, py]), Some([ex, _])) = (
                    local_map.scene_pt_to_pixel(PointPt::new(guide.span_start_pt, guide.position_pt)),
                    local_map.scene_pt_to_pixel(PointPt::new(guide.span_end_pt, guide.position_pt)),
                ) {
                    let y = (py.round().max(0.0) as u32).min(ph.saturating_sub(1));
                    let x0 = (sx.min(ex).round().max(0.0) as u32).min(pw);
                    let x1 = (sx.max(ex).round().max(0.0) as u32).min(pw);
                    for px in x0..x1 {
                        img.blend_pixel(px, y, color);
                    }
                }
            }
        }
    }

    // R10.15: Hide graph series chips and layout panel selection chrome unless hovered or selected.
    if let Some(gs) = app.document().graph_sheet(sheet_id) {
        let series_count = app
            .document()
            .data_sheet(&gs.spec.primary_source_sheet_id)
            .map_or(0, |ds| crate::inspect::table_dimensions_summary(&ds.archetype).1)
            .min(12);
        for s_idx in 0..series_count {
            if should_show_graph_series_chip(app, s_idx) {
                let bx = 8 + (s_idx as u32) * 14;
                if bx + 10 < pw && 18 < ph {
                    let col = variable_role_color(lumencraft_doc::VariableRole::Continuous, &app.theme);
                    for y in 8..16 {
                        for x in bx..bx + 10 {
                            img.blend_pixel(x, y, col);
                        }
                    }
                }
            }
        }
    }
    if let Some(ls) = app.document().layout_sheet(sheet_id) {
        for (idx, item) in ls.spec.items.iter().enumerate() {
            let panel_id = format!("panel.{idx}");
            if should_show_layout_panel_chrome(app, &panel_id) {
                let bounds_pt = match item {
                    LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
                        *bounds_pt
                    }
                };
                if let (Some([x0, y0]), Some([x1, y1])) = (
                    local_map.scene_pt_to_pixel(PointPt::new(bounds_pt.x, bounds_pt.y)),
                    local_map.scene_pt_to_pixel(PointPt::new(bounds_pt.x + bounds_pt.width, bounds_pt.y + bounds_pt.height)),
                ) {
                    let rx0 = (x0.round().max(0.0) as u32).min(pw.saturating_sub(1));
                    let ry0 = (y0.round().max(0.0) as u32).min(ph.saturating_sub(1));
                    let rx1 = (x1.round().max(0.0) as u32).min(pw.saturating_sub(1));
                    let ry1 = (y1.round().max(0.0) as u32).min(ph.saturating_sub(1));
                    for x in rx0..=rx1 {
                        img.blend_pixel(x, ry0, pal.accent);
                        img.blend_pixel(x, ry1, pal.accent);
                    }
                    for y in ry0..=ry1 {
                        img.blend_pixel(rx0, y, pal.accent);
                        img.blend_pixel(rx1, y, pal.accent);
                    }
                }
            }
        }
    }
    if let Some(CanvasDragTarget::LayoutItem { preview_rect_pt: Some(preview), .. }) = &app.interaction.canvas_drag
        && let (Some([x0, y0]), Some([x1, y1])) = (
            local_map.scene_pt_to_pixel(PointPt::new(preview.x, preview.y)),
            local_map.scene_pt_to_pixel(PointPt::new(preview.x + preview.width, preview.y + preview.height)),
        )
    {
        let rx0 = (x0.round().max(0.0) as u32).min(pw.saturating_sub(1));
        let ry0 = (y0.round().max(0.0) as u32).min(ph.saturating_sub(1));
        let rx1 = (x1.round().max(0.0) as u32).min(pw.saturating_sub(1));
        let ry1 = (y1.round().max(0.0) as u32).min(ph.saturating_sub(1));
        for x in rx0..=rx1 {
            img.blend_pixel(x, ry0, pal.accent);
            img.blend_pixel(x, ry1, pal.accent);
        }
        for y in ry0..=ry1 {
            img.blend_pixel(rx0, y, pal.accent);
            img.blend_pixel(rx1, y, pal.accent);
        }
    }
    let _ = preflight_check_scene(scene);
    let _ = geom;
}

fn build_inspector_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, None, rect, builder.palette.border, 1);
    builder.push_clip(rect);

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let hdr = LayoutRectPx::new(rect.x + 8, rect.y + 6, rect.width.saturating_sub(16), 22);
    let hdr_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("inspector.title"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: hdr,
        a11y: AccessibilityMeta::new(WidgetRole::Label, builder.app.canvas_dialogs.active_inspector.label(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &hdr_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr,
            pad_x: 2,
            text: builder.app.canvas_dialogs.active_inspector.label(),
            style: &UiTextStyle::ui_bold(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let modes = [(ActiveInspector::FormatGraph, "Graph"), (ActiveInspector::FormatAxes, "Axes"), (ActiveInspector::PairwiseComparisons, "P-Val")];
    let mut tx = rect.x + 8;
    let tw = ((rect.width.saturating_sub(24)) / 3).max(40);
    for (idx, (mode, lbl)) in modes.iter().enumerate() {
        if tx + i32::try_from(tw).unwrap_or(40) > rect.right() - 4 {
            break;
        }
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("inspector.mode.{idx}"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(tx, rect.y + 32, tw.saturating_sub(2), 22),
                label: lbl,
                keyboard_path: &format!("inspector.mode.{idx}"),
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: builder.app.canvas_dialogs.active_inspector == *mode,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::ToggleInspectorTab(*mode),
            },
        );
        tx += i32::try_from(tw + 2).unwrap_or(42);
    }

    let mut cy = rect.y + 62;
    if rect.width >= 120 && cy + 28 < rect.bottom() {
        let norm_zoom = ((builder.app.canvas_dialogs.canvas.zoom - 0.5) / 1.5).clamp(0.0, 1.0);
        emit_slider(
            builder,
            SliderWidgetSpec {
                id: "inspector.zoom_slider",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(rect.x + 8, cy, rect.width.saturating_sub(16), 22),
                label: "Zoom",
                normalized: norm_zoom,
                enablement: ControlEnablement::enabled(),
            },
        );
        cy += 28;
    }

    if builder.app.canvas_dialogs.active_inspector == ActiveInspector::PairwiseComparisons && cy + 24 < rect.bottom() {
        let mode_lbl = pairwise_display_mode_str(builder.app.canvas_dialogs.pairwise.display_mode);
        let row = LayoutRectPx::new(rect.x + 8, cy, rect.width.saturating_sub(16), 22);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "inspector.pairwise_mode",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: row,
                label: mode_lbl,
                keyboard_path: "inspector.pairwise_mode",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::OpenDropdown(String::from("inspector.pairwise_mode")),
            },
        );
    }

    builder.pop_clip();
}

fn build_status_bar_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.y], [rect.right(), rect.y], builder.palette.border, 1);

    let active_title = builder
        .app
        .document()
        .active_sheet_id
        .as_ref()
        .and_then(|id| {
            let doc = builder.app.document();
            doc.data_sheet(id)
                .map(|s| s.title.as_str())
                .or_else(|| doc.result_sheet(id).map(ResultSheet::title))
                .or_else(|| doc.graph_sheet(id).map(|s| s.title.as_str()))
                .or_else(|| doc.layout_sheet(id).map(|s| s.title.as_str()))
                .or_else(|| doc.info_sheet(id).map(|s| s.title.as_str()))
        })
        .unwrap_or("Ready");
    let stats_text = if let Some(qs) = &builder.app.grid.quick_stats
        && qs.numeric_count > 0
    {
        if let Some(mean) = qs.mean {
            format!("{active_title} | n={} | mean={mean:.3}", qs.numeric_count)
        } else {
            format!("{active_title} | n={}", qs.numeric_count)
        }
    } else {
        format!("{active_title} | Zoom {:.0}% | {}", builder.app.canvas_dialogs.canvas.zoom * 100.0, builder.app.locale)
    };
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from("status.summary"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, stats_text.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    let size_px = (11.5 * builder.scale()).clamp(8.5, 20.0);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect,
            pad_x: 8,
            text: &stats_text,
            style: &UiTextStyle::ui_sans(size_px),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
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
        a11y: AccessibilityMeta::new(WidgetRole::Dialog, "Modal Backdrop", FocusScope::ModalDialog).with_keyboard_path("Escape"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalBackdrop, Some(&scrim_wid), full_rect, Rgba8::rgba(15, 23, 42, 128));

    let title = modal_title(&builder.app.canvas_dialogs.active_dialog);
    let modal_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.dialog"),
        parent: None,
        z_layer: ZLayer::ModalDialog,
        rect_px: modal_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Dialog, title.clone(), FocusScope::ModalDialog).with_keyboard_path("Escape"),
        state: WidgetVisualState::default(),
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
        ActiveDialog::PowerAndSampleSize => build_power_wizard_modal_body(builder, parent, body_rect),
        ActiveDialog::CommandPalette => build_command_palette_body(builder, parent, body_rect),
        _ => build_descriptor_modal_body(builder, parent, body_rect),
    }
}

fn build_analyze_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let left_w = (body_rect.width / 3).clamp(110, 210);
    let left_rect = LayoutRectPx::new(body_rect.x, body_rect.y, left_w, body_rect.height);
    let right_rect = LayoutRectPx::new(left_rect.right() + 6, body_rect.y, body_rect.width.saturating_sub(left_w + 6), body_rect.height);

    // Categories on the left (`All`, plus all 7 `AnalysisCategory::ALL`)
    let cat_row_h = 24u32;
    let mut cy = left_rect.y;
    for (idx, cat_opt) in std::iter::once(None).chain(AnalysisCategory::ALL.into_iter().map(Some)).enumerate() {
        if cy + i32::try_from(cat_row_h).unwrap_or(24) > left_rect.bottom() {
            break;
        }
        let label = cat_opt.map_or("All Compatible", AnalysisCategory::label);
        let cat_val = cat_opt.map_or("all", |c| match c {
            AnalysisCategory::Transform => "transform",
            AnalysisCategory::Column => "column",
            AnalysisCategory::Grouped => "grouped",
            AnalysisCategory::XyAndRegression => "xy",
            AnalysisCategory::Survival => "survival",
            AnalysisCategory::Contingency => "contingency",
            AnalysisCategory::Multivariate => "multivariate",
        });
        let selected = builder.app.canvas_dialogs.analyze.category_filter == cat_opt;
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("analyze.cat.{cat_val}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(left_rect.x, cy, left_rect.width, cat_row_h - 2),
                label,
                keyboard_path: &format!("analyze.cat.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10 + idx as u32),
                primary: false,
                selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::SelectAnalyzeCategory(cat_val.to_owned()),
            },
        );
        cy += i32::try_from(cat_row_h).unwrap_or(24);
    }

    // Compatible entries in scrollable list on the right (zero 8-item truncation cap!)
    let entries = builder.app.canvas_dialogs.analyze.compatible_entries();
    let row_h = 24u32;
    let content_h = (entries.len() as u32).saturating_mul(row_h) + 8;
    let scroll_id = String::from("scroll.analyze.entries");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), right_rect, [right_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let list_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("analyze.entries_list"),
        parent: Some(parent),
        z_layer: ZLayer::ModalDialog,
        rect_px: right_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Dialog, "Compatible Analyses", FocusScope::ModalDialog).with_keyboard_path("ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&list_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&list_wid), right_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&list_wid), right_rect, builder.palette.border, 1);

    let list_clip = LayoutRectPx::new(right_rect.x + 2, right_rect.y + 2, right_rect.width.saturating_sub(14), right_rect.height.saturating_sub(4));
    builder.push_clip(list_clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    for (idx, item) in entries.iter().enumerate() {
        let ry = list_clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
        if ry + i32::try_from(row_h).unwrap_or(24) <= list_clip.y || ry >= list_clip.bottom() {
            continue;
        }
        let item_rect = LayoutRectPx::new(list_clip.x, ry, list_clip.width, row_h - 2);
        let id = format!("analyze.entry.{}", item.id);
        let selected = builder.app.canvas_dialogs.analyze.selected_entry_id.as_deref() == Some(item.id);
        let st = widget_state(builder, &id, ControlEnablement::enabled());
        let mut a11y =
            AccessibilityMeta::new(WidgetRole::RadioButton, item.label, FocusScope::ModalDialog).with_keyboard_path(format!("analyze.entry.{idx}"));
        a11y.selected = Some(selected);
        a11y.checked = Some(selected);
        a11y.tab_order = Some(100 + idx as u32);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(id),
            parent: Some(list_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: item_rect,
            a11y,
            state: st.clone(),
            action: WidgetAction::SelectAnalyzeEntry(item.id.to_owned()),
        });
        let bg = if selected {
            builder.palette.accent
        } else if st.hovered {
            builder.palette.header
        } else {
            builder.palette.surface
        };
        let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
        builder.paint_fill(ZLayer::ModalDialog, Some(&wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: item_rect,
                pad_x: 6,
                text: item.label,
                style: &UiTextStyle::ui_sans(size_px),
                color: fg,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(list_wid));
    let _ = ANALYSIS_CATALOG;
}

fn build_edit_params_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let tabs = [
        ("experimental_design", "Experimental Design"),
        ("repeated_measures", "Repeated Measures"),
        ("multiple_comparisons", "Multiple Comparisons"),
        ("options", "Options"),
    ];
    let tab_w = ((body_rect.width.saturating_sub(12)) / (tabs.len() as u32)).max(56);
    for (idx, (tid, lbl)) in tabs.iter().enumerate() {
        let tx = body_rect.x + i32::try_from(idx as u32 * (tab_w + 4)).unwrap_or(0);
        let en = evaluate_edit_params_tab(&builder.app.canvas_dialogs.edit_params, tid);
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("edit.tab.{tid}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(tx, body_rect.y, tab_w, 24),
                label: lbl,
                keyboard_path: &format!("edit.tab.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20 + idx as u32),
                primary: false,
                selected: builder.app.canvas_dialogs.edit_params.active_tab == *tid,
                shortcut: None,
                enablement: en,
                action: WidgetAction::SelectEditParamsTab((*tid).to_owned()),
            },
        );
    }

    let content_rect = LayoutRectPx::new(body_rect.x, body_rect.y + 30, body_rect.width, body_rect.height.saturating_sub(30));
    if content_rect.height < 28 {
        return;
    }
    let edit = &builder.app.canvas_dialogs.edit_params;
    let row_h = 26u32;
    let mut y = content_rect.y + 4;

    if y + i32::try_from(row_h).unwrap_or(26) <= content_rect.bottom() {
        let assume_en = evaluate_dialog_widget(&builder.app.canvas_dialogs, builder.app.document(), "edit.assume_gaussian");
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "edit.assume_gaussian",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(content_rect.x + 4, y, content_rect.width.saturating_sub(16), row_h - 2),
                label: "Assume Gaussian (Parametric ANOVA / t-test)",
                keyboard_path: "edit.assume_gaussian",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(30),
                checked: edit.assume_gaussian,
                is_radio: false,
                enablement: assume_en,
                scroll_container: None,
                action: WidgetAction::ToggleDialogCheckbox(String::from("assume_gaussian")),
            },
        );
        y += i32::try_from(row_h).unwrap_or(26);
    }

    if y + i32::try_from(row_h).unwrap_or(26) <= content_rect.bottom() {
        let paired_en = evaluate_dialog_widget(&builder.app.canvas_dialogs, builder.app.document(), "edit.paired_or_matched");
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "edit.paired_or_matched",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(content_rect.x + 4, y, content_rect.width.saturating_sub(16), row_h - 2),
                label: "Paired / Repeated Measures Matching",
                keyboard_path: "edit.paired_or_matched",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(31),
                checked: edit.paired_or_matched,
                is_radio: false,
                enablement: paired_en,
                scroll_container: None,
                action: WidgetAction::ToggleDialogCheckbox(String::from("paired_or_matched")),
            },
        );
        y += i32::try_from(row_h).unwrap_or(26);
    }

    if y + i32::try_from(row_h).unwrap_or(26) <= content_rect.bottom() {
        let cmp_en = evaluate_dialog_widget(&builder.app.canvas_dialogs, builder.app.document(), "edit.posthoc.tukey");
        let mult_label = match &edit.draft {
            Some(d) => match &d.kind {
                AnalysisKind::OneWayAnova(spec) => format!("{:?}", spec.multiplicity.method),
                _ => String::from("Default"),
            },
            None => String::from("TukeyHsd"),
        };
        emit_dropdown(
            builder,
            DropdownSpec {
                id: "edit.multiplicity_method",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(content_rect.x + 4, y, content_rect.width.saturating_sub(16), row_h - 2),
                label: "Multiple Comparisons",
                selected_value: &mult_label,
                keyboard_path: "edit.multiplicity_method",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(32),
                enablement: cmp_en,
            },
        );
        y += i32::try_from(row_h + 2).unwrap_or(28);
    }

    if y + i32::try_from(row_h).unwrap_or(26) <= content_rect.bottom() {
        let alpha_val = edit.draft.as_ref().map_or(0.05, |d| (1.0 - d.confidence_level.percent() / 100.0).max(0.001));
        emit_numeric_spinner(
            builder,
            SpinnerSpec {
                id_prefix: "edit.alpha",
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(content_rect.x + 4, y, (content_rect.width / 2).max(160), row_h - 2),
                label: "Alpha",
                value_text: &format!("{alpha_val:.3}"),
                step: 0.01,
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(33),
                enablement: ControlEnablement::enabled(),
            },
        );
        y += i32::try_from(row_h + 2).unwrap_or(28);
    }

    let extra_h = content_rect.bottom() - y;
    if extra_h >= 24
        && let Some((_, _, _, _, controls, _)) = crate::inspect::dialog_widget_descriptors(builder.app)
    {
        let extra_rect = LayoutRectPx::new(content_rect.x, y, content_rect.width, extra_h as u32);
        emit_descriptor_controls_grid(builder, &parent, extra_rect, &controls, 40);
    }
}

fn build_power_wizard_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let pw = &builder.app.canvas_dialogs.power_wizard;
    let stages = crate::power_wizard::PowerWizardStage::ALL;
    let tab_w = ((body_rect.width.saturating_sub(12)) / (stages.len() as u32)).max(48);
    for (i, st) in stages.iter().enumerate() {
        let tx = body_rect.x + i32::try_from(i as u32 * (tab_w + 4)).unwrap_or(0);
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("power.stage.{}", i + 1),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(tx, body_rect.y, tab_w, 22),
                label: st.label(),
                keyboard_path: &format!("power.stage.{}", i + 1),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10 + i as u32),
                primary: false,
                selected: pw.stage == *st,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::InvokeCommand { command_id: String::from("ui.powerWizard.open"), params: json!({}) },
            },
        );
    }

    let summary_rect = LayoutRectPx::new(body_rect.x, body_rect.y + 24, body_rect.width, 20u32.min(body_rect.height.saturating_sub(24)));
    let summary = format!("{} | {} | Target Power: {:.2}", pw.stage.label(), pw.family.label(), pw.target_power);
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    if summary_rect.height >= 12 {
        paint_text_fitted(
            builder,
            &parent,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: summary_rect,
                pad_x: 6,
                text: &summary,
                style: &UiTextStyle::ui_sans(size_px),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }

    let avail_h = body_rect.height.saturating_sub(48);
    let ctrl_h = if avail_h >= 120 { avail_h / 2 } else { 0 };
    if ctrl_h >= 24
        && let Some((_, _, _, _, controls, _)) = crate::inspect::dialog_widget_descriptors(builder.app)
    {
        let ctrl_rect = LayoutRectPx::new(body_rect.x, body_rect.y + 46, body_rect.width, ctrl_h);
        emit_descriptor_controls_grid(builder, &parent, ctrl_rect, &controls, 30);
    }

    let curve_y = body_rect.y + 48 + i32::try_from(ctrl_h).unwrap_or(0);
    let curve_h = (body_rect.bottom() - curve_y - 4).max(0) as u32;
    if curve_h >= 28 && body_rect.width >= 80 {
        let curve_rect = LayoutRectPx::new(body_rect.x + 6, curve_y, body_rect.width.saturating_sub(12), curve_h);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from("power.curve_hover"),
            parent: Some(parent),
            z_layer: ZLayer::ModalDialog,
            rect_px: curve_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Canvas, "Power vs N Curve", FocusScope::ModalDialog).with_keyboard_path("ArrowLeft/Right"),
            state: WidgetVisualState::default(),
            action: WidgetAction::FocusWidget(String::from("power.curve_hover")),
        });
        builder.paint_fill(ZLayer::ModalDialog, Some(&wid), curve_rect, builder.palette.panel);
        builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), curve_rect, builder.palette.border, 1);
    }
}

fn build_command_palette_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let search_rect = LayoutRectPx::new(body_rect.x, body_rect.y, body_rect.width, 26);
    emit_text_field(
        builder,
        TextFieldSpec {
            id: "palette.query",
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: search_rect,
            label: "Command Search",
            fallback_text: &builder.app.canvas_dialogs.command_palette.query,
            keyboard_path: "Cmd+K",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(10),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    let matches = builder.app.canvas_dialogs.command_palette.matches_for_session(builder.app.session());
    let list_rect = LayoutRectPx::new(body_rect.x, body_rect.y + 32, body_rect.width, body_rect.height.saturating_sub(32));
    if list_rect.height < 20 {
        return;
    }
    builder.push_clip(list_rect);
    let row_h = 22u32;
    let mut y = list_rect.y;
    for (idx, item) in matches.iter().enumerate() {
        if y + i32::try_from(row_h).unwrap_or(22) > list_rect.bottom() {
            break;
        }
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("palette.hit.{idx}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(list_rect.x, y, list_rect.width, row_h - 2),
                label: &format!("{} ({})", item.label, item.id),
                keyboard_path: &format!("palette.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20 + idx as u32),
                primary: false,
                selected: idx == 0,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::InvokeCommand { command_id: item.id.to_owned(), params: json!({}) },
            },
        );
        y += i32::try_from(row_h).unwrap_or(22);
    }
    builder.pop_clip();
}

fn build_descriptor_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let Some((_, _, _, tabs, controls, _)) = crate::inspect::dialog_widget_descriptors(builder.app) else {
        return;
    };
    let mut content_y = body_rect.y;
    if !tabs.is_empty() && body_rect.height >= 48 {
        let tab_w = (body_rect.width.saturating_sub(8) / (tabs.len() as u32).max(1)).max(56);
        for (idx, tab) in tabs.iter().enumerate() {
            let tx = body_rect.x + i32::try_from(idx as u32 * (tab_w + 4)).unwrap_or(0);
            if tx + i32::try_from(tab_w).unwrap_or(56) > body_rect.right() {
                break;
            }
            let selected = tab.value.as_ref().and_then(|v| v.get("selected")).and_then(|v| v.as_bool()).unwrap_or(false);
            let en = ControlEnablement { enabled: tab.enabled, disabled_reason: tab.disabled_reason.clone() };
            emit_tab(
                builder,
                ButtonWidgetSpec {
                    id: &tab.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(tx, body_rect.y, tab_w, 22),
                    label: &tab.label,
                    keyboard_path: &tab.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(10 + idx as u32),
                    primary: false,
                    selected,
                    shortcut: None,
                    enablement: en,
                    action: WidgetAction::ClickDialogControl(tab.widget_id.clone()),
                },
            );
        }
        content_y += 26;
    }
    let ctrl_h = (body_rect.bottom() - content_y).max(0) as u32;
    if ctrl_h >= 20 {
        emit_descriptor_controls_grid(builder, &parent, LayoutRectPx::new(body_rect.x, content_y, body_rect.width, ctrl_h), &controls, 30);
    }
}

fn emit_descriptor_controls_grid(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    controls: &[crate::inspect::UiWidgetNode],
    base_tab_order: u32,
) {
    let remaining: Vec<&crate::inspect::UiWidgetNode> = controls.iter().filter(|c| builder.tree.node(&c.widget_id).is_none()).collect();
    if remaining.is_empty() || rect.width < 60 || rect.height < 18 {
        return;
    }
    let cols = if rect.width >= 500 {
        3u32
    } else if rect.width >= 300 {
        2u32
    } else {
        1u32
    };
    let row_h = 20u32;
    let total_rows = (remaining.len() as u32).div_ceil(cols);
    let content_h = total_rows.saturating_mul(row_h) + 6;
    let scroll_id = String::from("scroll.modal.controls");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.controls_panel"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Dialog, "Dialog Controls", FocusScope::ModalDialog).with_keyboard_path("ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.panel);

    let clip = LayoutRectPx::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(14), rect.height.saturating_sub(4));
    builder.push_clip(clip);
    let col_w = (clip.width.saturating_sub(cols.saturating_sub(1) * 6) / cols).max(60);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (11.0 * builder.scale()).clamp(8.5, 20.0);
    let st = UiTextStyle::ui_sans(size_px);

    for (idx, ctrl) in remaining.iter().enumerate() {
        let r_idx = (idx as u32) / cols;
        let c_idx = (idx as u32) % cols;
        let cy = clip.y + i32::try_from(r_idx * row_h).unwrap_or(0) - y_scroll;
        if cy + i32::try_from(row_h).unwrap_or(22) <= clip.y || cy >= clip.bottom() {
            continue;
        }
        let cx = clip.x + i32::try_from(c_idx * (col_w + 6)).unwrap_or(0);
        let item_rect = LayoutRectPx::new(cx, cy, col_w, row_h - 2);
        let en = ControlEnablement { enabled: ctrl.enabled, disabled_reason: ctrl.disabled_reason.clone() };
        emit_single_descriptor_control(builder, &panel_wid, item_rect, ctrl, en, base_tab_order + idx as u32, &st, &scroll_id);
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(panel_wid));
}

#[allow(clippy::too_many_arguments)]
fn emit_single_descriptor_control(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    ctrl: &crate::inspect::UiWidgetNode,
    en: ControlEnablement,
    tab_order: u32,
    text_style: &UiTextStyle,
    scroll_id: &str,
) {
    let val_str = ctrl.value.as_ref().map_or_else(String::new, |v| match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    });
    match ctrl.role.as_str() {
        "checkbox" | "radio" => {
            let checked = ctrl.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);
            emit_toggle(
                builder,
                ToggleWidgetSpec {
                    id: &ctrl.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label: &ctrl.label,
                    keyboard_path: &ctrl.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(tab_order),
                    checked,
                    is_radio: ctrl.role == "radio",
                    enablement: en,
                    scroll_container: Some(scroll_id),
                    action: WidgetAction::ClickDialogControl(ctrl.widget_id.clone()),
                },
            );
        }
        "textbox" => {
            emit_text_field(
                builder,
                TextFieldSpec {
                    id: &ctrl.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label: &ctrl.label,
                    fallback_text: &val_str,
                    keyboard_path: &ctrl.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(tab_order),
                    multiline: false,
                    enablement: en,
                },
            );
        }
        "label" => {
            let lbl = if val_str.is_empty() { ctrl.label.clone() } else { format!("{}: {}", ctrl.label, val_str) };
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(ctrl.widget_id.as_str()),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect_px: rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, lbl.clone(), FocusScope::ModalDialog),
                state: widget_state(builder, &ctrl.widget_id, en),
                action: WidgetAction::NoOp,
            });
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    pad_x: 4,
                    text: &lbl,
                    style: text_style,
                    color: builder.palette.text,
                    scroll_container: Some(scroll_id),
                },
            );
        }
        _ => {
            let lbl = if val_str.is_empty() { ctrl.label.clone() } else { format!("{}: {}", ctrl.label, val_str) };
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &ctrl.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label: &lbl,
                    keyboard_path: &ctrl.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(tab_order),
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: en,
                    action: WidgetAction::ClickDialogControl(ctrl.widget_id.clone()),
                },
            );
        }
    }
}

fn modal_title(dialog: &ActiveDialog) -> String {
    match dialog {
        ActiveDialog::None => String::new(),
        ActiveDialog::AnalyzeSheet { .. } => String::from("Analyze Data"),
        ActiveDialog::EditAnalysisParameters { .. } => String::from("Edit Analysis Parameters"),
        ActiveDialog::DuplicateAnalysisPrompt { .. } => String::from("Analysis Already Exists"),
        ActiveDialog::ChangeDataAnalyzed { .. } => String::from("Change Data Analyzed"),
        ActiveDialog::FormatAxes { .. } => String::from("Format Axes"),
        ActiveDialog::FormatGraph { .. } => String::from("Format Graph"),
        ActiveDialog::ChangeGraphType { .. } => String::from("Change Graph Type"),
        ActiveDialog::PairwiseComparisons { .. } => String::from("Pairwise Comparisons"),
        ActiveDialog::BubblePlotInspector { .. } => String::from("Bubble Plot Channels"),
        ActiveDialog::ApplyStyleFrom { .. } => String::from("Apply Style From"),
        ActiveDialog::PageLayoutComposer { .. } => String::from("Page Layout Composer"),
        ActiveDialog::ExportFigure { .. } => String::from("Export Figure"),
        ActiveDialog::Print { .. } => String::from("Print"),
        ActiveDialog::PowerAndSampleSize => String::from("Power & Sample Size"),
        ActiveDialog::CommandPalette => String::from("Command Palette"),
        ActiveDialog::ConvertArchetype { .. } => String::from("Convert Table Archetype"),
        ActiveDialog::ImportReshape => String::from("Import Reshape"),
        ActiveDialog::ExcludeWithReason { .. } => String::from("Exclude with Reason"),
        ActiveDialog::FormatSelectedPoints { .. } => String::from("Format Selected Points"),
        ActiveDialog::RandomizeGroups => String::from("Randomize Groups"),
        ActiveDialog::RandomSampleRows { .. } => String::from("Random Sample Rows"),
        ActiveDialog::SimulateDistributions => String::from("Simulate Distributions"),
        ActiveDialog::InsertModelEquation { .. } => String::from("Insert Model Equation"),
        ActiveDialog::ConstantHookPicker { .. } => String::from("Hook Constant"),
        ActiveDialog::ResultViewer { .. } => String::from("Result Viewer"),
        ActiveDialog::About => String::from("About LumenCraft"),
    }
}
