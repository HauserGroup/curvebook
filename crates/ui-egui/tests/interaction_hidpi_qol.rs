//! Comprehensive integration tests covering HiDPI `1.0x`/`1.5x`/`2.0x`/`3.0x` coordinate
//! transforms, LTR/RTL interactive pointer & inline-editing event dispatch (`G6`, `M7`),
//! `QOL-1`..`QOL-10` workflows, `UI_COMMANDS`, and `G11` EMF clipboard bundles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet,
    GraphAnnotation, GraphKind, GraphSheet, GroupedTable, InfoConstant, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec,
    ProjectDocument, RectPt, RemediationKind, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetFolderKind, SheetId,
    SubcolumnMode, TableArchetype, TableKind, TwoGroupMethod, TwoGroupSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::CanvasPreviewFilter;
use curvebook_tools::canvas::SnapOrientation;
use curvebook_tools::{CanvasToolMode, GridCellCoord};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner, compute_workspace_layout};
use curvebook_ui_egui::{
    ClipboardBundle, GridViewState, KeyModifiers, PointerButton, UiApp, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder, WorkspacePixelRect,
    write_os_clipboard_bundle,
};
use serde_json::{Value, json};

fn rect_center_logical(rect: WorkspacePixelRect, metrics: ViewportMetrics) -> [f64; 2] {
    let cx = f64::from(rect.x) + f64::from(rect.width) * 0.5;
    let cy = f64::from(rect.y) + f64::from(rect.height) * 0.5;
    metrics.physical_to_logical_px([cx, cy])
}

fn build_qol_test_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("QOL Interactive Suite").expect("project");

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).expect("xy");
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[1].title = String::from("Drug");
    let mut ds_xy = DataSheet::new("data-xy", "Dose Curve", TableArchetype::Xy(xy)).expect("ds xy");
    for (row, (x, y1, y2)) in [(1.0, 10.0, 22.0), (2.0, 14.0, 28.0), (3.0, 18.0, 35.0), (4.0, 21.0, 41.0)].into_iter().enumerate() {
        ds_xy.set_x_or_time_cell(row, 0, Cell::number(x)).expect("x cell");
        ds_xy.set_cell(CellCoord::new(row, 0, 0), Cell::number(y1)).expect("y1");
        ds_xy.set_cell(CellCoord::new(row, 1, 0), Cell::number(y2)).expect("y2");
    }
    doc.add_data_sheet(ds_xy).expect("add xy");

    let grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 4).expect("grouped");
    let mut ds_grp = DataSheet::new("data-grp", "Cohort Assay", TableArchetype::Grouped(grp)).expect("ds grp");
    for r in 0..3 {
        ds_grp.set_cell(CellCoord::new(r, 0, 0), Cell::number(5.0 + r as f64)).expect("g0");
        ds_grp.set_cell(CellCoord::new(r, 0, 1), Cell::number(5.5 + r as f64)).expect("g0b");
        ds_grp.set_cell(CellCoord::new(r, 1, 0), Cell::number(11.0 + r as f64)).expect("g1");
    }
    doc.add_data_sheet(ds_grp).expect("add grp");

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).expect("col");
    col.data_sets[0].title = String::from("Ctrl");
    col.data_sets[1].title = String::from("Treated");
    let mut ds_col = DataSheet::new("data-col", "Two Group Assay", TableArchetype::Column(col)).expect("ds col");
    for (r, (a, b)) in [(10.0, 20.0), (11.0, 35.0), (10.5, 18.0), (10.2, 42.0), (10.8, 29.0)].into_iter().enumerate() {
        ds_col.set_cell(CellCoord::new(r, 0, 0), Cell::number(a)).expect("c0");
        ds_col.set_cell(CellCoord::new(r, 1, 0), Cell::number(b)).expect("c1");
    }
    doc.add_data_sheet(ds_col).expect("add col");

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Protocol").expect("info");
    info.constants.push(InfoConstant::new("Target_pH", ConstantValue::Number(7.4), Some(String::from("pH"))).expect("const"));
    info.notes_markdown = String::from("Initial buffer protocol.");
    doc.add_info_sheet(info).expect("add info");

    let mut spec = AnalysisSpec::new("res-t", "data-col", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).expect("spec");
    spec.selected_data_sets = vec![0, 1];
    let mut res = ResultSheet::new(spec).expect("res");
    res.status = ResultStatus::UpToDate;
    res.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Tabular Results"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("1")],
        rows: vec![vec![ResultCell::Text(String::from("p value")), ResultCell::Number { value: 0.04, prefix: None, units: None }]],
        footnotes: vec![],
    });
    res.tabs.push(ResultTab {
        id: String::from("diagnostics"),
        title: String::from("Diagnostics"),
        kind: ResultTabKind::Diagnostics,
        column_headers: vec![String::from("Check"), String::from("Statistic")],
        row_labels: vec![String::from("1")],
        rows: vec![vec![ResultCell::Text(String::from("Levene's test")), ResultCell::Text(String::from("F = 8.42, p = 0.008"))]],
        footnotes: vec![],
    });
    res.checklist = vec![ChecklistItem {
        assumption: String::from("Equal variances (Levene's test)"),
        status: ChecklistStatus::Violated,
        diagnostic_metric: Some(String::from("F = 8.42, p = 0.008")),
        explanation: String::from("Group variance spread dispersion differed across samples; consider Welch's t test."),
        remediation: Some(RemediationKind::WelchT),
    }];
    doc.add_result_sheet(res).expect("add res");

    let mut graph = GraphSheet::new_uninitialized_for_data("graph-xy", doc.data_sheet(&"data-xy".into()).expect("xy")).expect("graph");
    graph.initialized = true;
    graph.title = String::from("Response Graph");
    graph.spec.kind = GraphKind::Xy;
    graph.spec.title = String::from("Response Curve");
    graph.spec.x_axis.title = String::from("Dose");
    graph.spec.y_axis.title = String::from("Signal");
    graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(140.0, 90.0, 90.0, 32.0),
        markdown: String::from("EC50 Note"),
        bordered: true,
    });
    doc.add_graph_sheet(graph).expect("add graph");

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::try_new("graph-xy").expect("sid"),
        bounds_pt: RectPt::new(40.0, 40.0, 180.0, 120.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("callout-b"),
        markdown: String::from("Figure Legend"),
        bounds_pt: RectPt::new(260.0, 60.0, 140.0, 60.0),
        font_size_pt: 9.0,
    });
    let layout = LayoutSheet::new("layout-1", "Composite Figure", layout_spec).expect("layout");
    doc.add_layout_sheet(layout).expect("add layout");

    doc.active_sheet_id = Some(SheetId::try_new("data-xy").expect("id"));
    doc
}

#[test]
fn hidpi_scales_and_rtl_coordinate_roundtrip_drag_select_and_inline_edit() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.set_locale("de");

    for scale in [1.0_f64, 1.5, 2.0, 3.0] {
        let metrics = ViewportMetrics::new(1024.0, 640.0, scale);
        assert_eq!(metrics.physical_width_px(), (1024.0 * scale).round() as u32);
        assert_eq!(metrics.physical_height_px(), (640.0 * scale).round() as u32);
        let spec = metrics.to_render_target_spec(96);
        assert_eq!(spec.dpi, (96.0 * scale).round() as u16);

        let sample_logical = [317.25, 241.5];
        let phys = metrics.logical_to_physical_px(sample_logical);
        let back = metrics.physical_to_logical_px(phys);
        assert!((back[0] - sample_logical[0]).abs() < 1e-9);
        assert!((back[1] - sample_logical[1]).abs() < 1e-9);

        let frame = app.render_rgba_for_metrics(metrics, 96).expect("hidpi rgba render");
        assert_eq!(frame.width_px, metrics.physical_width_px());
        assert_eq!(frame.height_px, metrics.physical_height_px());

        // Verify double-click on cell (0, 0) (`col=0` = Vehicle) -> localized decimal comma edit commits numeric!
        let map = app.layout_map(metrics);
        let grid = map.grid_geometry.as_ref().expect("grid geometry");
        let target_cell_rect = grid.cell_rect(0, 0).expect("visible cell (0,0)");
        let click_pos = rect_center_logical(target_cell_rect, metrics);

        let dbl = app
            .dispatch_ui_event(
                UiInputEvent::DoubleClick { x: click_pos[0], y: click_pos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            )
            .expect("dblclick");
        assert_eq!(dbl.action, "grid.inline_edit_begin");
        assert!(app.inline_editor.active);

        app.inline_editor.buffer.clear();
        let edited_text = format!("12,{scale:.0}");
        let expected_num: f64 = format!("12.{scale:.0}").parse().expect("num");
        app.dispatch_ui_event(UiInputEvent::TextInput { text: edited_text }, metrics).expect("text");
        let commit = app.dispatch_ui_event(UiInputEvent::CommitInlineEdit, metrics).expect("commit");
        assert_eq!(commit.command_id.as_deref(), Some("table.setCell"));

        let ds = app.document().data_sheet(&"data-xy".into()).expect("ds");
        let stored = ds.cell(CellCoord::new(0, 0, 0)).expect("cell");
        assert_eq!(stored.value, CellValue::Number(expected_num));
    }

    // Verify formula inline-edit (`=A1*2`) on `(1, 1)` (`col=1`) & CancelInlineEdit preserves existing cell.
    let metrics_2x = ViewportMetrics::new(1024.0, 640.0, 2.0);
    let map_2x = app.layout_map(metrics_2x);
    let formula_rect = map_2x.grid_geometry.as_ref().unwrap().cell_rect(1, 1).expect("cell (1,1)");
    let fpos = rect_center_logical(formula_rect, metrics_2x);
    app.dispatch_ui_event(
        UiInputEvent::DoubleClick { x: fpos[0], y: fpos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics_2x,
    )
    .expect("dblclick formula");
    app.inline_editor.buffer.clear();
    app.dispatch_ui_event(UiInputEvent::TextInput { text: String::from("=A1*2") }, metrics_2x).expect("formula input");
    app.dispatch_ui_event(UiInputEvent::CommitInlineEdit, metrics_2x).expect("commit formula");
    let formula_cell = app.document().data_sheet(&"data-xy".into()).unwrap().cell(CellCoord::new(1, 1, 0)).unwrap();
    assert_eq!(formula_cell.value, CellValue::Text(String::from("=A1*2")));

    app.dispatch_ui_event(
        UiInputEvent::DoubleClick { x: fpos[0], y: fpos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics_2x,
    )
    .expect("reopen edit");
    app.dispatch_ui_event(UiInputEvent::TextInput { text: String::from("SHOULD_NOT_SAVE") }, metrics_2x).expect("type");
    app.dispatch_ui_event(UiInputEvent::CancelInlineEdit, metrics_2x).expect("cancel");
    assert!(!app.inline_editor.active);
    let unchanged = app.document().data_sheet(&"data-xy".into()).unwrap().cell(CellCoord::new(1, 1, 0)).unwrap();
    assert_eq!(unchanged.value, CellValue::Text(String::from("=A1*2")));

    // Compare LTR ("en") vs RTL ("ar") navigator/inspector mirror & grid column order + drag-select!
    app.set_locale("en");
    let ltr_map = app.layout_map(metrics_2x);
    let ltr_c0 = ltr_map.grid_geometry.as_ref().unwrap().cell_rect(0, 0).unwrap();
    let ltr_c1 = ltr_map.grid_geometry.as_ref().unwrap().cell_rect(0, 1).unwrap();
    assert!(!ltr_map.is_rtl);
    assert!(ltr_map.frame.navigator.x < ltr_map.frame.inspector.x);
    assert!(ltr_c0.x < ltr_c1.x);

    app.set_locale("ar");
    let rtl_map = app.layout_map(metrics_2x);
    let rtl_c0 = rtl_map.grid_geometry.as_ref().unwrap().cell_rect(0, 0).unwrap();
    let rtl_c1 = rtl_map.grid_geometry.as_ref().unwrap().cell_rect(0, 1).unwrap();
    let rtl_c2 = rtl_map.grid_geometry.as_ref().unwrap().cell_rect(2, 1).unwrap();
    assert!(rtl_map.is_rtl);
    assert!(rtl_map.frame.navigator.x > rtl_map.frame.inspector.x);
    assert!(rtl_c0.x > rtl_c1.x);

    let p0 = rect_center_logical(rtl_c0, metrics_2x);
    let p2 = rect_center_logical(rtl_c2, metrics_2x);
    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: p0[0], y: p0[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics_2x,
    )
    .expect("rtl down");
    app.dispatch_ui_event(UiInputEvent::PointerMove { x: p2[0], y: p2[1], modifiers: KeyModifiers::default() }, metrics_2x).expect("rtl drag");
    app.dispatch_ui_event(
        UiInputEvent::PointerUp { x: p2[0], y: p2[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics_2x,
    )
    .expect("rtl up");
    assert_eq!(app.grid.tool.anchor, Some(GridCellCoord::new(0, 0, 0)));
    assert_eq!(app.grid.tool.active, Some(GridCellCoord::new(2, 1, 0)));
}

#[test]
fn navigator_drag_reorder_info_sheet_and_canvas_title_inline_edit() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.5);

    // 1) Drag first data sheet onto second data sheet in Navigator to commit `sheet.reorder`.
    let map = app.layout_map(metrics);
    let nav_ds0 = map.navigator_entries.iter().find(|e| e.folder == SheetFolderKind::Data && e.index_in_folder == 0).expect("ds0 entry");
    let nav_ds1 = map.navigator_entries.iter().find(|e| e.folder == SheetFolderKind::Data && e.index_in_folder == 1).expect("ds1 entry");
    assert_eq!(nav_ds0.sheet_id.as_str(), "data-xy");
    assert_eq!(nav_ds1.sheet_id.as_str(), "data-grp");

    let start = rect_center_logical(nav_ds0.rect, metrics);
    let end = rect_center_logical(nav_ds1.rect, metrics);
    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: start[0], y: start[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("nav down");
    app.dispatch_ui_event(UiInputEvent::PointerMove { x: end[0], y: end[1], modifiers: KeyModifiers::default() }, metrics).expect("nav hover");
    let drop_out = app
        .dispatch_ui_event(
            UiInputEvent::PointerUp { x: end[0], y: end[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("nav drop");
    assert_eq!(drop_out.command_id.as_deref(), Some("sheet.reorder"));
    assert_eq!(app.document().data_sheets[0].id.as_str(), "data-grp");

    // 2) Select InfoSheet -> click constant & notes to inline-edit cleanly.
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" })).expect("select info");
    let info_map = app.layout_map(metrics);
    let target_const = info_map.info_constant_rows.iter().find(|r| r.name == "Target_pH").expect("Target_pH row");
    let cpos = rect_center_logical(target_const.rect, metrics);
    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: cpos[0], y: cpos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("click constant");
    app.inline_editor.buffer = String::from("7.2");
    let const_commit = app.dispatch_ui_event(UiInputEvent::CommitInlineEdit, metrics).expect("commit const");
    assert_eq!(const_commit.command_id.as_deref(), Some("info.setConstant"));
    let updated_const = app.document().info_sheet(&"info-1".into()).unwrap().get_constant("Target_pH").unwrap();
    assert_eq!(updated_const.value, ConstantValue::Number(7.2));

    let notes_rect = info_map.info_notes_rect.expect("notes rect");
    let npos = rect_center_logical(notes_rect, metrics);
    app.dispatch_ui_event(
        UiInputEvent::PointerDown { x: npos[0], y: npos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("click notes");
    app.inline_editor.buffer = String::from("Incubated 45 min at 37 C.");
    let notes_commit = app.dispatch_ui_event(UiInputEvent::CommitInlineEdit, metrics).expect("commit notes");
    assert_eq!(notes_commit.command_id.as_deref(), Some("info.setNote"));
    assert_eq!(app.document().info_sheet(&"info-1".into()).unwrap().notes_markdown, "Incubated 45 min at 37 C.");

    // 3) Select GraphSheet -> double-click canvas title & x-axis title to inline-edit titles.
    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).expect("select graph");
    let gmap = app.layout_map(metrics);
    let tpos = rect_center_logical(gmap.canvas_title_rect.expect("canvas title"), metrics);
    app.dispatch_ui_event(
        UiInputEvent::DoubleClick { x: tpos[0], y: tpos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("dblclick title");
    app.inline_editor.buffer = String::from("Normalized Fluorescence");
    let gtitle_commit = app.dispatch_ui_event(UiInputEvent::CommitInlineEdit, metrics).expect("commit graph title");
    assert_eq!(gtitle_commit.command_id.as_deref(), Some("graph.setSpec"));
    assert_eq!(app.document().graph_sheet(&"graph-xy".into()).unwrap().spec.title, "Normalized Fluorescence");

    let xpos = rect_center_logical(gmap.x_axis_title_rect.expect("x axis title"), metrics);
    app.dispatch_ui_event(
        UiInputEvent::DoubleClick { x: xpos[0], y: xpos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("dblclick x axis");
    app.inline_editor.buffer = String::from("Log[Agonist] (M)");
    let xtitle_commit = app.dispatch_ui_event(UiInputEvent::CommitInlineEdit, metrics).expect("commit x axis title");
    assert_eq!(xtitle_commit.command_id.as_deref(), Some("graph.setAxes"));
    assert_eq!(app.document().graph_sheet(&"graph-xy".into()).unwrap().spec.x_axis.title, "Log[Agonist] (M)");
}

#[test]
fn canvas_pinch_zoom_inertial_pan_and_annotation_panel_drag_snap() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    let metrics = ViewportMetrics::new(1280.0, 800.0, 2.0);

    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).expect("select graph");
    app.canvas_dialogs.canvas.mode = CanvasToolMode::Select;
    app.canvas_dialogs.canvas.snap_to_grid = true;
    app.canvas_dialogs.canvas.snap_grid_pt = 10.0;

    // Test PinchZoom, WheelZoom, and InertialPan (impulse + coast).
    let gmap = app.layout_map(metrics);
    let paper = gmap.scene_paper.as_ref().expect("scene paper");
    let center = rect_center_logical(paper.paper, metrics);

    app.dispatch_ui_event(UiInputEvent::PinchZoom { x: center[0], y: center[1], scale_delta: 1.5 }, metrics).expect("pinch");
    assert!(app.canvas_dialogs.canvas.zoom > 1.4);

    app.dispatch_ui_event(UiInputEvent::WheelZoom { x: center[0], y: center[1], delta_y: -120.0, modifiers: KeyModifiers::default() }, metrics)
        .expect("wheel zoom");
    assert!(app.canvas_dialogs.canvas.zoom > 1.5);

    app.dispatch_ui_event(UiInputEvent::InertialPan { delta_x: 16.0, delta_y: -8.0, dt_sec: 0.016, step_coast: false }, metrics)
        .expect("pan impulse");
    let pan_after_impulse = app.canvas_dialogs.canvas.pan_pt;
    assert!(pan_after_impulse.x.abs() > 1.0);

    app.dispatch_ui_event(UiInputEvent::InertialPan { delta_x: 0.0, delta_y: 0.0, dt_sec: 0.016, step_coast: true }, metrics).expect("pan coast");
    assert!(app.canvas_dialogs.canvas.pan_pt.x > pan_after_impulse.x);

    // Reset camera cleanly via shortcut / command and drag the graph annotation with grid snap!
    app.handle_key_combo("Cmd+0").expect("fit sheet shortcut");
    assert!((app.canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    let gmap2 = app.layout_map(metrics);
    let paper2 = gmap2.scene_paper.as_ref().expect("paper2");
    let ann_center_pt = curvebook_ui_egui::PointPt::new(185.0, 106.0);
    let ann_px = paper2.scene_pt_to_pixel(ann_center_pt).expect("ann px");
    let ann_log = metrics.physical_to_logical_px(ann_px);
    let drag_target_px = paper2.scene_pt_to_pixel(curvebook_ui_egui::PointPt::new(208.0, 129.0)).expect("dst px");
    let drag_target_log = metrics.physical_to_logical_px(drag_target_px);

    let down_out = app
        .dispatch_ui_event(
            UiInputEvent::PointerDown { x: ann_log[0], y: ann_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("ann down");
    assert_eq!(down_out.action, "graph.annotation_drag_start");

    app.dispatch_ui_event(UiInputEvent::PointerMove { x: drag_target_log[0], y: drag_target_log[1], modifiers: KeyModifiers::default() }, metrics)
        .expect("ann drag move");
    assert!(app.canvas_dialogs.canvas.active_snap_guides.iter().any(|g| g.orientation == SnapOrientation::Vertical));

    let up_out = app
        .dispatch_ui_event(
            UiInputEvent::PointerUp {
                x: drag_target_log[0],
                y: drag_target_log[1],
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            },
            metrics,
        )
        .expect("ann drop");
    assert_eq!(up_out.command_id.as_deref(), Some("graph.setSpec"));
    let updated_ann = &app.document().graph_sheet(&"graph-xy".into()).unwrap().spec.annotations[0];
    match updated_ann {
        GraphAnnotation::TextBox { bounds_pt, .. } => {
            assert!((bounds_pt.x - 160.0).abs() < 1e-6 && (bounds_pt.y - 110.0).abs() < 1e-6, "snapped bounds: {bounds_pt:?}");
        }
        other => panic!("unexpected annotation variant: {other:?}"),
    }

    // Now test dragging a LayoutItem panel on LayoutSheet.
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" })).expect("select layout");
    let lmap = app.layout_map(metrics);
    let lpaper = lmap.scene_paper.as_ref().expect("layout paper");
    let panel_px = lpaper.scene_pt_to_pixel(curvebook_ui_egui::PointPt::new(130.0, 100.0)).expect("panel px");
    let panel_log = metrics.physical_to_logical_px(panel_px);
    let panel_dst_px = lpaper.scene_pt_to_pixel(curvebook_ui_egui::PointPt::new(150.0, 120.0)).expect("dst");
    let panel_dst_log = metrics.physical_to_logical_px(panel_dst_px);

    let l_down = app
        .dispatch_ui_event(
            UiInputEvent::PointerDown { x: panel_log[0], y: panel_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("layout down");
    assert_eq!(l_down.action, "layout.item_drag_start");
    app.dispatch_ui_event(UiInputEvent::PointerMove { x: panel_dst_log[0], y: panel_dst_log[1], modifiers: KeyModifiers::default() }, metrics)
        .expect("layout move");
    let l_up = app
        .dispatch_ui_event(
            UiInputEvent::PointerUp { x: panel_dst_log[0], y: panel_dst_log[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("layout up");
    assert_eq!(l_up.command_id.as_deref(), Some("layout.setSpec"));
}

#[test]
fn qol4_qol5_split_workspace_divider_drag_and_two_way_point_brushing() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.5);

    // Toggle split view on (`ui.split.toggle`, shortcut `Cmd+\`).
    let exec = app.handle_key_combo("Cmd+\\").expect("split shortcut").expect("matched");
    assert!(exec.ui);
    assert!(app.split.enabled);

    let split_map = app.layout_map(metrics);
    assert!(split_map.split_primary.is_some());
    let divider = split_map.split_divider.expect("split divider rect");
    assert!(split_map.split_secondary.is_some());

    // Drag divider rightward by 60 logical px -> updates split ratio within [0.2, 0.8].
    let dcenter = rect_center_logical(divider, metrics);
    let before_ratio = app.split.ratio;
    let div_down = app
        .dispatch_ui_event(
            UiInputEvent::PointerDown { x: dcenter[0], y: dcenter[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("div down");
    assert_eq!(div_down.action, "split_divider.drag_start");
    app.dispatch_ui_event(UiInputEvent::PointerMove { x: dcenter[0] + 60.0, y: dcenter[1], modifiers: KeyModifiers::default() }, metrics)
        .expect("div move");
    app.dispatch_ui_event(
        UiInputEvent::PointerUp { x: dcenter[0] + 60.0, y: dcenter[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        metrics,
    )
    .expect("div up");
    assert!(app.split.ratio > before_ratio && app.split.ratio <= 0.8);

    // Verify RTL reverses split pane horizontal ordering!
    app.set_locale("ar");
    let rtl_split = app.layout_map(metrics);
    assert!(rtl_split.split_primary.unwrap().x > rtl_split.split_secondary.unwrap().x);
    app.set_locale("en");

    // Verify two-way point brushing (`QOL-5`):
    // 1) Graph plotted point -> grid selection & brushed_cell
    // 2) Grid cell click -> brushed_cell & visual highlight difference in RGBA render
    let unbrushed_rgba = app.render_rgba_for_metrics(metrics, 96).expect("unbrushed");
    let scene = app.session().render_sheet_scene(&"graph-xy".into()).expect("graph scene");
    let plotted = scene.plotted_points.first().expect("plotted point metadata");
    let brushed =
        app.brush_point_from_graph_pt(&"graph-xy".into(), plotted.center_pt, 8.0).expect("ChartScene::hit_test_point should match plotted point");
    assert_eq!(brushed, plotted.source_cell);
    assert_eq!(app.brushed_cell.as_ref(), Some(&plotted.source_cell));
    assert_eq!(app.grid.tool.active, Some(GridCellCoord::new(plotted.source_cell.row, plotted.source_cell.col, plotted.source_cell.subcol)));

    let brushed_rgba = app.render_rgba_for_metrics(metrics, 96).expect("brushed");
    assert_ne!(unbrushed_rgba.rgba, brushed_rgba.rgba);

    // Also test clicking a visible grid cell (row=2, col=1) triggers `ui.brush.point`.
    let gcell_rect = split_map.grid_geometry.as_ref().unwrap().cell_rect(2, 1).expect("cell (2,1)");
    let gcenter = rect_center_logical(gcell_rect, metrics);
    let click_cell = app
        .dispatch_ui_event(
            UiInputEvent::PointerDown { x: gcenter[0], y: gcenter[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("grid cell brush click");
    assert_eq!(click_cell.command_id.as_deref(), Some("ui.brush.point"));
    assert_eq!(app.brushed_cell.as_ref().map(|c| (c.row, c.col)), Some((2, 1)));
}

#[test]
fn qol1_qol2_convert_archetype_and_import_reshape() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));

    // QOL-1a: Lossless conversion (Grouped -> Column).
    app.preview_convert_archetype(&"data-grp".into(), TableKind::Column).expect("preview convert");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { sheet_id: SheetId::try_new("data-grp").unwrap() });
    assert!(app.canvas_dialogs.convert_archetype.preview_ready);
    assert!(!app.canvas_dialogs.convert_archetype.preview_lossy);
    app.commit_convert_archetype().expect("lossless convert");
    assert_eq!(app.document().data_sheet(&"data-grp".into()).unwrap().kind(), TableKind::Column);

    // QOL-1b: Lossy conversion (`data-xy` with excluded X cell -> Grouped drops non-numeric X spacing unless allow_lossy = true).
    app.execute_command("table.setXCells", json!({ "sheet_id": "data-xy", "row": 3, "value": 4.0, "excluded": true })).expect("exclude x");
    app.preview_convert_archetype(&"data-xy".into(), TableKind::Grouped).expect("preview lossy convert");
    assert!(app.canvas_dialogs.convert_archetype.preview_ready);
    assert!(app.canvas_dialogs.convert_archetype.preview_lossy);
    assert!(app.canvas_dialogs.convert_archetype.dropped_cells > 0);
    assert!(app.commit_convert_archetype().is_err(), "lossy convert must require allow_lossy confirmation");
    app.canvas_dialogs.convert_archetype.allow_lossy = true;
    app.commit_convert_archetype().expect("confirmed lossy convert");
    assert_eq!(app.document().data_sheet(&"data-xy".into()).unwrap().kind(), TableKind::Grouped);
    app.execute_command("edit.undo", json!({})).expect("undo lossy convert for later QOL-3 test");
    app.execute_command("edit.undo", json!({})).expect("undo exclude x");

    // QOL-2: Plate96, Plate384, and TidyLongPivot import reshape flows.
    app.grid.import_reshape.mode = curvebook_ui_egui::ImportReshapeMode::Plate96;
    app.grid.import_reshape.title = String::from("Plate 96 Screening");
    app.grid.import_reshape.plate_values = vec![Some(10.0); 96];
    let mut roles_96 = vec![json!("Blank"); 96];
    for (idx, slot) in roles_96.iter_mut().enumerate().skip(12) {
        let dose = 10.0 * ((idx % 12) as f64 + 1.0);
        *slot = json!({ "group_name": "CompoundA", "dilution_x": dose, "replicate": (idx / 12) });
    }
    app.grid.import_reshape.plate_roles = roles_96;
    let p96_intent = app.grid.import_reshape.build_import_intent().expect("plate96 intent");
    assert_eq!(p96_intent.command_id, "table.importPlateMap");
    let p96_out = app.execute_command(&p96_intent.command_id, p96_intent.params).expect("import 96");
    assert!(app.document().data_sheets.iter().any(|s| s.id.as_str() == p96_out.payload["sheet_id"].as_str().unwrap()));

    app.grid.import_reshape.mode = curvebook_ui_egui::ImportReshapeMode::Plate384;
    app.grid.import_reshape.title = String::from("Plate 384 Check");
    app.grid.import_reshape.plate_values = vec![Some(2.0); 384];
    app.grid.import_reshape.plate_roles = vec![json!({ "group_name": "Ctrl", "dilution_x": 1.0, "replicate": 1 }); 384];
    let p384_intent = app.grid.import_reshape.build_import_intent().expect("plate384 intent");
    assert_eq!(p384_intent.params["geometry"], "Wells384");

    app.grid.import_reshape.mode = curvebook_ui_egui::ImportReshapeMode::TidyLongPivot;
    app.grid.import_reshape.title = String::from("Tidy Assay");
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.grid.import_reshape.x_col = None;
    app.grid.import_reshape.tidy_csv = String::from("Group,Value\nCtrl,10.2\nCtrl,11.0\nTreated,22.5\nTreated,24.0\n");
    let tidy_intent = app.grid.import_reshape.build_import_intent().expect("tidy intent");
    assert_eq!(tidy_intent.command_id, "table.importTidyLong");
    app.execute_command(&tidy_intent.command_id, tidy_intent.params).expect("import tidy");
}

#[test]
fn qol3_qol6_qol7_exclude_reason_report_and_remediation_sibling() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);

    // QOL-3: Auditable exclusion-with-reason (`Cmd+Shift+E` / `ui.grid.excludeWithReason`) + tooltip.
    app.execute_command("sheet.select", json!({ "sheet_id": "data-xy" })).expect("select data-xy");
    app.grid.tool.select_single(GridCellCoord::new(0, 1, 0));
    let excl_shortcut = app.handle_key_combo("Cmd+Shift+E").expect("shortcut").expect("bound");
    assert_eq!(excl_shortcut.command_id, "ui.grid.excludeWithReason");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { sheet_id: SheetId::try_new("data-xy").unwrap() });
    app.canvas_dialogs.exclude_with_reason.reason_tag = String::from("Bubble");
    app.canvas_dialogs.exclude_with_reason.note = String::from("Bubble in cuvette");
    let excl_intent = app.canvas_dialogs.exclude_with_reason.commit_intent().expect("exclude intent");
    app.execute_command(&excl_intent.command_id, excl_intent.params).expect("commit exclusion");
    app.canvas_dialogs.close_dialog();

    let ds_after_excl = app.document().data_sheet(&"data-xy".into()).unwrap();
    assert!(ds_after_excl.cell(CellCoord::new(0, 1, 0)).unwrap().excluded);
    let tooltip = GridViewState::cell_exclusion_tooltip(ds_after_excl, GridCellCoord::new(0, 1, 0)).expect("tooltip");
    assert!(tooltip.contains("Bubble"));
    assert!(tooltip.contains("Bubble in cuvette"));

    // QOL-6: Copy Methods & Statistical Reporting (`skip_os_clipboard: true` keeps pasteboard isolated).
    app.execute_command("sheet.select", json!({ "sheet_id": "res-t" })).expect("select res-t");
    for fmt in ["plain", "markdown", "latex"] {
        let rep = curvebook_ui_egui::dispatch_ui_command(
            &mut app,
            "ui.result.copyMethodsReport",
            &json!({ "result_sheet_id": "res-t", "format": fmt, "skip_os_clipboard": true }),
        )
        .expect("report ok")
        .expect("ui cmd");
        let text = rep["report"].as_str().expect("report str");
        assert!(!text.is_empty(), "empty report for format {fmt}");
    }

    // QOL-7: AssumptionDiagnosticBanner is strictly diagnostics-only: clicking banner switches to
    // the Diagnostics/Residuals tab on the SAME ResultSheet without creating sibling ResultSheets
    // or mutating ProjectDocument, and `ui.result.applyRemediation` is rejected by policy.
    let rs_before = app.document().result_sheet(&"res-t".into()).expect("res-t");
    let banner = AssumptionDiagnosticBanner::from_result(rs_before).expect("diagnostic banner");
    assert_eq!(banner.source_result_id.as_str(), "res-t");
    assert_eq!(banner.severity, ChecklistStatus::Violated);
    assert!(banner.diagnostic_metric.is_some(), "expected numeric diagnostic_metric on banner: {banner:?}");
    assert!(!banner.detail_tab_ids.is_empty(), "expected detail tab link on same ResultSheet: {banner:?}");
    assert!(
        !banner.summary.to_ascii_lowercase().contains("consider") && !banner.limitation_note.to_ascii_lowercase().contains("consider"),
        "banner must be diagnostics-only without prescribing tests: {banner:?}"
    );
    let initial_result_count = app.document().result_sheets.len();
    let initial_undo_depth = app.session().undo_depth();

    let rmap = app.layout_map(metrics);
    let brect = rmap.result_banner_rect.expect("banner rect rendered on ResultSheet");
    let bpos = rect_center_logical(brect, metrics);
    let banner_click = app
        .dispatch_ui_event(
            UiInputEvent::PointerDown { x: bpos[0], y: bpos[1], button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            metrics,
        )
        .expect("banner diagnostics click");
    assert_eq!(banner_click.command_id.as_deref(), Some("ui.result.inspectDiagnostics"));
    assert_eq!(app.document().result_sheets.len(), initial_result_count, "zero sibling ResultSheet creation");
    assert_eq!(app.session().undo_depth(), initial_undo_depth, "inspectDiagnostics must not mutate undo stack");
    assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("res-t"));
    assert_eq!(banner_click.command_result.as_ref().unwrap()["tab_id"], json!(banner.detail_tab_ids[0]));

    // Deprecated `ui.result.applyRemediation` must fail with clear `diagnostics_only_policy` error & zero mutation.
    let rem_err = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.result.applyRemediation", &json!({ "result_sheet_id": "res-t" }))
        .expect_err("QOL-7 diagnostics-only must reject applyRemediation");
    assert!(rem_err.contains("diagnostics_only_policy"));
    assert_eq!(app.document().result_sheets.len(), initial_result_count);

    // Original analysis remains untouched until user switches explicitly via Cmd+T / EditParameters.
    let orig_after = app.document().result_sheet(&"res-t".into()).unwrap();
    assert!(matches!(orig_after.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::UnpairedT));
}

#[test]
fn qol7_assumption_banner_reports_small_n_info_and_blocks_remediation_mutation() {
    let mut doc = build_qol_test_project();
    {
        let res_t = doc.result_sheet_mut(&"res-t".into()).expect("res-t");
        // Small-N / inconclusive Info checklist entry even without heuristic keywords triggers diagnostics-only banner.
        res_t.checklist = vec![ChecklistItem {
            assumption: String::from("Shapiro-Wilk normality check"),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(String::from("n = 4")),
            explanation: String::from("Inconclusive normality power due to small sample size (n = 4); consider Mann-Whitney U."),
            remediation: Some(RemediationKind::MannWhitneyU),
        }];
    }

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.execute_command("sheet.select", json!({ "sheet_id": "res-t" })).expect("select res-t");
    let rs = app.document().result_sheet(&"res-t".into()).expect("res-t");
    let banner = AssumptionDiagnosticBanner::from_result(rs).expect("banner from inconclusive small-N Info item");
    assert_eq!(banner.severity, ChecklistStatus::Info);
    assert_eq!(banner.diagnostic_metric.as_deref(), Some("n = 4"));
    assert!(
        !banner.limitation_note.to_ascii_lowercase().contains("consider"),
        "limitation_note must omit prescriptive test advice: {}",
        banner.limitation_note
    );

    let inspected = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.result.inspectDiagnostics", &json!({ "result_sheet_id": "res-t" }))
        .expect("inspectDiagnostics ok")
        .expect("handled");
    assert_eq!(inspected["mutated"], false);
    assert_eq!(inspected["tab_id"], "diagnostics");

    let initial_count = app.document().result_sheets.len();
    let err = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.result.applyRemediation", &json!({ "result_sheet_id": "res-t" }))
        .expect_err("applyRemediation must reject under QOL-7 diagnostics-only policy");
    assert!(err.contains("diagnostics_only_policy"));
    assert_eq!(app.document().result_sheets.len(), initial_count);
}

#[test]
fn qol8_qol9_qol10_presets_preflight_cvd_non_mutating_presentation_and_virtual_workspace() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).expect("select graph-xy");
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);

    // QOL-8: Apply DoubleColumn180mm canvas preset & run preflight check.
    let preset_res = curvebook_ui_egui::dispatch_ui_command(
        &mut app,
        "ui.canvas.applyPreset",
        &json!({ "sheet_id": "graph-xy", "preset": "DoubleColumn180mm", "resize_mode": "KeepTypographyPt" }),
    )
    .expect("apply preset")
    .expect("ui handled");
    assert_eq!(preset_res["preset"], "DoubleColumn180mm");
    let preflight_res = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.canvas.preflight", &json!({ "sheet_id": "graph-xy" }))
        .expect("preflight")
        .expect("handled");
    assert!(preflight_res["issue_count"].as_u64().is_some());

    // QOL-9: CVD & Grayscale viewport preview filters change rendered RGBA pixels WITHOUT mutating GraphSpec!
    let original_spec = app.document().graph_sheet(&"graph-xy".into()).unwrap().spec.clone();
    let normal_rgba = app.render_rgba_for_metrics(metrics, 96).expect("normal render");

    for filter_name in ["deuteranopia", "protanopia", "tritanopia", "grayscale"] {
        curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.previewFilter.set", &json!({ "filter": filter_name })).expect("set preview filter");
        let filtered_rgba = app.render_rgba_for_metrics(metrics, 96).expect("filtered render");
        assert_ne!(normal_rgba.rgba, filtered_rgba.rgba, "filter {filter_name} should alter rendered viewport");
        assert_eq!(
            app.document().graph_sheet(&"graph-xy".into()).unwrap().spec,
            original_spec,
            "preview filter {filter_name} must NEVER mutate GraphSpec"
        );
    }
    assert_eq!(app.preview_filter, CanvasPreviewFilter::GrayscalePrint);

    // QOL-10a: Presentation mode collapses editing chrome & enlarges canvas viewport (`Cmd+Shift+P`).
    let normal_layout = compute_workspace_layout(&app, 1024, 640);
    assert!(normal_layout.navigator.width > 0);
    assert!(normal_layout.inspector.width > 0);

    let pres_exec = app.handle_key_combo("Cmd+Shift+P").expect("pres shortcut").expect("bound");
    assert_eq!(pres_exec.command_id, "ui.presentation.toggle");
    assert!(app.presentation_mode);
    let pres_layout = compute_workspace_layout(&app, 1024, 640);
    assert_eq!(pres_layout.navigator.width, 0);
    assert_eq!(pres_layout.inspector.width, 0);
    assert!(pres_layout.canvas.width > normal_layout.canvas.width);

    // QOL-10b: VirtualWorkspaceFolder in-memory `.curvebook` archive + unpacked snapshot round-trip + path safety.
    assert!(VirtualWorkspaceFolder::normalize_rel_path("../etc/passwd").is_err());
    assert!(VirtualWorkspaceFolder::normalize_rel_path("/abs/path").is_err());
    let archive_bytes_len = app.save_to_virtual_workspace("experiment.curvebook").expect("save virtual archive");
    assert!(archive_bytes_len > 0);
    let mut reloaded_app = UiApp::new();
    reloaded_app.virtual_workspace = app.virtual_workspace.clone();
    reloaded_app.load_from_virtual_workspace("experiment.curvebook").expect("load virtual archive");
    assert_eq!(reloaded_app.document().title, app.document().title);

    let reloaded_doc = reloaded_app.document().clone();
    let snap_bytes = reloaded_app.virtual_workspace.save_unpacked_snapshot(&reloaded_doc).expect("unpacked snap");
    assert!(snap_bytes > archive_bytes_len);
    assert!(reloaded_app.virtual_workspace.contains("document.json"));
    assert!(reloaded_app.virtual_workspace.contains("manifest.json"));
    let opened_snap = reloaded_app.virtual_workspace.open_unpacked_snapshot().expect("open unpacked snap");
    assert_eq!(opened_snap.data_sheets.len(), app.document().data_sheets.len());
}

#[test]
fn g11_emf_clipboard_bundle_and_control_ui_event_dispatch() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    let bundle = ClipboardBundle::from_sheet(app.session(), &"graph-xy".into(), 144).expect("bundle");

    let emf = bundle.emf_bytes.as_ref().expect("emf_bytes populated");
    assert!(emf.len() > 88);
    assert_eq!(u32::from_le_bytes([emf[0], emf[1], emf[2], emf[3]]), 1, "EMR_HEADER record type");
    assert_eq!(u32::from_le_bytes([emf[40], emf[41], emf[42], emf[43]]), 0x464D_4520, "EMF signature ` EMF`");
    assert!(bundle.available_flavors().contains(&"emf"));
    if let Err(err) = write_os_clipboard_bundle(&bundle) {
        // Headless CI pasteboard rejection must return structured Err without panicking.
        assert!(!err.is_empty());
    }

    // Verify `ui.event.dispatch` works over JSON-lines control channel.
    let line = json!({
        "id": "ev1",
        "method": "command.run",
        "command": "ui.event.dispatch",
        "params": {
            "event": { "PinchZoom": { "x": 400.0, "y": 300.0, "scale_delta": 1.25 } },
            "metrics": { "logical_width": 1024.0, "logical_height": 640.0, "scale_factor": 2.0 }
        }
    })
    .to_string();
    let reply_raw = app.dispatch_control_line(&line);
    let reply: Value = serde_json::from_str(&reply_raw).expect("valid json reply");
    assert_eq!(reply["ok"], Value::Bool(true));
    assert_eq!(reply["result"]["handled"], Value::Bool(true));
    assert_eq!(reply["result"]["action"], json!("canvas.pinch_zoom"));
}
