#![forbid(unsafe_code)]

//! `R13.8` Phase B2 Part 1 (`R13.2`, `R13.3`, `R13.5`) desktop `egui 0.37` verification tests.
//!
//! Exercises every desktop view surface added for `R13.2`, `R13.3`, and `R13.5` through real
//! `egui` pointer/keyboard events via [`EguiHumanPathDriver`], plus `gui-layout-spec.md` §9.4
//! layout gates across all `14` active Beta locales, `Light`/`Dark`/`HighContrast` themes,
//! `640x420` & `1440x600` viewports, and `1.0x`/`2.0x` DPI scales.

use std::error::Error;

use curvebook_doc::{
    Cell, DataSheet, ErrorBarMetric, FramePreset, GraphicalDataObject, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapLabelSpec,
    HeatmapRowLabelMode, LayoutItem, ProjectDocument, RegressionOverlaySpec, ReplicateLineMode, SheetId, SubcolumnMode, SurvivalDisplayMetric,
    SurvivalTable, TableArchetype, TickDirection, TickLabelFormat, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::Rgba8;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::{UiApp, UiThemeMode, dispatch_gui_action};
use serde_json::json;

type TestRes = Result<(), Box<dyn Error>>;

struct TestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    outside_click_captured: bool,
    outside_wheel_captured: bool,
    paper_rect_pt: Option<egui::Rect>,
}

impl TestState {
    fn new(app: UiApp) -> Self {
        let mut views = app.egui_views.clone();
        views.canvas_view.synchronous_raster = true;
        Self { app, views, intents: PendingUiIntents::new(), outside_click_captured: false, outside_wheel_captured: false, paper_rect_pt: None }
    }
}

fn make_driver<'a>(state: TestState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, TestState> {
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        st.views.canvas_view.synchronous_raster = true;
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.outside_click_captured |= rep.outside_click_captured;
        st.outside_wheel_captured |= rep.outside_wheel_captured;
        st.paper_rect_pt = rep.regions.canvas_rect_pt;
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
            st.views.canvas_view.synchronous_raster = true;
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn modal_parity_violations(report: &curvebook_ui_egui::inspect::ParityReport) -> Vec<&curvebook_ui_egui::inspect::ParityViolation> {
    report
        .violations
        .iter()
        .filter(|v| v.widget_id != "canvas.viewport" && v.widget_id != "canvas.split.viewport" && !v.widget_id.starts_with("canvas.layout_panel."))
        .collect()
}

fn build_seeded_project() -> Result<UiApp, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("R13.8 Part 1 Suite")?;
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4)?;
    xy.row_titles = vec![String::from("Baseline"), String::from("Mid"), String::from("Peak"), String::from("Late")];
    xy.row_titles_expanded = true;
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[1].title = String::from("Treated");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64) * 5.0);
        xy.data_sets[0].cells[r][1] = Cell::number(11.0 + (r as f64) * 4.5);
        xy.data_sets[1].cells[r][0] = Cell::number(14.0 + (r as f64) * 6.0);
        xy.data_sets[1].cells[r][1] = Cell::number(15.5 + (r as f64) * 5.5);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "Assay Kinetics", TableArchetype::Xy(xy))?)?;

    let mut surv = SurvivalTable::new(2, 4)?;
    surv.cohorts[0].title = String::from("Control");
    surv.cohorts[1].title = String::from("Drug");
    for (r, (t, e0, e1)) in [(5.0, 1.0, 0.0), (10.0, 1.0, 1.0), (15.0, 0.0, 0.0), (20.0, 1.0, 1.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(e0);
        surv.cohorts[1].cells[r][0] = Cell::number(e1);
    }
    doc.add_data_sheet(DataSheet::new("surv-1", "Cohort Survival", TableArchetype::Survival(surv))?)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;

    app.execute_command("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy", "title": "Primary XY" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-xy-2", "data_sheet_id": "xy-1", "kind": "xy", "title": "Secondary XY" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-surv", "data_sheet_id": "surv-1", "kind": "survival", "title": "Survival KM" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-heat", "data_sheet_id": "xy-1", "kind": "heatmap", "title": "Assay Heatmap" }))?;

    let reg_res = app.execute_command("analysis.run", json!({ "source_sheet_id": "xy-1", "kind": "linear_regression", "sheet_id": "res-lin" }))?;
    let res_id = reg_res.payload["sheet_id"].as_str().unwrap_or("res-lin").to_owned();
    app.execute_command(
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "series_index": 0,
            "regression_overlay": RegressionOverlaySpec {
                result_sheet_id: SheetId::new(res_id),
                sample_segments: 256,
                show_confidence_band: false,
                show_prediction_band: false,
                shaded_ribbon: true,
            },
        }),
    )?;

    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }))?;
    app.execute_command("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy", "panel_id": "p-a" }))?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy-2", "panel_id": "p-b", "scale_percent": 70.0 }),
    )?;

    app.sync_views();
    Ok(app)
}

fn series_alpha(gdo: &GraphicalDataObject) -> u8 {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => marker.map(|m| m.fill_color.a).or_else(|| line.map(|l| l.color.a)).unwrap_or(255),
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => fill_color.a,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color.a,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => line_color.a,
        GraphicalDataObject::PartsOfWhole { .. } => 255,
    }
}

#[test]
fn r13_2_format_graph_opacity_overlays_single_undo_survival_row_titles_and_spaghetti() -> TestRes {
    let mut app = build_seeded_project()?;

    // 1. Format Graph: Opacity 50%, hide raw data points, hide fitted curve, apply to all data sets (single undo).
    let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open", "params": { "sheet_id": "g-xy" } }))?;
    app.sync_views();
    let mut driver = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    driver.click_widget_id("format_graph.alpha.50")?;
    driver.click_widget_id("format_graph.show_raw_data")?;
    driver.click_widget_id("format_graph.show_fitted_curve")?;
    driver.click_widget_id("format_graph.apply_globally")?;
    driver.click_widget_id("modal.ok")?;

    let mut app = driver.state().app.clone();
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
        assert!(!gs.spec.series[0].show_raw_data);
        assert!(!gs.spec.series[1].show_raw_data);
        assert!(gs.spec.series[0].regression_overlay.is_none());
        let a0 = series_alpha(&gs.spec.series[0].gdo);
        let a1 = series_alpha(&gs.spec.series[1].gdo);
        assert!((120..=135).contains(&a0), "expected ~50% alpha on series 0, got {a0}");
        assert!((120..=135).contains(&a1), "expected ~50% alpha on series 1, got {a1}");
    }

    // Single undo reverts `graph.setSeries` atomically across ALL series.
    app.execute_command("edit.undo", json!({}))?;
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after undo")?;
        assert!(gs.spec.series[0].show_raw_data, "series 0 must revert in one undo");
        assert!(gs.spec.series[1].show_raw_data, "series 1 must revert in one undo");
        assert!(gs.spec.series[0].regression_overlay.is_some());
        assert_eq!(series_alpha(&gs.spec.series[0].gdo), 255);
        assert_eq!(series_alpha(&gs.spec.series[1].gdo), 255);
    }

    // 2. TrailingStudio live GraphInspector Data & Canvas controls.
    app.execute_command("sheet.select", json!({ "sheet_id": "g-xy" }))?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();
    let mut studio_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    studio_drv.click_widget_id("inspector.graph.alpha.25")?;
    studio_drv.click_widget_id("inspector.graph.show_raw_data")?;
    studio_drv.click_widget_id("inspector.graph.tab.canvas")?;
    studio_drv.click_widget_id("inspector.graph.paper_fill.cream")?;
    let mut app = studio_drv.state().app.clone();
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after studio")?;
        assert!(!gs.spec.series[0].show_raw_data);
        assert!((58..=70).contains(&series_alpha(&gs.spec.series[0].gdo)));
        assert_eq!(gs.spec.paper_fill, Some(Rgba8::rgb(255, 251, 235)));
    }

    // 3. Change Graph Type survival metric / point-to-point / error display.
    let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": "g-surv" } }))?;
    app.sync_views();
    let mut cgt_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    cgt_drv.click_widget_id("graph_type.survival.metric.pct_event")?;
    cgt_drv.click_widget_id("graph_type.survival.point_to_point")?;
    cgt_drv.click_widget_id("graph_type.survival.err.sem")?;
    cgt_drv.click_widget_id("modal.ok")?;
    let mut app = cgt_drv.state().app.clone();
    {
        let gs_surv = app.document().graph_sheet(&SheetId::new("g-surv")).ok_or("missing g-surv")?;
        match gs_surv.spec.series[0].gdo {
            GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, show_greenwood_ci, .. } => {
                assert_eq!(display_metric, SurvivalDisplayMetric::PercentCumulativeEvent);
                assert!(!staircase);
                assert_eq!(error_display, Some(ErrorBarMetric::Sem));
                assert!(show_greenwood_ci);
            }
            ref other => return Err(format!("expected KaplanMeierStep, got {other:?}").into()),
        }
    }

    // 4. View > Row Titles toggle (`ui.view.rowTitles.toggle`) synced immediately to `data_grid`.
    app.execute_command("sheet.select", json!({ "sheet_id": "xy-1" }))?;
    let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.view.rowTitles.toggle" }))?;
    app.sync_views();
    assert_eq!(app.grid.row_titles_collapsed_override, Some(true));
    assert!(app.egui_views.column_grid.row_titles_collapsed);
    let grid_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(grid_drv.state().views.column_grid.row_titles_collapsed);
    assert!(grid_drv.frame_meta().find_by_widget_id("grid.row_titles.toggle").is_some());

    // 5. Spaghetti trajectory hover HUD (`Rep N`).
    let mut app = grid_drv.state().app.clone();
    app.execute_command(
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "series_index": 0,
            "show_raw_data": true,
            "replicate_lines": ReplicateLineMode::IndividualReplicateTrajectories,
        }),
    )?;
    app.execute_command("sheet.select", json!({ "sheet_id": "g-xy" }))?;
    app.sync_views();
    let scene = app.session().render_sheet_scene(&SheetId::new("g-xy"))?;
    let first_pt = scene.plotted_points.first().ok_or("missing plotted point for spaghetti hover")?;
    let mut sp_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    let canvas_rect = sp_drv.frame_meta().find_by_widget_id("canvas.viewport").ok_or("missing canvas.viewport")?.clipped_rect;
    let margin = 16.0_f32;
    let avail_w = (canvas_rect.width() - margin * 2.0).max(80.0);
    let avail_h = (canvas_rect.height() - margin * 2.0).max(80.0);
    let sx = (avail_w / (scene.width_pt.max(1.0) as f32)).min(avail_h / (scene.height_pt.max(1.0) as f32));
    let paper_w = (scene.width_pt as f32) * sx;
    let paper_h = (scene.height_pt as f32) * sx;
    let paper_min = egui::pos2(canvas_rect.center().x - paper_w * 0.5, canvas_rect.center().y - paper_h * 0.5);
    let screen_pt = egui::pos2(paper_min.x + (first_pt.center_pt.x as f32) * sx, paper_min.y + (first_pt.center_pt.y as f32) * sx);
    sp_drv.hover_at(screen_pt)?;
    let readout = sp_drv.state().views.canvas_view.hover_readout.as_ref().ok_or("expected spaghetti hover readout")?;
    assert!(readout.label.contains("Rep "), "expected Rep N in spaghetti label, got `{}`", readout.label);
    assert!(readout.value_summary.contains("Rep "), "expected Rep N in summary, got `{}`", readout.value_summary);

    Ok(())
}

#[test]
fn r13_3_create_graph_gallery_multiselect_apply_style_normalize_and_heatmap_labels() -> TestRes {
    let mut app = build_seeded_project()?;
    let initial_graphs = app.document().graph_sheets.len();

    // 1. Create New Graph dialog (`one_per_data_set` + `y_axis_title`).
    let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.create.open", "params": { "data_sheet_id": "xy-1" } }))?;
    app.sync_views();
    let mut cg_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    cg_drv.click_widget_id("create_graph.kind.xy")?;
    cg_drv.click_widget_id("create_graph.one_per_data_set")?;
    cg_drv.click_widget_id("create_graph.y_axis_title")?;
    cg_drv.type_text("Signal (RFU)")?;
    cg_drv.click_widget_id("modal.ok")?;

    let mut app = cg_drv.state().app.clone();
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(app.document().graph_sheets.len(), initial_graphs + 2);
    let created_last_id = app.document().graph_sheets.last().ok_or("missing created graph")?.id.clone();
    let created_last_y_title = app.document().graph_sheets.last().ok_or("missing created graph")?.spec.y_axis.title.clone();
    assert_eq!(created_last_y_title, "Signal (RFU)");

    // 2. Portfolio Gallery `Cmd/Ctrl+Click` + `Shift+Click` -> batch `ApplyStyleFrom`.
    app.execute_command("graph.setSpec", json!({ "sheet_id": "g-xy", "frame_preset": "PlainBoxed" }))?;
    let _ = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.gallery.toggle", "params": { "open": true, "folder": "graphs" } }),
    )?;
    app.sync_views();
    let mut gal_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    let p_xy2 = gal_drv.frame_meta().find_by_widget_id("gallery.card.g-xy-2").ok_or("missing g-xy-2 card")?.clipped_rect.center();
    gal_drv.click_at_button(p_xy2, PointerButton::Primary, KeyModifiers { ctrl: true, ..Default::default() })?;
    let p_last = gal_drv
        .frame_meta()
        .find_by_widget_id(&format!("gallery.card.{}", created_last_id.as_str()))
        .ok_or("missing last card")?
        .clipped_rect
        .center();
    gal_drv.click_at_button(p_last, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    assert!(gal_drv.state().app.nav.selected_gallery_ids().len() >= 2, "expected multi-selection via Shift+Click");
    gal_drv.click_widget_id("gallery.apply_style_from")?;
    assert!(matches!(gal_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. }));
    gal_drv.click_widget_id("apply_style.donor.0")?;
    gal_drv.click_widget_id("apply_style.target.select_all")?;
    gal_drv.click_widget_id("modal.ok")?;

    let mut app = gal_drv.state().app.clone();
    assert_eq!(app.document().graph_sheet(&SheetId::new("g-xy-2")).ok_or("g-xy-2")?.spec.frame_preset, FramePreset::PlainBoxed);

    // 3. Standalone Normalize Transform modal `create_new_graph` checkbox.
    let graphs_before_norm = app.document().graph_sheets.len();
    let results_before_norm = app.document().result_sheets.len();
    app.execute_command("sheet.select", json!({ "sheet_id": "xy-1" }))?;
    app.sync_views();
    let mut norm_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    norm_drv.state_mut().intents.push_open_transform_modal(SheetId::new("xy-1"), TransformModalKind::Normalize);
    norm_drv.run_settled();
    norm_drv.run_settled();
    norm_drv.click_widget_id("edit.transform.normalize.create_graph")?;
    norm_drv.click_widget_id("modal.ok")?;
    let mut app = norm_drv.state().app.clone();
    assert_eq!(app.document().result_sheets.len(), results_before_norm + 1);
    assert_eq!(app.document().graph_sheets.len(), graphs_before_norm + 1);

    // 4. HeatmapLabelSpec controls in Format Graph.
    let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open", "params": { "sheet_id": "g-heat" } }))?;
    app.sync_views();
    let mut hm_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    hm_drv.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    hm_drv.click_widget_id("format_graph.heatmap.row_labels.numbers")?;
    hm_drv.click_widget_id("format_graph.heatmap.col_labels.letters")?;
    hm_drv.click_widget_id("format_graph.heatmap.angle.90")?;
    hm_drv.click_widget_id("modal.ok")?;
    let heat_gs = hm_drv.state().app.document().graph_sheet(&SheetId::new("g-heat")).ok_or("missing g-heat")?;
    assert_eq!(
        heat_gs.spec.heatmap_labels,
        HeatmapLabelSpec {
            row_labels: HeatmapRowLabelMode::RowNumbers,
            column_labels: HeatmapColumnLabelMode::ColumnLetters,
            column_angle: HeatmapColumnLabelAngle::Vertical90,
        }
    );
    Ok(())
}

#[test]
fn r13_5_axes_ticks_paper_fill_equalize_scaling_studio_zorder_and_context_menu() -> TestRes {
    let mut app = build_seeded_project()?;

    // 1. Format Axes: TickDirection::Inside, show_tick_labels: false, AbsoluteDecimal.
    let _ = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": "g-xy", "target_axis": "y" } }),
    )?;
    app.sync_views();
    let mut ax_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    ax_drv.click_widget_id("edit.tab.axes.y")?;
    ax_drv.click_widget_id("axes.tick_format.abs_decimal")?;
    ax_drv.click_widget_id("axes.tick_dir.inside")?;
    ax_drv.click_widget_id("axes.show_tick_labels")?;
    ax_drv.click_widget_id("modal.ok")?;
    let mut app = ax_drv.state().app.clone();
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
        assert_eq!(gs.spec.y_axis.tick_format, TickLabelFormat::AbsoluteDecimal);
        assert_eq!(gs.spec.y_axis.tick_direction, TickDirection::Inside);
        assert!(!gs.spec.y_axis.show_tick_labels);
    }

    // 2. Format Graph: Page Background Fill (`Transparent` -> None).
    let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open", "params": { "sheet_id": "g-xy" } }))?;
    app.sync_views();
    let mut pf_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    pf_drv.click_widget_id("edit.tab.format_graph.settings")?;
    pf_drv.click_widget_id("format_graph.paper_fill.transparent")?;
    pf_drv.click_widget_id("modal.ok")?;
    let mut app = pf_drv.state().app.clone();
    assert_eq!(app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("g-xy")?.spec.paper_fill, None);

    // 3. LayoutSheet: Trailing Studio `Equalize Scaling...`, Z-order chips, and right-click ZLayer::ContextMenu.
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" }))?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();
    let mut lay_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));

    // Click Studio `Equalize Scaling...` -> opens EqualizeScaling dialog -> choose Explicit 75% -> OK.
    lay_drv.click_widget_id("inspector.layout.equalize_scaling")?;
    assert!(matches!(lay_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::EqualizeScaling { .. }));
    lay_drv.click_widget_id("equalize_scaling.mode.explicit")?;
    lay_drv.click_widget_id("equalize_scaling.percent.75")?;
    lay_drv.click_widget_id("modal.ok")?;
    assert_eq!(lay_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    {
        let sub0 = lay_drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy"))?;
        let sub1 = lay_drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy-2"))?;
        let ls = lay_drv.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?;
        if let [LayoutItem::LiveGraph { bounds_pt: b0, scale_percent: s0, .. }, LayoutItem::LiveGraph { bounds_pt: b1, scale_percent: s1, .. }] =
            ls.spec.items.as_slice()
        {
            let eff0 = (b0.width / sub0.width_pt.max(1.0)).min(b0.height / sub0.height_pt.max(1.0)) * (*s0 / 100.0);
            let eff1 = (b1.width / sub1.width_pt.max(1.0)).min(b1.height / sub1.height_pt.max(1.0)) * (*s1 / 100.0);
            assert!((eff0 - 0.75).abs() < 1e-4 && (eff1 - 0.75).abs() < 1e-4, "expected 0.75 effective scale, got eff0={eff0}, eff1={eff1}");
        } else {
            return Err("expected 2 LiveGraph items on lay-1".into());
        }
    }

    // Studio Z-order chip: Bring panel 0 (`p-a`) to front.
    lay_drv.state_mut().app.egui_views.canvas_view.selected_panel_index = Some(0);
    lay_drv.state_mut().views.canvas_view.selected_panel_index = Some(0);
    lay_drv.run_settled();
    lay_drv.click_widget_id("inspector.layout.zorder.front")?;
    {
        let ls = lay_drv.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?;
        assert_eq!(ls.spec.items[0].id(), "p-b");
        assert_eq!(ls.spec.items[1].id(), "p-a");
    }

    // Right-click canvas.layout_panel.0 -> ZLayer::ContextMenu -> Bring to Front -> `p-b` returns to index 1.
    let p0_center = lay_drv.frame_meta().find_by_widget_id("canvas.layout_panel.0").ok_or("missing canvas.layout_panel.0")?.clipped_rect.center();
    lay_drv.click_at_button(p0_center, PointerButton::Secondary, KeyModifiers::default())?;
    let ctx_front = lay_drv.frame_meta().find_by_widget_id("canvas.layout_ctx.front").ok_or("missing canvas.layout_ctx.front")?;
    assert_eq!(ctx_front.z_layer, curvebook_ui_egui::inspect::ZLayer::ContextMenu);
    lay_drv.click_widget_id("canvas.layout_ctx.front")?;
    let ls = lay_drv.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1 final")?;
    assert_eq!(ls.spec.items[0].id(), "p-a");
    assert_eq!(ls.spec.items[1].id(), "p-b");
    Ok(())
}

#[test]
fn r13_8_p1_14_beta_locales_3_themes_2_sizes_2_dpis_and_modal_capture() -> TestRes {
    let base_app = build_seeded_project()?;
    assert_eq!(BETA_LOCALE_CODES.len(), 14);

    // Full 14 Beta locales x {640x420, 1440x600} for both CreateGraph and EqualizeScaling + capture checks.
    for locale in BETA_LOCALE_CODES {
        for [w_pt, h_pt] in [[640.0_f64, 420.0], [1440.0, 600.0]] {
            for (cmd_id, params) in
                [("ui.graph.create.open", json!({ "data_sheet_id": "xy-1" })), ("ui.layout.equalizeScaling.open", json!({ "sheet_id": "lay-1" }))]
            {
                let mut app = base_app.clone();
                let _ = app.set_locale(locale);
                let _ = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": cmd_id, "params": params }))?;
                app.sync_views();
                let mut drv = make_driver(TestState::new(app), ViewportMetrics::new(w_pt, h_pt, 1.0));
                let parity = drv.verify_pixel_to_hit_box_parity();
                let modal_v = modal_parity_violations(&parity);
                assert!(modal_v.is_empty(), "{cmd_id} `{locale}` @ {w_pt}x{h_pt} parity: {modal_v:?}");
                let sb = drv.check_scrollbar_non_overlap();
                assert!(sb.is_empty(), "{cmd_id} `{locale}` @ {w_pt}x{h_pt} scrollbar overlap: {sb:?}");

                let outside = egui::pos2(4.0, (h_pt as f32) * 0.5);
                drv.wheel_at(outside, egui::vec2(0.0, -60.0))?;
                drv.click_at(outside)?;
                assert!(drv.state().outside_click_captured, "{cmd_id} `{locale}` click capture");
                assert!(drv.state().outside_wheel_captured, "{cmd_id} `{locale}` wheel capture");
            }
        }
    }

    // Check 3 themes x {1.0x, 2.0x} DPI + AccessKit tree check cleanly.
    for theme in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
        for dpi in [1.0_f64, 2.0] {
            let mut app = base_app.clone();
            app.set_theme_mode(theme);
            let _ = dispatch_gui_action(
                &mut app,
                "ui.invoke_menu",
                json!({ "command_id": "ui.graph.create.open", "params": { "data_sheet_id": "xy-1" } }),
            )?;
            app.sync_views();
            let drv = make_driver(TestState::new(app), ViewportMetrics::new(1440.0, 600.0, dpi));
            let parity = drv.verify_pixel_to_hit_box_parity();
            let modal_v = modal_parity_violations(&parity);
            assert!(modal_v.is_empty(), "{theme:?} @ {dpi}x parity: {modal_v:?}");
            assert!(drv.check_scrollbar_non_overlap().is_empty());
        }
    }

    let mut app_ak = base_app;
    let doc_ak = app_ak.document().clone();
    assert!(app_ak.canvas_dialogs.open_equalize_scaling(&doc_ak, &SheetId::new("lay-1")));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut views = EguiShellViewState::default();
    views.canvas_view.synchronous_raster = true;
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &app_ak, &mut views, &mut meta, &mut intents);
        },
    );
    out.textures_delta.clear();
    let ak_update = out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?;
    check_accesskit_scrollbar_non_overlap(ak_update)?;
    Ok(())
}
