//! `Bubble Plot Channels` modal body (`gui-layout-spec.md` §8.11).

use curvebook_doc::SizeChannelMapping;
use egui::Ui;

use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};
use super::state::GraphModalEguiState;

pub(super) fn render_bubble_plot_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let bp = gctx.app.canvas_dialogs.bubble_plot.clone();
    let n_cols = bp.available_columns.len().max(1);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("X Column: #{} | Y Column: #{}", bp.x_column + 1, bp.y_column + 1));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.x_next",
                label: "Cycle X",
                command_id: None,
                selected: false,
                enabled: n_cols > 1,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            next.x_column = (next.x_column + 1) % n_cols;
            gctx.intents.push_update_bubble_plot_state(next);
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.y_next",
                label: "Cycle Y",
                command_id: None,
                selected: false,
                enabled: n_cols > 1,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            next.y_column = (next.y_column + 1) % n_cols;
            gctx.intents.push_update_bubble_plot_state(next);
        }
    });

    ui.add_space(4.0);
    match bp.size_channel {
        SizeChannelMapping::Constant { radius_pt } => {
            let cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.size_pt", "Bubble Radius", 1.0, 24.0)
            };
            let out = render_scrub_unit_field(ui, radius_pt, &mut state.mv_min_radius_spinner, cfg, gctx.tokens, gctx.meta);
            if let Some(v) = out.committed_value {
                let mut next = bp.clone();
                next.size_channel = SizeChannelMapping::Constant { radius_pt: v.clamp(1.0, 24.0) };
                gctx.intents.push_update_bubble_plot_state(next);
            }
        }
        SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt } => {
            let min_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.min_radius", "Min Radius", 1.0, 24.0)
            };
            let min_out = render_scrub_unit_field(ui, min_radius_pt, &mut state.mv_min_radius_spinner, min_cfg, gctx.tokens, gctx.meta);
            if let Some(v) = min_out.committed_value {
                let mut next = bp.clone();
                next.set_size_variable(column, v, max_radius_pt);
                gctx.intents.push_update_bubble_plot_state(next);
            }

            let max_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.max_radius", "Max Radius", 1.0, 36.0)
            };
            let max_out = render_scrub_unit_field(ui, max_radius_pt, &mut state.mv_max_radius_spinner, max_cfg, gctx.tokens, gctx.meta);
            if let Some(v) = max_out.committed_value {
                let mut next = bp.clone();
                next.set_size_variable(column, min_radius_pt, v);
                gctx.intents.push_update_bubble_plot_state(next);
            }
        }
    }

    ui.add_space(4.0);
    let mut hulls = bp.show_convex_hulls;
    let h_chk = ui.checkbox(&mut hulls, "Show Convex Hulls");
    record_checkbox(ui, &h_chk, gctx.meta, "bubble.show_convex_hulls", "Show Convex Hulls", hulls);
    if h_chk.changed() {
        gctx.intents.push_dialog_widget_click("bubble.show_convex_hulls");
    }

    let mut ellipses = bp.show_confidence_ellipses;
    let e_chk = ui.checkbox(&mut ellipses, "Show Confidence Ellipses");
    record_checkbox(ui, &e_chk, gctx.meta, "bubble.show_confidence_ellipses", "Show Confidence Ellipses", ellipses);
    if e_chk.changed() {
        gctx.intents.push_dialog_widget_click("bubble.show_confidence_ellipses");
    }
}
