//! `Analyze...` category filter and compatible analysis picker modal body (`R10.3`).

use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::{ANALYSIS_CATALOG, AnalysisCategory};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{ButtonWidgetSpec, FittedTextSpec, emit_button, emit_scrollbars_for_meta, paint_text_fitted, widget_state};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::{labeled_meta, selectable_meta};

/// Transient view state owned by the `Analyze...` picker shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnalyzePickerViewState {
    pub search_query: String,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `analyze_picker` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut AnalyzePickerViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the `Analyze Data` category sidebar and scrollable compatible analysis list inside `body_rect`.
pub fn build_analyze_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
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
        a11y: labeled_meta(WidgetRole::Dialog, "Compatible Analyses", FocusScope::ModalDialog, "ArrowUp/Down"),
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
        let mut a11y = selectable_meta(WidgetRole::RadioButton, item.label, FocusScope::ModalDialog, format!("analyze.entry.{idx}"), selected);
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
