//! Unit tests for `graph_modals` covering AUD-10 and AUD-13 remediations and the
//! `pub(super)` contract wired into `dialog_actions.rs`.

use curvebook_chart::{Path2D, PathCmd, PointPt};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, Cell, ColorChannelMapping, CropInsetsPt, DataSheet, DiagnosticPlotKind, FramePreset, GraphAxisScale,
    GraphKind, GraphSheet, GraphSpec, HeatmapColumnLabelAngle, JournalWidthPreset, LayoutItem, LinearRegressionSpec, MultipleVariablesTable,
    MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, OneWayAnovaSpec, PageOrientation, PassingBablokSpec, ProjectDocument, RectPt,
    RegressionOverlaySpec, ResultSheet, SheetId, SimpleLogisticSpec, SizeChannelMapping, SplineLowessSpec, SubcolumnMode, TableArchetype,
    TickLabelFormat, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, PaletteId};
use egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AxisTarget, BubblePlotInspectorState, FormatImageDialogState, PageLayoutComposerState, VariableCandidate};
use crate::egui_shell::unit_spinners::ScrubUnitKind;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use crate::i18n::LocalePreset;

use super::bubble_plot::cycle_bubble_color_column;
use super::change_graph_type::{flatten_path_subpaths, get_or_compile_preview, triangulate_polygon_ear_clip};
use super::format_axes::{switch_active_segment, sync_format_axes_local_drafts};
use super::format_graph::mv::{apply_mv_color_clamp_drafts, clamp_mv_max_radius_pt, refresh_mv_levels_from_doc};
use super::format_graph::series::{apply_palette_selection, is_xy_overlay_compatible_result, switch_format_graph_series};
use super::helpers::convert_committed_length_to_pt;
use super::page_layout::{arrange_layout_grid_oriented, next_unique_callout_id, oriented_page_size_pt};
use super::r13_modals::{
    apply_format_image_height, apply_format_image_width, clamp_image_crop_to_bounds, format_drag_value_localized, parse_drag_value_localized,
};
use super::{
    AdditionalTickDraft, FormatAxesTab, FormatGraphSchema, FormatGraphTab, GraphModalEguiState, GraphTypeFamily, evaluate_graph_modal_button,
    format_axes_commit_intent, format_graph_commit_intents, format_graph_jump_params, graph_modal_local_validation_error, on_format_axes_committed,
    on_format_graph_committed, render_graph_modal, reset_graph_modal_on_close, sync_format_graph_local_drafts,
};

fn build_xy_app_with_two_series() -> (UiApp, SheetId, SheetId) {
    let mut doc = ProjectDocument::new("test").expect("doc");
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).expect("xy table");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 2) as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(((r + 1) * 3) as f64);
    }
    let ds_id = SheetId::new("ds-xy");
    let ds = DataSheet::new(ds_id.clone(), "XY", TableArchetype::Xy(xy)).expect("ds");
    let g_id = SheetId::new("g-xy");
    let mut gs = GraphSheet::new_uninitialized_for_data(g_id.clone(), &ds).expect("gs");
    gs.initialized = true;
    gs.title = String::from("XY Graph");
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_graph_sheet(gs).expect("add gs");
    doc.active_sheet_id = Some(g_id.clone());

    let app = UiApp::from_session(Session::from_document(doc));
    (app, ds_id, g_id)
}

fn build_mv_app() -> (UiApp, SheetId, SheetId) {
    let mut doc = ProjectDocument::new("mv-test").expect("doc");
    let mut mv = MultipleVariablesTable::new(3, 3).expect("mv table");
    mv.variables[0].title = String::from("Treatment");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].cells = vec![Cell::text("Ctrl"), Cell::text("Drug"), Cell::text("Ctrl")];
    mv.variables[1].title = String::from("Sex");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[1].cells = vec![Cell::text("F"), Cell::text("M"), Cell::text("F")];
    mv.variables[2].title = String::from("Response");
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[2].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)];

    let ds_id = SheetId::new("ds-mv");
    let ds = DataSheet::new(ds_id.clone(), "MV", TableArchetype::MultipleVariables(mv)).expect("ds");
    let g_id = SheetId::new("g-mv");
    let mut gs = GraphSheet::new_uninitialized_for_data(g_id.clone(), &ds).expect("gs");
    gs.initialized = true;
    gs.title = String::from("MV Graph");
    gs.spec.kind = GraphKind::MultipleVariables;
    gs.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 2,
        y_column: 2,
        color: ColorChannelMapping::Categorical { column: 0, palette: PaletteId::OkabeIto, level_order_override: None },
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_graph_sheet(gs).expect("add gs");

    let app = UiApp::from_session(Session::from_document(doc));
    (app, ds_id, g_id)
}

fn build_heatmap_app(empty_series: bool) -> (UiApp, SheetId) {
    let (app, ds_id, _) = build_xy_app_with_two_series();
    let ds = app.document().data_sheet(&ds_id).expect("ds").clone();
    let g_id = SheetId::new("g-heat");
    let mut spec = GraphSpec::default_for_data_sheet(&ds);
    spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    if empty_series {
        spec.series.clear();
    }
    spec.heatmap_labels.column_angle = HeatmapColumnLabelAngle::Angled45;
    spec.x_axis.tick_angle_deg = 0.0;
    spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 0,
        color: ColorChannelMapping::Continuous {
            column: 0,
            ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: false },
            domain_clamp: Some((0.0, 10.0)),
        },
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    let mut gs = GraphSheet::new_uninitialized_for_data(g_id.clone(), &ds).expect("heatmap gs");
    gs.initialized = true;
    gs.title = String::from("Heatmap");
    gs.spec = spec;
    let mut doc = app.document().clone();
    doc.add_graph_sheet(gs).expect("add heatmap");
    let app = UiApp::from_session(Session::from_document(doc));
    (app, g_id)
}

fn run_modal_frame(ctx: &Context, app: &UiApp, state: &mut GraphModalEguiState, events: Vec<Event>) -> (FrameInspectMeta, PendingUiIntents) {
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::default();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1024.0, 720.0))), events, ..RawInput::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        meta.begin_pass(ui.ctx());
        let _ = render_graph_modal(ui.ctx(), app, state, &tokens, &mut meta, &mut intents);
    });
    out.textures_delta.clear();
    (meta, intents)
}

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

#[test]
fn aud_10_01_mv_max_radius_clamp_never_panics_above_36pt() {
    assert_eq!(clamp_mv_max_radius_pt(40.0, 50.0), (40.0, 40.0));
    assert_eq!(clamp_mv_max_radius_pt(40.0, 20.0), (40.0, 40.0));
    assert_eq!(clamp_mv_max_radius_pt(5.0, 20.0), (5.0, 20.0));
    assert_eq!(clamp_mv_max_radius_pt(5.0, 99.0), (5.0, 36.0));
    assert_eq!(clamp_mv_max_radius_pt(f64::NAN, 12.0), (2.5, 12.0));
    assert_eq!(clamp_mv_max_radius_pt(10.0, f64::INFINITY), (10.0, 36.0));
}

#[test]
fn aud_10_11_length_spinner_uses_pre_toggle_unit() {
    let pt_val = convert_committed_length_to_pt(8.0, ScrubUnitKind::Points, 1.0, 24.0);
    assert!((pt_val - 8.0).abs() < 1e-9);
    let mm_val = convert_committed_length_to_pt(10.0, ScrubUnitKind::Millimeters, 1.0, 100.0);
    assert!((mm_val - 10.0 * (72.0 / 25.4)).abs() < 1e-6);
}

#[test]
fn aud_10_12_format_graph_resyncs_on_external_spec_change_and_close() {
    let (mut app, g_id) = build_heatmap_app(true);
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };

    let mut state = GraphModalEguiState::default();
    sync_format_graph_local_drafts(&app, &mut state, &g_id);
    assert_eq!(state.heatmap_colormap, ColormapId::Plasma);
    state.heatmap_colormap = ColormapId::DivergingBlueRed;
    state.heatmap_color_dirty = true;

    // External spec change (e.g. Undo or graph.setSpec) invalidates synced snapshot even without reset.
    let mut new_enc = gs.spec.multivariate.clone().expect("mv");
    new_enc.color = ColorChannelMapping::Continuous {
        column: 0,
        ramp: ColorRamp::Named { id: ColormapId::Cividis, reversed: true },
        domain_clamp: Some((1.0, 5.0)),
    };
    app.execute_command("graph.setSpec", json!({ "sheet_id": g_id.as_str(), "multivariate": new_enc })).expect("setSpec");
    sync_format_graph_local_drafts(&app, &mut state, &g_id);
    assert_eq!(state.heatmap_colormap, ColormapId::Cividis);
    assert!(state.heatmap_reversed);
    assert!(!state.heatmap_color_dirty);

    // Close via on_format_graph_committed(close=true) resets all local drafts and synced snapshot.
    app.egui_views.graph_modals = state;
    app.egui_views.graph_modals.heatmap_color_dirty = true;
    on_format_graph_committed(&mut app, true);
    assert!(!app.egui_views.graph_modals.format_graph_session_synced);
    assert!(app.egui_views.graph_modals.format_graph_synced_spec.is_none());
    assert!(!app.egui_views.graph_modals.heatmap_color_dirty);
}

#[test]
fn aud_10_13_format_axes_atomic_intent_honors_explicit_frame_preset_with_y2() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    let gs = app.document().graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::Y2);
    app.canvas_dialogs.format_axes.enable_y2 = true;
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id };

    app.egui_views.graph_modals.format_axes_frame_preset = FramePreset::PlainBoxed;
    app.egui_views.graph_modals.format_axes_frame_preset_dirty = true;

    let intent = format_axes_commit_intent(&app).expect("valid").expect("some intent");
    assert_eq!(intent.command_id, "graph.setAxes");
    assert_eq!(intent.params.get("frame_preset"), Some(&json!(FramePreset::PlainBoxed)));

    // Local axis error blocks commit.
    app.egui_views.graph_modals.axis_bounds_error = Some(String::from("bad bounds"));
    assert!(format_axes_commit_intent(&app).is_err());

    // on_format_axes_committed clears dirty flag without wiping session on apply, and resets on close.
    app.egui_views.graph_modals.axis_bounds_error = None;
    app.egui_views.graph_modals.format_axes_session_synced = true;
    on_format_axes_committed(&mut app.egui_views.graph_modals, false);
    assert!(!app.egui_views.graph_modals.format_axes_frame_preset_dirty);
    assert!(app.egui_views.graph_modals.format_axes_session_synced);
    on_format_axes_committed(&mut app.egui_views.graph_modals, true);
    assert!(!app.egui_views.graph_modals.format_axes_session_synced);
}

#[test]
fn aud_10_30_footer_predicates_local_errors_and_modal_apply_id() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    // 1. InsertModelEquation with no result sheet disables OK.
    app.canvas_dialogs.insert_model_equation.result_sheet_id = None;
    app.canvas_dialogs.active_dialog = ActiveDialog::InsertModelEquation { graph_sheet_id: g_id.clone() };
    let state = GraphModalEguiState::default();
    assert!(!evaluate_graph_modal_button(&app, &state, "ok").0);

    // 2. FormatAxes local errors block commit while keeping modal.ok click-to-reject feedback live.
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id.clone() };
    let err_state = GraphModalEguiState { axis_major_interval_error: Some(String::from("bad interval")), ..GraphModalEguiState::default() };
    assert!(graph_modal_local_validation_error(&app, &err_state).is_some());
    app.egui_views.graph_modals = err_state;
    assert!(format_axes_commit_intent(&app).is_err());

    // 3. FormatGraph local heatmap / MV clamp errors block commit.
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };
    let fg_err_state = GraphModalEguiState { mv_clamp_error: Some(String::from("bad clamp")), ..GraphModalEguiState::default() };
    assert!(graph_modal_local_validation_error(&app, &fg_err_state).is_some());
    app.egui_views.graph_modals = fg_err_state;
    assert!(format_graph_commit_intents(&app).is_err());

    // 4. ChangeGraphType uses "modal.apply" rather than "graph_type.apply".
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.change_graph_type.open_for_graph(&gs, &doc);
    app.canvas_dialogs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: g_id };
    let ctx = Context::default();
    let mut cgt_state = GraphModalEguiState::default();
    // Run 2 frames so egui::Modal settles its centered Area rect before inspecting footer buttons.
    let _ = run_modal_frame(&ctx, &app, &mut cgt_state, Vec::new());
    let (meta, _) = run_modal_frame(&ctx, &app, &mut cgt_state, Vec::new());
    assert!(meta.find_by_widget_id("modal.apply").is_some());
    assert!(meta.find_by_widget_id("graph_type.apply").is_none());
}

#[test]
fn aud_10_31_heatmap_untouched_no_spurious_set_axes_and_skips_empty_set_series() {
    let (mut app, g_id) = build_heatmap_app(true);
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };

    let mut state = GraphModalEguiState::default();
    sync_format_graph_local_drafts(&app, &mut state, &g_id);
    assert!((state.heatmap_tick_angle_deg - 45.0).abs() < 1e-6);
    assert!((state.heatmap_synced_tick_angle_deg - 45.0).abs() < 1e-6);
    app.egui_views.graph_modals = state;

    let intents = format_graph_commit_intents(&app).expect("valid heatmap intents");
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].command_id, "graph.setSpec");
    assert!(!intents.iter().any(|i| i.command_id == "graph.setSeries"));
    assert!(!intents.iter().any(|i| i.command_id == "graph.setAxes"));

    // Dirty colormap + tick angle merges multivariate into single graph.setSpec and appends 1 graph.setAxes.
    app.egui_views.graph_modals.heatmap_colormap = ColormapId::Viridis;
    app.egui_views.graph_modals.heatmap_color_dirty = true;
    app.egui_views.graph_modals.heatmap_tick_angle_deg = 90.0;
    app.egui_views.graph_modals.heatmap_tick_angle_dirty = true;
    let intents2 = format_graph_commit_intents(&app).expect("valid dirty heatmap intents");
    assert_eq!(intents2.iter().filter(|i| i.command_id == "graph.setSpec").count(), 1);
    assert!(intents2[0].params.get("multivariate").is_some());
    assert_eq!(intents2.iter().filter(|i| i.command_id == "graph.setAxes").count(), 1);

    // Invalid heatmap range returns Err.
    app.egui_views.graph_modals.heatmap_auto_range = false;
    app.egui_views.graph_modals.heatmap_min_draft = String::from("10");
    app.egui_views.graph_modals.heatmap_max_draft = String::from("5");
    assert!(format_graph_commit_intents(&app).is_err());
}

#[test]
fn aud_10_33_and_34_escape_cancels_spinner_and_categorical_guard_before_modal() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    let gs = app.document().graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::X);
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id };

    let ctx = Context::default();
    let mut state = GraphModalEguiState { format_axes_tab: FormatAxesTab::TitlesAndTypography, ..GraphModalEguiState::default() };
    let _ = run_modal_frame(&ctx, &app, &mut state, Vec::new());

    // Simulate active spinner editing: Escape must NOT close the modal.
    state.axis_stroke_spinner.editing = true;
    state.axis_stroke_spinner.draft_text = String::from("4.5");
    let (_, intents_esc1) = run_modal_frame(
        &ctx,
        &app,
        &mut state,
        vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
    );
    assert!(intents_esc1.is_empty(), "Escape while editing spinner must not emit cancel");
    assert!(!state.axis_stroke_spinner.editing);
    assert!(state.axis_stroke_spinner.draft_text.is_empty());

    // Categorical guard: Escape must clear ALL 4 guard fields without closing the modal.
    state.format_axes_tab = FormatAxesTab::XAxis;
    state.categorical_guard_confirmed = true;
    state.categorical_guard_pending_axis = Some(String::from("X"));
    state.format_axes_guard_pending_scale = Some(GraphAxisScale::Log10);
    state.format_axes_guard_pending_tick_format = Some(TickLabelFormat::Decimal);
    let (_, intents_esc2) = run_modal_frame(
        &ctx,
        &app,
        &mut state,
        vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
    );
    assert!(intents_esc2.is_empty(), "Escape on categorical guard must not cancel modal");
    assert!(!state.has_pending_categorical_guard());
    assert!(!state.categorical_guard_confirmed);

    // Next Escape (with no active spinner or categorical guard) emits modal cancel.
    let (_, intents_esc3) = run_modal_frame(
        &ctx,
        &app,
        &mut state,
        vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
    );
    assert!(
        intents_esc3.intents().iter().any(|i| matches!(i, UiIntent::DialogButtonAction { action } if action == "cancel")),
        "Escape with no active spinner or guard must emit DialogButtonAction::cancel"
    );

    // reset_graph_modal_on_close clears ALL spinners across all graph modals.
    state.eq_font_spinner.editing = true;
    state.eq_font_spinner.draft_text = String::from("36");
    state.pts_size_spinner.dragging = true;
    state.bar_border_spinner.editing = true;
    state.series_offset_x_spinner.dragging = true;
    reset_graph_modal_on_close(&mut state);
    assert!(!state.eq_font_spinner.editing);
    assert!(state.eq_font_spinner.draft_text.is_empty());
    assert!(!state.pts_size_spinner.dragging);
    assert!(!state.bar_border_spinner.editing);
    assert!(!state.series_offset_x_spinner.dragging);
    assert!(!state.any_spinner_editing());
}

#[test]
fn aud_10_35_and_37_independent_tick_errors_and_multi_segment_drafts() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    let gs = app.document().graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::Y);
    assert!(app.canvas_dialogs.format_axes.add_segment());
    app.canvas_dialogs.format_axes.axis_draft.segments[1] = AxisSegment {
        min: Some(100.0),
        max: Some(500.0),
        scale: GraphAxisScale::Log10,
        length_share: 2.0,
        major_tick_interval: Some(50.0),
        minor_ticks_per_interval: 4,
    };

    let mut state = GraphModalEguiState::default();
    sync_format_axes_local_drafts(&app, &mut state);

    // Set independent major and minor errors on segment 0: clearing minor keeps major error!
    state.axis_major_interval_error = Some(String::from("bad major"));
    state.axis_minor_ticks_error = Some(String::from("bad minor"));
    state.axis_minor_ticks_error = None;
    assert!(state.has_any_axis_errors());

    // Switch to segment 1: segment 0's error stays cached and still blocks commit!
    let preset = app.locale_preset.clone();
    switch_active_segment(&mut state, &app.canvas_dialogs.format_axes, 1, &preset);
    assert_eq!(state.format_axes_selected_segment, 1);
    assert_eq!(state.axis_min_draft, "100");
    assert_eq!(state.axis_max_draft, "500");
    assert_eq!(state.axis_length_share_draft, "2");
    assert_eq!(state.axis_major_interval_draft, "50");
    assert_eq!(state.axis_minor_ticks_draft, 4);
    assert!(state.axis_major_interval_error.is_none());
    assert!(state.has_any_axis_errors(), "cached error on segment 0 must still block commit");

    // Switching back to segment 0 restores its drafts and error.
    switch_active_segment(&mut state, &app.canvas_dialogs.format_axes, 0, &preset);
    assert!(state.axis_major_interval_error.is_some());
}

#[test]
fn aud_10_36_format_image_aspect_lock_without_pixel_size_and_crop_clamp() {
    let mut fi = FormatImageDialogState {
        pixel_size: None,
        bounds_pt: RectPt::new(0.0, 0.0, 200.0, 100.0),
        lock_aspect_ratio: true,
        crop: CropInsetsPt { left_pt: 60.0, right_pt: 60.0, top_pt: 40.0, bottom_pt: 40.0 },
        ..FormatImageDialogState::default()
    };

    apply_format_image_width(&mut fi, 300.0);
    assert!((fi.bounds_pt.width - 300.0).abs() < 1e-6);
    assert!((fi.bounds_pt.height - 150.0).abs() < 1e-6, "aspect lock must scale height when pixel_size is None");

    apply_format_image_width(&mut fi, 100.0);
    assert!((fi.bounds_pt.width - 100.0).abs() < 1e-6);
    assert!((fi.bounds_pt.height - 50.0).abs() < 1e-6);
    assert!(fi.crop.left_pt + fi.crop.right_pt <= fi.bounds_pt.width - 2.0 + 1e-9);
    assert!(fi.crop.top_pt + fi.crop.bottom_pt <= fi.bounds_pt.height - 2.0 + 1e-9);

    apply_format_image_height(&mut fi, 100.0);
    assert!((fi.bounds_pt.width - 200.0).abs() < 1e-6);

    let mut bad_crop = CropInsetsPt { left_pt: 80.0, right_pt: 40.0, top_pt: 0.0, bottom_pt: 0.0 };
    clamp_image_crop_to_bounds(&mut bad_crop, 62.0, 62.0);
    assert!((bad_crop.left_pt - 40.0).abs() < 1e-6);
    assert!((bad_crop.right_pt - 20.0).abs() < 1e-6);
}

#[test]
fn aud_10_38_and_39_mv_clamp_validation_level_refresh_and_categorical_palette() {
    let (app, _, g_id) = build_mv_app();
    let gs = app.document().graph_sheet(&g_id).expect("gs");
    let mut fg = crate::dialogs_canvas::FormatGraphState::default();
    fg.sync_from_graph_with_doc(gs, Some(app.document()));
    assert_eq!(fg.mv_levels, vec!["Ctrl", "Drug"]);

    // AUD-10-38: Categorical color must NEVER be converted to Continuous by clamp drafts.
    let mut state =
        GraphModalEguiState { mv_clamp_min_draft: String::from("0"), mv_clamp_max_draft: String::from("10"), ..GraphModalEguiState::default() };
    let mut enc = fg.mv_encoding.clone().expect("enc");
    assert!(!apply_mv_color_clamp_drafts(&mut state, &mut enc, &app.locale_preset));
    assert!(matches!(enc.color, ColorChannelMapping::Categorical { .. }));

    // AUD-10-38: Continuous color rejects lo >= hi and partial bounds with inline error.
    let mut cont_enc = enc.clone();
    cont_enc.color = ColorChannelMapping::Continuous { column: 2, ramp: ColorRamp::default(), domain_clamp: None };
    state.mv_clamp_min_draft = String::from("5");
    state.mv_clamp_max_draft = String::from("5");
    assert!(!apply_mv_color_clamp_drafts(&mut state, &mut cont_enc, &app.locale_preset));
    assert!(state.mv_clamp_error.is_some());

    state.mv_clamp_min_draft = String::from("1");
    state.mv_clamp_max_draft = String::from("9");
    assert!(apply_mv_color_clamp_drafts(&mut state, &mut cont_enc, &app.locale_preset));
    assert!(state.mv_clamp_error.is_none());
    assert!(matches!(cont_enc.color, ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. } if lo == 1.0 && hi == 9.0));

    // AUD-10-39: Switching color column from Treatment (0) to Sex (1) refreshes mv_levels.
    if let Some(ref mut e) = fg.mv_encoding {
        e.color = ColorChannelMapping::Categorical { column: 1, palette: PaletteId::OkabeIto, level_order_override: None };
    }
    refresh_mv_levels_from_doc(&mut fg, app.document());
    assert_eq!(fg.mv_levels, vec!["F", "M"]);
    assert!(!fg.mv_level_is_x);
    assert_eq!(fg.mv_selected_level.as_deref(), Some("F"));

    // AUD-10-39: Palette switch updates Categorical::palette and marks mv_levels_dirty.
    fg.mv_levels_dirty = false;
    apply_palette_selection(&mut fg, PaletteId::PaulTolVibrant);
    assert!(fg.mv_levels_dirty);
    assert!(matches!(fg.mv_encoding.as_ref().map(|e| &e.color), Some(ColorChannelMapping::Categorical { palette: PaletteId::PaulTolVibrant, .. })));
}

#[test]
fn aud_10_40_and_41_staged_series_edits_checkbox_enabled_and_jump_sheet_id() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };

    // AUD-10-41: Jump buttons include sheet_id.
    assert_eq!(format_graph_jump_params(&app), json!({ "sheet_id": g_id.as_str() }));

    // AUD-10-40: Edit series 0, switch to series 1, edit series 1, verify commit emits BOTH series!
    let mut state = GraphModalEguiState::default();
    let mut draft = app.canvas_dialogs.format_graph.clone();
    draft.marker_size_pt = 11.0;
    draft.line_dash = LineDashStyle::Dashed;
    draft.bind_to_right_y2 = true;
    draft.bind_to_right_y2_dirty = true;

    switch_format_graph_series(&mut state, &mut draft, 1, app.document());
    assert_eq!(draft.selected_series_index, 1);
    assert!(state.staged_series_edits.contains_key(&0));
    draft.marker_size_pt = 7.5;
    draft.offset_x_data = 0.25;
    draft.offset_x_data_dirty = true;

    app.canvas_dialogs.format_graph = draft.clone();
    app.egui_views.graph_modals = state.clone();
    let intents = format_graph_commit_intents(&app).expect("valid multi-series commit");
    let series_cmds: Vec<_> = intents.iter().filter(|i| i.command_id == "graph.setSeries").collect();
    assert_eq!(series_cmds.len(), 2, "must emit graph.setSeries for both active series 1 and staged series 0");
    assert!(series_cmds.iter().any(|c| c.params.get("series_index") == Some(&json!(0))));
    assert!(series_cmds.iter().any(|c| c.params.get("series_index") == Some(&json!(1))));

    // Switching back to series 0 restores staged edits.
    switch_format_graph_series(&mut state, &mut draft, 0, app.document());
    assert!((draft.marker_size_pt - 11.0).abs() < 1e-6);
    assert_eq!(draft.line_dash, LineDashStyle::Dashed);
    assert!(draft.bind_to_right_y2);

    // AUD-10-40: format_graph.show_fitted_curve reports enabled=false in inspect meta when no overlay.
    let ctx = Context::default();
    let (meta, _) = run_modal_frame(&ctx, &app, &mut state, Vec::new());
    let fit_chk = meta.find_by_widget_id("format_graph.show_fitted_curve").expect("fit chk");
    assert!(!fit_chk.enabled);
}

#[test]
fn aud_10_52_localized_drag_value_roundtrip_all_comma_locales() {
    for loc in ["de", "es", "fr", "pt-BR", "it", "nl", "pl", "ru", "tr"] {
        let preset = LocalePreset::for_locale(loc);
        assert_eq!(preset.decimal_separator, ',', "locale {loc} should be comma-decimal");
        assert_eq!(format_drag_value_localized(7.5, &preset), "7,5");
        assert_eq!(parse_drag_value_localized("7,5", " pt", &preset), Some(7.5));
        assert_eq!(parse_drag_value_localized("7,5 pt", " pt", &preset), Some(7.5));
        assert_eq!(parse_drag_value_localized("125,25%", "%", &preset), Some(125.25));
    }
    let en = LocalePreset::for_locale("en");
    assert_eq!(format_drag_value_localized(7.5, &en), "7.5");
    assert_eq!(parse_drag_value_localized("7.5 pt", " pt", &en), Some(7.5));
}

#[test]
fn aud_10_53_unique_callout_id_oriented_arrange_and_reset() {
    let items = vec![LayoutItem::TextCallout {
        id: String::from("callout-2"),
        markdown: String::from("existing"),
        bounds_pt: RectPt::new(0.0, 0.0, 50.0, 20.0),
        font_size_pt: 10.0,
    }];
    assert_eq!(next_unique_callout_id(&items), "callout-3");

    let landscape_a4 = oriented_page_size_pt(JournalWidthPreset::A4Portrait, PageOrientation::Landscape);
    assert!(landscape_a4.width > landscape_a4.height);
    let mut pl = PageLayoutComposerState {
        preset: JournalWidthPreset::A4Portrait,
        orientation: PageOrientation::Landscape,
        rows: 1,
        cols: 2,
        ..PageLayoutComposerState::default()
    };
    arrange_layout_grid_oriented(&mut pl, &[SheetId::new("g-1"), SheetId::new("g-2")], 24.0, 12.0);
    assert_eq!(pl.items.len(), 2);
    if let LayoutItem::LiveGraph { bounds_pt, .. } = &pl.items[1] {
        assert!(bounds_pt.x + bounds_pt.width > 650.0, "landscape A4 panel 2 should exceed portrait width (595 pt)");
    } else {
        panic!("expected LiveGraph");
    }

    let mut st = GraphModalEguiState::default();
    st.additional_ticks.value_draft = String::from("12");
    st.additional_ticks.shaded_zone = true;
    st.additional_tick_draw_gridline = false;
    st.page_callout_draft = String::from("note");
    st.graph_type_family = GraphTypeFamily::Xy;
    st.graph_type_show_all = true;
    reset_graph_modal_on_close(&mut st);
    assert!(st.additional_ticks.value_draft.is_empty());
    assert!(!st.additional_ticks.shaded_zone);
    assert!(st.additional_tick_draw_gridline);
    assert!(st.page_callout_draft.is_empty());
    assert_eq!(st.graph_type_family, GraphTypeFamily::Recommended);
    assert!(!st.graph_type_show_all);
}

#[test]
fn aud_10_54_preview_memoization_curve_subdivision_ear_clip_and_bubble_cols() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.change_graph_type.open_for_graph(&gs, &doc);
    let cgt = app.canvas_dialogs.change_graph_type.clone();
    let mut state = GraphModalEguiState::default();

    let (res1, recompiled1) = get_or_compile_preview(&mut state, &app, &cgt);
    assert!(res1.is_some() && recompiled1);
    let (res2, recompiled2) = get_or_compile_preview(&mut state, &app, &cgt);
    assert!(res2.is_some() && !recompiled2, "identical preview frame must hit memoization cache");

    let mut cgt_alt = cgt;
    cgt_alt.selected_option_id = String::from("xy.scatter_line");
    let (_, recompiled3) = get_or_compile_preview(&mut state, &app, &cgt_alt);
    assert!(recompiled3, "changing option must invalidate preview cache");

    // Bezier subdivision emits intermediate points.
    let path = Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(0.0, 0.0)),
            PathCmd::QuadTo { control: PointPt::new(5.0, 10.0), to: PointPt::new(10.0, 0.0) },
            PathCmd::CubicTo { c1: PointPt::new(12.0, 8.0), c2: PointPt::new(18.0, 8.0), to: PointPt::new(20.0, 0.0) },
        ],
    };
    let subpaths = flatten_path_subpaths(&path, |p| Pos2::new(p.x as f32, p.y as f32));
    assert_eq!(subpaths.len(), 1);
    assert!(subpaths[0].0.len() >= 19);

    // Concave C-shape polygon ear-clipping must not place any triangle centroid inside the notch (x in 4..10, y in 3..7).
    let concave = vec![
        Pos2::new(0.0, 0.0),
        Pos2::new(10.0, 0.0),
        Pos2::new(10.0, 3.0),
        Pos2::new(4.0, 3.0),
        Pos2::new(4.0, 7.0),
        Pos2::new(10.0, 7.0),
        Pos2::new(10.0, 10.0),
        Pos2::new(0.0, 10.0),
    ];
    let (verts, tris) = triangulate_polygon_ear_clip(&concave);
    assert_eq!(tris.len(), 6);
    for [i0, i1, i2] in tris {
        let a = verts[i0 as usize];
        let b = verts[i1 as usize];
        let c = verts[i2 as usize];
        let cx = (a.x + b.x + c.x) / 3.0;
        let cy = (a.y + b.y + c.y) / 3.0;
        let inside_notch = cx > 4.05 && cx < 9.95 && cy > 3.05 && cy < 6.95;
        assert!(!inside_notch, "triangle centroid ({cx}, {cy}) fell inside concave notch");
    }

    // BubblePlotInspector: empty columns no-op, role-adaptive cycling.
    let mut bp = BubblePlotInspectorState::default();
    assert!(!cycle_bubble_color_column(&mut bp));
    bp.available_columns = vec![
        VariableCandidate {
            col_index: 0,
            title: String::from("Dose"),
            role: VariableRole::Continuous,
            role_inferred: false,
            selected: true,
            is_response: false,
        },
        VariableCandidate {
            col_index: 1,
            title: String::from("Group"),
            role: VariableRole::Categorical,
            role_inferred: false,
            selected: true,
            is_response: false,
        },
    ];
    assert!(cycle_bubble_color_column(&mut bp));
    assert!(matches!(bp.color_channel, ColorChannelMapping::Continuous { column: 0, .. }));
    assert!(cycle_bubble_color_column(&mut bp));
    assert!(matches!(bp.color_channel, ColorChannelMapping::Categorical { column: 1, .. }));
}

#[test]
fn aud_13_16_and_17_simple_logistic_passing_bablok_and_deming_overlays_compatible() {
    let (mut app, ds_id, g_id) = build_xy_app_with_two_series();
    let make_rs = |id: &str, kind: AnalysisKind| -> ResultSheet {
        let mut spec = AnalysisSpec::new(id, ds_id.clone(), kind).expect("spec");
        spec.id = SheetId::new(id);
        let mut rs = ResultSheet::new(spec).expect("rs");
        rs.id = SheetId::new(id);
        rs
    };

    let rs_lin = make_rs("r-lin", AnalysisKind::LinearRegression(LinearRegressionSpec::default()));
    let dem_spec = LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() };
    let rs_dem = make_rs("r-dem", AnalysisKind::LinearRegression(dem_spec));
    let rs_nl = make_rs("r-nl", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten")));
    let rs_sp = make_rs("r-sp", AnalysisKind::FitSplineLowess(SplineLowessSpec::default()));
    let rs_log = make_rs("r-log", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default()));
    let rs_pb = make_rs("r-pb", AnalysisKind::PassingBablok(PassingBablokSpec::default()));
    let rs_anova = make_rs("r-anova", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));

    assert!(is_xy_overlay_compatible_result(&rs_lin));
    assert!(is_xy_overlay_compatible_result(&rs_dem));
    assert!(is_xy_overlay_compatible_result(&rs_nl));
    assert!(is_xy_overlay_compatible_result(&rs_sp));
    assert!(is_xy_overlay_compatible_result(&rs_log));
    assert!(is_xy_overlay_compatible_result(&rs_pb));
    assert!(!is_xy_overlay_compatible_result(&rs_anova));

    // Verify end-to-end Deming Model II analysis emits FittedCurveOverlay and attaches via format_graph_commit_intents.
    let dem_kind = AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() });
    let out = app
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": ds_id.as_str(),
                "kind": dem_kind,
            }),
        )
        .expect("deming run");
    let dem_id = SheetId::new(out.payload.get("sheet_id").and_then(serde_json::Value::as_str).expect("sheet_id"));
    let dem_rs = app.document().result_sheet(&dem_id).expect("deming rs");
    assert!(!dem_rs.fitted_curves.is_empty(), "Wave 1 default Deming must emit FittedCurveOverlay");
    assert!(is_xy_overlay_compatible_result(dem_rs));

    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.format_graph.cached_regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: dem_id.clone(),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    app.canvas_dialogs.format_graph.pending_overlay_result_id = Some(Some(dem_id));
    app.canvas_dialogs.format_graph.show_fitted_curve = true;
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id };

    let intents = format_graph_commit_intents(&app).expect("deming attach intents");
    assert!(intents.iter().any(|i| i.command_id == "graph.setRegressionOverlay"));
}

#[test]
fn aud_10_32_single_update_state_emitted_per_modal_frame() {
    let (mut app, _, g_id) = build_xy_app_with_two_series();
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id };

    let ctx = Context::default();
    let mut state = GraphModalEguiState::default();
    let (_, intents) = run_modal_frame(&ctx, &app, &mut state, Vec::new());
    let fg_updates = intents.intents().iter().filter(|i| matches!(i, UiIntent::UpdateFormatGraphState(_))).count();
    assert!(fg_updates <= 1);
}
