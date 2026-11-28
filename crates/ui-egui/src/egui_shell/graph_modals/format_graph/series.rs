//! Series-based `Format Graph` schema tabs: Appearance, Series & Arrangement, Graph Settings,
//! and Value Annotations.

use curvebook_doc::{
    AnalysisKind, ColorChannelMapping, EquationDisplayMode, ErrorBarDirection, ErrorBarMetric, GraphKind, GraphicalDataObject, GroupedArrangement,
    ProjectDocument, RegressionOverlaySpec, ResultSheet,
};
use curvebook_palette::{CvdKind, LineDashStyle, MarkerFillMode, PaletteId, Rgba8};
use egui::{CornerRadius, Sense, Ui, Vec2};

use crate::dialogs_canvas::FormatGraphState;
use crate::i18n::tr;

use super::super::super::unit_spinners::{ScrubUnitFieldConfig, render_scrub_unit_field};
use super::super::helpers::{
    ButtonEntry, CheckboxMeta, GraphModalCtx, LengthSpinnerSpec, record_button, record_checkbox, record_checkbox_enabled, render_length_spinner_pt,
    rgba_to_color32,
};
use super::super::state::{GraphModalEguiState, StagedSeriesEdit};
use super::render_legend_toggles;

/// Return `true` when `rs` is a regression/curve analysis eligible as an XY fitted-curve overlay
/// (`AUD-13-16`, `AUD-13-17`).
#[must_use]
pub(crate) fn is_xy_overlay_compatible_result(rs: &ResultSheet) -> bool {
    matches!(
        rs.analysis.kind,
        AnalysisKind::LinearRegression(_)
            | AnalysisKind::NonlinearRegression(_)
            | AnalysisKind::FitSplineLowess(_)
            | AnalysisKind::SimpleLogisticRegression(_)
            | AnalysisKind::PassingBablok(_)
    )
}

/// Stage unsaved edits for the currently selected series and load `target_idx` (restoring any
/// previously staged edits for `target_idx` so switching series never drops edits — `AUD-10-40`).
pub(crate) fn switch_format_graph_series(state: &mut GraphModalEguiState, draft: &mut FormatGraphState, target_idx: usize, doc: &ProjectDocument) {
    let Some(gs) = draft.graph_sheet_id.as_ref().and_then(|id| doc.graph_sheet(id)) else {
        draft.selected_series_index = target_idx;
        return;
    };
    let series_len = gs.spec.series.len();
    if series_len == 0 {
        draft.selected_series_index = 0;
        return;
    }
    let prev_idx = draft.selected_series_index.min(series_len - 1);
    let next_idx = target_idx.min(series_len - 1);
    if prev_idx == next_idx {
        return;
    }

    let captured = StagedSeriesEdit::capture(draft);
    if captured.differs_from_spec(&gs.spec, prev_idx) {
        state.staged_series_edits.insert(prev_idx, captured);
    } else {
        state.staged_series_edits.remove(&prev_idx);
    }

    draft.selected_series_index = next_idx;
    draft.sync_selected_series_from_spec(&gs.spec);
    if let Some(staged) = state.staged_series_edits.get(&next_idx) {
        staged.apply_to(draft);
    }

    for sp in [&mut state.symbol_size_spinner, &mut state.line_thickness_spinner, &mut state.bar_border_spinner, &mut state.series_offset_x_spinner] {
        sp.editing = false;
        sp.dragging = false;
        sp.draft_text.clear();
    }
}

pub(super) fn render_fg_series_appearance(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    let series_count = draft.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).map_or(1, |gs| gs.spec.series.len().max(1));
    let mut pending_series_switch: Option<usize> = None;

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
                enabled: draft.selected_series_index > 0,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && draft.selected_series_index > 0
        {
            pending_series_switch = Some(draft.selected_series_index - 1);
        }
        ui.label(format!("{} / {}", draft.selected_series_index + 1, series_count));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.series_next",
                label: ">",
                command_id: None,
                selected: false,
                enabled: draft.selected_series_index + 1 < series_count,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && draft.selected_series_index + 1 < series_count
        {
            pending_series_switch = Some(draft.selected_series_index + 1);
        }

        let mut apply_all = draft.apply_all_data_sets;
        let all_lbl = tr(gctx.locale, "fg.apply_globally");
        let chk = ui.checkbox(&mut apply_all, all_lbl);
        record_checkbox(ui, &chk, gctx.meta, "format_graph.apply_globally", all_lbl, apply_all);
        if chk.changed() {
            draft.apply_all_data_sets = apply_all;
            *dirty = true;
        }
    });

    ui.add_space(6.0);
    render_fg_marker_line_alpha_controls(ui, state, gctx, draft, dirty);
    render_fg_visibility_and_overlay_controls(ui, gctx, draft, dirty);
    render_fg_gdo_and_bar_controls(ui, state, gctx, draft, dirty);

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
    render_fg_error_bars_section(ui, state, gctx, draft, dirty);

    if draft.initial_violin_options.is_some() {
        ui.add_space(4.0);
        let mut half = draft.half_violin_with_points;
        let half_lbl = "Half-Violin + Points (Raincloud)";
        let chk = ui.checkbox(&mut half, half_lbl);
        record_checkbox(ui, &chk, gctx.meta, "format_graph.half_violin_with_points", half_lbl, half);
        if chk.changed() {
            draft.half_violin_with_points = half;
            *dirty = true;
        }
    }

    // Apply deferred series switch AFTER all widgets on the outgoing series have processed blur-commits.
    if let Some(target_idx) = pending_series_switch {
        switch_format_graph_series(state, draft, target_idx, gctx.app.document());
        *dirty = true;
    }
}

fn render_fg_marker_line_alpha_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    if let Some(committed) = render_length_spinner_pt(
        ui,
        LengthSpinnerSpec {
            widget_id: "format_graph.marker_size_pt",
            label: tr(gctx.locale, "fg.symbol_size"),
            value_pt: draft.marker_size_pt,
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
        draft.marker_size_pt = committed;
        *dirty = true;
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
                    selected: draft.marker_fill_mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.marker_fill_mode = mode;
                *dirty = true;
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
                    selected: draft.line_dash == dash,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.line_dash = dash;
                *dirty = true;
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
                    selected: draft.series_alpha_pct == pct,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.series_alpha_pct = pct;
                draft.alpha_dirty = true;
                *dirty = true;
            }
        }
    });
}

fn render_fg_visibility_and_overlay_controls(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatGraphState, dirty: &mut bool) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let mut raw_on = draft.show_raw_data;
        let raw_lbl = tr(gctx.locale, "r13.fg.show_data_points");
        let raw_chk = ui.checkbox(&mut raw_on, raw_lbl);
        record_checkbox(ui, &raw_chk, gctx.meta, "format_graph.show_raw_data", raw_lbl, raw_on);
        if raw_chk.changed() {
            draft.show_raw_data = raw_on;
            *dirty = true;
        }

        let mut sym_on = draft.show_symbols;
        let sym_lbl = "Show symbols";
        let sym_chk = ui.checkbox(&mut sym_on, sym_lbl);
        record_checkbox(ui, &sym_chk, gctx.meta, "format_graph.show_symbols", sym_lbl, sym_on);
        if sym_chk.changed() {
            draft.show_symbols = sym_on;
            draft.show_symbols_dirty = true;
            *dirty = true;
        }

        let mut fit_on = draft.show_fitted_curve;
        let fit_lbl = tr(gctx.locale, "r13.fg.show_fitted_curve");
        let fit_enabled = draft.cached_regression_overlay.is_some();
        let fit_chk = ui.add_enabled(fit_enabled, egui::Checkbox::new(&mut fit_on, fit_lbl));
        record_checkbox_enabled(
            ui,
            &fit_chk,
            gctx.meta,
            CheckboxMeta {
                widget_id: "format_graph.show_fitted_curve",
                label: fit_lbl,
                checked: fit_on,
                enabled: fit_enabled,
                disabled_reason: (!fit_enabled).then_some("Attach a regression overlay to this series first"),
            },
        );
        if fit_chk.changed() && fit_enabled {
            draft.show_fitted_curve = fit_on;
            *dirty = true;
        }
    });

    let active_gs = draft.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id));
    let effective_kind = draft.graph_kind.or_else(|| active_gs.map(|gs| gs.spec.kind));
    if effective_kind == Some(GraphKind::Xy) {
        let compatible_fits: Vec<&ResultSheet> = gctx.app.document().result_sheets.iter().filter(|rs| is_xy_overlay_compatible_result(rs)).collect();
        if !compatible_fits.is_empty() || draft.cached_regression_overlay.is_some() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                ui.label(tr(gctx.locale, "fg.regression_overlay"));
                for (idx, rs) in compatible_fits.iter().enumerate() {
                    let selected = draft.cached_regression_overlay.as_ref().is_some_and(|o| o.result_sheet_id == rs.id);
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
                        draft.cached_regression_overlay = Some(RegressionOverlaySpec {
                            result_sheet_id: rs.id.clone(),
                            sample_segments: 256,
                            show_confidence_band: true,
                            show_prediction_band: false,
                            shaded_ribbon: false,
                        });
                        draft.pending_overlay_result_id = Some(Some(rs.id.clone()));
                        draft.show_fitted_curve = true;
                        *dirty = true;
                    }
                }
                if draft.cached_regression_overlay.is_some()
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
                    draft.cached_regression_overlay = None;
                    draft.pending_overlay_result_id = Some(None);
                    draft.show_fitted_curve = false;
                    *dirty = true;
                }
            });
        }
    }
}

fn render_fg_gdo_and_bar_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    let active_gs = draft.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id));
    let active_series = active_gs.and_then(|gs| gs.spec.series.get(draft.selected_series_index));
    let effective_kind = draft.graph_kind.or_else(|| active_gs.map(|gs| gs.spec.kind));
    let raw_is_scatter = active_series.is_some_and(|s| matches!(s.gdo, GraphicalDataObject::ScatterAndLine { .. }));
    let is_scatter_series = if draft.series_gdo_dirty { !draft.series_gdo_bar } else { raw_is_scatter };

    if matches!(effective_kind, Some(GraphKind::Xy | GraphKind::Column | GraphKind::Grouped)) {
        let scatter_selected = if draft.series_gdo_dirty { !draft.series_gdo_bar } else { raw_is_scatter };
        let bar_selected = draft.series_gdo_bar;
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
                    draft.series_gdo_bar = target_bar;
                    draft.series_gdo_dirty = true;
                    *dirty = true;
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
                        selected: draft.replicate_lines == mode,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.replicate_lines = mode;
                    draft.replicate_lines_dirty = true;
                    *dirty = true;
                }
            }
        });
    }

    let has_bar_border = if draft.series_gdo_dirty {
        draft.series_gdo_bar
    } else if draft.apply_all_data_sets {
        active_gs.is_some_and(|gs| {
            gs.spec.series.iter().any(|s| matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. }))
        })
    } else {
        active_series.is_some_and(|s| matches!(s.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. }))
    };
    if has_bar_border {
        ui.add_space(4.0);
        if let Some(w) = render_length_spinner_pt(
            ui,
            LengthSpinnerSpec {
                widget_id: "format_graph.bar_border_width_pt",
                label: tr(gctx.locale, "fg.bar_border"),
                value_pt: draft.bar_border_width_pt,
                min_pt: 0.0,
                max_pt: 6.0,
                step_pt: 0.25,
                enabled: true,
                disabled_reason: None,
            },
            &mut state.bar_border_spinner,
            gctx.tokens,
            gctx.meta,
        ) {
            draft.bar_border_width_pt = w;
            draft.bar_border_width_dirty = true;
            *dirty = true;
        }
    }

    if draft.graph_kind == Some(curvebook_doc::GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::ForestPlot)) {
        ui.add_space(4.0);
        let mut pooled_on = draft.forest_show_pooled_diamond;
        let pooled_lbl = "Show pooled summary diamond";
        let pooled_chk = ui.checkbox(&mut pooled_on, pooled_lbl);
        record_checkbox(ui, &pooled_chk, gctx.meta, "format_graph.forest_show_pooled_diamond", pooled_lbl, pooled_on);
        if pooled_chk.changed() {
            draft.forest_show_pooled_diamond = pooled_on;
            draft.forest_show_pooled_diamond_dirty = true;
            *dirty = true;
        }
    }
    if draft.graph_kind == Some(curvebook_doc::GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::EstimationPlot)) {
        ui.add_space(4.0);
        let mut paired_on = draft.estimation_paired_lines;
        let paired_lbl = "Connect matched pairs with slope lines";
        let paired_chk = ui.checkbox(&mut paired_on, paired_lbl);
        record_checkbox(ui, &paired_chk, gctx.meta, "format_graph.estimation_paired_lines", paired_lbl, paired_on);
        if paired_chk.changed() {
            draft.estimation_paired_lines = paired_on;
            draft.estimation_paired_lines_dirty = true;
            *dirty = true;
        }
    }
}

fn render_fg_error_bars_section(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    let mut eb_enabled = draft.error_bar_enabled;
    let eb_lbl = tr(gctx.locale, "inspector.error_bars");
    let eb_chk = ui.checkbox(&mut eb_enabled, eb_lbl);
    record_checkbox(ui, &eb_chk, gctx.meta, "format_graph.error_bar_enabled", eb_lbl, eb_enabled);
    if eb_chk.changed() {
        draft.error_bar_enabled = eb_enabled;
        *dirty = true;
    }

    if draft.error_bar_enabled {
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
                        selected: draft.error_bar_spec.metric == metric,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.error_bar_spec.metric = metric;
                    *dirty = true;
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
                        selected: draft.error_bar_spec.direction == dir,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.error_bar_spec.direction = dir;
                    *dirty = true;
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
            render_scrub_unit_field(ui, draft.error_bar_spec.stroke_width_pt, &mut state.line_thickness_spinner, stroke_cfg, gctx.tokens, gctx.meta);
        if let Some(committed) = stroke_out.committed_value {
            draft.error_bar_spec.stroke_width_pt = committed.clamp(0.25, 6.0);
            *dirty = true;
        }
    }
}

pub(super) fn render_fg_series_arrangement(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    draft: &mut FormatGraphState,
    dirty: &mut bool,
) {
    let mut pending_series_switch: Option<usize> = None;

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
                    selected: draft.grouped_arrangement == arr,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.grouped_arrangement = arr;
                *dirty = true;
            }
        }
    });

    if matches!(draft.graph_kind, Some(curvebook_doc::GraphKind::Column | curvebook_doc::GraphKind::Grouped)) {
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
                        selected: draft.orientation == target,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.orientation = target;
                    draft.orientation_dirty = true;
                    *dirty = true;
                }
            }
        });
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new(tr(gctx.locale, "fg.active_series_list")).strong());
    if let Some(gs) = draft.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)) {
        for (idx, series) in gs.spec.series.iter().enumerate() {
            let swatch = draft.preview_swatch(idx);
            let selected = idx == draft.selected_series_index;
            let bind_y2 = if selected && draft.bind_to_right_y2_dirty {
                draft.bind_to_right_y2
            } else if let Some(staged) = state.staged_series_edits.get(&idx)
                && staged.bind_to_right_y2_dirty
            {
                staged.bind_to_right_y2
            } else {
                series.bind_to_right_y2
            };
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
                    pending_series_switch = Some(idx);
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
                            selected: draft.bind_to_right_y2 == y2_target,
                            enabled: true,
                            disabled_reason: None,
                        },
                        gctx.tokens,
                    )
                    .clicked()
                    {
                        draft.bind_to_right_y2 = y2_target;
                        draft.bind_to_right_y2_dirty = true;
                        *dirty = true;
                    }
                }
            });

            ui.add_space(4.0);
            let off_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.05,
                decimals: 2,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.series_offset_x", "Horizontal Nudge (X)", -1000.0, 1000.0)
            };
            let off_out = render_scrub_unit_field(ui, draft.offset_x_data, &mut state.series_offset_x_spinner, off_cfg, gctx.tokens, gctx.meta);
            if let Some(x_val) = off_out.committed_value {
                draft.offset_x_data = x_val;
                draft.offset_x_data_dirty = true;
                *dirty = true;
            }
        }
    }

    if let Some(target_idx) = pending_series_switch {
        switch_format_graph_series(state, draft, target_idx, gctx.app.document());
        *dirty = true;
    }
}

pub(in crate::egui_shell::graph_modals) fn apply_palette_selection(draft: &mut FormatGraphState, pal: PaletteId) {
    draft.palette = pal;
    if let Some(enc) = &mut draft.mv_encoding
        && let ColorChannelMapping::Categorical { palette, .. } = &mut enc.color
    {
        *palette = pal;
        draft.mv_levels_dirty = true;
    }
}

pub(super) fn render_fg_settings(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatGraphState, dirty: &mut bool) {
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
                    selected: draft.palette == pal,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                apply_palette_selection(draft, pal);
                *dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for idx in 0..draft.palette.swatches().len().min(8) {
            let c = draft.preview_swatch(idx);
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
                    selected: draft.cvd_preview == cvd,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.cvd_preview = cvd;
                *dirty = true;
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
                    selected: draft.paper_fill == fill_opt,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.paper_fill = fill_opt;
                draft.paper_fill_dirty = true;
                *dirty = true;
            }
        }
    });

    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, draft, dirty);
}

pub(super) fn render_fg_annotations(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatGraphState, dirty: &mut bool) {
    ui.label(egui::RichText::new(tr(gctx.locale, "fg.annotations.heading")).strong());
    ui.add_space(4.0);
    let has_eq = draft.model_equation_mode.is_some();
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
                        selected: draft.model_equation_mode == Some(mode),
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    draft.set_model_equation_mode(mode);
                    *dirty = true;
                }
            }
        });
    } else {
        ui.label(tr(gctx.locale, "fg.annotations.empty"));
    }
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, draft, dirty);
}
