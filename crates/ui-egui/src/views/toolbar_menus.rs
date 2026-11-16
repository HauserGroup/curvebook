//! Menu bar, contextual toolbar, dropdown popup menus, and status bar view builders (`R10.6`).

use curvebook_doc::ResultSheet;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{ButtonWidgetSpec, FittedTextSpec, emit_button, emit_scrollbars_for_meta, paint_text_fitted, widget_state};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer, UiTextStyle};

use super::a11y::labeled_meta;

/// Transient view state owned by the toolbar / menus / status bar shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolbarMenusViewState {
    pub overflow_open: bool,
    pub active_context_menu_id: Option<String>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `toolbar_menus` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ToolbarMenusViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the top menu bar + toolbar strip inside `rect`.
pub fn build_toolbar_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
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

/// Build the open top-level menu dropdown popup anchored below `toolbar_rect`.
pub fn build_open_menu_popup(builder: &mut LayoutBuilder<'_>, menu_key: &str, toolbar_rect: LayoutRectPx) {
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
        a11y: labeled_meta(WidgetRole::ContextMenu, format!("{} Menu", section.title), FocusScope::PopupMenu, menu_key),
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
        let mut a11y = labeled_meta(
            WidgetRole::MenuItem,
            tip.localized_label,
            FocusScope::PopupMenu,
            if tip.shortcut.is_empty() { tip.command_id } else { tip.shortcut.as_str() },
        );
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

/// Build the bottom workspace status bar inside `rect`.
pub fn build_status_bar_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
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
