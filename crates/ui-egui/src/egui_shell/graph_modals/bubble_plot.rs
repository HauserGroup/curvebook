//! `Bubble Plot Channels` modal body (`gui-layout-spec.md` §8.11).

use curvebook_doc::{ColorChannelMapping, GraphicalDataObject, MultivariatePlotMode, SizeChannelMapping, VariableRole, WhiskerMethod};
use curvebook_palette::{ColormapId, PaletteId};
use egui::Ui;

use crate::dialogs_canvas::BubblePlotInspectorState;

use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};
use super::state::GraphModalEguiState;

fn col_title(bp: &BubblePlotInspectorState, idx: usize) -> &str {
    bp.available_columns.get(idx).map_or("Var", |c| c.title.as_str())
}

pub(super) fn render_bubble_plot_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let bp = gctx.app.canvas_dialogs.bubble_plot.clone();
    let n_cols = bp.available_columns.len().max(1);

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Plot Mode:");
        let is_bubble = matches!(bp.plot_mode, MultivariatePlotMode::BubbleScatter);
        let is_cat_summary = matches!(bp.plot_mode, MultivariatePlotMode::CategoricalSummary { .. });
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.plot_mode.bubble_scatter",
                label: "Bubble Scatter",
                command_id: None,
                selected: is_bubble,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            next.plot_mode = MultivariatePlotMode::BubbleScatter;
            gctx.intents.push_update_bubble_plot_state(next);
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.plot_mode.categorical_summary",
                label: "Categorical Summary",
                command_id: None,
                selected: is_cat_summary,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            let color = PaletteId::OkabeIto.color_at(0);
            next.plot_mode = MultivariatePlotMode::CategoricalSummary {
                gdo: GraphicalDataObject::BoxAndWhisker {
                    whiskers: WhiskerMethod::Tukey1_5Iqr,
                    fill_color: color.with_alpha(190),
                    stroke_color: color,
                    show_outliers: true,
                    show_all_points: false,
                },
                y_error_bars: None,
            };
            gctx.intents.push_update_bubble_plot_state(next);
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!(
            "X Column: #{} ({}) | Y Column: #{} ({})",
            bp.x_column + 1,
            col_title(&bp, bp.x_column),
            bp.y_column + 1,
            col_title(&bp, bp.y_column)
        ));
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
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let color_summary = match &bp.color_channel {
            ColorChannelMapping::Constant(_) => String::from("Color: Uniform"),
            ColorChannelMapping::Categorical { column, .. } => format!("Color: Cat #{} ({})", column + 1, col_title(&bp, *column)),
            ColorChannelMapping::Continuous { column, .. } => format!("Color: Cont #{} ({})", column + 1, col_title(&bp, *column)),
        };
        ui.label(color_summary);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.color_col_next",
                label: "Cycle Color Col",
                command_id: None,
                selected: false,
                enabled: n_cols > 1 || matches!(bp.color_channel, ColorChannelMapping::Constant(_)),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            match &bp.color_channel {
                ColorChannelMapping::Constant(_) => {
                    let is_numeric =
                        bp.available_columns.first().is_some_and(|c| matches!(c.role, VariableRole::Continuous | VariableRole::Calculated));
                    if is_numeric {
                        next.set_color_continuous(0, ColormapId::Viridis, None);
                    } else {
                        next.set_color_categorical(0, PaletteId::OkabeIto);
                    }
                }
                ColorChannelMapping::Categorical { column, palette, .. } => {
                    next.set_color_categorical((column + 1) % n_cols, *palette);
                }
                ColorChannelMapping::Continuous { column, ramp, domain_clamp } => {
                    next.color_channel =
                        ColorChannelMapping::Continuous { column: (column + 1) % n_cols, ramp: ramp.clone(), domain_clamp: *domain_clamp };
                }
            }
            gctx.intents.push_update_bubble_plot_state(next);
        }
    });

    ui.add_space(4.0);
    let is_size_var = matches!(bp.size_channel, SizeChannelMapping::Variable { .. });
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.size_variable",
                label: "Variable Size",
                command_id: None,
                selected: is_size_var,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            if is_size_var {
                next.size_channel = SizeChannelMapping::Constant { radius_pt: 5.0 };
            } else {
                next.size_channel = SizeChannelMapping::Variable { column: bp.x_column.min(n_cols - 1), min_radius_pt: 2.5, max_radius_pt: 14.0 };
            }
            gctx.intents.push_update_bubble_plot_state(next);
        }
        if let SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt } = bp.size_channel {
            ui.label(format!("Size: #{} ({})", column + 1, col_title(&bp, column)));
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "bubble.size_col_next",
                    label: "Cycle Size Col",
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
                next.set_size_variable((column + 1) % n_cols, min_radius_pt, max_radius_pt);
                gctx.intents.push_update_bubble_plot_state(next);
            }
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
