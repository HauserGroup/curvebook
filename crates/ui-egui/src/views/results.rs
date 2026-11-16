//! Multi-column results viewer view builder (`R10.5`).

use curvebook_doc::ResultSheet;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{AssumptionDiagnosticBanner, ResultSheetViewState};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{ButtonWidgetSpec, FittedTextSpec, emit_button, emit_scrollbars_for_meta, emit_tab, paint_text_fitted};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer, UiTextStyle};

/// Transient view state owned by the results viewer shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResultsViewState {
    pub filter_text: String,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `results` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ResultsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the scrollable multi-tab [`ResultSheet`] viewer inside `rect`.
pub fn build_result_viewer_view(builder: &mut LayoutBuilder<'_>, rs: &ResultSheet, rect: LayoutRectPx, slot_tag: &str) {
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
        if slot_tag != "secondary" || builder.tree.result_banner_rect.is_none() {
            builder.tree.result_banner_rect = Some(banner_rect.to_workspace_rect());
            builder.tree.result_banner_sheet_id = Some(rs.id.clone());
        }
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
