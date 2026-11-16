//! Interactive Power & Sample Size 5-stage wizard modal body (`R10.3`).

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{FocusScope, WidgetRole};
use crate::layout::widgets::{ButtonWidgetSpec, FittedTextSpec, emit_tab, paint_text_fitted};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::labeled_meta;
use super::dialogs_graph_misc::emit_descriptor_controls_grid;

/// Transient view state owned by the Power & Sample Size wizard shard.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PowerWizardViewState {
    pub hover_sample_size: Option<u32>,
    pub hover_power: Option<f64>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `power_wizard` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut PowerWizardViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the 5-stage Power & Sample Size wizard body inside `body_rect`.
pub fn build_power_wizard_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
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
            a11y: labeled_meta(WidgetRole::Canvas, "Power vs N Curve", FocusScope::ModalDialog, "ArrowLeft/Right"),
            state: WidgetVisualState::default(),
            action: WidgetAction::FocusWidget(String::from("power.curve_hover")),
        });
        builder.paint_fill(ZLayer::ModalDialog, Some(&wid), curve_rect, builder.palette.panel);
        builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), curve_rect, builder.palette.border, 1);
    }
}
