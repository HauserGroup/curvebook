//! `Insert Model Equation` modal body (`gui-layout-spec.md` §8.11).

use curvebook_doc::EquationDisplayMode;
use egui::Ui;
use serde_json::{Map, Value, json};

use super::super::UiIntent;
use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};
use super::state::GraphModalEguiState;

pub(super) fn render_insert_equation_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let eq = &gctx.app.canvas_dialogs.insert_model_equation;
    if eq.available_result_ids.is_empty() {
        ui.label(egui::RichText::new("No regression ResultSheet available to link.").color(gctx.tokens.text_muted));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Regression Result:");
            for (idx, rid) in eq.available_result_ids.iter().enumerate() {
                let selected = eq.result_sheet_id.as_ref() == Some(rid);
                let title = gctx.app.document().result_sheet(rid).map_or_else(|| rid.as_str().to_owned(), |r| r.title().to_owned());
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("eq.result.{idx}"),
                        label: &title,
                        command_id: None,
                        selected,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut patch = Map::new();
                    patch.insert(String::from("result_sheet_id"), Value::String(rid.as_str().to_owned()));
                    gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
                }
            }
        });
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Display Mode:");
        for (lbl, mode, slug) in
            [("Substituted Values", EquationDisplayMode::Substituted, "substituted"), ("Symbolic Formula", EquationDisplayMode::Symbolic, "symbolic")]
        {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("eq.mode.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: eq.mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut patch = Map::new();
                patch.insert(String::from("mode"), Value::String(slug.to_owned()));
                gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
            }
        }
    });

    ui.add_space(4.0);
    let font_cfg = ScrubUnitFieldConfig {
        step_per_pt: 1.0,
        decimals: 1,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("eq.font_size", "Equation Font Size", 6.0, 36.0)
    };
    let out = render_scrub_unit_field(ui, eq.font_size_pt, &mut state.eq_font_spinner, font_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = out.committed_value {
        let mut patch = Map::new();
        patch.insert(String::from("font_size_pt"), json!(val.clamp(6.0, 36.0)));
        gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
    }

    let mut bordered = eq.bordered;
    let brd_chk = ui.checkbox(&mut bordered, "Draw Bordered Callout Box");
    record_checkbox(ui, &brd_chk, gctx.meta, "eq.bordered", "Draw Bordered Callout Box", bordered);
    if brd_chk.changed() {
        let mut patch = Map::new();
        patch.insert(String::from("bordered"), Value::Bool(bordered));
        gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
    }
}
