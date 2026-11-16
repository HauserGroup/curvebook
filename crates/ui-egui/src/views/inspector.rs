//! Live right-hand inspector view builder (`R10.18`).

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveInspector, pairwise_display_mode_str};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::widgets::{ButtonWidgetSpec, FittedTextSpec, SliderWidgetSpec, emit_button, emit_slider, emit_tab, paint_text_fitted};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

/// Transient view state owned by the right-hand inspector shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InspectorViewState {
    pub collapsed_sections: Vec<String>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `inspector` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut InspectorViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the docked right-hand inspector panel inside `rect`.
pub fn build_inspector_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
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
