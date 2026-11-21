#![forbid(unsafe_code)]

//! Targeted `EguiHumanPathDriver` acceptance suite for Wave B Graph/Inspector GUI gaps:
//! - `WF-51`/`WF-54`/`WF-60`: Attach/switch/clear `series.regression_overlay` in `Format Graph`
//! - `WF-54`: Per-series GDO `BarWithReplicates` <-> `ScatterAndLine` switch on `XY`/`Column`/`Grouped`
//! - `WF-32`/`WF-34`: `MultivariatePlotMode::CategoricalSummary` vs `BubbleScatter` in `BubblePlot` & `FormatGraph`
//! - `WF-45`: `Grouped` heatmap colormap, reverse, and range persistence via `graph.setSpec`
//! - `WF-48`: MV color/size column cycling, `bubble.size_variable`, localized `color_clamp_min`/`max`, and `oor_clamp`/`oor_omit`
//! - `WF-57`: Canvas rulers (`top_r`, `left_r`, `corner`) clip tick labels strictly inside ruler bounds at `1440x600`

use curvebook_chart::compile_graph;
use curvebook_doc::{
    Cell, CellCoord, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, GraphicalDataObject, GroupedTable, LayoutSheet,
    LayoutSpec, MultipleVariablesTable, MultivariatePlotMode, ProjectDocument, SheetId, SizeChannelMapping, SubcolumnMode, SummaryMeasure,
    TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_tools::canvas::PhysicalUnit;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, UiBackend};
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Key, Modifiers};
use serde_json::json;

type TestRes = Result<(), String>;

struct TestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_error: Option<String>,
}

impl TestState {
    fn new(app: UiApp) -> Self {
        let mut views = app.egui_views.clone();
        views.canvas_view.synchronous_raster = true;
        Self { app, views, intents: PendingUiIntents::new(), last_error: None }
    }
}

fn new_egui_app(doc: ProjectDocument) -> UiApp {
    let mut app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    app.set_human_path_only(true);
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app
}

fn make_driver<'a>(app: UiApp, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, TestState> {
    let mut driver = EguiHumanPathDriver::new(TestState::new(app), metrics, |ui, meta, st| {
        st.views.canvas_view.synchronous_raster = true;
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_error = Some(err.to_string());
            }
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

fn assert_modal_parity(driver: &EguiHumanPathDriver<'_, TestState>, label: &str) -> TestRes {
    let parity = driver.verify_pixel_to_hit_box_parity();
    let filtered: Vec<_> = parity.violations.iter().filter(|v| v.widget_id != "canvas.viewport" && v.widget_id != "canvas.split.viewport").collect();
    if !filtered.is_empty() {
        return Err(format!("{label}: parity violations: {filtered:?}"));
    }
    let sb = driver.check_scrollbar_non_overlap();
    if !sb.is_empty() {
        return Err(format!("{label}: scrollbar overlap: {sb:?}"));
    }
    Ok(())
}

fn replace_input_text(driver: &mut EguiHumanPathDriver<'_, TestState>, widget_id: &str, text: &str) -> TestRes {
    driver.click_widget_id(widget_id)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    if !text.is_empty() {
        driver.type_text(text)?;
    }
    driver.press_egui_key(Key::Enter, Modifiers::NONE);
    driver.run_settled();
    Ok(())
}

fn open_format_graph_on(driver: &mut EguiHumanPathDriver<'_, TestState>, gid: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    if !driver.state_mut().app.canvas_dialogs.open_format_graph(&doc, gid) {
        return Err(format!("failed to open FormatGraph for `{gid}`"));
    }
    driver.state_mut().app.sync_views();
    driver.state_mut().views = driver.state().app.egui_views.clone();
    driver.run_settled();
    Ok(())
}

fn open_bubble_plot_on(driver: &mut EguiHumanPathDriver<'_, TestState>, gid: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    if !driver.state_mut().app.canvas_dialogs.open_bubble_plot(&doc, gid) {
        return Err(format!("failed to open BubblePlotInspector for `{gid}`"));
    }
    driver.state_mut().app.sync_views();
    driver.state_mut().views = driver.state().app.egui_views.clone();
    driver.run_settled();
    Ok(())
}

fn seed_xy_doc_with_fits() -> Result<(UiApp, SheetId, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::new("R10.28 XY Fits").map_err(|e| e.to_string())?;
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 5)
        .map_err(|e| e.to_string())?;
    xy.x_title = String::from("Dose");
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[1].title = String::from("Drug");
    for r in 0..5 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0);
        xy.data_sets[1].cells[r][0] = Cell::number(3.5 * x + 2.0);
    }
    let xy_id = SheetId::try_new("ds-xy").map_err(|e| e.to_string())?;
    let ds = DataSheet::new(xy_id.clone(), "XY Kinetics", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    let gid = SheetId::try_new("g-xy").map_err(|e| e.to_string())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(gid.clone(), &ds).map_err(|e| e.to_string())?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::Xy;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&gid).map_err(|e| e.to_string())?;

    let mut app = new_egui_app(doc);
    let lin_out = app
        .execute_command("analysis.run", json!({ "sheet_id": "res-lin", "source_sheet_id": xy_id.as_str(), "kind": "linear_regression" }))
        .map_err(|e| e.to_string())?;
    let lin_id = SheetId::try_new(lin_out.payload["sheet_id"].as_str().ok_or("missing res-lin id")?).map_err(|e| e.to_string())?;
    let nl_out = app
        .execute_command(
            "analysis.run",
            json!({ "sheet_id": "res-nl", "source_sheet_id": xy_id.as_str(), "kind": "nonlinear_regression", "equation_id": "straight_line" }),
        )
        .map_err(|e| e.to_string())?;
    let nl_id = SheetId::try_new(nl_out.payload["sheet_id"].as_str().ok_or("missing res-nl id")?).map_err(|e| e.to_string())?;
    // Add a non-regression Transform result sheet to confirm only Linear/Nonlinear fits appear in overlay attach list.
    app.execute_command("transform.run", json!({ "sheet_id": "res-norm", "source_sheet_id": xy_id.as_str(), "kind": "normalize" }))
        .map_err(|e| e.to_string())?;
    app.sync_views();
    Ok((app, gid, lin_id, nl_id))
}

fn seed_mv_doc(cols: usize, rows: usize) -> Result<(UiApp, SheetId, SheetId), String> {
    let mut doc = ProjectDocument::new("R10.28 MV").map_err(|e| e.to_string())?;
    let mut mv = MultipleVariablesTable::new(cols, rows).map_err(|e| e.to_string())?;
    mv.variables[0].title = String::from("XVar");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("YVar");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].title = String::from("SizeVar");
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[3].title = String::from("ColorVar");
    mv.variables[3].role = VariableRole::Continuous;
    if cols > 4 {
        mv.variables[4].title = String::from("Group");
        mv.variables[4].role = VariableRole::Categorical;
    }
    let mut ds = DataSheet::new(SheetId::new("ds-mv"), "MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    for r in 0..rows {
        let base = (r + 1) as f64;
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(base)).map_err(|e| e.to_string())?;
        ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(base * 2.5)).map_err(|e| e.to_string())?;
        ds.set_cell(CellCoord::new(r, 2, 0), Cell::number(if r + 1 == rows { 1000.0 } else { base * 4.0 })).map_err(|e| e.to_string())?;
        ds.set_cell(CellCoord::new(r, 3, 0), Cell::number(if r + 1 == rows { 500.0 } else { base * 5.0 })).map_err(|e| e.to_string())?;
        if cols > 4 {
            ds.set_cell(CellCoord::new(r, 4, 0), Cell::text(if r % 2 == 0 { "Ctrl" } else { "Treated" })).map_err(|e| e.to_string())?;
        }
    }
    let gid = SheetId::new("g-mv");
    let mut gs = GraphSheet::new_uninitialized_for_data(gid.clone(), &ds).map_err(|e| e.to_string())?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::MultipleVariables;
    let ds_id = ds.id.clone();
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&gid).map_err(|e| e.to_string())?;

    let mut app = new_egui_app(doc);
    app.sync_views();
    Ok((app, ds_id, gid))
}

#[test]
fn wf51_54_60_regression_overlay_attach_switch_and_clear_via_gui() -> TestRes {
    let (app, gid, lin_id, nl_id) = seed_xy_doc_with_fits()?;
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    open_format_graph_on(&mut driver, &gid)?;
    assert_modal_parity(&driver, "FormatGraph XY initial")?;

    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.attach.0").is_some());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.attach.1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.attach.2").is_none());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.clear").is_none());

    // Attach linear fit via button -> Apply keeps dialog open and attaches overlay via graph.setRegressionOverlay.
    driver.click_widget_id("format_graph.overlay.attach.0")?;
    assert!(driver.state().app.canvas_dialogs.format_graph.show_fitted_curve);
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.clear").is_some());
    driver.click_widget_id("modal.apply")?;
    assert!(driver.state().last_error.is_none(), "unexpected error on Apply: {:?}", driver.state().last_error);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. }));

    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing g-xy after Apply")?;
        let ov = gs.spec.series[0].regression_overlay.as_ref().ok_or("expected series[0].regression_overlay after Apply")?;
        assert_eq!(ov.result_sheet_id, lin_id);
        assert!(ov.show_confidence_band);
        assert!(!ov.show_prediction_band);
    }

    // Switch overlay to nonlinear fit -> OK closes dialog and persists overlay.
    driver.click_widget_id("format_graph.overlay.attach.1")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(driver.state().last_error.is_none(), "unexpected error on OK: {:?}", driver.state().last_error);

    {
        let doc = driver.state().app.document();
        let gs = doc.graph_sheet(&gid).ok_or("missing g-xy after OK")?;
        let ov = gs.spec.series[0].regression_overlay.as_ref().ok_or("expected nonlinear overlay")?;
        assert_eq!(ov.result_sheet_id, nl_id);
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        assert!(!scene.primitives.is_empty());
    }

    // Re-open, clear overlay on series 0, switch to series 1, attach linear fit on series 1.
    open_format_graph_on(&mut driver, &gid)?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.clear").is_some());
    driver.click_widget_id("format_graph.overlay.clear")?;
    assert!(!driver.state().app.canvas_dialogs.format_graph.show_fitted_curve);
    driver.click_widget_id("modal.ok")?;
    assert!(
        driver.state().app.document().graph_sheet(&gid).ok_or("missing g-xy")?.spec.series[0].regression_overlay.is_none(),
        "clearing overlay must persist None"
    );

    open_format_graph_on(&mut driver, &gid)?;
    driver.click_widget_id("format_graph.series_next")?;
    assert_eq!(driver.state().app.canvas_dialogs.format_graph.selected_series_index, 1);
    driver.click_widget_id("format_graph.overlay.attach.0")?;
    driver.click_widget_id("modal.ok")?;
    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing g-xy series 1")?;
        assert!(gs.spec.series[0].regression_overlay.is_none());
        let ov1 = gs.spec.series[1].regression_overlay.as_ref().ok_or("expected overlay on series 1")?;
        assert_eq!(ov1.result_sheet_id, lin_id);
    }
    Ok(())
}

#[test]
fn wf54_per_series_gdo_bar_with_replicates_and_scatter_roundtrip() -> TestRes {
    let (app, gid, _, _) = seed_xy_doc_with_fits()?;
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    open_format_graph_on(&mut driver, &gid)?;

    assert!(driver.frame_meta().find_by_widget_id("format_graph.series_gdo.scatter_line").is_some());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.series_gdo.bar_replicates").is_some());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.bar_border_width_pt").is_none());

    // Switch series 1 to BarWithReplicates while leaving series 0 as ScatterAndLine.
    driver.click_widget_id("format_graph.series_next")?;
    driver.click_widget_id("format_graph.series_gdo.bar_replicates")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.bar_border_width_pt").is_some());
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_error.is_none(), "commit error: {:?}", driver.state().last_error);

    {
        let doc = driver.state().app.document();
        let gs = doc.graph_sheet(&gid).ok_or("missing g-xy after bar switch")?;
        assert!(matches!(gs.spec.series[0].gdo, GraphicalDataObject::ScatterAndLine { .. }));
        assert!(matches!(
            gs.spec.series[1].gdo,
            GraphicalDataObject::BarWithReplicates { summary: SummaryMeasure::Mean, replicate_overlay: None, .. }
        ));
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        assert!(!scene.primitives.is_empty());
    }

    // Re-open on series 1 (preserved index): verify series 0 vs series 1 sync and switch series 1 back to ScatterAndLine.
    open_format_graph_on(&mut driver, &gid)?;
    assert_eq!(driver.state().app.canvas_dialogs.format_graph.selected_series_index, 1);
    assert!(driver.state().app.canvas_dialogs.format_graph.series_gdo_bar);
    driver.click_widget_id("format_graph.series_prev")?;
    assert!(!driver.state().app.canvas_dialogs.format_graph.series_gdo_bar);
    driver.click_widget_id("format_graph.series_next")?;
    assert!(driver.state().app.canvas_dialogs.format_graph.series_gdo_bar);
    driver.click_widget_id("format_graph.series_gdo.scatter_line")?;
    driver.click_widget_id("modal.ok")?;
    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing g-xy after scatter switch")?;
        assert!(matches!(gs.spec.series[1].gdo, GraphicalDataObject::ScatterAndLine { marker: Some(_), line: Some(_), .. }));
    }
    Ok(())
}

#[test]
fn wf32_34_mv_plot_mode_categorical_summary_vs_bubble_scatter() -> TestRes {
    let (app, _ds_id, gid) = seed_mv_doc(5, 6)?;
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));

    // 1. Switch to CategoricalSummary via BubblePlotInspector.
    open_bubble_plot_on(&mut driver, &gid)?;
    assert_modal_parity(&driver, "BubblePlot plot_mode")?;
    assert!(driver.frame_meta().find_by_widget_id("bubble.plot_mode.bubble_scatter").is_some());
    assert!(driver.frame_meta().find_by_widget_id("bubble.plot_mode.categorical_summary").is_some());

    // Set X to categorical column 4 ("Group") before switching to CategoricalSummary so scene renders box-whiskers.
    for _ in 0..4 {
        driver.click_widget_id("bubble.x_next")?;
    }
    driver.click_widget_id("bubble.plot_mode.categorical_summary")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_error.is_none(), "bubble commit error: {:?}", driver.state().last_error);

    {
        let doc = driver.state().app.document();
        let gs = doc.graph_sheet(&gid).ok_or("missing g-mv after CategoricalSummary")?;
        let enc = gs.spec.multivariate.as_ref().ok_or("missing mv encoding")?;
        assert!(matches!(
            enc.plot_mode,
            MultivariatePlotMode::CategoricalSummary { gdo: GraphicalDataObject::BoxAndWhisker { .. }, y_error_bars: None }
        ));
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        assert!(!scene.primitives.is_empty());
    }

    // 2. Switch back to BubbleScatter and then CategoricalSummary via FormatGraph -> AxisVariables.
    open_format_graph_on(&mut driver, &gid)?;
    driver.click_widget_id("edit.tab.format_graph.axis_variables")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.mv.plot_mode.bubble_scatter").is_some());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.mv.plot_mode.categorical_summary").is_some());
    driver.click_widget_id("format_graph.mv.plot_mode.bubble_scatter")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(
        driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).map(|m| &m.plot_mode),
        Some(&MultivariatePlotMode::BubbleScatter)
    );

    open_format_graph_on(&mut driver, &gid)?;
    driver.click_widget_id("edit.tab.format_graph.axis_variables")?;
    driver.click_widget_id("format_graph.mv.plot_mode.categorical_summary")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(
        driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).map(|m| &m.plot_mode),
        Some(MultivariatePlotMode::CategoricalSummary { gdo: GraphicalDataObject::BoxAndWhisker { .. }, y_error_bars: None })
    ));
    Ok(())
}

#[test]
fn wf45_grouped_heatmap_colormap_and_range_persistence() -> TestRes {
    let mut doc = ProjectDocument::new("R10.28 Grouped Heatmap").map_err(|e| e.to_string())?;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 3, 3).map_err(|e| e.to_string())?;
    grp.row_titles = vec![String::from("R1"), String::from("R2"), String::from("R3")];
    grp.data_sets[0].title = String::from("A");
    grp.data_sets[1].title = String::from("B");
    grp.data_sets[2].title = String::from("C");
    for r in 0..3 {
        for c in 0..3 {
            grp.data_sets[c].cells[r][0] = Cell::number(((r * 3 + c + 1) * 10) as f64);
        }
    }
    let grp_ds = DataSheet::new(SheetId::new("ds-grp"), "Grouped Matrix", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?;
    let hm_gid = SheetId::new("g-grp-hm");
    let mut hm_gs = GraphSheet::new_uninitialized_for_data(hm_gid.clone(), &grp_ds).map_err(|e| e.to_string())?;
    hm_gs.initialized = true;
    hm_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    assert!(hm_gs.spec.multivariate.is_none(), "Grouped heatmap starts with spec.multivariate == None");
    doc.add_data_sheet(grp_ds).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(hm_gs).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&hm_gid).map_err(|e| e.to_string())?;

    let mut driver = make_driver(new_egui_app(doc), ViewportMetrics::new(1280.0, 800.0, 1.0));
    open_format_graph_on(&mut driver, &hm_gid)?;
    assert_modal_parity(&driver, "Grouped Heatmap FormatGraph")?;

    driver.click_widget_id("format_graph.heatmap.cmap.Plasma")?;
    driver.click_widget_id("format_graph.heatmap.reversed")?;
    driver.click_widget_id("format_graph.heatmap.auto_range")?;
    replace_input_text(&mut driver, "format_graph.heatmap.min", "5")?;
    replace_input_text(&mut driver, "format_graph.heatmap.max", "50")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_error.is_none(), "grouped heatmap commit error: {:?}", driver.state().last_error);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let doc_after = driver.state().app.document();
    let hm_after = doc_after.graph_sheet(&hm_gid).ok_or("missing grouped heatmap after OK")?;
    let enc = hm_after.spec.multivariate.as_ref().ok_or("expected spec.multivariate persisted on Grouped heatmap")?;
    assert!(matches!(
        enc.color,
        ColorChannelMapping::Continuous {
            ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: true },
            domain_clamp: Some((lo, hi)),
            ..
        } if (lo - 5.0).abs() < 1e-6 && (hi - 50.0).abs() < 1e-6
    ));
    let scene = compile_graph(hm_after, doc_after).map_err(|e| e.to_string())?;
    assert!(!scene.primitives.is_empty());
    Ok(())
}

fn exercise_wf48_bubble_plot_controls(driver: &mut EguiHumanPathDriver<'_, TestState>, gid: &SheetId) -> TestRes {
    open_bubble_plot_on(driver, gid)?;
    assert_modal_parity(driver, "BubblePlot WF48")?;
    assert!(driver.frame_meta().find_by_widget_id("bubble.color_col_next").is_some());
    assert!(driver.frame_meta().find_by_widget_id("bubble.size_variable").is_some());
    assert!(driver.frame_meta().find_by_widget_id("bubble.size_col_next").is_none());

    driver.click_widget_id("bubble.color_col_next")?; // Constant -> Continuous col 0
    driver.click_widget_id("bubble.color_col_next")?; // col 0 -> col 1
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.color_channel, ColorChannelMapping::Continuous { column: 1, .. }));

    driver.click_widget_id("bubble.size_variable")?;
    assert!(driver.frame_meta().find_by_widget_id("bubble.size_col_next").is_some());
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.size_channel, SizeChannelMapping::Variable { column: 0, .. }));
    driver.click_widget_id("bubble.size_col_next")?; // 0 -> 1
    driver.click_widget_id("bubble.size_col_next")?; // 1 -> 2
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.size_channel, SizeChannelMapping::Variable { column: 2, .. }));

    // Toggle off -> Constant, then back on -> Variable again, then OK.
    driver.click_widget_id("bubble.size_variable")?;
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.size_channel, SizeChannelMapping::Constant { .. }));
    driver.click_widget_id("bubble.size_variable")?;
    driver.click_widget_id("bubble.size_col_next")?; // 0 -> 1
    driver.click_widget_id("bubble.size_col_next")?; // 1 -> 2
    driver.click_widget_id("modal.ok")?;
    if let Some(err) = &driver.state().last_error {
        return Err(format!("bubble commit err: {err}"));
    }
    Ok(())
}

fn exercise_wf48_format_graph_mv_clamping(driver: &mut EguiHumanPathDriver<'_, TestState>, gid: &SheetId) -> TestRes {
    open_format_graph_on(driver, gid)?;
    driver.click_widget_id("edit.tab.format_graph.axis_variables")?;
    assert_modal_parity(driver, "FormatGraph MV AxisVariables")?;
    driver.click_widget_id("format_graph.mv.color_continuous")?; // y_column = 1
    driver.click_widget_id("format_graph.mv.color_col_next")?; // 1 -> 2
    driver.click_widget_id("format_graph.mv.color_col_next")?; // 2 -> 3
    assert!(matches!(
        driver.state().app.canvas_dialogs.format_graph.mv_encoding.as_ref().map(|e| &e.color),
        Some(ColorChannelMapping::Continuous { column: 3, .. })
    ));

    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    assert_modal_parity(driver, "FormatGraph MV DataObjects")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.mv.size_col_next").is_some());
    driver.click_widget_id("format_graph.mv.size_col_next")?; // 2 -> 3
    assert!(matches!(
        driver.state().app.canvas_dialogs.format_graph.mv_encoding.as_ref().map(|e| e.size),
        Some(SizeChannelMapping::Variable { column: 3, .. })
    ));

    replace_input_text(driver, "format_graph.mv.color_clamp_min", "0")?;
    replace_input_text(driver, "format_graph.mv.color_clamp_max", "30")?;
    driver.click_widget_id("modal.ok")?;
    if let Some(err) = &driver.state().last_error {
        return Err(format!("format_graph clamp err: {err}"));
    }

    {
        let doc = driver.state().app.document();
        let gs = doc.graph_sheet(gid).ok_or("missing g-mv after clamp")?;
        let enc = gs.spec.multivariate.as_ref().ok_or("missing mv enc after clamp")?;
        assert!(matches!(
            enc.color,
            ColorChannelMapping::Continuous { column: 3, domain_clamp: Some((lo, hi)), .. }
            if (lo - 0.0).abs() < 1e-6 && (hi - 30.0).abs() < 1e-6
        ));
        assert!(matches!(enc.size, SizeChannelMapping::Variable { column: 3, .. }));
        let scene = compile_graph(gs, doc).map_err(|e| e.to_string())?;
        assert!(!scene.primitives.is_empty());
    }

    // Re-open FormatGraph -> drafts mirror stored (0, 30); test oor_omit clears clamp.
    open_format_graph_on(driver, gid)?;
    assert_eq!(driver.state().views.graph_modals.mv_clamp_min_draft, "0");
    assert_eq!(driver.state().views.graph_modals.mv_clamp_max_draft, "30");
    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    driver.click_widget_id("format_graph.mv.oor_omit")?;
    assert_eq!(driver.state().views.graph_modals.mv_clamp_min_draft, "");
    assert_eq!(driver.state().views.graph_modals.mv_clamp_max_draft, "");

    replace_input_text(driver, "format_graph.mv.color_clamp_min", "2")?;
    replace_input_text(driver, "format_graph.mv.color_clamp_max", "20")?;
    driver.click_widget_id("format_graph.mv.oor_clamp")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(
        driver.state().app.document().graph_sheet(gid).and_then(|g| g.spec.multivariate.as_ref()).map(|e| &e.color),
        Some(ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. })
        if (*lo - 2.0).abs() < 1e-6 && (*hi - 20.0).abs() < 1e-6
    ));
    Ok(())
}

#[test]
fn wf48_mv_color_size_cycling_domain_clamp_and_bubble_controls() -> TestRes {
    let (app, _ds_id, gid) = seed_mv_doc(4, 6)?;
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    exercise_wf48_bubble_plot_controls(&mut driver, &gid)?;
    exercise_wf48_format_graph_mv_clamping(&mut driver, &gid)?;

    // Verify localized comma bounds (`de`) and clearing when both inputs are empty.
    let _ = driver.state_mut().app.set_locale("de");
    open_format_graph_on(&mut driver, &gid)?;
    assert_eq!(driver.state().views.graph_modals.mv_clamp_min_draft, "2");
    assert_eq!(driver.state().views.graph_modals.mv_clamp_max_draft, "20");
    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    replace_input_text(&mut driver, "format_graph.mv.color_clamp_min", "0,5")?;
    replace_input_text(&mut driver, "format_graph.mv.color_clamp_max", "12,5")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(
        driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).map(|e| &e.color),
        Some(ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. })
        if (*lo - 0.5).abs() < 1e-6 && (*hi - 12.5).abs() < 1e-6
    ));

    open_format_graph_on(&mut driver, &gid)?;
    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    replace_input_text(&mut driver, "format_graph.mv.color_clamp_min", "")?;
    replace_input_text(&mut driver, "format_graph.mv.color_clamp_max", "")?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(
        driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).map(|e| &e.color),
        Some(ColorChannelMapping::Continuous { domain_clamp: None, .. })
    ));
    Ok(())
}

#[test]
fn wf57_canvas_rulers_1440x600_no_text_bleed() -> TestRes {
    let mut doc = ProjectDocument::new("R10.28 WF57 Rulers").map_err(|e| e.to_string())?;
    let lid = SheetId::try_new("lay-57").map_err(|e| e.to_string())?;
    let ls = LayoutSheet::new(lid.clone(), "Schematic Layout", LayoutSpec::default()).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;

    let mut app = new_egui_app(doc);
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-57" })).map_err(|e| e.to_string())?;
    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 600.0, 1.0));
    driver.set_bleed_policy(TextBleedPolicy::StrictAllEdges);

    // Toggle rulers on via visible LayoutSheet toolbar action (`ui.rulers.toggle`).
    driver.click_widget_id("toolbar.rulers")?;
    if !driver.state().app.canvas_dialogs.rulers_visible {
        return Err(String::from("expected rulers_visible == true after clicking toolbar.rulers"));
    }

    for unit in [PhysicalUnit::Points, PhysicalUnit::Inches, PhysicalUnit::Millimeters] {
        driver.state_mut().app.canvas_dialogs.ruler_unit = unit;
        driver.run_settled();

        // Standard layout parity (AllowVerticalScrollClip: catches horizontal bleed on top_r/left_r ticks).
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        assert_modal_parity(&driver, &format!("WF-57 rulers {unit:?} 1440x600"))?;

        // StrictAllEdges specifically verifying no ruler tick/unit text overflows top_r/left_r/corner on any edge.
        driver.set_bleed_policy(TextBleedPolicy::StrictAllEdges);
        let strict_report = driver.verify_pixel_to_hit_box_parity();
        let ruler_bleeds: Vec<_> = strict_report
            .violations
            .iter()
            .filter(|v| {
                v.check == "text_unclipped_bleed"
                    && v.detail.split('`').nth(1).is_some_and(|txt| txt.parse::<i32>().is_ok() || matches!(txt, "pt" | "in" | "mm"))
            })
            .collect();
        if !ruler_bleeds.is_empty() {
            return Err(format!("WF-57 strict ruler text bleed for unit={unit:?} at 1440x600: {ruler_bleeds:?}"));
        }
    }

    // Verify rulers toggle back off cleanly as well.
    driver.click_widget_id("toolbar.rulers")?;
    if driver.state().app.canvas_dialogs.rulers_visible {
        return Err(String::from("expected rulers_visible == false after second click"));
    }
    Ok(())
}
