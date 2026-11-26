#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

//! Regression tests for UI & Inspector gap audit findings:
//! - `GAP-06-02`: `UiApp::execute_command` surfaces non-fatal figure export warnings in `status_banner`.
//! - `GAP-07-01`: Split-view Trailing Studio reads panel/annotation/media selection from the active canvas view (`primary` vs `secondary_canvas_view`).
//! - `GAP-07-02`: `GridViewState` binds selection (`tool`/`selected_coords`) to its `SheetId`, preventing cross-sheet exclusion or styling in split view.
//! - `GAP-07-04`: `UiApp::open_import_project_mode_dialog` refuses to overwrite an already-open modal dialog.
//! - `GAP-07-06`: Multi-frame drag on Page Layout Panel Width/Height `DragValue` previews live and commits exactly 1 undo entry on release.

use super::common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, CropInsetsPt, CurveSamplePoint, DataSheet, FittedCurveOverlay,
    GraphAnnotation, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec, NonlinearFitSpec, ProjectDocument, RectPt, RegressionOverlaySpec, ResultSheet,
    ResultStatus, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::center_stage::SplitPaneSlot;
use curvebook_ui_egui::egui_shell::data_grid::resolve_grid_selection;
use curvebook_ui_egui::egui_shell::trailing_studio::GraphStudioTab;
use curvebook_ui_egui::interaction::{KeyModifiers, UiInputEvent, ViewportMetrics, dispatch_ui_event};
use egui::{Event, Modifiers, PointerButton, pos2};
use serde_json::json;

use common::{ShellTestState, make_shell_driver_with_state};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

fn make_col_sheet(id: &str, title: &str, values: &[&[f64]]) -> Result<DataSheet, Box<dyn std::error::Error>> {
    let rows = values.iter().map(|c| c.len()).max().unwrap_or(1);
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, values.len(), rows)?;
    for (c, col_vals) in values.iter().enumerate() {
        for (r, &v) in col_vals.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    Ok(DataSheet::new(id, title, TableArchetype::Column(table))?)
}

fn make_xy_sheet(id: &str, title: &str, pts: &[(f64, f64)]) -> Result<DataSheet, Box<dyn std::error::Error>> {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, pts.len())?;
    for (r, &(x, y)) in pts.iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    Ok(DataSheet::new(id, title, TableArchetype::Xy(table))?)
}

fn make_live_graph_panel(id: &str, graph_id: &str, x: f64, y: f64, w: f64, h: f64) -> LayoutItem {
    LayoutItem::LiveGraph {
        id: id.to_owned(),
        graph_sheet_id: SheetId::new(graph_id),
        bounds_pt: RectPt::new(x, y, w, h),
        scale_percent: 100.0,
        badge_override: None,
    }
}

fn default_tokens(app: &UiApp) -> Tokens {
    Tokens::from_theme_spec(&app.theme, &app.locale)
}

#[test]
fn gap_06_02_ui_app_figure_export_surfaces_and_clears_warning_banner() -> BoxRes {
    let mut doc = ProjectDocument::new("Export Warning UI")?;
    let ds = make_xy_sheet("xy-1", "Dose", &[(1.0, 2.0), (2.0, 4.1), (3.0, 5.9), (4.0, 8.2)])?;
    doc.add_data_sheet(ds.clone())?;

    let spec = AnalysisSpec::new("rs-fit", "xy-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear")))?;
    let mut res = ResultSheet::new(spec)?;
    res.status = ResultStatus::Dirty;
    let pt = |x: f64, y: f64| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None };
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![pt(1.0, 2.0), pt(4.0, 8.2)],
    });
    doc.add_result_sheet(res)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("rs-fit"),
        sample_segments: 32,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    doc.add_graph_sheet(graph)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let out = app.execute_command("export.svg", json!({ "sheet_id": "g-1" }))?;
    assert!(out.warnings.iter().any(|w| w.contains("pending_regression_result")), "expected warning, got {:?}", out.warnings);
    assert!(
        app.canvas_dialogs.status_banner.as_deref().is_some_and(|b| b.contains("pending_regression_result")),
        "status_banner must surface figure export warning, got {:?}",
        app.canvas_dialogs.status_banner
    );

    // Mark fit UpToDate; subsequent clean export clears the figure export warning banner.
    let mut clean_doc = app.document().clone();
    clean_doc.result_sheet_mut(&SheetId::new("rs-fit")).expect("rs-fit").status = ResultStatus::UpToDate;
    let mut clean_app = UiApp::from_session(Session::from_document(clean_doc));
    clean_app.canvas_dialogs.status_banner = app.canvas_dialogs.status_banner.clone();
    let clean = clean_app.execute_command("export.svg", json!({ "sheet_id": "g-1" }))?;
    assert!(clean.warnings.is_empty());
    assert!(clean_app.canvas_dialogs.status_banner.is_none(), "clean export should clear prior export warning banner");
    Ok(())
}

#[test]
fn gap_07_01_split_secondary_canvas_selection_targets_layout_and_graph_inspector() -> BoxRes {
    // 1. Two LayoutSheets in Split Comparison: primary `lay-1` selects panel 0; secondary `lay-2` selects panel 2.
    let mut doc = ProjectDocument::new("Split Layout Inspector")?;
    let ds = make_col_sheet("ds-1", "Data", &[&[1.0, 2.0], &[3.0, 4.0]])?;
    let g1 = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    let mut g2 = GraphSheet::new_uninitialized_for_data("g-2", &ds)?;
    g2.spec.annotations.push(GraphAnnotation::EmbeddedImage {
        asset_id: String::from("img-0"),
        bounds_pt: RectPt::new(10.0, 10.0, 60.0, 40.0),
        crop: CropInsetsPt::default(),
        rotation_deg: 0.0,
        lock_aspect_ratio: true,
        border: None,
    });
    g2.spec.annotations.push(GraphAnnotation::EmbeddedImage {
        asset_id: String::from("img-1"),
        bounds_pt: RectPt::new(90.0, 10.0, 80.0, 50.0),
        crop: CropInsetsPt::default(),
        rotation_deg: 0.0,
        lock_aspect_ratio: true,
        border: None,
    });
    doc.add_data_sheet(ds)?;
    doc.add_graph_sheet(g1)?;
    doc.add_graph_sheet(g2)?;

    let mut lay1 = LayoutSheet::new("lay-1", "Layout Primary", LayoutSpec::default())?;
    lay1.spec.items = vec![make_live_graph_panel("p1", "g-1", 10.0, 10.0, 100.0, 80.0), make_live_graph_panel("p2", "g-2", 120.0, 10.0, 110.0, 90.0)];
    let mut lay2 = LayoutSheet::new("lay-2", "Layout Secondary", LayoutSpec::default())?;
    lay2.spec.items = vec![
        make_live_graph_panel("s1", "g-1", 10.0, 10.0, 120.0, 80.0),
        make_live_graph_panel("s2", "g-2", 140.0, 10.0, 150.0, 90.0),
        make_live_graph_panel("s3", "g-1", 10.0, 120.0, 240.0, 130.0),
    ];
    doc.add_layout_sheet(lay1)?;
    doc.add_layout_sheet(lay2)?;
    doc.active_sheet_id = Some(SheetId::new("lay-2"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app.nav.inspector_open = true;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("lay-1"));
    app.split.secondary_sheet_id = Some(SheetId::new("lay-2"));
    let tokens = default_tokens(&app);

    let mut st = ShellTestState::new(app).with_commit_all().with_sync_raster();
    // Primary canvas view holds stale selection panel 0 for lay-1; secondary holds panel 2 for lay-2.
    st.views.canvas_view.active_viewport_sheet = Some(SheetId::new("lay-1"));
    st.views.canvas_view.selected_panel_index = Some(0);
    st.views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
    st.views.center_stage.secondary_canvas_view.active_viewport_sheet = Some(SheetId::new("lay-2"));
    st.views.center_stage.secondary_canvas_view.selected_panel_index = Some(2);

    let mut driver = make_shell_driver_with_state(st, ViewportMetrics::new(1440.0, 900.0, 1.0), tokens);
    driver.run_settled();

    // Panel Width spinbutton must reflect lay-2's panel 2 (width 240.0 pt), NOT primary's panel 0 (100.0 pt).
    let w_entry = driver.frame_meta().find_by_widget_id("inspector.layout.panel.width_pt").expect("panel width widget");
    assert_eq!(w_entry.value.as_ref().and_then(|v| v.get("panel_index")).and_then(|v| v.as_u64()), Some(2));
    assert_eq!(w_entry.value.as_ref().and_then(|v| v.get("width_pt")).and_then(|v| v.as_f64()), Some(240.0));

    // Clicking Z-order "Send to Back" must reorder panel 2 of lay-2 (moving width 240.0 to index 0) without touching lay-1.
    driver.click_widget_id("inspector.layout.zorder.back")?;
    let lay2_after = driver.state().app.document().layout_sheet(&SheetId::new("lay-2")).expect("lay-2");
    let first_w = match &lay2_after.spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt.width,
        _ => panic!("expected LiveGraph"),
    };
    assert_eq!(first_w, 240.0, "Z-order Back in Inspector must move secondary pane's selected panel 2 to index 0");
    let lay1_after = driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).expect("lay-1");
    let lay1_first_w = match &lay1_after.spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt.width,
        _ => panic!("expected LiveGraph"),
    };
    assert_eq!(lay1_first_w, 100.0, "primary layout must remain untouched");

    // 2. Graph annotation selection in secondary split pane: Format Image button targets annotation 1 of g-2.
    {
        let st = driver.state_mut();
        st.app.split.primary_sheet_id = Some(SheetId::new("g-1"));
        st.app.split.secondary_sheet_id = Some(SheetId::new("g-2"));
        let _ = st.app.execute_command("sheet.select", json!({ "sheet_id": "g-2" }))?;
        st.views.canvas_view.active_viewport_sheet = Some(SheetId::new("g-1"));
        st.views.canvas_view.selected_graph_annotation = Some(0);
        st.views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
        st.views.center_stage.secondary_canvas_view.active_viewport_sheet = Some(SheetId::new("g-2"));
        st.views.center_stage.secondary_canvas_view.selected_graph_annotation = Some(1);
        st.views.trailing_studio.graph_tab = GraphStudioTab::Canvas;
    }
    driver.run_settled();
    driver.click_widget_id("inspector.canvas.format_image")?;
    assert!(
        matches!(
            &driver.state().app.canvas_dialogs.active_dialog,
            ActiveDialog::FormatImage { sheet_id, target_index: 1, is_layout: false } if sheet_id == &SheetId::new("g-2")
        ),
        "Format Image must target secondary pane's selected annotation 1 on g-2, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );
    Ok(())
}

#[test]
fn gap_07_02_split_view_grid_selection_is_sheet_bound_and_never_excludes_wrong_sheet() -> BoxRes {
    let mut doc = ProjectDocument::new("Split Sheet Selection")?;
    let ds_a = make_col_sheet("ds-a", "Sheet A", &[&[10.0, 20.0, 30.0], &[40.0, 50.0, 60.0]])?;
    let ds_b = make_col_sheet("ds-b", "Sheet B", &[&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]])?;
    doc.add_data_sheet(ds_a)?;
    doc.add_data_sheet(ds_b)?;
    doc.active_sheet_id = Some(SheetId::new("ds-b"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app.nav.inspector_open = true;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("ds-a"));
    app.split.secondary_sheet_id = Some(SheetId::new("ds-b"));
    app.split.inspector_pinned = true;
    app.split.inspector_target_sheet_id = Some(SheetId::new("ds-b"));

    // Select cells (0..=1, col 0) in Sheet A via sheet-bound selection (matching real grid click).
    app.grid.select_single_for_sheet(&SheetId::new("ds-a"), GridCellCoord::new(0, 0, 0));
    let ds_a_ref = app.document().data_sheet(&SheetId::new("ds-a")).expect("ds-a").clone();
    app.grid.extend_shift_range(&ds_a_ref, GridCellCoord::new(1, 0, 0));

    // Selection queries against Sheet B MUST return empty!
    let ds_b_ref = app.document().data_sheet(&SheetId::new("ds-b")).expect("ds-b").clone();
    assert!(resolve_grid_selection(&app, &ds_b_ref).is_none(), "Sheet A's selection must not resolve on Sheet B");
    assert!(app.grid.all_selected_coords(&ds_b_ref).is_empty(), "all_selected_coords(ds-b) must be empty");
    assert!(app.grid.selected_graph_point_targets(&ds_b_ref, 2).is_empty(), "graph point targets for ds-b must be empty");
    assert!(resolve_grid_selection(&app, &ds_a_ref).is_some(), "Sheet A's selection must still resolve on Sheet A");

    // Render settled frame with Trailing Studio pinned to Sheet B: Exclude button must be disabled.
    let tokens = default_tokens(&app);
    let st = ShellTestState::new(app).with_commit_all().with_sync_raster();
    let mut driver = make_shell_driver_with_state(st, ViewportMetrics::new(1440.0, 900.0, 1.0), tokens);
    driver.run_settled();

    let excl_btn = driver.frame_meta().find_by_widget_id("inspector.table.exclude_subset").expect("exclude button");
    assert!(!excl_btn.enabled, "Exclude Selected Subset pinned to Sheet B must be disabled when only Sheet A has selection");
    assert_eq!(driver.state().views.trailing_studio.selected_subset_rows_summary, "No cells selected in this table");

    // Now select cell (2, 0, 0) in Sheet B -> Exclude button becomes enabled and excludes ONLY Sheet B's (2, 0, 0).
    driver.state_mut().app.grid.select_single_for_sheet(&SheetId::new("ds-b"), GridCellCoord::new(2, 0, 0));
    driver.run_settled();
    let excl_btn_b = driver.frame_meta().find_by_widget_id("inspector.table.exclude_subset").expect("exclude button enabled");
    assert!(excl_btn_b.enabled, "Exclude Selected Subset must enable once Sheet B owns the selection");
    driver.click_widget_id("inspector.table.exclude_subset")?;

    let doc_after = driver.state().app.document();
    let b_after = doc_after.data_sheet(&SheetId::new("ds-b")).expect("ds-b");
    let a_after = doc_after.data_sheet(&SheetId::new("ds-a")).expect("ds-a");
    assert!(b_after.cell(CellCoord::new(2, 0, 0)).is_some_and(|c| c.excluded), "Sheet B (2,0,0) should be excluded");
    assert!(b_after.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| !c.excluded), "Sheet B (0,0,0) must not be excluded");
    assert!(a_after.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| !c.excluded), "Sheet A must remain untouched");
    Ok(())
}

#[test]
fn gap_07_04_open_import_project_mode_dialog_refuses_to_clobber_active_modal() -> BoxRes {
    let mut doc = ProjectDocument::new("Modal Guard")?;
    doc.add_data_sheet(make_col_sheet("ds-1", "Data", &[&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]])?)?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    let rs = app.execute_command("analysis.run", json!({ "source_sheet_id": "ds-1", "kind": "one_way_anova", "sheet_id": "rs-1", "alpha": 0.05 }))?;
    let rs_id = SheetId::new(rs.payload["sheet_id"].as_str().expect("rs_id"));
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &rs_id));
    assert!(app.has_open_modal());

    let opened = app.open_import_project_mode_dialog("table.importPrism", "incoming.prism", false);
    assert!(!opened, "open_import_project_mode_dialog must refuse to overwrite an open modal");
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { ref result_sheet_id } if *result_sheet_id == rs_id));
    assert!(app.canvas_dialogs.edit_params.draft.is_some());
    Ok(())
}

#[test]
fn gap_07_06_layout_panel_size_drag_value_multi_frame_drag_produces_one_undo_entry() -> BoxRes {
    let mut doc = ProjectDocument::new("Drag Undo Coalesce")?;
    let ds = make_col_sheet("ds-1", "Data", &[&[1.0, 2.0], &[3.0, 4.0]])?;
    let g = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    let mut lay = LayoutSheet::new("lay-1", "Layout 1", LayoutSpec::default())?;
    lay.spec.items = vec![make_live_graph_panel("p0", "g-1", 20.0, 20.0, 200.0, 150.0)];
    doc.add_data_sheet(ds)?;
    doc.add_graph_sheet(g)?;
    doc.add_layout_sheet(lay)?;
    doc.active_sheet_id = Some(SheetId::new("lay-1"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app.nav.inspector_open = true;
    let tokens = default_tokens(&app);

    let st = ShellTestState::new(app).with_commit_all().with_sync_raster();
    let mut driver = make_shell_driver_with_state(st, ViewportMetrics::new(1440.0, 900.0, 1.0), tokens);
    driver.run_settled();

    let w_rect = driver.frame_meta().find_by_widget_id("inspector.layout.panel.width_pt").expect("width DragValue").clipped_rect;
    let start = w_rect.center();
    let undo_before = driver.state().app.session().undo_depth();

    // Press and drag across 6 intermediate frames before releasing.
    driver.dispatch_events(vec![
        Event::PointerMoved(start),
        Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    for step in 1..=6 {
        let pos = pos2(start.x + (step as f32) * 8.0, start.y);
        driver.dispatch_events(vec![Event::PointerMoved(pos)]);
        assert_eq!(driver.state().app.session().undo_depth(), undo_before, "intermediate drag frame {step} must NOT push an undo entry");
    }

    // Live preview draft must be active with updated width before release.
    let preview_w = driver.state().views.trailing_studio.panel_size_draft.as_ref().map(|d| d.width_pt).expect("active drag preview draft");
    assert!(preview_w > 200.0, "expected live preview width > 200.0 pt during drag, got {preview_w}");

    // Release pointer -> commits final width in a single undo entry.
    let end = pos2(start.x + 48.0, start.y);
    driver.dispatch_events(vec![Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    driver.run_settled();

    assert_eq!(driver.state().app.session().undo_depth(), undo_before + 1, "multi-frame width drag must produce exactly 1 undo entry");
    let final_w = match &driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).expect("lay-1").spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt.width,
        _ => panic!("expected LiveGraph"),
    };
    assert!((final_w - preview_w).abs() < 1e-6 && final_w > 200.0, "final width {final_w} should match preview {preview_w}");
    assert!(driver.state().views.trailing_studio.panel_size_draft.is_none(), "draft must clear after commit");

    // A single Undo must restore the exact original 200.0 pt width.
    let _ = driver.state_mut().app.execute_command("edit.undo", json!({}))?;
    let restored_w = match &driver.state().app.document().layout_sheet(&SheetId::new("lay-1")).expect("lay-1").spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt.width,
        _ => panic!("expected LiveGraph"),
    };
    assert_eq!(restored_w, 200.0, "single undo must restore initial 200.0 pt panel width");
    Ok(())
}

#[test]
fn gap_07_02_cross_sheet_cmd_e_and_row_gutter_exclude_respect_sheet_ownership() -> BoxRes {
    // 1. `Cmd/Ctrl+E` shortcut (`interaction::toggle_exclude_active_grid_cells`) must never fall back to another sheet's `tool.active` / `active_x_cell`.
    let mut doc_xy = ProjectDocument::new("Cross Sheet Shortcut Exclude")?;
    let xy_a = make_xy_sheet("xy-a", "XY A", &[(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)])?;
    let xy_b = make_xy_sheet("xy-b", "XY B", &[(5.0, 50.0), (6.0, 60.0), (7.0, 70.0)])?;
    doc_xy.add_data_sheet(xy_a)?;
    doc_xy.add_data_sheet(xy_b)?;
    doc_xy.active_sheet_id = Some(SheetId::new("xy-b"));

    let mut app_xy = UiApp::from_session(Session::from_document(doc_xy));
    app_xy.nav.welcome.open = false;
    app_xy.egui_views.launcher_open = false;

    // Selection bound to `xy-a` (Y cell) while `xy-b` is active -> Ctrl+E must be a no-op.
    app_xy.grid.select_single_for_sheet(&SheetId::new("xy-a"), GridCellCoord::new(1, 0, 0));
    let out_y = dispatch_ui_event(
        &mut app_xy,
        UiInputEvent::KeyDown { key: String::from("e"), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        ViewportMetrics::default(),
    )?;
    assert!(!out_y.handled, "Ctrl+E on xy-b must not exclude xy-a's active Y cell");
    assert!(app_xy.document().data_sheet(&SheetId::new("xy-b")).expect("xy-b").cell(CellCoord::new(1, 0, 0)).is_some_and(|c| !c.excluded));

    // Selection bound to `xy-a` (X cell) while `xy-b` is active -> Ctrl+E must be a no-op.
    app_xy.grid.select_x_cell_for_sheet(&SheetId::new("xy-a"), 1, 0);
    let out_x = dispatch_ui_event(
        &mut app_xy,
        UiInputEvent::KeyDown { key: String::from("e"), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        ViewportMetrics::default(),
    )?;
    assert!(!out_x.handled, "Ctrl+E on xy-b must not exclude xy-a's active X cell");
    assert!(app_xy.document().data_sheet(&SheetId::new("xy-b")).expect("xy-b").x_or_time_cell(1, 0).is_some_and(|c| !c.excluded));

    // Selection bound to `xy-b` (X cell) -> Ctrl+E excludes ONLY `xy-b` X cell (1, 0).
    app_xy.grid.select_x_cell_for_sheet(&SheetId::new("xy-b"), 1, 0);
    let out_own = dispatch_ui_event(
        &mut app_xy,
        UiInputEvent::KeyDown { key: String::from("e"), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        ViewportMetrics::default(),
    )?;
    assert!(out_own.handled, "Ctrl+E should exclude active X cell when owned by active sheet xy-b");
    assert!(app_xy.document().data_sheet(&SheetId::new("xy-b")).expect("xy-b").x_or_time_cell(1, 0).is_some_and(|c| c.excluded));
    assert!(app_xy.document().data_sheet(&SheetId::new("xy-a")).expect("xy-a").x_or_time_cell(1, 0).is_some_and(|c| !c.excluded));

    // 2. Row-gutter right-click (`handle_grid_ctx_exclude`) in split view must target clicked row `3` of `ds-b`, never stale `ds-a` `(0, 1, 0)`.
    let mut doc = ProjectDocument::new("Row Gutter Split Exclude")?;
    let ds_a = make_col_sheet("ds-a", "Sheet A", &[&[10.0, 20.0, 30.0], &[40.0, 50.0, 60.0]])?;
    let ds_b = make_col_sheet("ds-b", "Sheet B", &[&[1.0, 2.0, 3.0, 4.0], &[5.0, 6.0, 7.0, 8.0]])?;
    doc.add_data_sheet(ds_a)?;
    doc.add_data_sheet(ds_b)?;
    doc.active_sheet_id = Some(SheetId::new("ds-b"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new("ds-a"));
    app.split.secondary_sheet_id = Some(SheetId::new("ds-b"));
    app.grid.select_single_for_sheet(&SheetId::new("ds-a"), GridCellCoord::new(0, 1, 0));

    let tokens = default_tokens(&app);
    let mut st = ShellTestState::new(app).with_commit_all().with_sync_raster();
    st.views.center_stage.focused_split_pane = SplitPaneSlot::Secondary;
    let mut driver = make_shell_driver_with_state(st, ViewportMetrics::new(1440.0, 900.0, 1.0), tokens);
    driver.run_settled();

    let row3_pos = driver.frame_meta().find_by_widget_id("grid.row_num.3").expect("ds-b row 3 gutter").clipped_rect.center();
    driver.dispatch_events(vec![
        Event::PointerMoved(row3_pos),
        Event::PointerButton { pos: row3_pos, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: row3_pos, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    driver.run_settled();

    driver.click_widget_id("grid.ctx.exclude")?;
    let doc_after = driver.state().app.document();
    let b_after = doc_after.data_sheet(&SheetId::new("ds-b")).expect("ds-b");
    let a_after = doc_after.data_sheet(&SheetId::new("ds-a")).expect("ds-a");
    assert!(b_after.cell(CellCoord::new(3, 0, 0)).is_some_and(|c| c.excluded), "clicked row 3 on ds-b must be excluded");
    assert!(b_after.cell(CellCoord::new(0, 1, 0)).is_some_and(|c| !c.excluded), "ds-b (0, 1, 0) matching ds-a selection must remain unexcluded");
    assert!(a_after.cell(CellCoord::new(0, 1, 0)).is_some_and(|c| !c.excluded), "ds-a (0, 1, 0) must remain unexcluded");
    Ok(())
}
