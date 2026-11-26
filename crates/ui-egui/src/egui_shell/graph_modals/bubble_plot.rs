//! `Bubble Plot Channels` modal body (`gui-layout-spec.md` §8.11).

use curvebook_doc::{ColorChannelMapping, GraphicalDataObject, MultivariatePlotMode, SizeChannelMapping, VariableRole, WhiskerMethod};
use curvebook_palette::{ColormapId, PaletteId};
use egui::Ui;

use crate::dialogs_canvas::BubblePlotInspectorState;
use crate::egui_shell::UiIntent;

use super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox};
use super::state::GraphModalEguiState;

fn col_title(bp: &BubblePlotInspectorState, idx: usize) -> &str {
    bp.available_columns.get(idx).map_or("Var", |c| c.title.as_str())
}

fn is_numeric_column(bp: &BubblePlotInspectorState, idx: usize) -> bool {
    bp.available_columns.get(idx).is_some_and(|c| matches!(c.role, VariableRole::Continuous | VariableRole::Calculated))
}

/// Cycle the color column across available columns, adapting `Categorical` vs `Continuous` to the
/// target column's [`VariableRole`] and guarding empty column lists (`AUD-10-54`).
pub(super) fn cycle_bubble_color_column(draft: &mut BubblePlotInspectorState) -> bool {
    let n_cols = draft.available_columns.len();
    if n_cols == 0 {
        return false;
    }
    let next_col = match &draft.color_channel {
        ColorChannelMapping::Constant(_) => 0,
        ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => (column + 1) % n_cols,
    };
    if is_numeric_column(draft, next_col) {
        match &draft.color_channel {
            ColorChannelMapping::Continuous { ramp, domain_clamp, .. } => {
                draft.color_channel = ColorChannelMapping::Continuous { column: next_col, ramp: ramp.clone(), domain_clamp: *domain_clamp };
            }
            _ => draft.set_color_continuous(next_col, ColormapId::Viridis, None),
        }
    } else {
        let palette = match &draft.color_channel {
            ColorChannelMapping::Categorical { palette, .. } => *palette,
            _ => PaletteId::OkabeIto,
        };
        draft.set_color_categorical(next_col, palette);
    }
    true
}

pub(super) fn render_bubble_plot_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.bubble_plot.clone();
    let mut dirty = false;

    render_bubble_mode_and_axes(ui, gctx, &mut draft, &mut dirty);
    ui.add_space(4.0);
    render_bubble_color_controls(ui, gctx, &mut draft, &mut dirty);
    ui.add_space(4.0);
    render_bubble_size_controls(ui, state, gctx, &mut draft, &mut dirty);
    ui.add_space(4.0);
    render_bubble_label_and_overlays(ui, gctx, &mut draft, &mut dirty);

    if dirty {
        gctx.intents.push(UiIntent::UpdateBubblePlotState(Box::new(draft)));
    }
}

fn render_bubble_mode_and_axes(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut BubblePlotInspectorState, dirty: &mut bool) {
    let col_count = draft.available_columns.len();

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Plot Mode:");
        let is_bubble = matches!(draft.plot_mode, MultivariatePlotMode::BubbleScatter);
        let is_cat_summary = matches!(draft.plot_mode, MultivariatePlotMode::CategoricalSummary { .. });
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
            draft.plot_mode = MultivariatePlotMode::BubbleScatter;
            *dirty = true;
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
            let color = PaletteId::OkabeIto.color_at(0);
            draft.plot_mode = MultivariatePlotMode::CategoricalSummary {
                gdo: GraphicalDataObject::BoxAndWhisker {
                    whiskers: WhiskerMethod::Tukey1_5Iqr,
                    fill_color: color.with_alpha(190),
                    stroke_color: color,
                    show_outliers: true,
                    show_all_points: false,
                },
                y_error_bars: None,
            };
            *dirty = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!(
            "X Column: #{} ({}) | Y Column: #{} ({})",
            draft.x_column + 1,
            col_title(draft, draft.x_column),
            draft.y_column + 1,
            col_title(draft, draft.y_column)
        ));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.x_next",
                label: "Cycle X",
                command_id: None,
                selected: false,
                enabled: col_count > 1,
                disabled_reason: (col_count <= 1).then_some("Need at least 2 columns to cycle X"),
            },
            gctx.tokens,
        )
        .clicked()
            && col_count > 1
        {
            draft.x_column = (draft.x_column + 1) % col_count;
            *dirty = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.y_next",
                label: "Cycle Y",
                command_id: None,
                selected: false,
                enabled: col_count > 1,
                disabled_reason: (col_count <= 1).then_some("Need at least 2 columns to cycle Y"),
            },
            gctx.tokens,
        )
        .clicked()
            && col_count > 1
        {
            draft.y_column = (draft.y_column + 1) % col_count;
            *dirty = true;
        }
    });
}

fn render_bubble_color_controls(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut BubblePlotInspectorState, dirty: &mut bool) {
    let col_count = draft.available_columns.len();
    let has_cols = col_count > 0;
    let is_uniform = matches!(draft.color_channel, ColorChannelMapping::Constant(_));
    let is_cat = matches!(draft.color_channel, ColorChannelMapping::Categorical { .. });
    let is_cont = matches!(draft.color_channel, ColorChannelMapping::Continuous { .. });

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Color Mode:");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.color_mode.uniform",
                label: "Uniform",
                command_id: None,
                selected: is_uniform,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            draft.color_channel = ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0));
            *dirty = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.color_mode.categorical",
                label: "Categorical",
                command_id: None,
                selected: is_cat,
                enabled: has_cols,
                disabled_reason: (!has_cols).then_some("No columns available for categorical coloring"),
            },
            gctx.tokens,
        )
        .clicked()
            && has_cols
        {
            let col = match &draft.color_channel {
                ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => (*column).min(col_count - 1),
                ColorChannelMapping::Constant(_) => {
                    draft.available_columns.iter().position(|c| !matches!(c.role, VariableRole::Continuous | VariableRole::Calculated)).unwrap_or(0)
                }
            };
            draft.set_color_categorical(col, PaletteId::OkabeIto);
            *dirty = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.color_mode.continuous",
                label: "Continuous",
                command_id: None,
                selected: is_cont,
                enabled: has_cols,
                disabled_reason: (!has_cols).then_some("No columns available for continuous coloring"),
            },
            gctx.tokens,
        )
        .clicked()
            && has_cols
        {
            let col = match &draft.color_channel {
                ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => (*column).min(col_count - 1),
                ColorChannelMapping::Constant(_) => {
                    draft.available_columns.iter().position(|c| matches!(c.role, VariableRole::Continuous | VariableRole::Calculated)).unwrap_or(0)
                }
            };
            draft.set_color_continuous(col, ColormapId::Viridis, None);
            *dirty = true;
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let color_summary = match &draft.color_channel {
            ColorChannelMapping::Constant(_) => String::from("Color: Uniform"),
            ColorChannelMapping::Categorical { column, .. } => format!("Color: Cat #{} ({})", column + 1, col_title(draft, *column)),
            ColorChannelMapping::Continuous { column, .. } => format!("Color: Cont #{} ({})", column + 1, col_title(draft, *column)),
        };
        ui.label(color_summary);
        let can_cycle = has_cols && (col_count > 1 || matches!(draft.color_channel, ColorChannelMapping::Constant(_)));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.color_col_next",
                label: "Cycle Color Col",
                command_id: None,
                selected: false,
                enabled: can_cycle,
                disabled_reason: (!can_cycle).then_some("No additional columns available for color channel"),
            },
            gctx.tokens,
        )
        .clicked()
            && can_cycle
            && cycle_bubble_color_column(draft)
        {
            *dirty = true;
        }
    });
}

fn render_bubble_size_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut BubblePlotInspectorState,
    dirty: &mut bool,
) {
    let col_count = draft.available_columns.len();
    let is_size_var = matches!(draft.size_channel, SizeChannelMapping::Variable { .. });

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let can_toggle_size = col_count > 0 || is_size_var;
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.size_variable",
                label: "Variable Size",
                command_id: None,
                selected: is_size_var,
                enabled: can_toggle_size,
                disabled_reason: (!can_toggle_size).then_some("No columns available for variable bubble sizing"),
            },
            gctx.tokens,
        )
        .clicked()
            && can_toggle_size
        {
            if is_size_var {
                draft.size_channel = SizeChannelMapping::Constant { radius_pt: 5.0 };
            } else {
                draft.size_channel =
                    SizeChannelMapping::Variable { column: draft.x_column.min(col_count.saturating_sub(1)), min_radius_pt: 2.5, max_radius_pt: 14.0 };
            }
            *dirty = true;
        }
        if let SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt } = draft.size_channel {
            ui.label(format!("Size: #{} ({})", column + 1, col_title(draft, column)));
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "bubble.size_col_next",
                    label: "Cycle Size Col",
                    command_id: None,
                    selected: false,
                    enabled: col_count > 1,
                    disabled_reason: (col_count <= 1).then_some("Need at least 2 columns to cycle size channel"),
                },
                gctx.tokens,
            )
            .clicked()
                && col_count > 1
            {
                draft.set_size_variable((column + 1) % col_count, min_radius_pt, max_radius_pt);
                *dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    match draft.size_channel {
        SizeChannelMapping::Constant { radius_pt } => {
            let cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.size_pt", "Bubble Radius", 1.0, 24.0)
            };
            let out = render_scrub_unit_field(ui, radius_pt, &mut state.mv_min_radius_spinner, cfg, gctx.tokens, gctx.meta);
            if let Some(v) = out.committed_value {
                draft.size_channel = SizeChannelMapping::Constant { radius_pt: v.clamp(1.0, 24.0) };
                *dirty = true;
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
            let mut active_min = min_radius_pt;
            let mut active_max = max_radius_pt;
            if let Some(v) = min_out.committed_value {
                draft.set_size_variable(column, v, active_max);
                if let SizeChannelMapping::Variable { min_radius_pt: lo, max_radius_pt: hi, .. } = draft.size_channel {
                    active_min = lo;
                    active_max = hi;
                }
                *dirty = true;
            }

            let max_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.max_radius", "Max Radius", 1.0, active_min.max(36.0))
            };
            let max_out = render_scrub_unit_field(ui, active_max, &mut state.mv_max_radius_spinner, max_cfg, gctx.tokens, gctx.meta);
            if let Some(v) = max_out.committed_value {
                draft.set_size_variable(column, active_min, v);
                *dirty = true;
            }
        }
    }
}

fn render_bubble_label_and_overlays(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut BubblePlotInspectorState, dirty: &mut bool) {
    let col_count = draft.available_columns.len();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let label_summary = match draft.label_column {
            None => String::from("Label Column: None"),
            Some(col) => format!("Label Column: #{} ({})", col + 1, col_title(draft, col)),
        };
        ui.label(label_summary);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.label_col_next",
                label: "Cycle Label Col",
                command_id: None,
                selected: draft.label_column.is_some(),
                enabled: col_count > 0,
                disabled_reason: (col_count == 0).then_some("No columns available for point labels"),
            },
            gctx.tokens,
        )
        .clicked()
            && col_count > 0
        {
            draft.label_column = match draft.label_column {
                None => Some(0),
                Some(c) if c + 1 < col_count => Some(c + 1),
                Some(_) => None,
            };
            *dirty = true;
        }
        if draft.label_column.is_some()
            && record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "bubble.label_col_clear",
                    label: "Clear Label",
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
        {
            draft.label_column = None;
            *dirty = true;
        }
    });

    ui.add_space(4.0);
    let mut hulls = draft.show_convex_hulls;
    let h_chk = ui.checkbox(&mut hulls, "Show Convex Hulls");
    record_checkbox(ui, &h_chk, gctx.meta, "bubble.show_convex_hulls", "Show Convex Hulls", hulls);
    if h_chk.changed() {
        draft.show_convex_hulls = hulls;
        *dirty = true;
    }

    let mut ellipses = draft.show_confidence_ellipses;
    let e_chk = ui.checkbox(&mut ellipses, "Show Confidence Ellipses");
    record_checkbox(ui, &e_chk, gctx.meta, "bubble.show_confidence_ellipses", "Show Confidence Ellipses", ellipses);
    if e_chk.changed() {
        draft.show_confidence_ellipses = ellipses;
        *dirty = true;
    }
}
