//! Statistical and group-comparison `Edit Analysis Parameters` modal view builder (`R10.3`).

use curvebook_doc::AnalysisKind;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::FocusScope;
use crate::layout::widgets::{DropdownSpec, SpinnerSpec, ToggleWidgetSpec, emit_dropdown, emit_numeric_spinner, emit_tab, emit_toggle};
use crate::layout::{LayoutBuilder, LayoutRectPx, WidgetAction, WidgetId, ZLayer};
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_widget, evaluate_edit_params_tab};

use super::dialogs_graph_misc::emit_descriptor_controls_grid;
use super::fit_params::{build_fit_params_extra_section, is_fit_analysis_draft};

/// Transient view state owned by the statistical parameters modal shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatParamsViewState {
    pub active_section: String,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `stat_params` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut StatParamsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the multi-tab `Edit Analysis Parameters` modal body inside `body_rect`.
pub fn build_edit_params_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
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
            crate::layout::widgets::ButtonWidgetSpec {
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
        if is_fit_analysis_draft(builder.app) {
            build_fit_params_extra_section(builder, &parent, extra_rect, &controls);
        } else {
            emit_descriptor_controls_grid(builder, &parent, extra_rect, &controls, 40);
        }
    }
}
