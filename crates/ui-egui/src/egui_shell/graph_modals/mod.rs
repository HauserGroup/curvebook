//! Graph formatting & configuration modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md`
//! §8.11B–D): `Change Graph Type`, 3-schema `Format Graph`, `Format Axes` (+ `Categorical Axis
//! Guard` and `Additional Ticks / Thresholds` sub-modal), `Pairwise Comparisons`, `Bubble Plot
//! Channels`, `Apply Style From`, `Format Selected Points`, `Insert Model Equation`, and
//! `Page Layout Composer`.

mod apply_style;
mod bubble_plot;
mod change_graph_type;
mod format_axes;
mod format_graph;
mod helpers;
mod insert_equation;
mod page_layout;
mod pairwise;
mod r13_modals;
mod selected_points;
mod state;

use curvebook_doc::{ColorChannelMapping, HeatmapColumnLabelAngle, MultivariateGraphEncoding, MultivariatePlotMode, SheetId, SizeChannelMapping};
use curvebook_palette::{ColorRamp, ColormapId};
use egui::scroll_area::ScrollBarVisibility;
use egui::{Context, Ui};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectMeta, PendingUiIntents, Tokens};

use apply_style::render_apply_style_body;
use bubble_plot::render_bubble_plot_body;
use change_graph_type::render_change_graph_type_body;
use format_axes::render_format_axes_body;
use format_graph::render_format_graph_body;
use helpers::{
    ButtonEntry, GraphModalCtx, gtr, record_button, record_tab, reset_format_axes_session_state, reset_format_graph_local_drafts,
    validate_heatmap_range,
};
use insert_equation::render_insert_equation_body;
use page_layout::render_page_layout_body;
use pairwise::render_pairwise_body;
use r13_modals::{render_create_graph_body, render_equalize_scaling_body, render_format_embedded_table_body, render_format_image_body};
use selected_points::render_format_selected_points_body;

pub(super) use helpers::{LengthSpinnerSpec, render_length_spinner_pt};
use state::HeatmapColorSnapshot;
pub use state::{
    AdditionalTickDraft, FormatAxesTab, FormatGraphSchema, FormatGraphTab, GraphModalEguiState, GraphModalFrameReport, GraphTypeFamily,
    MvOutOfRangePolicy, is_graph_modal_active,
};

/// Render the active graph or page-layout configuration modal when active.
pub fn render_graph_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut GraphModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> GraphModalFrameReport {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }) {
        reset_format_axes_session_state(state);
        state.axis_ticks_error = None;
    }
    if let ActiveDialog::FormatGraph { graph_sheet_id } = &app.canvas_dialogs.active_dialog {
        sync_format_graph_local_drafts(app, state, graph_sheet_id);
    } else {
        reset_format_graph_local_drafts(state);
    }
    let locale = normalize_locale(&app.locale);
    let (widget_id, title, preferred_w) = match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ChangeGraphType { .. } => ("dialog.change_graph_type", tr(locale, "cmd.graph.setKind"), 760.0_f32),
        ActiveDialog::FormatGraph { .. } => ("dialog.format_graph", tr(locale, "inspector.format_graph"), 680.0_f32),
        ActiveDialog::FormatAxes { .. } => ("dialog.format_axes", tr(locale, "inspector.format_axes"), 680.0_f32),
        ActiveDialog::PairwiseComparisons { .. } => ("dialog.pairwise_comparisons", tr(locale, "inspector.pairwise_comparisons"), 600.0_f32),
        ActiveDialog::BubblePlotInspector { .. } => ("dialog.bubble_plot", tr(locale, "w6.view.modal.bubble_plot_channels"), 580.0_f32),
        ActiveDialog::ApplyStyleFrom { .. } => ("dialog.apply_style_from", tr(locale, "btn.apply_style_from"), 520.0_f32),
        ActiveDialog::FormatSelectedPoints { .. } => ("dialog.format_selected_points", tr(locale, "cmd.graph.formatSelectedPoints"), 540.0_f32),
        ActiveDialog::InsertModelEquation { .. } => ("dialog.insert_model_equation", tr(locale, "w6.view.modal.insert_model_equation"), 540.0_f32),
        ActiveDialog::PageLayoutComposer { .. } => ("dialog.page_layout_composer", tr(locale, "w6.view.modal.page_layout_composer"), 620.0_f32),
        ActiveDialog::CreateGraph { .. } => ("dialog.create_graph", tr(locale, "cmd.ui.graph.create.open"), 520.0_f32),
        ActiveDialog::EqualizeScaling { .. } => ("dialog.equalize_scaling", tr(locale, "cmd.ui.layout.equalizeScaling.open"), 500.0_f32),
        ActiveDialog::FormatEmbeddedTable { .. } => ("dialog.format_embedded_table", tr(locale, "cmd.ui.embeddedTable.format.open"), 520.0_f32),
        ActiveDialog::FormatImage { .. } => ("dialog.format_image", tr(locale, "cmd.ui.image.format.open"), 540.0_f32),
        _ => return GraphModalFrameReport::default(),
    };

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 28.0).clamp(260.0, 920.0);
    let max_h = (screen.height() - 28.0).clamp(220.0, 660.0);
    let inner_w = max_w.min(preferred_w).max(260.0);
    let body_w = (inner_w - 28.0).max(200.0);

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_graph_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(inner_w);
            ui.heading(title);
        });
        ui.add_space(4.0);

        let mut gctx = GraphModalCtx { app, tokens, locale, meta, intents, body_w };
        let header_tabs_h = render_dialog_top_strip(ui, state, &mut gctx);
        let footer_h = if inner_w < 460.0 { 68.0 } else { 42.0 };
        let body_h = (max_h - 46.0 - header_tabs_h - footer_h).max(110.0);

        egui::ScrollArea::vertical()
            .id_salt(("graph_modal.scroll", widget_id))
            .max_height(body_h)
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);
                render_dialog_scroll_body(ui, state, &mut gctx);
            });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        render_dialog_footer(ui, state, &mut gctx);
    });

    modal_resp.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Dialog, true, title));
    let capture = finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    GraphModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_dialog_top_strip(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) -> f32 {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatGraph { .. } => {
            let schema = FormatGraphSchema::detect(gctx.app);
            let active_tab = schema.clamp_tab(state.format_graph_tab);
            state.format_graph_tab = active_tab;
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let cgt_lbl = gtr(gctx.locale, "fg.jump.change_graph_type");
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "format_graph.open_change_graph_type",
                        label: cgt_lbl,
                        command_id: Some("ui.changeGraphType.open"),
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    gctx.intents.push_ui("ui.changeGraphType.open", json!({}));
                }
                let axes_lbl = gtr(gctx.locale, "fg.jump.format_axes");
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "format_graph.open_format_axes",
                        label: axes_lbl,
                        command_id: Some("ui.graph.axes.open"),
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    gctx.intents.push_ui("ui.graph.axes.open", json!({}));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                for &tab in schema.tabs() {
                    let lbl = gtr(gctx.locale, tab.label_key());
                    if record_tab(ui, gctx.meta, tab.widget_id(), lbl, tab == active_tab, gctx.tokens).clicked() {
                        state.format_graph_tab = tab;
                    }
                }
            });
            ui.add_space(4.0);
            64.0
        }
        ActiveDialog::FormatAxes { .. } => {
            if state.additional_ticks.open {
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(gctx.body_w);
                    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.add_tick.submodal_title")).strong());
                });
                ui.add_space(4.0);
                return 28.0;
            }
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                for &tab in &FormatAxesTab::ALL {
                    let lbl = gtr(gctx.locale, tab.label_key());
                    if record_tab(ui, gctx.meta, tab.widget_id(), lbl, tab == state.format_axes_tab, gctx.tokens).clicked() {
                        state.format_axes_tab = tab;
                        let target_slug = match tab {
                            FormatAxesTab::XAxis => Some("x"),
                            FormatAxesTab::LeftYAxis => Some("y"),
                            FormatAxesTab::RightYAxis => Some("y2"),
                            FormatAxesTab::FrameAndOrigin | FormatAxesTab::TitlesAndTypography => None,
                        };
                        if let Some(slug) = target_slug {
                            let mut patch = Map::new();
                            patch.insert(String::from("axes.target_axis"), Value::String(slug.to_owned()));
                            gctx.intents.push_patch_active_dialog(patch);
                        }
                    }
                }
            });
            ui.add_space(4.0);
            34.0
        }
        _ => 0.0,
    }
}

fn render_dialog_scroll_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::ChangeGraphType { .. } => render_change_graph_type_body(ui, state, gctx),
        ActiveDialog::FormatGraph { .. } => render_format_graph_body(ui, state, gctx),
        ActiveDialog::FormatAxes { .. } => render_format_axes_body(ui, state, gctx),
        ActiveDialog::PairwiseComparisons { .. } => render_pairwise_body(ui, state, gctx),
        ActiveDialog::BubblePlotInspector { .. } => render_bubble_plot_body(ui, state, gctx),
        ActiveDialog::ApplyStyleFrom { .. } => render_apply_style_body(ui, gctx),
        ActiveDialog::FormatSelectedPoints { .. } => render_format_selected_points_body(ui, state, gctx),
        ActiveDialog::InsertModelEquation { .. } => render_insert_equation_body(ui, state, gctx),
        ActiveDialog::PageLayoutComposer { .. } => render_page_layout_body(ui, state, gctx),
        ActiveDialog::CreateGraph { .. } => render_create_graph_body(ui, gctx),
        ActiveDialog::EqualizeScaling { .. } => render_equalize_scaling_body(ui, gctx),
        ActiveDialog::FormatEmbeddedTable { .. } => render_format_embedded_table_body(ui, gctx),
        ActiveDialog::FormatImage { .. } => render_format_image_body(ui, gctx),
        _ => {}
    }
}

fn render_dialog_footer(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let active = &gctx.app.canvas_dialogs.active_dialog;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if matches!(active, ActiveDialog::FormatAxes { .. }) && state.additional_ticks.open {
            let close_lbl = gtr(gctx.locale, "axes.add_tick.done");
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "axes.add_tick.close",
                    label: close_lbl,
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.additional_ticks.open = false;
            }
            return;
        }

        let cancel_lbl = tr(gctx.locale, "btn.cancel");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry { widget_id: "modal.cancel", label: cancel_lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
            gctx.tokens,
        )
        .clicked()
        {
            reset_format_axes_session_state(state);
            reset_format_graph_local_drafts(state);
            state.axis_ticks_error = None;
            gctx.intents.push_dialog_button("cancel");
        }

        let supports_apply = matches!(
            active,
            ActiveDialog::ChangeGraphType { .. }
                | ActiveDialog::FormatGraph { .. }
                | ActiveDialog::FormatAxes { .. }
                | ActiveDialog::PairwiseComparisons { .. }
                | ActiveDialog::FormatEmbeddedTable { .. }
                | ActiveDialog::FormatImage { .. }
        );
        if supports_apply {
            let apply_lbl = gtr(gctx.locale, "modal.apply");
            let apply_wid = if matches!(active, ActiveDialog::ChangeGraphType { .. }) { "graph_type.apply" } else { "modal.apply" };
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry { widget_id: apply_wid, label: apply_lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
                gctx.tokens,
            )
            .clicked()
            {
                commit_graph_modal_action(gctx, state, "apply");
            }
        }

        let ok_lbl = tr(gctx.locale, "btn.ok");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry { widget_id: "modal.ok", label: ok_lbl, command_id: None, selected: true, enabled: true, disabled_reason: None },
            gctx.tokens,
        )
        .clicked()
        {
            commit_graph_modal_action(gctx, state, "ok");
        }
    });
}

fn sync_format_graph_local_drafts(app: &UiApp, state: &mut GraphModalEguiState, sheet_id: &SheetId) {
    let key = sheet_id.as_str();
    if state.format_graph_session_synced && state.format_graph_last_sheet_id.as_deref() == Some(key) {
        return;
    }
    state.format_graph_session_synced = true;
    state.format_graph_last_sheet_id = Some(key.to_owned());
    let gs_opt = app.document().graph_sheet(sheet_id);
    let fg = &app.canvas_dialogs.format_graph;
    let enc_opt = fg.mv_encoding.as_ref().or_else(|| gs_opt.and_then(|g| g.spec.multivariate.as_ref()));

    if let Some(enc) = enc_opt
        && let ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. } = enc.color
    {
        state.mv_clamp_min_draft = helpers::format_localized_draft_f64(lo, &app.locale_preset);
        state.mv_clamp_max_draft = helpers::format_localized_draft_f64(hi, &app.locale_preset);
        state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
    } else {
        state.mv_clamp_min_draft.clear();
        state.mv_clamp_max_draft.clear();
        state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
    }

    if let Some(enc) = enc_opt
        && let ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id, reversed }, domain_clamp, .. } = &enc.color
    {
        state.heatmap_colormap = *id;
        state.heatmap_reversed = *reversed;
        if let Some((lo, hi)) = *domain_clamp {
            state.heatmap_auto_range = false;
            state.heatmap_min_draft = helpers::format_localized_draft_f64(lo, &app.locale_preset);
            state.heatmap_max_draft = helpers::format_localized_draft_f64(hi, &app.locale_preset);
        } else {
            state.heatmap_auto_range = true;
            state.heatmap_min_draft = helpers::format_localized_draft_f64(-1.0, &app.locale_preset);
            state.heatmap_max_draft = helpers::format_localized_draft_f64(1.0, &app.locale_preset);
        }
    } else {
        state.heatmap_colormap = ColormapId::Viridis;
        state.heatmap_reversed = false;
        state.heatmap_auto_range = true;
        state.heatmap_min_draft = helpers::format_localized_draft_f64(-1.0, &app.locale_preset);
        state.heatmap_max_draft = helpers::format_localized_draft_f64(1.0, &app.locale_preset);
    }
    state.heatmap_range_error = None;
    state.heatmap_color_dirty = false;
    state.heatmap_synced_color_snapshot = Some(HeatmapColorSnapshot {
        colormap: state.heatmap_colormap,
        reversed: state.heatmap_reversed,
        auto_range: state.heatmap_auto_range,
        min_draft: state.heatmap_min_draft.clone(),
        max_draft: state.heatmap_max_draft.clone(),
    });

    state.heatmap_annotate_cells = fg.heatmap_labels.annotate_cells;
    state.heatmap_cell_gap_pt = fg.heatmap_labels.cell_gap_pt;
    let axis_angle = gs_opt.map_or(0.0, |g| g.spec.x_axis.tick_angle_deg);
    let label_angle_deg = match fg.heatmap_labels.column_angle {
        HeatmapColumnLabelAngle::Horizontal0 => 0.0,
        HeatmapColumnLabelAngle::Angled45 => 45.0,
        HeatmapColumnLabelAngle::Vertical90 => 90.0,
    };
    state.heatmap_tick_angle_deg = if axis_angle != 0.0 { axis_angle } else { label_angle_deg };
    state.heatmap_tick_angle_dirty = false;
    state.heatmap_synced_tick_angle_deg = axis_angle;
}

fn commit_format_graph_heatmap(gctx: &mut GraphModalCtx<'_>, state: &mut GraphModalEguiState, action: &str) {
    let fg = &gctx.app.canvas_dialogs.format_graph;
    let schema = FormatGraphSchema::detect(gctx.app);
    let Some(gid) = fg.graph_sheet_id.clone() else {
        gctx.intents.push_dialog_button(action);
        return;
    };
    if schema != FormatGraphSchema::Heatmap {
        gctx.intents.push_dialog_button(action);
        return;
    }

    let color_changed = state.heatmap_color_dirty
        || state.heatmap_synced_color_snapshot.as_ref().is_none_or(|snap| {
            snap.colormap != state.heatmap_colormap
                || snap.reversed != state.heatmap_reversed
                || snap.auto_range != state.heatmap_auto_range
                || snap.min_draft != state.heatmap_min_draft
                || snap.max_draft != state.heatmap_max_draft
        });
    let tick_angle_changed = state.heatmap_tick_angle_dirty || (state.heatmap_tick_angle_deg - state.heatmap_synced_tick_angle_deg).abs() > 1e-6;

    let validated_clamp = if color_changed {
        match validate_heatmap_range(state, &gctx.app.locale_preset) {
            Ok(bounds) => {
                state.heatmap_range_error = None;
                Some(bounds)
            }
            Err(msg) => {
                state.heatmap_range_error = Some(msg);
                state.format_graph_tab = FormatGraphTab::ColorMapping;
                return;
            }
        }
    } else {
        None
    };

    // Execute dialog button first so `FormatGraphState::commit_intents()` cannot overwrite explicit heatmap edits.
    gctx.intents.push_dialog_button(action);

    if let Some(clamp) = validated_clamp {
        let mut enc =
            fg.mv_encoding.clone().or_else(|| gctx.app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.clone())).unwrap_or_else(|| {
                MultivariateGraphEncoding {
                    plot_mode: MultivariatePlotMode::BubbleScatter,
                    x_column: 0,
                    y_column: 0,
                    color: ColorChannelMapping::Constant(fg.palette.color_at(0)),
                    size: SizeChannelMapping::Constant { radius_pt: 5.0 },
                    label_column: None,
                    x_level_order_override: None,
                    show_convex_hulls: false,
                    show_confidence_ellipses: false,
                }
            });
        let col = match enc.color {
            ColorChannelMapping::Continuous { column, .. } | ColorChannelMapping::Categorical { column, .. } => column,
            ColorChannelMapping::Constant(_) => enc.y_column,
        };
        enc.color = ColorChannelMapping::Continuous {
            column: col,
            ramp: ColorRamp::Named { id: state.heatmap_colormap, reversed: state.heatmap_reversed },
            domain_clamp: clamp,
        };
        gctx.intents.push_engine(
            "graph.setSpec",
            json!({
                "sheet_id": gid.as_str(),
                "multivariate": enc,
            }),
        );
    }

    if tick_angle_changed {
        gctx.intents.push_engine(
            "graph.setAxes",
            json!({
                "sheet_id": gid.as_str(),
                "axis": "x",
                "tick_angle_deg": state.heatmap_tick_angle_deg,
            }),
        );
    }

    if action != "ok" {
        state.heatmap_color_dirty = false;
        state.heatmap_tick_angle_dirty = false;
        state.heatmap_synced_tick_angle_deg = state.heatmap_tick_angle_deg;
        state.heatmap_synced_color_snapshot = Some(HeatmapColorSnapshot {
            colormap: state.heatmap_colormap,
            reversed: state.heatmap_reversed,
            auto_range: state.heatmap_auto_range,
            min_draft: state.heatmap_min_draft.clone(),
            max_draft: state.heatmap_max_draft.clone(),
        });
    }
}

fn commit_graph_modal_action(gctx: &mut GraphModalCtx<'_>, state: &mut GraphModalEguiState, action: &str) {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatGraph { .. } => commit_format_graph_heatmap(gctx, state, action),
        ActiveDialog::FormatAxes { .. } => {
            if state.axis_bounds_error.is_some() || state.axis_ticks_error.is_some() {
                return;
            }
            let fa = &gctx.app.canvas_dialogs.format_axes;
            if state.format_axes_frame_preset_dirty
                && let Some(gid) = &fa.graph_sheet_id
            {
                gctx.intents.push_engine(
                    "graph.setAxes",
                    json!({
                        "sheet_id": gid.as_str(),
                        "frame_preset": state.format_axes_frame_preset,
                    }),
                );
            }
            if action == "apply" {
                if let Some(intent) = fa.commit_intent() {
                    gctx.intents.push_engine(intent.command_id, intent.params);
                }
                state.format_axes_frame_preset_dirty = false;
            } else {
                reset_format_axes_session_state(state);
                state.axis_ticks_error = None;
                gctx.intents.push_dialog_button(action);
            }
        }
        _ => {
            gctx.intents.push_dialog_button(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AdditionalTickDraft, FormatAxesTab, FormatGraphSchema, FormatGraphTab, GraphModalEguiState};

    #[test]
    fn format_graph_schema_clamps_tabs_cleanly() {
        assert_eq!(FormatGraphSchema::Heatmap.clamp_tab(FormatGraphTab::Appearance), FormatGraphTab::ColorMapping);
        assert_eq!(FormatGraphSchema::MvChannelLayer.clamp_tab(FormatGraphTab::ColorMapping), FormatGraphTab::AxisVariables);
        assert_eq!(FormatGraphSchema::SeriesBased.clamp_tab(FormatGraphTab::DataObjects), FormatGraphTab::Appearance);
        assert_eq!(FormatAxesTab::ALL.len(), 5);
        let st = GraphModalEguiState::default();
        assert!(!st.categorical_guard_confirmed);
        assert!(!AdditionalTickDraft::default().open);
    }
}
