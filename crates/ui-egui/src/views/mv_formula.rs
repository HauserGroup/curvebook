//! Multiple-variables badges and calculated-variable formula bar helpers (`R10.8`).

use curvebook_doc::DataSheet;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::FocusScope;
use crate::layout::widgets::{ButtonWidgetSpec, TextFieldSpec, emit_button, emit_text_field};
use crate::layout::{LayoutBuilder, LayoutRectPx, WidgetAction, ZLayer};
use crate::modal_predicates::ControlEnablement;

/// Transient view state owned by the multiple-variables / formula bar shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MvFormulaViewState {
    pub function_search: String,
    pub show_formula_definitions: bool,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `mv_formula` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut MvFormulaViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Emit the calculated-variable formula bar strip when active or on a `MultipleVariables` sheet,
/// returning its allocated pixel height (`24` or `0`).
pub fn build_formula_bar_row(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, sheet_rect: LayoutRectPx, show_formula: bool) -> u32 {
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
    formula_h
}

/// Emit the per-column `fx` button inside a `MultipleVariables` data column header.
pub fn emit_mv_column_fx_button(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    col: usize,
    lbl_rect: LayoutRectPx,
    h_rect: LayoutRectPx,
    fx_w: u32,
) {
    if fx_w == 0 {
        return;
    }
    let fx_rect = LayoutRectPx::new(lbl_rect.right(), h_rect.y, fx_w, h_rect.height);
    let en = if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() };
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("formula_bar.open_col.{col}"),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: fx_rect,
            label: "fx",
            keyboard_path: "F2",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: builder.app.grid.formula_bar.active && builder.app.grid.formula_bar.target_column == Some(col),
            shortcut: None,
            enablement: en,
            action: WidgetAction::InvokeCommand { command_id: String::from("ui.table.openFormulaBar"), params: json!({ "column": col }) },
        },
    );
}
