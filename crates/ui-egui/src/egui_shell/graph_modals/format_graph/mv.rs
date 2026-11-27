//! Multivariate (MV) channel-layer `Format Graph` schema tabs: Axis Variables, Data Objects,
//! and Legends.

use curvebook_doc::{
    ColorChannelMapping, GraphicalDataObject, MultivariateGraphEncoding, MultivariatePlotMode, ProjectDocument, SheetId, SizeChannelMapping,
    TableArchetype, VariableRole, WhiskerMethod,
};
use curvebook_palette::ColorRamp;
use egui::{CornerRadius, Stroke, StrokeKind, Ui};

use crate::UiApp;
use crate::dialogs_canvas::FormatGraphState;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::tr;

use super::super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_input, record_label};
use super::super::state::{GraphModalEguiState, MvOutOfRangePolicy};
use super::series::render_fg_settings;

/// Safely clamp MV variable bubble `(min_radius_pt, max_radius_pt)` so `min_radius_pt > 36.0` or
/// non-finite values never trigger an `f64::clamp` panic (`AUD-10-01`).
#[must_use]
pub(crate) fn clamp_mv_max_radius_pt(min_radius_pt: f64, max_val: f64) -> (f64, f64) {
    let lo = if min_radius_pt.is_finite() { min_radius_pt.max(1.0) } else { 2.5 };
    let hi_cap = lo.max(36.0);
    let hi = if max_val.is_finite() { max_val.clamp(lo, hi_cap) } else { hi_cap };
    (lo, hi)
}

/// Recompute `mv_levels`, `mv_level_is_x`, and `mv_selected_level` from `doc` whenever the active
/// X column, plot mode, or Color channel mapping changes (`AUD-10-39`).
pub(crate) fn refresh_mv_levels_from_doc(fg: &mut FormatGraphState, doc: &ProjectDocument) {
    let Some(enc) = fg.mv_encoding.as_ref() else {
        fg.mv_levels.clear();
        fg.mv_selected_level = None;
        return;
    };
    let Some(gs) = fg.graph_sheet_id.as_ref().and_then(|id| doc.graph_sheet(id)) else {
        return;
    };
    let Some(ds) = doc
        .data_sheet(&gs.spec.primary_source_sheet_id)
        .or_else(|| doc.result_sheet(&gs.spec.primary_source_sheet_id).and_then(|r| r.analyzable_data.as_ref()))
    else {
        fg.mv_levels.clear();
        fg.mv_selected_level = None;
        return;
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        fg.mv_levels.clear();
        fg.mv_selected_level = None;
        return;
    };

    let x_cat = mv
        .variables
        .get(enc.x_column)
        .filter(|col| col.role == VariableRole::Categorical || matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
    if let Some(col) = x_cat {
        let lvls: Vec<String> =
            col.ordered_levels_with(enc.x_level_order_override.as_ref().unwrap_or(&col.level_order)).into_iter().map(|l| l.level).collect();
        if !lvls.is_empty() {
            fg.mv_levels = lvls;
            fg.mv_level_is_x = true;
            fg.mv_selected_level = resolve_level_selection(&fg.mv_levels, fg.mv_selected_level.as_deref());
            return;
        }
    }

    if let ColorChannelMapping::Categorical { column, level_order_override, .. } = &enc.color
        && let Some(col) = mv.variables.get(*column)
    {
        let lvls: Vec<String> =
            col.ordered_levels_with(level_order_override.as_ref().unwrap_or(&col.level_order)).into_iter().map(|l| l.level).collect();
        if !lvls.is_empty() {
            fg.mv_levels = lvls;
            fg.mv_level_is_x = false;
            fg.mv_selected_level = resolve_level_selection(&fg.mv_levels, fg.mv_selected_level.as_deref());
            return;
        }
    }

    fg.mv_levels.clear();
    fg.mv_selected_level = None;
}

fn resolve_level_selection(levels: &[String], current: Option<&str>) -> Option<String> {
    if levels.is_empty() {
        return None;
    }
    current.and_then(|cur| levels.iter().find(|l| l.as_str() == cur).cloned()).or_else(|| levels.first().cloned())
}

pub(super) fn render_fg_mv_axis_variables(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    let Some(mut enc) = draft
        .mv_encoding
        .clone()
        .or_else(|| draft.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).and_then(|gs| gs.spec.multivariate.clone()))
    else {
        ui.label("No Multivariate encoding attached to this graph.");
        return;
    };

    let mut enc_changed = false;
    let mut levels_need_refresh = false;

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
                label: tr(gctx.locale, "fg.mv.bubble_scatter"),
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
            enc_changed = true;
            levels_need_refresh = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.plot_mode.categorical_summary",
                label: tr(gctx.locale, "fg.mv.categorical_summary"),
                command_id: None,
                selected: is_cat_summary,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let swatch = draft.palette.color_at(0);
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
            enc_changed = true;
            levels_need_refresh = true;
        }
    });

    ui.add_space(6.0);
    let var_names = collect_mv_variable_names(gctx.app, draft.graph_sheet_id.as_ref());
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
            enc.x_level_order_override = None;
            enc_changed = true;
            levels_need_refresh = true;
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
            enc_changed = true;
        }
    });

    ui.add_space(6.0);
    let mut color_edit = MvColorEditState { enc: &mut enc, enc_changed: &mut enc_changed, levels_need_refresh: &mut levels_need_refresh };
    render_mv_color_channel_controls(ui, state, gctx, draft, &var_names, &mut color_edit);

    if enc_changed {
        draft.mv_encoding = Some(enc);
        draft.mv_levels_dirty = true;
        if levels_need_refresh {
            refresh_mv_levels_from_doc(draft, gctx.app.document());
        }
        *dirty = true;
    }

    if !draft.mv_levels.is_empty() {
        render_mv_level_reorder_controls(ui, gctx, draft, dirty);
    }
}

struct MvColorEditState<'a> {
    enc: &'a mut MultivariateGraphEncoding,
    enc_changed: &'a mut bool,
    levels_need_refresh: &'a mut bool,
}

fn render_mv_color_channel_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &FormatGraphState,
    var_names: &[String],
    edit: &mut MvColorEditState<'_>,
) {
    let enc = &mut *edit.enc;
    let enc_changed = &mut *edit.enc_changed;
    let levels_need_refresh = &mut *edit.levels_need_refresh;
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
            enc.color = ColorChannelMapping::Constant(draft.palette.color_at(0));
            state.mv_clamp_error = None;
            *enc_changed = true;
            *levels_need_refresh = true;
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
            enc.color = ColorChannelMapping::Categorical { column: col, palette: draft.palette, level_order_override: None };
            state.mv_clamp_error = None;
            *enc_changed = true;
            *levels_need_refresh = true;
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
            state.mv_clamp_error = None;
            *enc_changed = true;
            *levels_need_refresh = true;
        }
        let color_col_summary = match &enc.color {
            ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => {
                format!("Next Color Col (#{} {})", column + 1, var_name(var_names, *column))
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
                *enc_changed = true;
                *levels_need_refresh = true;
            }
        }
    });
}

fn render_mv_level_reorder_controls(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatGraphState, dirty: &mut bool) {
    ui.add_space(6.0);
    let sel_idx = draft.mv_selected_level.as_deref().and_then(|sel| draft.mv_levels.iter().position(|lvl| lvl == sel)).unwrap_or(0);
    let axis_tag = if draft.mv_level_is_x { "X Axis" } else { "Color" };
    let mut clicked_level: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Categorical Levels ({axis_tag}):"));
        for (idx, level) in draft.mv_levels.iter().enumerate() {
            let is_selected = idx == sel_idx;
            let text = if is_selected {
                egui::RichText::new(level).strong().color(gctx.tokens.text_strong)
            } else {
                egui::RichText::new(level).color(gctx.tokens.text_muted)
            };
            let fill = if is_selected { gctx.tokens.active_bg } else { gctx.tokens.panel_bg };
            let stroke = Stroke::new(if is_selected { 1.5 } else { 1.0 }, if is_selected { gctx.tokens.accent } else { gctx.tokens.border });
            let resp = egui::Frame::NONE
                .fill(fill)
                .stroke(stroke)
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::vec2(6.0, 2.0))
                .show(ui, |ui| ui.add(egui::Label::new(text).sense(egui::Sense::click())))
                .inner;
            if is_selected {
                ui.painter().rect_stroke(resp.rect.expand(2.0), CornerRadius::same(4), stroke, StrokeKind::Outside);
            }
            if resp.clicked() {
                clicked_level = Some(level.clone());
            }
        }
    });
    if let Some(level) = clicked_level {
        draft.mv_selected_level = Some(level);
        *dirty = true;
    }

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
                if op == "cycle" {
                    draft.cycle_selected_mv_level();
                } else {
                    let _ = draft.reorder_mv_level(op);
                }
                *dirty = true;
            }
        }
    });
}

pub(super) fn render_fg_mv_data_objects(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    let Some(mut enc) = draft
        .mv_encoding
        .clone()
        .or_else(|| draft.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).and_then(|gs| gs.spec.multivariate.clone()))
    else {
        ui.label("No Multivariate data layers attached.");
        return;
    };

    let mut enc_changed = false;

    ui.label(egui::RichText::new("Layer Overlays").strong());
    let mut ellipses = enc.show_confidence_ellipses;
    let ell_lbl = tr(gctx.locale, "fg.mv.ellipses");
    let ell_chk = ui.checkbox(&mut ellipses, ell_lbl);
    record_checkbox(ui, &ell_chk, gctx.meta, "format_graph.mv.confidence_ellipses", ell_lbl, ellipses);
    if ell_chk.changed() {
        enc.show_confidence_ellipses = ellipses;
        enc_changed = true;
    }

    let mut hulls = enc.show_convex_hulls;
    let hull_lbl = tr(gctx.locale, "fg.mv.hulls");
    let hull_chk = ui.checkbox(&mut hulls, hull_lbl);
    record_checkbox(ui, &hull_chk, gctx.meta, "format_graph.mv.convex_hulls", hull_lbl, hulls);
    if hull_chk.changed() {
        enc.show_convex_hulls = hulls;
        enc_changed = true;
    }

    ui.add_space(6.0);
    render_mv_size_controls(ui, state, gctx, draft, &mut enc, &mut enc_changed);

    ui.add_space(6.0);
    render_mv_color_clamp_controls(ui, state, gctx, &mut enc, &mut enc_changed);

    if enc_changed {
        draft.mv_encoding = Some(enc);
        draft.mv_levels_dirty = true;
        *dirty = true;
    }
}

fn render_mv_size_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &FormatGraphState,
    enc: &mut MultivariateGraphEncoding,
    enc_changed: &mut bool,
) {
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
            *enc_changed = true;
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
            *enc_changed = true;
        }
    });

    ui.add_space(4.0);
    let var_names = collect_mv_variable_names(gctx.app, draft.graph_sheet_id.as_ref());
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
                *enc_changed = true;
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
                    *enc_changed = true;
                }
            });

            let cur_min = if min_radius_pt.is_finite() { min_radius_pt.max(1.0) } else { 2.5 };
            let min_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.min_radius", "Min Radius", 1.0, cur_min.max(24.0))
            };
            let min_out = render_scrub_unit_field(ui, cur_min, &mut state.mv_min_radius_spinner, min_cfg, gctx.tokens, gctx.meta);
            let mut active_min = cur_min;
            let mut active_max = if max_radius_pt.is_finite() { max_radius_pt.max(active_min) } else { active_min.max(14.0) };
            let active_col = match enc.size {
                SizeChannelMapping::Variable { column: c, .. } => c,
                _ => column,
            };
            if let Some(min_val) = min_out.committed_value {
                let lo = if min_val.is_finite() { min_val.clamp(1.0, 24.0) } else { 2.5 };
                active_min = lo;
                active_max = active_max.max(lo);
                enc.size = SizeChannelMapping::Variable { column: active_col, min_radius_pt: active_min, max_radius_pt: active_max };
                *enc_changed = true;
            }

            let max_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.max_radius", "Max Radius", 1.0, active_min.max(36.0))
            };
            let max_out = render_scrub_unit_field(ui, active_max, &mut state.mv_max_radius_spinner, max_cfg, gctx.tokens, gctx.meta);
            if let Some(max_val) = max_out.committed_value {
                let (lo, hi) = clamp_mv_max_radius_pt(active_min, max_val);
                enc.size = SizeChannelMapping::Variable { column: active_col, min_radius_pt: lo, max_radius_pt: hi };
                *enc_changed = true;
            }
        }
    }
}

fn render_mv_color_clamp_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    enc: &mut MultivariateGraphEncoding,
    enc_changed: &mut bool,
) {
    let is_continuous_color = matches!(enc.color, ColorChannelMapping::Continuous { .. });
    let disabled_reason = (!is_continuous_color).then_some("Switch Color Channel Mapping to Continuous Ramp to clamp domain bounds");
    if !is_continuous_color {
        state.mv_clamp_error = None;
    }

    ui.label(egui::RichText::new("Color Domain Clamp").strong());
    let mut clamp_edited = false;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Clamp Min:");
        let min_resp =
            ui.add_enabled(is_continuous_color, egui::TextEdit::singleline(&mut state.mv_clamp_min_draft).hint_text("Auto").desired_width(72.0));
        record_input(ui, &min_resp, gctx.meta, "format_graph.mv.color_clamp_min", "Clamp Min", &state.mv_clamp_min_draft, is_continuous_color);
        if is_continuous_color && (min_resp.changed() || min_resp.lost_focus()) {
            clamp_edited = true;
        }
        ui.label("Clamp Max:");
        let max_resp =
            ui.add_enabled(is_continuous_color, egui::TextEdit::singleline(&mut state.mv_clamp_max_draft).hint_text("Auto").desired_width(72.0));
        record_input(ui, &max_resp, gctx.meta, "format_graph.mv.color_clamp_max", "Clamp Max", &state.mv_clamp_max_draft, is_continuous_color);
        if is_continuous_color && (max_resp.changed() || max_resp.lost_focus()) {
            clamp_edited = true;
        }
    });
    if is_continuous_color && clamp_edited && apply_mv_color_clamp_drafts(state, enc, &gctx.app.locale_preset) {
        *enc_changed = true;
    }

    if let Some(err) = &state.mv_clamp_error {
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err).color(gctx.tokens.link_red)).wrap());
        record_label(ui, &err_resp, gctx.meta, "format_graph.mv.color_clamp_error", err);
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "fg.mv.out_of_range"));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.oor_clamp",
                label: tr(gctx.locale, "fg.mv.clamp_to_bounds"),
                command_id: None,
                selected: is_continuous_color && state.mv_out_of_range == MvOutOfRangePolicy::ClampToBounds,
                enabled: is_continuous_color,
                disabled_reason,
            },
            gctx.tokens,
        )
        .clicked()
            && is_continuous_color
        {
            state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
            if apply_mv_color_clamp_drafts(state, enc, &gctx.app.locale_preset) {
                *enc_changed = true;
            }
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.oor_omit",
                label: tr(gctx.locale, "fg.mv.auto_scale_no_clamp"),
                command_id: None,
                selected: is_continuous_color && state.mv_out_of_range == MvOutOfRangePolicy::Omit,
                enabled: is_continuous_color,
                disabled_reason,
            },
            gctx.tokens,
        )
        .clicked()
            && is_continuous_color
        {
            state.mv_out_of_range = MvOutOfRangePolicy::Omit;
            state.mv_clamp_error = None;
            state.mv_clamp_min_draft.clear();
            state.mv_clamp_max_draft.clear();
            if let ColorChannelMapping::Continuous { column, ramp, domain_clamp: Some(_) } = &enc.color {
                enc.color = ColorChannelMapping::Continuous { column: *column, ramp: ramp.clone(), domain_clamp: None };
                *enc_changed = true;
            }
        }
    });
}

pub(super) fn render_fg_legends(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatGraphState, dirty: &mut bool) {
    render_fg_settings(ui, gctx, draft, dirty);
    ui.add_space(4.0);
}

fn collect_mv_variable_names(app: &UiApp, gid: Option<&SheetId>) -> Vec<String> {
    let Some(id) = gid else {
        return Vec::new();
    };
    let Some(gs) = app.document().graph_sheet(id) else {
        return Vec::new();
    };
    let Some(ds) = app
        .document()
        .data_sheet(&gs.spec.primary_source_sheet_id)
        .or_else(|| app.document().result_sheet(&gs.spec.primary_source_sheet_id).and_then(|r| r.analyzable_data.as_ref()))
    else {
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

pub(crate) fn apply_mv_color_clamp_drafts(
    state: &mut GraphModalEguiState,
    enc: &mut MultivariateGraphEncoding,
    preset: &crate::i18n::LocalePreset,
) -> bool {
    let ColorChannelMapping::Continuous { column, ramp, domain_clamp } = &enc.color else {
        state.mv_clamp_error = None;
        return false;
    };
    let col = *column;
    let ramp = ramp.clone();
    let had_clamp = domain_clamp.is_some();

    let min_raw = state.mv_clamp_min_draft.trim();
    let max_raw = state.mv_clamp_max_draft.trim();
    if min_raw.is_empty() && max_raw.is_empty() {
        state.mv_clamp_error = None;
        if had_clamp {
            enc.color = ColorChannelMapping::Continuous { column: col, ramp, domain_clamp: None };
            return true;
        }
        return false;
    }
    if min_raw.is_empty() || max_raw.is_empty() {
        state.mv_clamp_error = Some(String::from("Enter both Clamp Min and Clamp Max, or leave both empty for Auto"));
        return false;
    }
    let Some(lo) = parse_localized_number_input(min_raw, preset) else {
        state.mv_clamp_error = Some(format!("Invalid Clamp Min `{min_raw}`"));
        return false;
    };
    let Some(hi) = parse_localized_number_input(max_raw, preset) else {
        state.mv_clamp_error = Some(format!("Invalid Clamp Max `{max_raw}`"));
        return false;
    };
    if !lo.is_finite() || !hi.is_finite() || lo >= hi {
        state.mv_clamp_error = Some(format!("Clamp Min ({lo}) must be strictly less than Clamp Max ({hi})"));
        return false;
    }
    state.mv_clamp_error = None;
    state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
    let next_color = ColorChannelMapping::Continuous { column: col, ramp, domain_clamp: Some((lo, hi)) };
    if enc.color == next_color {
        return false;
    }
    enc.color = next_color;
    true
}
