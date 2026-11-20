//! `Apply Style From` modal body (`gui-layout-spec.md` §8.11).

use egui::Ui;
use serde_json::{Map, Value};

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
                    gctx.intents.push_patch_active_dialog(patch);
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
            gctx.intents.push_dialog_widget_click(wid);
        }
    }
}
