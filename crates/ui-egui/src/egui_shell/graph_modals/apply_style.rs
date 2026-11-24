//! `Apply Style From` modal body (`gui-layout-spec.md` §8.11).

use crate::egui_shell::UiIntent;
use curvebook_doc::SheetId;
use egui::Ui;
use serde_json::{Map, Value};

use crate::i18n::tr;

use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};

pub(super) fn render_apply_style_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let asf = &gctx.app.canvas_dialogs.apply_style;
    ui.label(egui::RichText::new("Source Graph Style Donor").strong());
    if asf.donor_candidates.is_empty() {
        ui.label(egui::RichText::new("Create another graph sheet to clone styling from.").color(gctx.tokens.text_muted));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (id, title)) in asf.donor_candidates.iter().enumerate() {
                let selected = asf.donor_graph_id.as_ref() == Some(id);
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("apply_style.donor.{idx}"),
                        label: title,
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
                    patch.insert(String::from("donor_graph_id"), Value::String(id.as_str().to_owned()));
                    gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
                }
            }
        });
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.apply_style.targets")).strong());
    if !asf.target_candidates.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            let sel_all_lbl = tr(gctx.locale, "r13.apply_style.select_all");
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "apply_style.target.select_all",
                    label: sel_all_lbl,
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = asf.clone();
                let mut all_targets: Vec<SheetId> =
                    asf.target_candidates.iter().filter(|(id, _)| asf.donor_graph_id.as_ref() != Some(id)).map(|(id, _)| id.clone()).collect();
                if all_targets.is_empty() {
                    all_targets = asf.target_candidates.iter().map(|(id, _)| id.clone()).collect();
                }
                next.target_graph_id = all_targets.first().cloned();
                next.target_graph_ids = all_targets;
                gctx.intents.push(UiIntent::UpdateApplyStyleState(Box::new(next)));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (id, title)) in asf.target_candidates.iter().enumerate() {
                let mut checked = asf.target_graph_ids.contains(id) || (asf.target_graph_ids.is_empty() && asf.target_graph_id.as_ref() == Some(id));
                let chk = ui.checkbox(&mut checked, title);
                record_checkbox(ui, &chk, gctx.meta, &format!("apply_style.target.{idx}"), title, checked);
                if chk.changed() {
                    let mut next = asf.clone();
                    if next.target_graph_ids.is_empty()
                        && let Some(cur) = next.target_graph_id.clone()
                    {
                        next.target_graph_ids.push(cur);
                    }
                    if checked {
                        if !next.target_graph_ids.contains(id) {
                            next.target_graph_ids.push(id.clone());
                        }
                    } else {
                        next.target_graph_ids.retain(|t| t != id);
                    }
                    next.target_graph_id = next.target_graph_ids.first().cloned();
                    gctx.intents.push(UiIntent::UpdateApplyStyleState(Box::new(next)));
                }
            }
        });
    }

    ui.add_space(6.0);
    for (wid, lbl, val) in [
        ("apply_style.clone_palette", "Clone Palette & Swatches", asf.clone_palette),
        ("apply_style.clone_frame_and_axes", "Clone Frame & Axis Styling", asf.clone_frame_and_axes),
        ("apply_style.clone_error_bars_and_legend", "Clone Error Bars & Legend", asf.clone_error_bars_and_legend),
    ] {
        let mut cur = val;
        let chk = ui.checkbox(&mut cur, lbl);
        record_checkbox(ui, &chk, gctx.meta, wid, lbl, cur);
        if chk.changed() {
            gctx.intents.push(UiIntent::DialogWidgetClick { widget_id: (wid).into() });
        }
    }
}
