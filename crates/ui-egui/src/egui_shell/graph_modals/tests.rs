//! Unit tests for `graph_modals` covering AUD-10/AUD-13 `pub(super)` contracts.

use curvebook_chart::{Path2D, PathCmd, PointPt};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, Cell, ColorChannelMapping, CropInsetsPt, DataSheet, DiagnosticPlotKind, FramePreset, GraphAxisScale,
    GraphKind, GraphSheet, HeatmapColumnLabelAngle, JournalWidthPreset, LayoutItem, LinearRegressionSpec, MultipleVariablesTable,
    MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, OneWayAnovaSpec, PageOrientation, PassingBablokSpec, ProjectDocument, RectPt,
    ResultSheet, SheetId, SimpleLogisticSpec, SizeChannelMapping, SplineLowessSpec, SubcolumnMode, TableArchetype, TickLabelFormat, VariableRole,
    XyTable,
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

fn build_xy_app() -> (UiApp, SheetId, SheetId) {
    let mut doc = ProjectDocument::new("test").expect("doc");
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4).expect("xy");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 2) as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(((r + 1) * 3) as f64);
    }
    let (ds_id, g_id) = (SheetId::new("ds-xy"), SheetId::new("g-xy"));
    let ds = DataSheet::new(ds_id.clone(), "XY", TableArchetype::Xy(xy)).expect("ds");
    let mut gs = GraphSheet::new_uninitialized_for_data(g_id.clone(), &ds).expect("gs");
    gs.initialized = true;
    doc.add_data_sheet(ds).expect("ds");
    doc.add_graph_sheet(gs).expect("gs");
    doc.active_sheet_id = Some(g_id.clone());
    (UiApp::from_session(Session::from_document(doc)), ds_id, g_id)
}

fn mv_enc(color: ColorChannelMapping, xy_col: usize) -> MultivariateGraphEncoding {
    MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: xy_col,
        y_column: xy_col,
        color,
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    }
}

fn run_frame(ctx: &Context, app: &UiApp, state: &mut GraphModalEguiState, events: Vec<Event>) -> (FrameInspectMeta, PendingUiIntents) {
    let (mut meta, mut intents) = (FrameInspectMeta::default(), PendingUiIntents::default());
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1024.0, 720.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        meta.begin_pass(ui.ctx());
        let _ = render_graph_modal(ui.ctx(), app, state, &tokens, &mut meta, &mut intents);
    });
    out.textures_delta.clear();
    (meta, intents)
}

#[test]
fn aud_10_pure_math_locale_image_and_geometry_invariants() {
    for (s, t, e) in [
        (FormatGraphSchema::Heatmap, FormatGraphTab::Appearance, FormatGraphTab::ColorMapping),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::ColorMapping, FormatGraphTab::AxisVariables),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::DataObjects, FormatGraphTab::Appearance),
    ] {
        assert_eq!(s.clamp_tab(t), e);
    }
    assert_eq!(FormatAxesTab::ALL.len(), 5);
    assert!(!GraphModalEguiState::default().categorical_guard_confirmed && !AdditionalTickDraft::default().open);

    for (lo, hi, exp) in [
        (40.0, 50.0, (40.0, 40.0)),
        (40.0, 20.0, (40.0, 40.0)),
        (5.0, 20.0, (5.0, 20.0)),
        (5.0, 99.0, (5.0, 36.0)),
        (f64::NAN, 12.0, (2.5, 12.0)),
        (10.0, f64::INFINITY, (10.0, 36.0)),
    ] {
        assert_eq!(clamp_mv_max_radius_pt(lo, hi), exp);
    }

    assert!((convert_committed_length_to_pt(8.0, ScrubUnitKind::Points, 1.0, 24.0) - 8.0).abs() < 1e-9);
    assert!((convert_committed_length_to_pt(10.0, ScrubUnitKind::Millimeters, 1.0, 100.0) - 10.0 * (72.0 / 25.4)).abs() < 1e-6);

    let mut fi = FormatImageDialogState {
        pixel_size: None,
        bounds_pt: RectPt::new(0.0, 0.0, 200.0, 100.0),
        lock_aspect_ratio: true,
        crop: CropInsetsPt { left_pt: 60.0, right_pt: 60.0, top_pt: 40.0, bottom_pt: 40.0 },
        ..Default::default()
    };
    apply_format_image_width(&mut fi, 300.0);
    assert!((fi.bounds_pt.width - 300.0).abs() < 1e-6 && (fi.bounds_pt.height - 150.0).abs() < 1e-6);
    apply_format_image_width(&mut fi, 100.0);
    assert!(fi.crop.left_pt + fi.crop.right_pt <= fi.bounds_pt.width - 2.0 + 1e-9);
    assert!(fi.crop.top_pt + fi.crop.bottom_pt <= fi.bounds_pt.height - 2.0 + 1e-9);
    apply_format_image_height(&mut fi, 100.0);
    assert!((fi.bounds_pt.width - 200.0).abs() < 1e-6);
    let mut bad_crop = CropInsetsPt { left_pt: 80.0, right_pt: 40.0, top_pt: 0.0, bottom_pt: 0.0 };
    clamp_image_crop_to_bounds(&mut bad_crop, 62.0, 62.0);
    assert!((bad_crop.left_pt - 40.0).abs() < 1e-6 && (bad_crop.right_pt - 20.0).abs() < 1e-6);

    for loc in ["de", "es", "fr", "pt-BR", "it", "nl", "pl", "ru", "tr"] {
        let p = LocalePreset::for_locale(loc);
        assert!(p.decimal_separator == ',' && format_drag_value_localized(7.5, &p) == "7,5");
        assert_eq!(parse_drag_value_localized("7,5 pt", " pt", &p), Some(7.5));
        assert_eq!(parse_drag_value_localized("125,25%", "%", &p), Some(125.25));
    }
    let en = LocalePreset::for_locale("en");
    assert!(format_drag_value_localized(7.5, &en) == "7.5" && parse_drag_value_localized("7.5 pt", " pt", &en) == Some(7.5));
}

#[test]
fn aud_10_53_54_layout_preview_triangulation_and_bubble_invariants() {
    let callout =
        LayoutItem::TextCallout { id: "callout-2".into(), markdown: "x".into(), bounds_pt: RectPt::new(0.0, 0.0, 50.0, 20.0), font_size_pt: 10.0 };
    assert_eq!(next_unique_callout_id(&[callout]), "callout-3");
    let land = oriented_page_size_pt(JournalWidthPreset::A4Portrait, PageOrientation::Landscape);
    assert!(land.width > land.height);
    let mut pl = PageLayoutComposerState {
        preset: JournalWidthPreset::A4Portrait,
        orientation: PageOrientation::Landscape,
        rows: 1,
        cols: 2,
        ..Default::default()
    };
    arrange_layout_grid_oriented(&mut pl, &[SheetId::new("g-1"), SheetId::new("g-2")], 24.0, 12.0);
    assert!(matches!(&pl.items[1], LayoutItem::LiveGraph { bounds_pt, .. } if bounds_pt.x + bounds_pt.width > 650.0));

    let mut st = GraphModalEguiState {
        additional_tick_draw_gridline: false,
        page_callout_draft: "n".into(),
        graph_type_family: GraphTypeFamily::Xy,
        graph_type_show_all: true,
        ..Default::default()
    };
    st.additional_ticks.value_draft = "12".into();
    st.additional_ticks.shaded_zone = true;
    reset_graph_modal_on_close(&mut st);
    assert!(st.additional_ticks.value_draft.is_empty() && !st.additional_ticks.shaded_zone && st.additional_tick_draw_gridline);
    assert!(st.page_callout_draft.is_empty() && st.graph_type_family == GraphTypeFamily::Recommended && !st.graph_type_show_all);

    let (mut app, _, g_id) = build_xy_app();
    let doc = app.document().clone();
    app.canvas_dialogs.change_graph_type.open_for_graph(doc.graph_sheet(&g_id).expect("gs"), &doc);
    let mut cgt = app.canvas_dialogs.change_graph_type.clone();
    assert!(matches!(get_or_compile_preview(&mut st, &app, &cgt), (Some(_), true)));
    assert!(matches!(get_or_compile_preview(&mut st, &app, &cgt), (Some(_), false)));
    cgt.selected_option_id = "xy.scatter_line".into();
    assert!(get_or_compile_preview(&mut st, &app, &cgt).1);

    let path = Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(0.0, 0.0)),
            PathCmd::QuadTo { control: PointPt::new(5.0, 10.0), to: PointPt::new(10.0, 0.0) },
            PathCmd::CubicTo { c1: PointPt::new(12.0, 8.0), c2: PointPt::new(18.0, 8.0), to: PointPt::new(20.0, 0.0) },
        ],
    };
    let sub = flatten_path_subpaths(&path, |p| Pos2::new(p.x as f32, p.y as f32));
    assert!(sub.len() == 1 && sub[0].0.len() >= 19);

    let concave =
        [(0.0, 0.0), (10.0, 0.0), (10.0, 3.0), (4.0, 3.0), (4.0, 7.0), (10.0, 7.0), (10.0, 10.0), (0.0, 10.0)].map(|(x, y)| Pos2::new(x, y)).to_vec();
    let (verts, tris) = triangulate_polygon_ear_clip(&concave);
    assert_eq!(tris.len(), 6);
    for [a, b, c] in tris {
        let cx = (verts[a as usize].x + verts[b as usize].x + verts[c as usize].x) / 3.0;
        let cy = (verts[a as usize].y + verts[b as usize].y + verts[c as usize].y) / 3.0;
        assert!(!(cx > 4.05 && cx < 9.95 && cy > 3.05 && cy < 6.95));
    }

    let mut bp = BubblePlotInspectorState::default();
    assert!(!cycle_bubble_color_column(&mut bp));
    bp.available_columns = [(0, "Dose", VariableRole::Continuous), (1, "Group", VariableRole::Categorical)]
        .into_iter()
        .map(|(col_index, t, role)| VariableCandidate { col_index, title: t.into(), role, role_inferred: false, selected: true, is_response: false })
        .collect();
    assert!(cycle_bubble_color_column(&mut bp) && matches!(bp.color_channel, ColorChannelMapping::Continuous { column: 0, .. }));
    assert!(cycle_bubble_color_column(&mut bp) && matches!(bp.color_channel, ColorChannelMapping::Categorical { column: 1, .. }));
}

#[test]
fn aud_10_12_31_38_39_heatmap_and_mv_sync_invariants() {
    let (base, ds_id, _) = build_xy_app();
    let ds = base.document().data_sheet(&ds_id).expect("ds").clone();
    let g_id = SheetId::new("g-heat");
    let mut gs = GraphSheet::new_uninitialized_for_data(g_id.clone(), &ds).expect("gs");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    gs.spec.series.clear();
    gs.spec.heatmap_labels.column_angle = HeatmapColumnLabelAngle::Angled45;
    gs.spec.x_axis.tick_angle_deg = 0.0;
    gs.spec.multivariate = Some(mv_enc(
        ColorChannelMapping::Continuous {
            column: 0,
            ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: false },
            domain_clamp: Some((0.0, 10.0)),
        },
        0,
    ));
    let mut doc = base.document().clone();
    doc.add_graph_sheet(gs.clone()).expect("hm");
    let mut app = UiApp::from_session(Session::from_document(doc.clone()));
    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };
    let mut st = GraphModalEguiState::default();
    sync_format_graph_local_drafts(&app, &mut st, &g_id);
    assert!(st.heatmap_colormap == ColormapId::Plasma && (st.heatmap_tick_angle_deg - 45.0).abs() < 1e-6);

    st.heatmap_colormap = ColormapId::DivergingBlueRed;
    st.heatmap_color_dirty = true;
    let new_enc = mv_enc(
        ColorChannelMapping::Continuous {
            column: 0,
            ramp: ColorRamp::Named { id: ColormapId::Cividis, reversed: true },
            domain_clamp: Some((1.0, 5.0)),
        },
        0,
    );
    app.execute_command("graph.setSpec", json!({ "sheet_id": g_id.as_str(), "multivariate": new_enc })).expect("setSpec");
    sync_format_graph_local_drafts(&app, &mut st, &g_id);
    assert!(st.heatmap_colormap == ColormapId::Cividis && st.heatmap_reversed && !st.heatmap_color_dirty);

    app.egui_views.graph_modals = st;
    let clean = format_graph_commit_intents(&app).expect("clean");
    assert!(clean.len() == 1 && clean[0].command_id == "graph.setSpec");
    app.egui_views.graph_modals.heatmap_colormap = ColormapId::Viridis;
    app.egui_views.graph_modals.heatmap_color_dirty = true;
    app.egui_views.graph_modals.heatmap_tick_angle_deg = 90.0;
    app.egui_views.graph_modals.heatmap_tick_angle_dirty = true;
    let dirty = format_graph_commit_intents(&app).expect("dirty");
    assert!(dirty.iter().filter(|i| i.command_id == "graph.setSpec").count() == 1 && dirty[0].params.get("multivariate").is_some());
    assert_eq!(dirty.iter().filter(|i| i.command_id == "graph.setAxes").count(), 1);
    app.egui_views.graph_modals.heatmap_auto_range = false;
    app.egui_views.graph_modals.heatmap_min_draft = "10".into();
    app.egui_views.graph_modals.heatmap_max_draft = "5".into();
    assert!(format_graph_commit_intents(&app).is_err());
    on_format_graph_committed(&mut app, true);
    assert!(!app.egui_views.graph_modals.format_graph_session_synced && !app.egui_views.graph_modals.heatmap_color_dirty);

    // AUD-10-38 & AUD-10-39: MV categorical clamp guard, continuous validation, level refresh, palette switch.
    let mut mv_doc = ProjectDocument::new("mv").expect("doc");
    let mut mv = MultipleVariablesTable::new(3, 3).expect("mv");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].cells = vec![Cell::text("Ctrl"), Cell::text("Drug"), Cell::text("Ctrl")];
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[1].cells = vec![Cell::text("F"), Cell::text("M"), Cell::text("F")];
    mv.variables[2].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)];
    let mv_ds = DataSheet::new(SheetId::new("ds-mv"), "MV", TableArchetype::MultipleVariables(mv)).expect("ds");
    let mut mv_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv"), &mv_ds).expect("gs");
    mv_gs.spec.kind = GraphKind::MultipleVariables;
    mv_gs.spec.multivariate =
        Some(mv_enc(ColorChannelMapping::Categorical { column: 0, palette: PaletteId::OkabeIto, level_order_override: None }, 2));
    mv_doc.add_data_sheet(mv_ds).expect("ds");
    mv_doc.add_graph_sheet(mv_gs.clone()).expect("gs");
    let mv_app = UiApp::from_session(Session::from_document(mv_doc));
    let mut fg = crate::dialogs_canvas::FormatGraphState::default();
    fg.sync_from_graph_with_doc(&mv_gs, Some(mv_app.document()));
    assert_eq!(fg.mv_levels, vec!["Ctrl", "Drug"]);

    let mut mst = GraphModalEguiState { mv_clamp_min_draft: "0".into(), mv_clamp_max_draft: "10".into(), ..Default::default() };
    let mut enc = fg.mv_encoding.clone().expect("enc");
    assert!(!apply_mv_color_clamp_drafts(&mut mst, &mut enc, &mv_app.locale_preset) && matches!(enc.color, ColorChannelMapping::Categorical { .. }));
    enc.color = ColorChannelMapping::Continuous { column: 2, ramp: ColorRamp::default(), domain_clamp: None };
    mst.mv_clamp_min_draft = "5".into();
    mst.mv_clamp_max_draft = "5".into();
    assert!(!apply_mv_color_clamp_drafts(&mut mst, &mut enc, &mv_app.locale_preset) && mst.mv_clamp_error.is_some());
    mst.mv_clamp_min_draft = "1".into();
    mst.mv_clamp_max_draft = "9".into();
    assert!(apply_mv_color_clamp_drafts(&mut mst, &mut enc, &mv_app.locale_preset) && mst.mv_clamp_error.is_none());
    assert!(matches!(enc.color, ColorChannelMapping::Continuous { domain_clamp: Some((1.0, 9.0)), .. }));

    if let Some(ref mut e) = fg.mv_encoding {
        e.color = ColorChannelMapping::Categorical { column: 1, palette: PaletteId::OkabeIto, level_order_override: None };
    }
    refresh_mv_levels_from_doc(&mut fg, mv_app.document());
    assert!(fg.mv_levels == vec!["F", "M"] && !fg.mv_level_is_x && fg.mv_selected_level.as_deref() == Some("F"));
    fg.mv_levels_dirty = false;
    apply_palette_selection(&mut fg, PaletteId::PaulTolVibrant);
    assert!(
        fg.mv_levels_dirty
            && matches!(fg.mv_encoding.as_ref().map(|e| &e.color), Some(ColorChannelMapping::Categorical { palette: PaletteId::PaulTolVibrant, .. }))
    );
}

#[test]
fn aud_10_13_35_37_40_41_and_13_16_17_axes_series_and_overlay_invariants() {
    let (mut app, ds_id, g_id) = build_xy_app();
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();

    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::Y2);
    app.canvas_dialogs.format_axes.enable_y2 = true;
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id.clone() };
    app.egui_views.graph_modals.format_axes_frame_preset = FramePreset::PlainBoxed;
    app.egui_views.graph_modals.format_axes_frame_preset_dirty = true;
    let ax = format_axes_commit_intent(&app).expect("valid").expect("intent");
    assert!(ax.command_id == "graph.setAxes" && ax.params.get("frame_preset") == Some(&json!(FramePreset::PlainBoxed)));
    app.egui_views.graph_modals.axis_bounds_error = Some("bad".into());
    assert!(format_axes_commit_intent(&app).is_err());
    app.egui_views.graph_modals.axis_bounds_error = None;
    app.egui_views.graph_modals.format_axes_session_synced = true;
    on_format_axes_committed(&mut app.egui_views.graph_modals, false);
    assert!(!app.egui_views.graph_modals.format_axes_frame_preset_dirty && app.egui_views.graph_modals.format_axes_session_synced);
    on_format_axes_committed(&mut app.egui_views.graph_modals, true);
    assert!(!app.egui_views.graph_modals.format_axes_session_synced);

    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::Y);
    assert!(app.canvas_dialogs.format_axes.add_segment());
    app.canvas_dialogs.format_axes.axis_draft.segments[1] = AxisSegment {
        min: Some(100.0),
        max: Some(500.0),
        scale: GraphAxisScale::Log10,
        length_share: 2.0,
        major_tick_interval: Some(50.0),
        minor_ticks_per_interval: 4,
        ..AxisSegment::default()
    };
    let mut seg = GraphModalEguiState::default();
    sync_format_axes_local_drafts(&app, &mut seg);
    seg.axis_major_interval_error = Some("bad".into());
    seg.axis_minor_ticks_error = None;
    assert!(seg.has_any_axis_errors());
    switch_active_segment(&mut seg, &app.canvas_dialogs.format_axes, 1, &app.locale_preset);
    assert!(seg.format_axes_selected_segment == 1 && seg.axis_min_draft == "100" && seg.axis_max_draft == "500" && seg.axis_minor_ticks_draft == 4);
    assert!(seg.axis_major_interval_error.is_none() && seg.has_any_axis_errors());
    switch_active_segment(&mut seg, &app.canvas_dialogs.format_axes, 0, &app.locale_preset);
    assert!(seg.axis_major_interval_error.is_some());

    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };
    assert_eq!(format_graph_jump_params(&app), json!({ "sheet_id": g_id.as_str() }));
    let mut st = GraphModalEguiState::default();
    let mut draft = app.canvas_dialogs.format_graph.clone();
    draft.marker_size_pt = 11.0;
    draft.line_dash = LineDashStyle::Dashed;
    draft.bind_to_right_y2 = true;
    draft.bind_to_right_y2_dirty = true;
    switch_format_graph_series(&mut st, &mut draft, 1, &doc);
    assert!(draft.selected_series_index == 1 && st.staged_series_edits.contains_key(&0));
    draft.marker_size_pt = 7.5;
    draft.offset_x_data = 0.25;
    draft.offset_x_data_dirty = true;
    app.canvas_dialogs.format_graph = draft.clone();
    app.egui_views.graph_modals = st.clone();
    let sc: Vec<_> = format_graph_commit_intents(&app).expect("multi").into_iter().filter(|i| i.command_id == "graph.setSeries").collect();
    assert!(sc.len() == 2 && sc.iter().any(|c| c.params["series_index"] == json!(0)) && sc.iter().any(|c| c.params["series_index"] == json!(1)));
    switch_format_graph_series(&mut st, &mut draft, 0, &doc);
    assert!((draft.marker_size_pt - 11.0).abs() < 1e-6 && draft.line_dash == LineDashStyle::Dashed && draft.bind_to_right_y2);

    for (id, kind, ok) in [
        ("r-lin", AnalysisKind::LinearRegression(LinearRegressionSpec::default()), true),
        ("r-dem", AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..Default::default() }), true),
        ("r-nl", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten")), true),
        ("r-sp", AnalysisKind::FitSplineLowess(SplineLowessSpec::default()), true),
        ("r-log", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default()), true),
        ("r-pb", AnalysisKind::PassingBablok(PassingBablokSpec::default()), true),
        ("r-av", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()), false),
    ] {
        let mut spec = AnalysisSpec::new(id, ds_id.clone(), kind).expect("spec");
        spec.id = SheetId::new(id);
        let mut rs = ResultSheet::new(spec).expect("rs");
        rs.id = SheetId::new(id);
        assert_eq!(is_xy_overlay_compatible_result(&rs), ok, "{id}");
    }
}

#[test]
fn aud_10_30_32_33_34_frame_render_footer_and_escape_invariants() {
    let (mut app, _, g_id) = build_xy_app();
    let doc = app.document().clone();
    let gs = doc.graph_sheet(&g_id).expect("gs").clone();
    let ctx = Context::default();

    app.canvas_dialogs.insert_model_equation.result_sheet_id = None;
    app.canvas_dialogs.active_dialog = ActiveDialog::InsertModelEquation { graph_sheet_id: g_id.clone() };
    assert!(!evaluate_graph_modal_button(&app, &GraphModalEguiState::default(), "ok").0);

    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id.clone() };
    let ax_err = GraphModalEguiState { axis_major_interval_error: Some("bad".into()), ..Default::default() };
    assert!(graph_modal_local_validation_error(&app, &ax_err).is_some());
    app.egui_views.graph_modals = ax_err;
    assert!(format_axes_commit_intent(&app).is_err());

    app.canvas_dialogs.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: g_id.clone() };
    let fg_err = GraphModalEguiState { mv_clamp_error: Some("bad".into()), ..Default::default() };
    assert!(graph_modal_local_validation_error(&app, &fg_err).is_some());
    app.egui_views.graph_modals = fg_err;
    assert!(format_graph_commit_intents(&app).is_err());

    app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc));
    let mut fg_st = GraphModalEguiState::default();
    let (fg_meta, fg_intents) = run_frame(&ctx, &app, &mut fg_st, Vec::new());
    assert!(fg_intents.intents().iter().filter(|i| matches!(i, UiIntent::UpdateFormatGraphState(_))).count() <= 1);
    assert!(!fg_meta.find_by_widget_id("format_graph.show_fitted_curve").expect("chk").enabled);

    app.canvas_dialogs.change_graph_type.open_for_graph(&gs, &doc);
    app.canvas_dialogs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: g_id.clone() };
    let mut cgt_st = GraphModalEguiState::default();
    let _ = run_frame(&ctx, &app, &mut cgt_st, Vec::new());
    let (cgt_meta, _) = run_frame(&ctx, &app, &mut cgt_st, Vec::new());
    assert!(cgt_meta.find_by_widget_id("modal.apply").is_some() && cgt_meta.find_by_widget_id("graph_type.apply").is_none());

    let esc = || vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }];
    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::X);
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g_id };
    let mut st = GraphModalEguiState { format_axes_tab: FormatAxesTab::TitlesAndTypography, ..Default::default() };
    let _ = run_frame(&ctx, &app, &mut st, Vec::new());
    st.axis_stroke_spinner.editing = true;
    st.axis_stroke_spinner.draft_text = "4.5".into();
    assert!(run_frame(&ctx, &app, &mut st, esc()).1.is_empty() && !st.axis_stroke_spinner.editing && st.axis_stroke_spinner.draft_text.is_empty());
    st.format_axes_tab = FormatAxesTab::XAxis;
    st.categorical_guard_confirmed = true;
    st.categorical_guard_pending_axis = Some("X".into());
    st.format_axes_guard_pending_scale = Some(GraphAxisScale::Log10);
    st.format_axes_guard_pending_tick_format = Some(TickLabelFormat::Decimal);
    assert!(run_frame(&ctx, &app, &mut st, esc()).1.is_empty() && !st.has_pending_categorical_guard() && !st.categorical_guard_confirmed);
    assert!(
        run_frame(&ctx, &app, &mut st, esc()).1.intents().iter().any(|i| matches!(i, UiIntent::DialogButtonAction { action } if action == "cancel"))
    );

    st.eq_font_spinner.editing = true;
    st.eq_font_spinner.draft_text = "36".into();
    st.pts_size_spinner.dragging = true;
    st.bar_border_spinner.editing = true;
    st.series_offset_x_spinner.dragging = true;
    reset_graph_modal_on_close(&mut st);
    assert!(
        !st.any_spinner_editing()
            && st.eq_font_spinner.draft_text.is_empty()
            && !st.pts_size_spinner.dragging
            && !st.series_offset_x_spinner.dragging
    );
}
