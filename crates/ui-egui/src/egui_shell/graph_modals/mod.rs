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

use curvebook_doc::{
    ColorChannelMapping, DiagnosticPlotKind, GraphKind, HeatmapColumnLabelAngle, MultivariateGraphEncoding, MultivariatePlotMode, SheetId,
    SizeChannelMapping,
};
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_tools::ToolCommandIntent;
use egui::scroll_area::ScrollBarVisibility;
use egui::{Context, Ui};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AxisTarget};
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use apply_style::render_apply_style_body;
use bubble_plot::render_bubble_plot_body;
use change_graph_type::render_change_graph_type_body;
use format_axes::render_format_axes_body;
use format_graph::render_format_graph_body;
use helpers::{
    ButtonEntry, GraphModalCtx, record_button, record_tab, reset_all_graph_modal_transient_state, reset_format_axes_session_state,
    reset_format_graph_local_drafts, validate_heatmap_range,
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

/// Reset all transient graph-modal drafts, staged edits, spinners, and preview caches on dialog close.
pub(super) fn reset_graph_modal_on_close(state: &mut GraphModalEguiState) {
    reset_all_graph_modal_transient_state(state);
}

/// Build the single atomic `graph.setAxes` commit intent for `FormatAxes`, enforcing local axis
/// validation errors and honoring an explicit user-chosen `frame_preset` even when `enable_y2` is true.
pub(super) fn format_axes_commit_intent(app: &UiApp) -> Result<Option<ToolCommandIntent>, String> {
    let gm = &app.egui_views.graph_modals;
    if let Some(err) = gm.first_axis_error() {
        return Err(err);
    }
    let fa = &app.canvas_dialogs.format_axes;
    if fa.graph_sheet_id.is_none() {
        return Ok(None);
    }
    if let Err(err) = fa.axis_draft.validate() {
        return Err(err.to_string());
    }
    let mut intent = fa.commit_intent().ok_or_else(|| String::from("Axis configuration is invalid and cannot be committed"))?;
    if gm.format_axes_frame_preset_dirty
        && let Some(obj) = intent.params.as_object_mut()
    {
        let preset_val = serde_json::to_value(gm.format_axes_frame_preset).map_err(|e| e.to_string())?;
        obj.insert(String::from("frame_preset"), preset_val);
    }
    Ok(Some(intent))
}

/// Update `FormatAxes` transient state after commit succeeds (`apply` vs `ok` close).
pub(super) fn on_format_axes_committed(state: &mut GraphModalEguiState, close: bool) {
    state.format_axes_frame_preset_dirty = false;
    if close {
        reset_graph_modal_on_close(state);
    }
}

fn heatmap_color_changed(state: &GraphModalEguiState) -> bool {
    state.heatmap_color_dirty
        || state.heatmap_synced_color_snapshot.as_ref().is_none_or(|snap| {
            snap.colormap != state.heatmap_colormap
                || snap.reversed != state.heatmap_reversed
                || snap.auto_range != state.heatmap_auto_range
                || snap.min_draft != state.heatmap_min_draft
                || snap.max_draft != state.heatmap_max_draft
        })
}

fn heatmap_tick_angle_changed(state: &GraphModalEguiState) -> bool {
    state.heatmap_tick_angle_dirty || (state.heatmap_tick_angle_deg - state.heatmap_synced_tick_angle_deg).abs() > 1e-6
}

fn build_heatmap_multivariate_encoding(app: &UiApp, gid: &SheetId, clamp: Option<(f64, f64)>) -> MultivariateGraphEncoding {
    let fg = &app.canvas_dialogs.format_graph;
    let state = &app.egui_views.graph_modals;
    let mut enc = fg.mv_encoding.clone().or_else(|| app.document().graph_sheet(gid).and_then(|g| g.spec.multivariate.clone())).unwrap_or_else(|| {
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
    enc
}

/// Build the full atomic command list for `FormatGraph`, enforcing MV/Heatmap validation,
/// skipping invalid `graph.setSeries` for `Heatmap`/empty-series graphs, merging heatmap
/// colormap/clamp into `graph.setSpec`, appending `graph.setAxes` only when `tick_angle_deg`
/// changed, and emitting staged per-series edits (`AUD-10-31`, `AUD-10-38`, `AUD-10-40`).
pub(super) fn format_graph_commit_intents(app: &UiApp) -> Result<Vec<ToolCommandIntent>, String> {
    let fg = &app.canvas_dialogs.format_graph;
    let state = &app.egui_views.graph_modals;
    if let Some(err) = &state.mv_clamp_error {
        return Err(err.clone());
    }
    if let Some(err) = &state.heatmap_range_error {
        return Err(err.clone());
    }
    let Some(gid) = &fg.graph_sheet_id else {
        return Ok(Vec::new());
    };
    let gs_opt = app.document().graph_sheet(gid);
    let schema = FormatGraphSchema::detect(app);
    let hm_color_dirty = schema == FormatGraphSchema::Heatmap && heatmap_color_changed(state);
    let hm_angle_dirty = schema == FormatGraphSchema::Heatmap && heatmap_tick_angle_changed(state);
    let validated_clamp = if hm_color_dirty { Some(validate_heatmap_range(state, &app.locale_preset)?) } else { None };

    let eff_kind = gs_opt.map(|g| g.spec.kind).or(fg.graph_kind);
    let skip_series = matches!(eff_kind, Some(GraphKind::MultipleVariables | GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)))
        || gs_opt.is_some_and(|g| g.spec.series.is_empty());

    let mut intents = fg.commit_intents();
    if skip_series {
        intents.retain(|i| i.command_id != "graph.setSeries" && i.command_id != "graph.setRegressionOverlay");
    }

    if let Some(clamp) = validated_clamp {
        let enc = build_heatmap_multivariate_encoding(app, gid, clamp);
        let enc_val = serde_json::to_value(&enc).map_err(|e| e.to_string())?;
        if let Some(spec_cmd) = intents.iter_mut().find(|i| i.command_id == "graph.setSpec")
            && let Some(obj) = spec_cmd.params.as_object_mut()
        {
            obj.insert(String::from("multivariate"), enc_val);
        } else {
            intents.push(ToolCommandIntent::new(
                "graph.setSpec",
                json!({
                    "sheet_id": gid.as_str(),
                    "multivariate": enc,
                }),
            ));
        }
    }

    if !skip_series && !fg.apply_all_data_sets {
        let series_limit = gs_opt.map_or(usize::MAX, |g| g.spec.series.len());
        for (&idx, staged) in &state.staged_series_edits {
            if idx == fg.selected_series_index || idx >= series_limit {
                continue;
            }
            let mut tmp = fg.clone();
            tmp.selected_series_index = idx;
            tmp.apply_all_data_sets = false;
            staged.apply_to(&mut tmp);
            for extra in tmp.commit_intents() {
                if extra.command_id == "graph.setSeries" || extra.command_id == "graph.setRegressionOverlay" {
                    intents.push(extra);
                }
            }
        }
    }

    if hm_angle_dirty {
        intents.push(ToolCommandIntent::new(
            "graph.setAxes",
            json!({
                "sheet_id": gid.as_str(),
                "axis": "x",
                "tick_angle_deg": state.heatmap_tick_angle_deg,
            }),
        ));
    }

    Ok(intents)
}

/// Update `FormatGraph` local drafts, staged edits, and synced spec snapshot after commit succeeds.
pub(super) fn on_format_graph_committed(app: &mut UiApp, close: bool) {
    if close {
        reset_graph_modal_on_close(&mut app.egui_views.graph_modals);
        return;
    }
    let gid = app.canvas_dialogs.format_graph.graph_sheet_id.clone();
    let fresh_spec = gid.as_ref().and_then(|id| app.document().graph_sheet(id)).map(|g| g.spec.clone());
    let state = &mut app.egui_views.graph_modals;
    state.staged_series_edits.clear();
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
    state.format_graph_synced_spec = fresh_spec;

    let fg = &mut app.canvas_dialogs.format_graph;
    fg.annotations_dirty = false;
    fg.mv_levels_dirty = false;
    fg.alpha_dirty = false;
    fg.paper_fill_dirty = false;
    fg.heatmap_labels_dirty = false;
    fg.orientation_dirty = false;
    fg.show_symbols_dirty = false;
    fg.replicate_lines_dirty = false;
    fg.bind_to_right_y2_dirty = false;
    fg.offset_x_data_dirty = false;
    fg.bar_border_width_dirty = false;
    fg.forest_show_pooled_diamond_dirty = false;
    fg.pending_overlay_result_id = None;
    fg.series_gdo_dirty = false;
}

fn graph_modal_local_validation_error(app: &UiApp, state: &GraphModalEguiState) -> Option<String> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatAxes { .. } => state.first_axis_error(),
        ActiveDialog::FormatGraph { .. } => state.heatmap_range_error.clone().or_else(|| state.mv_clamp_error.clone()),
        _ => None,
    }
}

fn evaluate_graph_modal_button(app: &UiApp, _state: &GraphModalEguiState, action: &str) -> (bool, Option<String>) {
    if let Some(eval) = crate::modal_predicates::evaluate_dialog_button_for_app(app, action)
        && !eval.enabled
    {
        return (false, eval.disabled_reason);
    }
    (true, None)
}

fn sync_transient_sheet_state(app: &UiApp, state: &mut GraphModalEguiState) {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }) {
        reset_format_axes_session_state(state);
    }
    if let ActiveDialog::FormatGraph { graph_sheet_id } = &app.canvas_dialogs.active_dialog {
        sync_format_graph_local_drafts(app, state, graph_sheet_id);
    } else {
        reset_format_graph_local_drafts(state);
    }
    if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::PageLayoutComposer { .. }) {
        let cur_id = app.canvas_dialogs.page_layout.layout_sheet_id.as_ref().map(|s| s.as_str().to_owned());
        if state.page_layout_last_sheet_id != cur_id {
            state.page_callout_draft.clear();
            state.page_layout_last_sheet_id = cur_id;
        }
    } else {
        state.page_callout_draft.clear();
        state.page_layout_last_sheet_id = None;
    }
    if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let cur_id = app.canvas_dialogs.change_graph_type.graph_sheet_id.as_ref().map(|s| s.as_str().to_owned());
        if state.change_graph_type_last_sheet_id != cur_id {
            state.graph_type_family = GraphTypeFamily::Recommended;
            state.graph_type_show_all = false;
            state.graph_type_preview_cache = None;
            state.change_graph_type_last_sheet_id = cur_id;
        }
    } else {
        state.graph_type_family = GraphTypeFamily::Recommended;
        state.graph_type_show_all = false;
        state.graph_type_preview_cache = None;
        state.change_graph_type_last_sheet_id = None;
    }
}

fn handle_post_modal_keys(
    ctx: &Context,
    gctx: &mut GraphModalCtx<'_>,
    state: &mut GraphModalEguiState,
    spinner_editing_before: bool,
    wants_kb_before: bool,
) {
    let active = &gctx.app.canvas_dialogs.active_dialog;
    if !spinner_editing_before && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        if matches!(active, ActiveDialog::FormatAxes { .. }) && state.additional_ticks.open {
            state.additional_ticks.open = false;
        } else if matches!(active, ActiveDialog::FormatAxes { .. }) && state.has_pending_categorical_guard() {
            state.clear_categorical_guard();
        } else {
            reset_graph_modal_on_close(state);
            gctx.intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
        }
        return;
    }

    if !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && !(matches!(active, ActiveDialog::FormatAxes { .. }) && state.additional_ticks.open)
        && evaluate_graph_modal_button(gctx.app, state, "ok").0
        && graph_modal_local_validation_error(gctx.app, state).is_none()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
    {
        commit_graph_modal_action(gctx, state, "ok");
    }
}

/// Render the active graph or page-layout configuration modal when active.
pub fn render_graph_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut GraphModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> GraphModalFrameReport {
    sync_transient_sheet_state(app, state);
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

    let spinner_editing_before = state.any_spinner_editing();
    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
    let mut gctx = GraphModalCtx { app, tokens, locale, meta, intents, body_w };
    handle_post_modal_keys(ctx, &mut gctx, state, spinner_editing_before, wants_kb_before);
    GraphModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn format_graph_jump_params(app: &UiApp) -> Value {
    app.canvas_dialogs.format_graph.graph_sheet_id.as_ref().map_or_else(|| json!({}), |gid| json!({ "sheet_id": gid.as_str() }))
}

fn render_dialog_top_strip(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) -> f32 {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatGraph { .. } => {
            let schema = FormatGraphSchema::detect(gctx.app);
            let active_tab = schema.clamp_tab(state.format_graph_tab);
            state.format_graph_tab = active_tab;
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let cgt_lbl = tr(gctx.locale, "fg.jump.change_graph_type");
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
                    gctx.intents.push_ui("ui.changeGraphType.open", format_graph_jump_params(gctx.app));
                }
                let axes_lbl = tr(gctx.locale, "fg.jump.format_axes");
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
                    gctx.intents.push_ui("ui.graph.axes.open", format_graph_jump_params(gctx.app));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                for &tab in schema.tabs() {
                    let lbl = tr(gctx.locale, tab.label_key());
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
                    ui.label(egui::RichText::new(tr(gctx.locale, "axes.add_tick.submodal_title")).strong());
                });
                ui.add_space(4.0);
                return 28.0;
            }
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                for &tab in &FormatAxesTab::ALL {
                    let lbl = tr(gctx.locale, tab.label_key());
                    let is_active = state.format_axes_pending_tab.unwrap_or(state.format_axes_tab) == tab;
                    if record_tab(ui, gctx.meta, tab.widget_id(), lbl, is_active, gctx.tokens).clicked() {
                        let target_pair = match tab {
                            FormatAxesTab::XAxis => Some((AxisTarget::X, "x")),
                            FormatAxesTab::LeftYAxis => Some((AxisTarget::Y, "y")),
                            FormatAxesTab::RightYAxis => Some((AxisTarget::Y2, "y2")),
                            FormatAxesTab::FrameAndOrigin | FormatAxesTab::TitlesAndTypography => None,
                        };
                        if let Some((target_axis, slug)) = target_pair {
                            if gctx.app.canvas_dialogs.format_axes.target_axis == target_axis {
                                state.format_axes_tab = tab;
                                state.format_axes_pending_tab = None;
                            } else {
                                state.format_axes_pending_tab = Some(tab);
                                let mut patch = Map::new();
                                patch.insert(String::from("axes.target_axis"), Value::String(slug.to_owned()));
                                gctx.intents.push(UiIntent::PatchActiveDialog { fields: patch });
                            }
                        } else {
                            state.format_axes_tab = tab;
                            state.format_axes_pending_tab = None;
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
            let close_lbl = tr(gctx.locale, "axes.add_tick.done");
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
            reset_graph_modal_on_close(state);
            gctx.intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
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
            let apply_lbl = tr(gctx.locale, "btn.apply");
            let (apply_enabled, apply_reason) = evaluate_graph_modal_button(gctx.app, state, "apply");
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "modal.apply",
                    label: apply_lbl,
                    command_id: None,
                    selected: false,
                    enabled: apply_enabled,
                    disabled_reason: apply_reason.as_deref(),
                },
                gctx.tokens,
            )
            .clicked()
                && apply_enabled
            {
                commit_graph_modal_action(gctx, state, "apply");
            }
        }

        let ok_lbl = tr(gctx.locale, "btn.ok");
        let (ok_enabled, ok_reason) = evaluate_graph_modal_button(gctx.app, state, "ok");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "modal.ok",
                label: ok_lbl,
                command_id: None,
                selected: true,
                enabled: ok_enabled,
                disabled_reason: ok_reason.as_deref(),
            },
            gctx.tokens,
        )
        .clicked()
            && ok_enabled
        {
            commit_graph_modal_action(gctx, state, "ok");
        }
    });
}

fn sync_format_graph_local_drafts(app: &UiApp, state: &mut GraphModalEguiState, sheet_id: &SheetId) {
    let key = sheet_id.as_str();
    let gs_opt = app.document().graph_sheet(sheet_id);
    let doc_spec = gs_opt.map(|g| &g.spec);
    let spec_changed = state.format_graph_synced_spec.as_ref() != doc_spec;
    if state.format_graph_session_synced && state.format_graph_last_sheet_id.as_deref() == Some(key) && !spec_changed {
        return;
    }
    let external_spec_change = state.format_graph_synced_spec.is_some() && spec_changed;
    state.format_graph_session_synced = true;
    state.format_graph_last_sheet_id = Some(key.to_owned());
    state.format_graph_synced_spec = doc_spec.cloned();
    state.staged_series_edits.clear();
    state.mv_clamp_error = None;

    let fg = &app.canvas_dialogs.format_graph;
    let enc_opt = if external_spec_change {
        gs_opt.and_then(|g| g.spec.multivariate.as_ref()).or(fg.mv_encoding.as_ref())
    } else {
        fg.mv_encoding.as_ref().or_else(|| gs_opt.and_then(|g| g.spec.multivariate.as_ref()))
    };

    if let Some(enc) = enc_opt
        && let ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. } = enc.color
    {
        state.mv_clamp_min_draft = helpers::format_localized_draft_f64(lo, &app.locale_preset);
        state.mv_clamp_max_draft = helpers::format_localized_draft_f64(hi, &app.locale_preset);
        state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
    } else {
        state.mv_clamp_min_draft.clear();
        state.mv_clamp_max_draft.clear();
        state.mv_out_of_range = MvOutOfRangePolicy::Omit;
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

    let axis_angle = gs_opt.map_or(0.0, |g| g.spec.x_axis.tick_angle_deg);
    let label_angle_deg = match fg.heatmap_labels.column_angle {
        HeatmapColumnLabelAngle::Horizontal0 => 0.0,
        HeatmapColumnLabelAngle::Angled45 => 45.0,
        HeatmapColumnLabelAngle::Vertical90 => 90.0,
    };
    state.heatmap_tick_angle_deg = if axis_angle != 0.0 { axis_angle } else { label_angle_deg };
    state.heatmap_tick_angle_dirty = false;
    state.heatmap_synced_tick_angle_deg = state.heatmap_tick_angle_deg;
}

fn commit_graph_modal_action(gctx: &mut GraphModalCtx<'_>, state: &mut GraphModalEguiState, action: &str) {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatGraph { .. } => {
            if state.mv_clamp_error.is_some() {
                return;
            }
            if FormatGraphSchema::detect(gctx.app) == FormatGraphSchema::Heatmap
                && (state.heatmap_range_error.is_some() || heatmap_color_changed(state))
            {
                match validate_heatmap_range(state, &gctx.app.locale_preset) {
                    Ok(_) => state.heatmap_range_error = None,
                    Err(msg) => {
                        state.heatmap_range_error = Some(msg);
                        state.format_graph_tab = FormatGraphTab::ColorMapping;
                        return;
                    }
                }
            }
            gctx.intents.push(UiIntent::DialogButtonAction { action: action.into() });
        }
        ActiveDialog::FormatAxes { .. } => {
            if state.has_any_axis_errors() {
                return;
            }
            gctx.intents.push(UiIntent::DialogButtonAction { action: action.into() });
        }
        _ => {
            gctx.intents.push(UiIntent::DialogButtonAction { action: action.into() });
        }
    }
}

#[cfg(test)]
mod tests;
