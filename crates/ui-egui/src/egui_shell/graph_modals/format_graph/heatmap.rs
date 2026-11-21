//! Heatmap `Format Graph` schema tabs: Color Mapping, Labels & Ticks, and Gaps & Borders.

use curvebook_doc::{HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapRowLabelMode};
use curvebook_palette::{ColormapId, simulate_cvd};
use egui::{CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::grid_view::parse_localized_number_input;
use crate::i18n::tr;

use super::super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::super::helpers::{
    ButtonEntry, GraphModalCtx, format_localized_draft_f64, record_button, record_checkbox, record_input, record_label, rgba_to_color32,
    validate_heatmap_range,
};
use super::super::state::GraphModalEguiState;
use super::render_legend_toggles;

pub(super) fn render_fg_heatmap_color_mapping(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = &gctx.app.canvas_dialogs.format_graph;
    ui.label(egui::RichText::new("Sequential & Diverging Colormaps").strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for &cmap in &ColormapId::ALL {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.heatmap.cmap.{cmap:?}"),
                    label: cmap.label(),
                    command_id: None,
                    selected: state.heatmap_colormap == cmap,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.heatmap_colormap = cmap;
            }
        }
    });

    ui.add_space(6.0);
    // Live gradient ramp strip (with optional CVD simulation).
    let (ramp_rect, _) = ui.allocate_exact_size(Vec2::new(gctx.body_w.min(360.0), 18.0), Sense::hover());
    let samples = state.heatmap_colormap.sample_discrete(24);
    let n = samples.len().max(1);
    let sw = ramp_rect.width() / (n as f32);
    for (i, base_c) in samples.iter().enumerate() {
        let idx = if state.heatmap_reversed { n - 1 - i } else { i };
        let c = samples.get(idx).copied().unwrap_or(*base_c);
        let preview_c = match fg.cvd_preview {
            Some(kind) => simulate_cvd(c, kind, 1.0),
            None => c,
        };
        let cell = Rect::from_min_size(Pos2::new(ramp_rect.left() + (i as f32) * sw, ramp_rect.top()), Vec2::new(sw + 0.5, ramp_rect.height()));
        ui.painter().rect_filled(cell, CornerRadius::ZERO, rgba_to_color32(preview_c));
    }
    ui.painter().rect_stroke(ramp_rect, CornerRadius::same(2), Stroke::new(1.0, gctx.tokens.border), StrokeKind::Inside);

    ui.add_space(6.0);
    let rev_chk = ui.checkbox(&mut state.heatmap_reversed, "Reverse Colormap");
    record_checkbox(ui, &rev_chk, gctx.meta, "format_graph.heatmap.reversed", "Reverse Colormap", state.heatmap_reversed);
    let auto_chk = ui.checkbox(&mut state.heatmap_auto_range, "Automatic Domain Bounds");
    record_checkbox(ui, &auto_chk, gctx.meta, "format_graph.heatmap.auto_range", "Automatic Domain Bounds", state.heatmap_auto_range);
    if auto_chk.changed() {
        if state.heatmap_auto_range {
            state.heatmap_range_error = None;
        } else {
            if state.heatmap_min_draft == "-1.0" && parse_localized_number_input(&state.heatmap_min_draft, &gctx.app.locale_preset).is_none() {
                state.heatmap_min_draft = format_localized_draft_f64(-1.0, &gctx.app.locale_preset);
            }
            if state.heatmap_max_draft == "1.0" && parse_localized_number_input(&state.heatmap_max_draft, &gctx.app.locale_preset).is_none() {
                state.heatmap_max_draft = format_localized_draft_f64(1.0, &gctx.app.locale_preset);
            }
        }
    }

    if !state.heatmap_auto_range {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Min:");
            let min_resp = ui.add(egui::TextEdit::singleline(&mut state.heatmap_min_draft).desired_width(72.0));
            record_input(ui, &min_resp, gctx.meta, "format_graph.heatmap.min", "Min", &state.heatmap_min_draft, true);
            ui.label("Max:");
            let max_resp = ui.add(egui::TextEdit::singleline(&mut state.heatmap_max_draft).desired_width(72.0));
            record_input(ui, &max_resp, gctx.meta, "format_graph.heatmap.max", "Max", &state.heatmap_max_draft, true);
        });
        match validate_heatmap_range(state, &gctx.app.locale_preset) {
            Ok(_) => state.heatmap_range_error = None,
            Err(err) => {
                state.heatmap_range_error = Some(err.clone());
                let err_resp = ui.add(egui::Label::new(egui::RichText::new(&err).color(gctx.tokens.link_red)).wrap());
                record_label(ui, &err_resp, gctx.meta, "format_graph.heatmap.range_error", &err);
            }
        }
    }
}

pub(super) fn render_fg_heatmap_labels(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let mut annotate = fg.heatmap_labels.annotate_cells;
    let ann_chk = ui.checkbox(&mut annotate, "Show Numeric Cell Annotations");
    record_checkbox(ui, &ann_chk, gctx.meta, "format_graph.heatmap.annotate_cells", "Show Numeric Cell Annotations", annotate);
    if ann_chk.changed() {
        state.heatmap_annotate_cells = annotate;
        let mut next = fg.clone();
        next.heatmap_labels.annotate_cells = annotate;
        next.heatmap_labels_dirty = true;
        gctx.intents.push_update_format_graph_state(next);
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.heatmap.row_labels"));
        for (slug, lbl_key, mode) in [
            ("titles", "r13.heatmap.row_titles", HeatmapRowLabelMode::RowTitles),
            ("numbers", "r13.heatmap.row_numbers", HeatmapRowLabelMode::RowNumbers),
            ("none", "r13.common.none", HeatmapRowLabelMode::None),
        ] {
            let lbl = tr(gctx.locale, lbl_key);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.heatmap.row_labels.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.heatmap_labels.row_labels == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.heatmap_labels.row_labels = mode;
                next.heatmap_labels_dirty = true;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "r13.heatmap.col_labels"));
        for (slug, lbl_key, mode) in [
            ("titles", "r13.heatmap.col_titles", HeatmapColumnLabelMode::ColumnTitles),
            ("letters", "r13.heatmap.col_letters", HeatmapColumnLabelMode::ColumnLetters),
            ("none", "r13.common.none", HeatmapColumnLabelMode::None),
        ] {
            let lbl = tr(gctx.locale, lbl_key);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.heatmap.col_labels.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.heatmap_labels.column_labels == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.heatmap_labels.column_labels = mode;
                next.heatmap_labels_dirty = true;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Tick Angle:");
        for (lbl, deg, angle_variant) in [
            ("0 deg", 0.0_f64, HeatmapColumnLabelAngle::Horizontal0),
            ("45 deg", 45.0_f64, HeatmapColumnLabelAngle::Angled45),
            ("90 deg", 90.0_f64, HeatmapColumnLabelAngle::Vertical90),
        ] {
            let selected = fg.heatmap_labels.column_angle == angle_variant;
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.heatmap.angle.{}", deg as i32),
                    label: lbl,
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.heatmap_tick_angle_deg = deg;
                let mut next = fg.clone();
                next.heatmap_labels.column_angle = angle_variant;
                next.heatmap_labels_dirty = true;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}

pub(super) fn render_fg_heatmap_borders(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let gap_cfg = ScrubUnitFieldConfig {
        step_per_pt: 0.5,
        decimals: 1,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("format_graph.heatmap.cell_gap", "Tile Gap", 0.0, 12.0)
    };
    let out = render_scrub_unit_field(ui, fg.heatmap_labels.cell_gap_pt, &mut state.heatmap_gap_spinner, gap_cfg, gctx.tokens, gctx.meta);
    if let Some(v) = out.committed_value {
        let clamped = v.clamp(0.0, 12.0);
        state.heatmap_cell_gap_pt = clamped;
        let mut next = fg.clone();
        next.heatmap_labels.cell_gap_pt = clamped;
        next.heatmap_labels_dirty = true;
        gctx.intents.push_update_format_graph_state(next);
    }
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}
