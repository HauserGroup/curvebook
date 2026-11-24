//! Series-based `Format Graph` schema tabs: Appearance, Series & Arrangement, Graph Settings,
//! and Value Annotations.

use crate::egui_shell::UiIntent;
use curvebook_doc::{
    AnalysisKind, EquationDisplayMode, ErrorBarDirection, ErrorBarMetric, GraphKind, GraphicalDataObject, GroupedArrangement, RegressionOverlaySpec,
};
use curvebook_palette::{CvdKind, LineDashStyle, MarkerFillMode, PaletteId, Rgba8};
use egui::{CornerRadius, Sense, Ui, Vec2};

use crate::i18n::tr;

use super::super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::super::helpers::{
    ButtonEntry, GraphModalCtx, LengthSpinnerSpec, record_button, record_checkbox, render_length_spinner_pt, rgba_to_color32,
};
use super::super::state::GraphModalEguiState;
use super::render_legend_toggles;

pub(super) fn render_fg_series_appearance(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let series_count = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).map_or(1, |gs| gs.spec.series.len().max(1));

    // Dataset Scope Header Bar + Apply Globally.
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(egui::RichText::new(tr(gctx.locale, "fg.scope.series")).strong());
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.series_prev",
                label: "<",
                command_id: None,
                selected: false,
                enabled: fg.selected_series_index > 0,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && fg.selected_series_index > 0
        {
            let mut next = fg.clone();
            next.selected_series_index -= 1;
            if let Some(gs) = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)) {
                next.sync_selected_series_from_spec(&gs.spec);
            }
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
        ui.label(format!("{} / {}", fg.selected_series_index + 1, series_count));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.series_next",
                label: ">",
                command_id: None,
                selected: false,
                enabled: fg.selected_series_index + 1 < series_count,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && fg.selected_series_index + 1 < series_count
        {
            let mut next = fg.clone();
            next.selected_series_index += 1;
            if let Some(gs) = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)) {
                next.sync_selected_series_from_spec(&gs.spec);
            }
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }

        let mut apply_all = fg.apply_all_data_sets;
        let all_lbl = tr(gctx.locale, "fg.apply_globally");
        let chk = ui.checkbox(&mut apply_all, all_lbl);
        record_checkbox(ui, &chk, gctx.meta, "format_graph.apply_globally", all_lbl, apply_all);
        if chk.changed() {
            state.format_graph_apply_globally = apply_all;
            let mut next = fg.clone();
            next.apply_all_data_sets = apply_all;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
    });

    ui.add_space(6.0);
    // Symbol Size via `unit_spinners` (switchable pt/mm/in, single-commit).
    if let Some(committed) = render_length_spinner_pt(
        ui,
        LengthSpinnerSpec {
            widget_id: "format_graph.marker_size_pt",
            label: tr(gctx.locale, "fg.symbol_size"),
            value_pt: fg.marker_size_pt,
            min_pt: 1.0,
            max_pt: 24.0,
            step_pt: 0.5,
            enabled: true,
            disabled_reason: None,
        },
        &mut state.symbol_size_spinner,
        gctx.tokens,
        gctx.meta,
    ) {
        let mut next = fg.clone();
        next.marker_size_pt = committed;
        gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "fg.symbol_fill"));
        for (slug, lbl, mode) in [
            ("filled", "Filled", MarkerFillMode::Filled),
            ("open", "Open", MarkerFillMode::Open),
            ("semi_top", "Semi-Top", MarkerFillMode::SemiFilledTop),
            ("semi_bottom", "Semi-Bottom", MarkerFillMode::SemiFilledBottom),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.fill.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.marker_fill_mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.marker_fill_mode = mode;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "fg.line_dash"));
        for (slug, lbl, dash) in [
            ("solid", "Solid", LineDashStyle::Solid),
            ("dashed", "Dashed", LineDashStyle::Dashed),
            ("dotted", "Dotted", LineDashStyle::Dotted),
            ("long_dash", "Long Dash", LineDashStyle::LongDash),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.line_dash.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.line_dash == dash,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.line_dash = dash;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(tr(gctx.locale, "fg.opacity"));
        for pct in [25u8, 50, 75, 100] {
            let lbl = format!("{pct}%");
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.alpha.{pct}"),
                    label: &lbl,
                    command_id: None,
                    selected: fg.series_alpha_pct == pct,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.series_alpha_pct = pct;
                next.alpha_dirty = true;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let mut raw_on = fg.show_raw_data;
        let raw_lbl = tr(gctx.locale, "r13.fg.show_data_points");
        let raw_chk = ui.checkbox(&mut raw_on, raw_lbl);
        record_checkbox(ui, &raw_chk, gctx.meta, "format_graph.show_raw_data", raw_lbl, raw_on);
        if raw_chk.changed() {
            let mut next = fg.clone();
            next.show_raw_data = raw_on;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }

        let mut sym_on = fg.show_symbols;
        let sym_lbl = "Show symbols";
        let sym_chk = ui.checkbox(&mut sym_on, sym_lbl);
        record_checkbox(ui, &sym_chk, gctx.meta, "format_graph.show_symbols", sym_lbl, sym_on);
        if sym_chk.changed() {
            let mut next = fg.clone();
            next.show_symbols = sym_on;
            next.show_symbols_dirty = true;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }

        let mut fit_on = fg.show_fitted_curve;
        let fit_lbl = tr(gctx.locale, "r13.fg.show_fitted_curve");
        let fit_enabled = fg.cached_regression_overlay.is_some();
        let fit_chk = ui.add_enabled(fit_enabled, egui::Checkbox::new(&mut fit_on, fit_lbl));
        record_checkbox(ui, &fit_chk, gctx.meta, "format_graph.show_fitted_curve", fit_lbl, fit_on);
        if fit_chk.changed() && fit_enabled {
            let mut next = fg.clone();
            next.show_fitted_curve = fit_on;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
    });

    let active_gs = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id));
    let active_series = active_gs.and_then(|gs| gs.spec.series.get(fg.selected_series_index));
    let effective_kind = fg.graph_kind.or_else(|| active_gs.map(|gs| gs.spec.kind));
    if effective_kind == Some(GraphKind::Xy) {
        let compatible_fits: Vec<&curvebook_doc::ResultSheet> = gctx
            .app
            .document()
            .result_sheets
            .iter()
            .filter(|rs| {
                matches!(
                    rs.analysis.kind,
                    AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) | AnalysisKind::FitSplineLowess(_)
                )
            })
            .collect();
        if !compatible_fits.is_empty() || fg.cached_regression_overlay.is_some() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                ui.label(tr(gctx.locale, "fg.regression_overlay"));
                for (idx, rs) in compatible_fits.iter().enumerate() {
                    let selected = fg.cached_regression_overlay.as_ref().is_some_and(|o| o.result_sheet_id == rs.id);
                    if record_button(
                        ui,
                        gctx.meta,
                        ButtonEntry {
                            widget_id: &format!("format_graph.overlay.attach.{idx}"),
                            label: rs.title(),
                            command_id: None,
                            selected,
                            enabled: true,
                            disabled_reason: None,
                        },
                        gctx.tokens,
                    )
                    .clicked()
                    {
                        let mut next = fg.clone();
                        next.cached_regression_overlay = Some(RegressionOverlaySpec {
                            result_sheet_id: rs.id.clone(),
                            sample_segments: 256,
                            show_confidence_band: true,
                            show_prediction_band: false,
                            shaded_ribbon: false,
                        });
                        next.pending_overlay_result_id = Some(Some(rs.id.clone()));
                        next.show_fitted_curve = true;
                        gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                    }
                }
                if fg.cached_regression_overlay.is_some()
                    && record_button(
                        ui,
                        gctx.meta,
                        ButtonEntry {
                            widget_id: "format_graph.overlay.clear",
                            label: tr(gctx.locale, "fg.clear_overlay"),
                            command_id: None,
                            selected: false,
                            enabled: true,
                            disabled_reason: None,
                        },
                        gctx.tokens,
                    )
                    .clicked()
                {
                    let mut next = fg.clone();
                    next.cached_regression_overlay = None;
                    next.pending_overlay_result_id = Some(None);
                    next.show_fitted_curve = false;
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            });
        }
    }

    let raw_is_scatter = active_series.is_some_and(|s| matches!(s.gdo, GraphicalDataObject::ScatterAndLine { .. }));
    let is_scatter_series = if fg.series_gdo_dirty { !fg.series_gdo_bar } else { raw_is_scatter };
    if matches!(effective_kind, Some(GraphKind::Xy | GraphKind::Column | GraphKind::Grouped)) {
        let scatter_selected = if fg.series_gdo_dirty { !fg.series_gdo_bar } else { raw_is_scatter };
        let bar_selected = fg.series_gdo_bar;
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Series Display:");
            for (slug, lbl, target_bar, selected) in
                [("scatter_line", "Scatter / Line", false, scatter_selected), ("bar_replicates", "Bar with Replicates", true, bar_selected)]
            {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.series_gdo.{slug}"),
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
                    let mut next = fg.clone();
                    next.series_gdo_bar = target_bar;
                    next.series_gdo_dirty = true;
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            }
        });
    }

    if is_scatter_series {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Connecting Line Mode:");
            for (slug, lbl, mode) in [
                ("summary_center", "Summary center", curvebook_doc::ReplicateLineMode::SummaryCenter),
                ("individual_trajectories", "Individual trajectories", curvebook_doc::ReplicateLineMode::IndividualReplicateTrajectories),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.replicate_lines.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.replicate_lines == mode,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.replicate_lines = mode;
                    next.replicate_lines_dirty = true;
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            }
        });
    }

    let has_bar_border = if fg.series_gdo_dirty {
        fg.series_gdo_bar
    } else if fg.apply_all_data_sets {
        active_gs.is_some_and(|gs| {
            gs.spec.series.iter().any(|s| matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. }))
        })
    } else {
        active_series.is_some_and(|s| matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. }))
    };
    if has_bar_border {
        ui.add_space(4.0);
        let spin_id = egui::Id::unique((
            "format_graph.bar_border_width_spinner",
            fg.graph_sheet_id.as_ref().map(curvebook_doc::SheetId::as_str),
            fg.selected_series_index,
        ));
        let mut bar_spin = ui.ctx().data_mut(|d| {
            d.get_temp::<crate::egui_shell::unit_spinners::ScrubUnitFieldState>(spin_id).unwrap_or_else(|| {
                crate::egui_shell::unit_spinners::ScrubUnitFieldState::new(crate::egui_shell::unit_spinners::ScrubUnitKind::Points)
            })
        });
        let committed = super::super::helpers::render_length_spinner_pt(
            ui,
            super::super::helpers::LengthSpinnerSpec {
                widget_id: "format_graph.bar_border_width_pt",
                label: tr(gctx.locale, "fg.bar_border"),
                value_pt: fg.bar_border_width_pt,
                min_pt: 0.0,
                max_pt: 6.0,
                step_pt: 0.25,
                enabled: true,
                disabled_reason: None,
            },
            &mut bar_spin,
            gctx.tokens,
            gctx.meta,
        );
        ui.ctx().data_mut(|d| d.insert_temp(spin_id, bar_spin));
        if let Some(w) = committed {
            let mut next = fg.clone();
            next.bar_border_width_pt = w;
            next.bar_border_width_dirty = true;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
    }

    if fg.graph_kind == Some(curvebook_doc::GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::ForestPlot)) {
        ui.add_space(4.0);
        let mut pooled_on = fg.forest_show_pooled_diamond;
        let pooled_lbl = "Show pooled summary diamond";
        let pooled_chk = ui.checkbox(&mut pooled_on, pooled_lbl);
        record_checkbox(ui, &pooled_chk, gctx.meta, "format_graph.forest_show_pooled_diamond", pooled_lbl, pooled_on);
        if pooled_chk.changed() {
            let mut next = fg.clone();
            next.forest_show_pooled_diamond = pooled_on;
            next.forest_show_pooled_diamond_dirty = true;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);

    // Error bars section.
    let mut eb_enabled = fg.error_bar_enabled;
    let eb_lbl = tr(gctx.locale, "inspector.error_bars");
    let eb_chk = ui.checkbox(&mut eb_enabled, eb_lbl);
    record_checkbox(ui, &eb_chk, gctx.meta, "format_graph.error_bar_enabled", eb_lbl, eb_enabled);
    if eb_chk.changed() {
        let mut next = fg.clone();
        next.error_bar_enabled = eb_enabled;
        gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
    }

    if eb_enabled {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, metric) in [
                ("sd", "SD", ErrorBarMetric::Sd),
                ("sem", "SEM", ErrorBarMetric::Sem),
                ("ci95", "95% CI", ErrorBarMetric::ConfidenceInterval95),
                ("iqr", "IQR", ErrorBarMetric::InterquartileRange),
                ("range", "Range", ErrorBarMetric::Range),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.error_bar_metric.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.error_bar_spec.metric == metric,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.error_bar_spec.metric = metric;
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, dir) in [
                ("both", "Both", ErrorBarDirection::Both),
                ("upper", "Upper", ErrorBarDirection::UpperOnly),
                ("lower", "Lower", ErrorBarDirection::LowerOnly),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.error_bar_dir.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.error_bar_spec.direction == dir,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.error_bar_spec.direction = dir;
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            }
        });

        let stroke_cfg = ScrubUnitFieldConfig {
            step_per_pt: 0.25,
            decimals: 2,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("format_graph.error_bar_stroke", tr(gctx.locale, "fg.error_stroke"), 0.25, 6.0)
        };
        let stroke_out =
            render_scrub_unit_field(ui, fg.error_bar_spec.stroke_width_pt, &mut state.line_thickness_spinner, stroke_cfg, gctx.tokens, gctx.meta);
        if let Some(committed) = stroke_out.committed_value {
            let mut next = fg.clone();
            next.error_bar_spec.stroke_width_pt = committed.clamp(0.25, 6.0);
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
    }

    if fg.initial_violin_options.is_some() {
        ui.add_space(4.0);
        let mut half = fg.half_violin_with_points;
        let half_lbl = "Half-Violin + Points (Raincloud)";
        let chk = ui.checkbox(&mut half, half_lbl);
        record_checkbox(ui, &chk, gctx.meta, "format_graph.half_violin_with_points", half_lbl, half);
        if chk.changed() {
            let mut next = fg;
            next.half_violin_with_points = half;
            gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
        }
    }
}

pub(super) fn render_fg_series_arrangement(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    ui.label(egui::RichText::new(tr(gctx.locale, "fg.grouped_arrangement")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, arr) in [
            ("interleaved", "Interleaved", GroupedArrangement::Interleaved),
            ("separated", "Separated", GroupedArrangement::Separated),
            ("stacked", "Stacked", GroupedArrangement::Stacked),
            ("superimposed", "Superimposed", GroupedArrangement::Superimposed),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.arrangement.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.grouped_arrangement == arr,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.grouped_arrangement = arr;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    if matches!(fg.graph_kind, Some(curvebook_doc::GraphKind::Column | curvebook_doc::GraphKind::Grouped)) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Plot Orientation").strong());
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, target) in [
                ("vertical", "Vertical", curvebook_doc::CartesianOrientation::Vertical),
                ("horizontal", "Horizontal", curvebook_doc::CartesianOrientation::Horizontal),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.orientation.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.orientation == target,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.orientation = target;
                    next.orientation_dirty = true;
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            }
        });
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "fg.active_series_list")).strong());
    if let Some(gs) = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)) {
        for (idx, series) in gs.spec.series.iter().enumerate() {
            let swatch = fg.preview_swatch(idx);
            let selected = idx == fg.selected_series_index;
            let bind_y2 = if selected && fg.bind_to_right_y2_dirty { fg.bind_to_right_y2 } else { series.bind_to_right_y2 };
            let axis_lbl = if bind_y2 { "Right Y2" } else { "Left Y" };
            let title = series.legend_label.clone().unwrap_or_else(|| format!("Series {}", idx + 1));
            let row_lbl = format!("#{}: {} ({axis_lbl})", idx + 1, title);
            ui.horizontal(|ui| {
                let (s_rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                ui.painter().rect_filled(s_rect, CornerRadius::same(gctx.tokens.metrics.radius_sm), rgba_to_color32(swatch));
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.series_select.{idx}"),
                        label: &row_lbl,
                        command_id: None,
                        selected,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.selected_series_index = idx;
                    next.sync_selected_series_from_spec(&gs.spec);
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            });
        }

        if !gs.spec.series.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                ui.label(tr(gctx.locale, "fg.series_y_axis"));
                for (slug, lbl, y2_target) in [("left_y", "Left Y", false), ("right_y2", "Right Y2", true)] {
                    if record_button(
                        ui,
                        gctx.meta,
                        ButtonEntry {
                            widget_id: &format!("format_graph.series_axis.{slug}"),
                            label: lbl,
                            command_id: None,
                            selected: fg.bind_to_right_y2 == y2_target,
                            enabled: true,
                            disabled_reason: None,
                        },
                        gctx.tokens,
                    )
                    .clicked()
                    {
                        let mut next = fg.clone();
                        next.bind_to_right_y2 = y2_target;
                        next.bind_to_right_y2_dirty = true;
                        gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                    }
                }
            });

            ui.add_space(4.0);
            let off_spin_id = egui::Id::unique((
                "format_graph.series_offset_x_spinner",
                fg.graph_sheet_id.as_ref().map(curvebook_doc::SheetId::as_str),
                fg.selected_series_index,
            ));
            let mut off_spin = ui.ctx().data_mut(|d| {
                d.get_temp::<crate::egui_shell::unit_spinners::ScrubUnitFieldState>(off_spin_id).unwrap_or_else(|| {
                    crate::egui_shell::unit_spinners::ScrubUnitFieldState::new(crate::egui_shell::unit_spinners::ScrubUnitKind::Generic)
                })
            });
            let off_cfg = crate::egui_shell::unit_spinners::ScrubUnitFieldConfig {
                step_per_pt: 0.05,
                decimals: 2,
                allow_unit_switch: false,
                ..crate::egui_shell::unit_spinners::ScrubUnitFieldConfig::new("format_graph.series_offset_x", "Horizontal Nudge (X)", -1000.0, 1000.0)
            };
            let off_out =
                crate::egui_shell::unit_spinners::render_scrub_unit_field(ui, fg.offset_x_data, &mut off_spin, off_cfg, gctx.tokens, gctx.meta);
            ui.ctx().data_mut(|d| d.insert_temp(off_spin_id, off_spin));
            if let Some(x_val) = off_out.committed_value {
                let mut next = fg.clone();
                next.offset_x_data = x_val;
                next.offset_x_data_dirty = true;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    }
}

pub(super) fn render_fg_settings(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    ui.label(egui::RichText::new(tr(gctx.locale, "fg.palette")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for &pal in &PaletteId::ALL {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.palette.{pal:?}"),
                    label: pal.label(),
                    command_id: None,
                    selected: fg.palette == pal,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.palette = pal;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for idx in 0..fg.palette.swatches().len().min(8) {
            let c = fg.preview_swatch(idx);
            let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(2), rgba_to_color32(c));
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("CVD Accessibility Preview").strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, cvd) in [
            ("none", "Normal Vision", None),
            ("protan", "Protanopia", Some(CvdKind::Protanopia)),
            ("deutan", "Deuteranopia", Some(CvdKind::Deuteranopia)),
            ("tritan", "Tritanopia", Some(CvdKind::Tritanopia)),
            ("achroma", "Grayscale", Some(CvdKind::Achromatopsia)),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.cvd.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.cvd_preview == cvd,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.cvd_preview = cvd;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "r13.fg.paper_fill")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl_key, fill_opt) in [
            ("white", "r13.fg.paper_white", Some(Rgba8::WHITE)),
            ("cream", "r13.fg.paper_cream", Some(Rgba8::rgb(255, 251, 235))),
            ("slate", "r13.fg.paper_slate", Some(Rgba8::rgb(241, 245, 249))),
            ("transparent", "r13.fg.paper_transparent", None),
        ] {
            let lbl = tr(gctx.locale, lbl_key);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.paper_fill.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.paper_fill == fill_opt,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.paper_fill = fill_opt;
                next.paper_fill_dirty = true;
                gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
            }
        }
    });

    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}

pub(super) fn render_fg_annotations(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    ui.label(egui::RichText::new(tr(gctx.locale, "fg.annotations.heading")).strong());
    ui.add_space(4.0);
    let has_eq = fg.model_equation_mode.is_some();
    if has_eq {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label(tr(gctx.locale, "fg.equation_mode"));
            for (slug, lbl, mode) in
                [("substituted", "Substituted", EquationDisplayMode::Substituted), ("symbolic", "Symbolic", EquationDisplayMode::Symbolic)]
            {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.equation.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.model_equation_mode == Some(mode),
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.set_model_equation_mode(mode);
                    gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(next)));
                }
            }
        });
    } else {
        ui.label(tr(gctx.locale, "fg.annotations.empty"));
    }
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}
