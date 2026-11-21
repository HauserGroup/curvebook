//! Multivariate (MV) channel-layer `Format Graph` schema tabs: Axis Variables, Data Objects,
//! and Legends.

use curvebook_doc::{
    ColorChannelMapping, GraphicalDataObject, MultivariateGraphEncoding, MultivariatePlotMode, SheetId, SizeChannelMapping, TableArchetype,
    WhiskerMethod,
};
use curvebook_palette::ColorRamp;
use egui::Ui;

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;

use super::super::super::PendingUiIntents;
use super::super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_input};
use super::super::state::{GraphModalEguiState, MvOutOfRangePolicy};
use super::series::render_fg_settings;

pub(super) fn render_fg_mv_axis_variables(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let Some(mut enc) = fg
        .mv_encoding
        .clone()
        .or_else(|| fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).and_then(|gs| gs.spec.multivariate.clone()))
    else {
        ui.label("No Multivariate encoding attached to this graph.");
        return;
    };

    ui.label(egui::RichText::new("Plot Mode").strong());
    let is_bubble = matches!(enc.plot_mode, MultivariatePlotMode::BubbleScatter);
    let is_cat_summary = matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. });
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.plot_mode.bubble_scatter",
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
            enc.plot_mode = MultivariatePlotMode::BubbleScatter;
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.plot_mode.categorical_summary",
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
            let swatch = fg.palette.color_at(0);
            enc.plot_mode = MultivariatePlotMode::CategoricalSummary {
                gdo: GraphicalDataObject::BoxAndWhisker {
                    whiskers: WhiskerMethod::Tukey1_5Iqr,
                    fill_color: swatch.with_alpha(190),
                    stroke_color: swatch,
                    show_outliers: true,
                    show_all_points: false,
                },
                y_error_bars: None,
            };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
    });

    ui.add_space(6.0);
    let var_names = collect_mv_variable_names(gctx.app, fg.graph_sheet_id.as_ref());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("X Variable: col {} ({})", enc.x_column + 1, var_name(&var_names, enc.x_column)));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.x_next",
                label: "Next X Col",
                command_id: None,
                selected: false,
                enabled: !var_names.is_empty(),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && !var_names.is_empty()
        {
            enc.x_column = (enc.x_column + 1) % var_names.len();
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        ui.label(format!("Y Variable: col {} ({})", enc.y_column + 1, var_name(&var_names, enc.y_column)));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.y_next",
                label: "Next Y Col",
                command_id: None,
                selected: false,
                enabled: !var_names.is_empty(),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && !var_names.is_empty()
        {
            enc.y_column = (enc.y_column + 1) % var_names.len();
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Color Channel Mapping").strong());
    let is_cat = matches!(enc.color, ColorChannelMapping::Categorical { .. });
    let is_cont = matches!(enc.color, ColorChannelMapping::Continuous { .. });
    let is_const = matches!(enc.color, ColorChannelMapping::Constant(_));
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_constant",
                label: "Uniform",
                command_id: None,
                selected: is_const,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            enc.color = ColorChannelMapping::Constant(fg.palette.color_at(0));
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_categorical",
                label: "Categorical",
                command_id: None,
                selected: is_cat,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let col = enc.x_column;
            enc.color = ColorChannelMapping::Categorical { column: col, palette: fg.palette, level_order_override: None };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_continuous",
                label: "Continuous Ramp",
                command_id: None,
                selected: is_cont,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let col = enc.y_column;
            enc.color = ColorChannelMapping::Continuous { column: col, ramp: ColorRamp::default(), domain_clamp: None };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        let color_col_summary = match &enc.color {
            ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => {
                format!("Next Color Col (#{} {})", column + 1, var_name(&var_names, *column))
            }
            ColorChannelMapping::Constant(_) => String::from("Next Color Col"),
        };
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_col_next",
                label: &color_col_summary,
                command_id: None,
                selected: false,
                enabled: (is_cat || is_cont) && !var_names.is_empty(),
                disabled_reason: (!is_cat && !is_cont).then_some("Select Categorical or Continuous color mapping first"),
            },
            gctx.tokens,
        )
        .clicked()
            && !var_names.is_empty()
        {
            let updated = match &mut enc.color {
                ColorChannelMapping::Categorical { column, level_order_override, .. } => {
                    *column = (*column + 1) % var_names.len();
                    *level_order_override = None;
                    true
                }
                ColorChannelMapping::Continuous { column, .. } => {
                    *column = (*column + 1) % var_names.len();
                    true
                }
                ColorChannelMapping::Constant(_) => false,
            };
            if updated {
                push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
            }
        }
    });

    if !fg.mv_levels.is_empty() {
        ui.add_space(6.0);
        ui.label(format!("Categorical Levels ({}): {}", if fg.mv_level_is_x { "X Axis" } else { "Color" }, fg.mv_levels.join(", ")));
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (wid, op, lbl) in [
                ("format_graph.mv.level_cycle", "cycle", "Next Level"),
                ("format_graph.mv.level_top", "top", "Top"),
                ("format_graph.mv.level_up", "up", "Move Up"),
                ("format_graph.mv.level_down", "down", "Move Down"),
                ("format_graph.mv.level_end", "end", "End"),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry { widget_id: wid, label: lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    if op == "cycle" {
                        next.cycle_selected_mv_level();
                    } else {
                        let _ = next.reorder_mv_level(op);
                    }
                    gctx.intents.push_update_format_graph_state(next);
                }
            }
        });
    }
}

pub(super) fn render_fg_mv_data_objects(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let Some(mut enc) = fg
        .mv_encoding
        .clone()
        .or_else(|| fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).and_then(|gs| gs.spec.multivariate.clone()))
    else {
        ui.label("No Multivariate data layers attached.");
        return;
    };

    ui.label(egui::RichText::new("Layer Overlays").strong());
    let mut ellipses = enc.show_confidence_ellipses;
    let ell_chk = ui.checkbox(&mut ellipses, "Confidence Ellipses (95%)");
    record_checkbox(ui, &ell_chk, gctx.meta, "format_graph.mv.confidence_ellipses", "Confidence Ellipses (95%)", ellipses);
    if ell_chk.changed() {
        enc.show_confidence_ellipses = ellipses;
        push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
    }

    let mut hulls = enc.show_convex_hulls;
    let hull_chk = ui.checkbox(&mut hulls, "Convex Hulls");
    record_checkbox(ui, &hull_chk, gctx.meta, "format_graph.mv.convex_hulls", "Convex Hulls", hulls);
    if hull_chk.changed() {
        enc.show_convex_hulls = hulls;
        push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Symbol Sizing Mode").strong());
    let is_var = matches!(enc.size, SizeChannelMapping::Variable { .. });
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.size_uniform",
                label: "Uniform Diameter",
                command_id: None,
                selected: !is_var,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            enc.size = SizeChannelMapping::Constant { radius_pt: 5.0 };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.size_variable",
                label: "Continuous Variable",
                command_id: None,
                selected: is_var,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            enc.size = SizeChannelMapping::Variable { column: enc.y_column, min_radius_pt: 2.5, max_radius_pt: 14.0 };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
    });

    ui.add_space(4.0);
    let var_names = collect_mv_variable_names(gctx.app, fg.graph_sheet_id.as_ref());
    match enc.size {
        SizeChannelMapping::Constant { radius_pt } => {
            let cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.constant_radius", "Symbol Radius", 1.0, 24.0)
            };
            let out = render_scrub_unit_field(ui, radius_pt, &mut state.mv_min_radius_spinner, cfg, gctx.tokens, gctx.meta);
            if let Some(val) = out.committed_value {
                enc.size = SizeChannelMapping::Constant { radius_pt: val.clamp(1.0, 24.0) };
                push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
            }
        }
        SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt } => {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                ui.label(format!("Size Variable: col {} ({})", column + 1, var_name(&var_names, column)));
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "format_graph.mv.size_col_next",
                        label: "Next Size Col",
                        command_id: None,
                        selected: false,
                        enabled: !var_names.is_empty(),
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                    && !var_names.is_empty()
                {
                    enc.size = SizeChannelMapping::Variable { column: (column + 1) % var_names.len(), min_radius_pt, max_radius_pt };
                    push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
                }
            });

            let min_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.min_radius", "Min Radius", 1.0, 24.0)
            };
            let min_out = render_scrub_unit_field(ui, min_radius_pt, &mut state.mv_min_radius_spinner, min_cfg, gctx.tokens, gctx.meta);
            if let Some(min_val) = min_out.committed_value {
                let lo = min_val.clamp(1.0, 24.0);
                enc.size = SizeChannelMapping::Variable { column, min_radius_pt: lo, max_radius_pt: max_radius_pt.max(lo) };
                push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
            }

            let max_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.max_radius", "Max Radius", 1.0, 36.0)
            };
            let max_out = render_scrub_unit_field(ui, max_radius_pt, &mut state.mv_max_radius_spinner, max_cfg, gctx.tokens, gctx.meta);
            if let Some(max_val) = max_out.committed_value {
                let hi = max_val.clamp(min_radius_pt, 36.0);
                enc.size = SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt: hi };
                push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
            }
        }
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Color Domain Clamp").strong());
    let mut clamp_edited = false;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Clamp Min:");
        let min_resp = ui.add(egui::TextEdit::singleline(&mut state.mv_clamp_min_draft).hint_text("Auto").desired_width(72.0));
        record_input(ui, &min_resp, gctx.meta, "format_graph.mv.color_clamp_min", "Clamp Min", &state.mv_clamp_min_draft, true);
        if min_resp.changed() || min_resp.lost_focus() {
            clamp_edited = true;
        }
        ui.label("Clamp Max:");
        let max_resp = ui.add(egui::TextEdit::singleline(&mut state.mv_clamp_max_draft).hint_text("Auto").desired_width(72.0));
        record_input(ui, &max_resp, gctx.meta, "format_graph.mv.color_clamp_max", "Clamp Max", &state.mv_clamp_max_draft, true);
        if max_resp.changed() || max_resp.lost_focus() {
            clamp_edited = true;
        }
    });
    if clamp_edited && apply_mv_color_clamp_drafts(state, &mut enc, &gctx.app.locale_preset) {
        push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Out-of-Range Policy:");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.oor_clamp",
                label: "Clamp to Bounds",
                command_id: None,
                selected: state.mv_out_of_range == MvOutOfRangePolicy::ClampToBounds,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
            if apply_mv_color_clamp_drafts(state, &mut enc, &gctx.app.locale_preset) {
                push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
            }
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.oor_omit",
                label: "Omit Outliers",
                command_id: None,
                selected: state.mv_out_of_range == MvOutOfRangePolicy::Omit,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.mv_out_of_range = MvOutOfRangePolicy::Omit;
            if let ColorChannelMapping::Continuous { column, ramp, domain_clamp: Some(_) } = &enc.color {
                enc.color = ColorChannelMapping::Continuous { column: *column, ramp: ramp.clone(), domain_clamp: None };
                state.mv_clamp_min_draft.clear();
                state.mv_clamp_max_draft.clear();
                push_mv_encoding(gctx.intents, fg, enc);
            }
        }
    });
}

pub(super) fn render_fg_legends(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    render_fg_settings(ui, gctx);
    ui.add_space(4.0);
    let _ = fg;
}

fn collect_mv_variable_names(app: &UiApp, gid: Option<&SheetId>) -> Vec<String> {
    let Some(id) = gid else {
        return Vec::new();
    };
    let Some(gs) = app.document().graph_sheet(id) else {
        return Vec::new();
    };
    let Some(ds) = app.document().data_sheet(&gs.spec.primary_source_sheet_id) else {
        return Vec::new();
    };
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.title.clone()).collect(),
        _ => Vec::new(),
    }
}

fn var_name(names: &[String], idx: usize) -> &str {
    names.get(idx).map_or("Var", String::as_str)
}

fn apply_mv_color_clamp_drafts(state: &mut GraphModalEguiState, enc: &mut MultivariateGraphEncoding, preset: &crate::i18n::LocalePreset) -> bool {
    let min_raw = state.mv_clamp_min_draft.trim();
    let max_raw = state.mv_clamp_max_draft.trim();
    if min_raw.is_empty() && max_raw.is_empty() {
        if let ColorChannelMapping::Continuous { column, ramp, domain_clamp: Some(_) } = &enc.color {
            enc.color = ColorChannelMapping::Continuous { column: *column, ramp: ramp.clone(), domain_clamp: None };
            return true;
        }
        return false;
    }
    let (Some(lo), Some(hi)) = (parse_localized_number_input(min_raw, preset), parse_localized_number_input(max_raw, preset)) else {
        return false;
    };
    if !lo.is_finite() || !hi.is_finite() || lo > hi {
        return false;
    }
    state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
    let (col, ramp) = match &enc.color {
        ColorChannelMapping::Continuous { column, ramp, .. } => (*column, ramp.clone()),
        ColorChannelMapping::Categorical { column, .. } => (*column, ColorRamp::default()),
        ColorChannelMapping::Constant(_) => (enc.y_column, ColorRamp::default()),
    };
    let next_color = ColorChannelMapping::Continuous { column: col, ramp, domain_clamp: Some((lo, hi)) };
    if enc.color == next_color {
        return false;
    }
    enc.color = next_color;
    true
}

fn push_mv_encoding(intents: &mut PendingUiIntents, mut fg: crate::dialogs_canvas::FormatGraphState, enc: MultivariateGraphEncoding) {
    fg.mv_encoding = Some(enc);
    fg.mv_levels_dirty = true;
    intents.push_update_format_graph_state(fg);
}
