#![forbid(unsafe_code)]

//! `R13.10` integration tests (`EguiHumanPathDriver`) for missing `Format Graph` and
//! `Graph Inspector` controls across `GWF-01`, `GWF-02`, `GWF-17`, `WF-63`, and `WF-64`:
//! - Plot Orientation (`format_graph.orientation.*` & `inspector.graph.orientation.*`)
//! - Show Symbols (`format_graph.show_symbols` -> `"marker": null`) & Replicate Lines (`format_graph.replicate_lines.*`)
//! - `retopologize_series` preservation of existing `ReplicateLineMode`
//! - Selectable active series (`format_graph.series_select.*`), Right Y2 binding (`format_graph.series_axis.*`),
//!   and horizontal nudge (`format_graph.series_offset_x`)
//! - Bar border width (`format_graph.bar_border_width_pt` + `Apply Globally`) & Forest pooled diamond (`format_graph.forest_show_pooled_diamond`)

use std::error::Error;

use curvebook_doc::{
    CartesianOrientation, Cell, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphKind, GraphicalDataObject, GroupedTable,
    MultipleVariablesTable, ProjectDocument, ReplicateLineMode, SheetId, SubcolumnMode, SummaryMeasure, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::{MarkerStyle, Rgba8};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents};
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::json;

type TestRes = Result<(), Box<dyn Error>>;

struct TestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_commit_err: Option<String>,
}

impl TestState {
    fn new(app: UiApp) -> Self {
        let mut views = app.egui_views.clone();
        views.canvas_view.synchronous_raster = true;
        Self { app, views, intents: PendingUiIntents::new(), last_commit_err: None }
    }
}

fn make_driver<'a>(state: TestState, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, TestState> {
    let mut driver = EguiHumanPathDriver::new(state, metrics, |ui, meta, st| {
        st.views.canvas_view.synchronous_raster = true;
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_commit_err = Some(err.to_string());
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

fn set_scrub_field_text(
    driver: &mut EguiHumanPathDriver<'_, TestState>,
    widget_id: &str,
    chip_offset_from_right_pt: f32,
    text: &str,
) -> Result<(), String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing scrub widget `{widget_id}`"))?.clone();
    let r = entry.clipped_rect;
    let click_pt = egui::pos2((r.right() - chip_offset_from_right_pt).max(r.left() + 6.0), r.center().y);
    driver.click_at(click_pt)?;
    for _ in 0..12 {
        driver.press_egui_key(egui::Key::Backspace, egui::Modifiers::NONE);
    }
    driver.type_text(text)?;
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    driver.run_settled();
    Ok(())
}

fn seed_r13_10_document() -> Result<UiApp, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("R13.10 Format Graph Controls")?;

    // 1. Grouped data table -> Grouped bar graph
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    grp.row_titles = vec![String::from("Vehicle"), String::from("Treated")];
    grp.data_sets[0].title = String::from("Low Dose");
    grp.data_sets[1].title = String::from("High Dose");
    for g in 0..2 {
        for r in 0..2 {
            for sub in 0..2 {
                grp.data_sets[g].cells[r][sub] = Cell::number(10.0 + (g * 5 + r * 3 + sub) as f64);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("grp-1", "Grouped Assay", TableArchetype::Grouped(grp))?)?;

    // 2. Column data table -> 2-series BarWithReplicates graph
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treatment");
    for (r, (v0, v1)) in [(12.0, 18.0), (13.0, 19.5), (11.5, 17.0)].into_iter().enumerate() {
        if let Some(c0) = col.data_sets[0].cell_mut(r, 0) {
            *c0 = Cell::number(v0);
        }
        if let Some(c1) = col.data_sets[1].cell_mut(r, 0) {
            *c1 = Cell::number(v1);
        }
    }
    doc.add_data_sheet(DataSheet::new("col-1", "Column Bars", TableArchetype::Column(col))?)?;

    // 3. XY table with 2 series & 2 replicates -> XY scatter+line graph
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4)?;
    xy.data_sets[0].title = String::from("Arm A");
    xy.data_sets[1].title = String::from("Arm B");
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(5.0 + r as f64 * 2.0);
        xy.data_sets[0].cells[r][1] = Cell::number(5.4 + r as f64 * 2.2);
        xy.data_sets[1].cells[r][0] = Cell::number(100.0 + r as f64 * 40.0);
        xy.data_sets[1].cells[r][1] = Cell::number(108.0 + r as f64 * 42.0);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "Dual Axis Kinetics", TableArchetype::Xy(xy))?)?;

    // 4. MV table -> Forest plot graph
    let mut mv = MultipleVariablesTable::new(2, 3)?;
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].role = VariableRole::Categorical;
    doc.add_data_sheet(DataSheet::new("mv-1", "Cohort MV", TableArchetype::MultipleVariables(mv))?)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;

    app.execute_command("graph.create", json!({ "sheet_id": "g-grp", "data_sheet_id": "grp-1", "kind": "grouped", "title": "Grouped Graph" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column", "title": "Column Bars" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy", "title": "Dual Axis XY" }))?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-forest", "data_sheet_id": "mv-1", "kind": "forest", "title": "Forest Plot" }))?;

    // Seed BarWithReplicates on both series of `g-col` so WF-64 bar border test edits both.
    app.execute_command(
        "graph.setSeries",
        json!({
            "sheet_id": "g-col",
            "all_series": true,
            "gdo": GraphicalDataObject::BarWithReplicates {
                summary: SummaryMeasure::Mean,
                fill_color: Rgba8::rgb(40, 120, 220),
                border_color: Rgba8::BLACK,
                border_width_pt: 1.0,
                replicate_overlay: None,
            },
        }),
    )?;

    // Seed ScatterAndLine on both series of `g-xy`.
    app.execute_command(
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "all_series": true,
            "gdo": GraphicalDataObject::ScatterAndLine {
                marker: Some(MarkerStyle::default()),
                line: None,
                area_fill: None,
                replicate_lines: ReplicateLineMode::SummaryCenter,
            },
        }),
    )?;

    Ok(app)
}

#[test]
fn gwf01_orientation_in_format_graph_and_graph_inspector() -> TestRes {
    let mut app = seed_r13_10_document()?;

    // 1. Format Graph -> Series & Arrangement -> Horizontal -> OK
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_format_graph(&doc, &SheetId::new("g-grp")));
    app.sync_views();
    let mut fg_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    fg_drv.click_widget_id("edit.tab.format_graph.series")?;
    fg_drv.click_widget_id("format_graph.orientation.horizontal")?;
    fg_drv.click_widget_id("modal.ok")?;
    assert!(fg_drv.state().last_commit_err.is_none(), "commit err: {:?}", fg_drv.state().last_commit_err);
    assert_eq!(fg_drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let mut app = fg_drv.state().app.clone();
    let gs = app.document().graph_sheet(&SheetId::new("g-grp")).ok_or("missing g-grp")?;
    assert_eq!(gs.spec.orientation, CartesianOrientation::Horizontal);

    // 2. Trailing Studio Graph Inspector -> Data tab -> Vertical chip
    app.execute_command("sheet.select", json!({ "sheet_id": "g-grp" }))?;
    app.nav.inspector_open = true;
    app.egui_views.trailing_collapsed_rail = false;
    app.sync_views();
    let mut studio_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(studio_drv.frame_meta().find_by_widget_id("inspector.graph.orientation.horizontal").is_some());
    studio_drv.click_widget_id("inspector.graph.orientation.vertical")?;

    let gs_after = studio_drv.state().app.document().graph_sheet(&SheetId::new("g-grp")).ok_or("missing g-grp after studio")?;
    assert_eq!(gs_after.spec.orientation, CartesianOrientation::Vertical);
    Ok(())
}

#[test]
fn gwf02_show_symbols_replicate_lines_and_retopologize_preserve() -> TestRes {
    let mut app = seed_r13_10_document()?;
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_format_graph(&doc, &SheetId::new("g-xy")));
    app.sync_views();

    let mut drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(drv.frame_meta().find_by_widget_id("format_graph.show_symbols").is_some());
    assert!(drv.frame_meta().find_by_widget_id("format_graph.replicate_lines.summary_center").is_some());
    assert!(drv.frame_meta().find_by_widget_id("format_graph.replicate_lines.individual_trajectories").is_some());

    // Toggle off `Show symbols` and select `Individual trajectories`.
    drv.click_widget_id("format_graph.show_symbols")?;
    drv.click_widget_id("format_graph.replicate_lines.individual_trajectories")?;
    drv.click_widget_id("modal.ok")?;

    let mut app = drv.state().app.clone();
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
        match &gs.spec.series[0].gdo {
            GraphicalDataObject::ScatterAndLine { marker, replicate_lines, .. } => {
                assert!(marker.is_none(), "unchecking Show symbols must clear marker (`marker: null`)");
                assert_eq!(*replicate_lines, ReplicateLineMode::IndividualReplicateTrajectories);
            }
            other => return Err(format!("expected ScatterAndLine, got {other:?}").into()),
        }
    }

    // Re-opening Format Graph must reflect `show_symbols == false`, and re-checking restores marker.
    let doc2 = app.document().clone();
    assert!(app.canvas_dialogs.open_format_graph(&doc2, &SheetId::new("g-xy")));
    app.sync_views();
    let mut drv2 = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(!drv2.state().app.canvas_dialogs.format_graph.show_symbols);
    drv2.click_widget_id("format_graph.show_symbols")?;
    drv2.click_widget_id("modal.ok")?;

    let mut app = drv2.state().app.clone();
    {
        let gs = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after reopen")?;
        match &gs.spec.series[0].gdo {
            GraphicalDataObject::ScatterAndLine { marker, replicate_lines, .. } => {
                assert!(marker.is_some(), "re-checking Show symbols must restore marker");
                assert_eq!(*replicate_lines, ReplicateLineMode::IndividualReplicateTrajectories);
            }
            other => return Err(format!("expected ScatterAndLine, got {other:?}").into()),
        }
    }

    // `retopologize_series` via inspector topology change to `"scatter"` must preserve `replicate_lines`.
    app.execute_command("sheet.select", json!({ "sheet_id": "g-xy" }))?;
    let mut shards = std::mem::take(&mut app.view_shards);
    curvebook_ui_egui::views::dispatch_custom_view_action(&mut app, &mut shards, "inspector", "fg.topology:scatter")
        .map_err(|e| format!("inspector topology change failed: {e}"))?;
    app.view_shards = shards;
    let gs_topo = app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after retopologize")?;
    match &gs_topo.spec.series[0].gdo {
        GraphicalDataObject::ScatterAndLine { replicate_lines, .. } => {
            assert_eq!(
                *replicate_lines,
                ReplicateLineMode::IndividualReplicateTrajectories,
                "retopologize_series must preserve existing replicate_lines"
            );
        }
        other => return Err(format!("expected ScatterAndLine after retopologize, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn gwf17_series_select_right_y2_and_horizontal_nudge() -> TestRes {
    let mut app = seed_r13_10_document()?;
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_format_graph(&doc, &SheetId::new("g-xy")));
    app.sync_views();

    let mut drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    drv.click_widget_id("edit.tab.format_graph.series")?;
    assert!(drv.frame_meta().find_by_widget_id("format_graph.series_select.0").is_some());
    assert!(drv.frame_meta().find_by_widget_id("format_graph.series_select.1").is_some());
    assert!(drv.frame_meta().find_by_widget_id("format_graph.series_axis.left_y").is_some());
    assert!(drv.frame_meta().find_by_widget_id("format_graph.series_axis.right_y2").is_some());
    assert!(drv.frame_meta().find_by_widget_id("format_graph.series_offset_x").is_some());

    // Select Series #2 (`series_select.1`), bind it to Right Y2, and set horizontal nudge to 0.25.
    drv.click_widget_id("format_graph.series_select.1")?;
    assert_eq!(drv.state().app.canvas_dialogs.format_graph.selected_series_index, 1);
    drv.click_widget_id("format_graph.series_axis.right_y2")?;
    set_scrub_field_text(&mut drv, "format_graph.series_offset_x", 24.0, "0.25")?;
    drv.click_widget_id("modal.ok")?;

    let gs = drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
    assert!(!gs.spec.series[0].bind_to_right_y2, "Series 0 must remain on Left Y");
    assert!(gs.spec.series[1].bind_to_right_y2, "Series 1 must bind to Right Y2");
    assert!((gs.spec.series[1].offset.x_data - 0.25).abs() < 1e-6, "Series 1 offset.x_data expected 0.25, got {}", gs.spec.series[1].offset.x_data);
    Ok(())
}

#[test]
fn wf64_bar_border_apply_globally_and_wf63_forest_pooled_diamond() -> TestRes {
    // 1. WF-64: Column bar graph -> Apply Globally + Bar Border 0.5 pt
    let mut app = seed_r13_10_document()?;
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_format_graph(&doc, &SheetId::new("g-col")));
    app.sync_views();

    let mut bar_drv = make_driver(TestState::new(app), ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(bar_drv.frame_meta().find_by_widget_id("format_graph.bar_border_width_pt").is_some());
    bar_drv.click_widget_id("format_graph.apply_globally")?;
    set_scrub_field_text(&mut bar_drv, "format_graph.bar_border_width_pt", 56.0, "0.5")?;
    bar_drv.click_widget_id("modal.ok")?;

    let gs_bar = bar_drv.state().app.document().graph_sheet(&SheetId::new("g-col")).ok_or("missing g-col")?;
    for (idx, s) in gs_bar.spec.series.iter().enumerate() {
        match s.gdo {
            GraphicalDataObject::BarWithReplicates { border_width_pt, .. } => {
                assert!((border_width_pt - 0.5).abs() < 1e-6, "series {idx} bar border expected 0.5, got {border_width_pt}");
            }
            ref other => return Err(format!("expected BarWithReplicates on series {idx}, got {other:?}").into()),
        }
    }

    // 2. WF-63: Diagnostic Forest Plot -> toggle `forest_show_pooled_diamond`
    let mut app2 = seed_r13_10_document()?;
    assert_eq!(
        app2.document().graph_sheet(&SheetId::new("g-forest")).ok_or("missing g-forest")?.spec.kind,
        GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot)
    );
    let initial_pooled = app2.document().graph_sheet(&SheetId::new("g-forest")).ok_or("missing g-forest")?.spec.forest_show_pooled_diamond;
    let doc_f = app2.document().clone();
    assert!(app2.canvas_dialogs.open_format_graph(&doc_f, &SheetId::new("g-forest")));
    app2.sync_views();

    let mut forest_drv = make_driver(TestState::new(app2), ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert!(forest_drv.frame_meta().find_by_widget_id("format_graph.forest_show_pooled_diamond").is_some());
    forest_drv.click_widget_id("format_graph.forest_show_pooled_diamond")?;
    forest_drv.click_widget_id("modal.ok")?;

    let gs_forest = forest_drv.state().app.document().graph_sheet(&SheetId::new("g-forest")).ok_or("missing g-forest after")?;
    assert_eq!(gs_forest.spec.forest_show_pooled_diamond, !initial_pooled);
    Ok(())
}
