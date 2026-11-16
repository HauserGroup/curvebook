//! Graph/layout canvas view, rulers, command palette, and descriptor-driven modal helpers (`R10.3`, `R10.15`).

use std::sync::Arc;

use curvebook_chart::{ChartScene, PointPt, preflight_check_scene};
use curvebook_doc::{LayoutItem, SheetId};
use curvebook_palette::{CanvasPreviewFilter, Rgba8};
use curvebook_render::{RasterImage, RenderTargetSpec, apply_canvas_preview_filter, render_scene};
use curvebook_tools::canvas::{SnapOrientation, SnapSource};
use curvebook_tools::{PhysicalUnit, build_ruler_ticks};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::compositor::{FramePalette, WorkspacePixelRect};
use crate::dialogs_canvas::{ActiveDialog, variable_role_color};
use crate::interaction::{CanvasDragTarget, ScenePaperMapping, UiEventOutcome};
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::canvas_viewport::{
    CanvasViewportGeometry, compute_aspect_ratio_paper_mapping, should_show_graph_series_chip, should_show_layout_panel_chrome,
};
use crate::layout::paint::{PaintOp, PaintOpKind};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, emit_button, emit_scrollbars_for_meta, emit_tab, emit_text_field, emit_toggle,
    paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::labeled_meta;

/// Transient view state owned by the graph / layout / miscellaneous dialogs shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DialogsGraphMiscViewState {
    pub active_section: String,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `dialogs_graph_misc` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Draw canvas top/left physical rulers when visible and return the remaining center viewport rectangle.
pub fn draw_rulers_if_visible(builder: &mut LayoutBuilder<'_>, canvas: WorkspacePixelRect) -> LayoutRectPx {
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

/// Render an aspect-ratio-preserving graph or page-layout sheet canvas into `outer_rect`.
pub fn build_canvas_or_layout_view(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, outer_rect: LayoutRectPx, slot_tag: &str) {
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
        a11y: labeled_meta(WidgetRole::Canvas, format!("Canvas {}", sheet_id.as_str()), FocusScope::Workspace, "Cmd+0"),
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

/// Emit zoom-to-fit, zoom-percentage buttons, and pan scrollbars for `geom`.
pub fn draw_canvas_zoom_controls(builder: &mut LayoutBuilder<'_>, geom: &CanvasViewportGeometry, slot_tag: &str) {
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

/// Draw active snap guides, hover/selected series chips, and layout panel selection chrome onto `img`.
pub fn draw_scene_overlays(
    img: &mut RasterImage,
    app: &UiApp,
    sheet_id: &SheetId,
    scene: &ChartScene,
    geom: &CanvasViewportGeometry,
    pal: &FramePalette,
) {
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
                    let col = variable_role_color(curvebook_doc::VariableRole::Continuous, &app.theme);
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

/// Build the `Cmd/Ctrl+K` command palette search input and matched command rows inside `body_rect`.
pub fn build_command_palette_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
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

/// Build a generic descriptor-driven modal body from [`crate::inspect::dialog_widget_descriptors`].
pub fn build_descriptor_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
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

/// Emit a scrollable multi-column grid of descriptor controls inside `rect`.
pub fn emit_descriptor_controls_grid(
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
        a11y: labeled_meta(WidgetRole::Dialog, "Dialog Controls", FocusScope::ModalDialog, "ArrowUp/Down"),
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

/// Emit a single descriptor-backed dialog control widget inside `rect`.
#[allow(clippy::too_many_arguments)]
pub fn emit_single_descriptor_control(
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
                crate::layout::widgets::ToggleWidgetSpec {
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

/// Return the canonical window title string for an [`ActiveDialog`].
#[must_use]
pub fn modal_title(dialog: &ActiveDialog) -> String {
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
        ActiveDialog::About => String::from("About CurveBook"),
    }
}
