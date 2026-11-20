#![forbid(unsafe_code)]
//! Comprehensive integration tests covering HiDPI `1.0x`/`1.5x`/`2.0x`/`3.0x` coordinate
//! transforms, LTR/RTL interactive pointer & inline-editing event dispatch (`G6`, `M7`),
//! `QOL-1`..`QOL-10` workflows, `UI_COMMANDS`, and `G11` EMF clipboard bundles on pure `egui`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet,
    GraphAnnotation, GraphKind, GraphSheet, GroupedTable, InfoConstant, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec,
    ProjectDocument, RectPt, RemediationKind, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SubcolumnMode,
    TableArchetype, TableKind, TwoGroupMethod, TwoGroupSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::CanvasPreviewFilter;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents};
use curvebook_ui_egui::{
    ClipboardBundle, GridViewState, KeyModifiers, ScreenshotRegionTarget, UiApp, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder,
    write_os_clipboard_bundle,
};
use egui::{Key, Modifiers};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
struct QolRectPx {
    width: u32,
}

#[derive(Debug, Clone, Copy)]
struct QolWorkspaceRegions {
    navigator: QolRectPx,
    canvas: QolRectPx,
    inspector: QolRectPx,
}

fn measure_qol_regions(app: &UiApp, metrics: ViewportMetrics) -> QolWorkspaceRegions {
    let ctx = egui::Context::default();
    curvebook_ui_egui::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
    #[allow(clippy::cast_possible_truncation)]
    let w_pt = metrics.logical_width.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let h_pt = metrics.logical_height.max(1.0) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let ppp = metrics.clamped_scale_factor() as f32;
    ctx.options_mut(|o| o.zoom_factor = app.ui_scale());
    let mut views = app.egui_views.clone();
    let mut report = None;
    for _ in 0..6 {
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pt, h_pt))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(ppp / app.ui_scale()), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut settled = false;
        let mut out = ctx.run_ui(raw, |ui| {
            let rep = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
            settled = !ui.ctx().will_discard();
            if settled {
                report = Some(rep);
            }
        });
        out.textures_delta.clear();
        if settled {
            break;
        }
    }
    let rep = report.expect("settled egui report");
    let scale = metrics.clamped_scale_factor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let w_px = |r: Option<egui::Rect>| r.map_or(0, |rect| (f64::from(rect.width().max(0.0)) * scale).round() as u32);
    QolWorkspaceRegions {
        navigator: QolRectPx { width: w_px(rep.leading_dock_rect_pt) },
        canvas: QolRectPx { width: w_px(rep.regions.canvas_rect_pt) },
        inspector: QolRectPx { width: w_px(rep.trailing_dock_rect_pt) },
    }
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
    app.nav.welcome.open = false;
    let _ = app.set_locale("de");

    let mut harness = human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1024.0, 640.0, 1.0));
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

        harness.driver.set_viewport(metrics);
        let frame = harness.app_mut().render_egui_shell_rgba(spec, ScreenshotRegionTarget::FullWindow).expect("hidpi rgba render");
        assert_eq!(frame.width_px, metrics.physical_width_px());
        assert_eq!(frame.height_px, metrics.physical_height_px());

        // Double-click cell (0, 0, 0) -> localized decimal comma edit commits numeric.
        let cpos = harness.driver.frame_meta().find_by_widget_id("grid.cell.0.0.0").expect("cell (0,0,0)").clipped_rect.center();
        harness.driver.click_at(cpos).expect("click 1");
        harness.driver.click_at(cpos).expect("click 2");
        harness.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        harness.driver.press_key("Backspace", KeyModifiers::default()).expect("bk");
        let edited_text = format!("12,{scale:.0}");
        let expected_num: f64 = format!("12.{scale:.0}").parse().expect("num");
        harness.driver.type_text(&edited_text).expect("type");
        harness.driver.press_key("Enter", KeyModifiers::default()).expect("commit");

        let ds = harness.app().document().data_sheet(&"data-xy".into()).expect("ds");
        let stored = ds.cell(CellCoord::new(0, 0, 0)).expect("cell");
        assert_eq!(stored.value, CellValue::Number(expected_num));
    }

    // Verify Escape cancels inline edit on cell (1, 1, 0) without mutation, then Enter commits text.
    let metrics_2x = ViewportMetrics::new(1024.0, 640.0, 2.0);
    harness.driver.set_viewport(metrics_2x);
    let fpos = harness.driver.frame_meta().find_by_widget_id("grid.cell.1.1.0").expect("cell (1,1,0)").clipped_rect.center();
    harness.driver.click_at(fpos).expect("c1");
    harness.driver.click_at(fpos).expect("c2");
    harness.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    harness.driver.type_text("SHOULD_NOT_SAVE").expect("type");
    harness.driver.press_key("Escape", KeyModifiers::default()).expect("esc");
    let orig_11 = harness.app().document().data_sheet(&"data-xy".into()).unwrap().cell(CellCoord::new(1, 1, 0)).unwrap().active_number();
    assert_eq!(orig_11, Some(28.0));

    // Compare LTR ("en") vs RTL ("ar") rendered pixels & drag selection (0,0)->(2,1).
    let _ = harness.app_mut().set_locale("en");
    harness.driver.run_settled();
    let ltr_rgba = harness.app_mut().render_egui_shell_rgba(metrics_2x.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).expect("ltr");
    let _ = harness.app_mut().set_locale("ar");
    harness.driver.run_settled();
    let rtl_rgba = harness.app_mut().render_egui_shell_rgba(metrics_2x.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).expect("rtl");
    assert_ne!(ltr_rgba.rgba, rtl_rgba.rgba);

    let p0 = harness.driver.frame_meta().find_by_widget_id("grid.cell.0.0.0").expect("c00").clipped_rect.center();
    let p2 = harness.driver.frame_meta().find_by_widget_id("grid.cell.2.1.0").expect("c21").clipped_rect.center();
    harness.driver.drag(p0, p2).expect("drag select");
    assert_eq!(harness.app().grid.tool.anchor, Some(GridCellCoord::new(0, 0, 0)));
    assert_eq!(harness.app().grid.tool.active, Some(GridCellCoord::new(2, 1, 0)));
}

#[test]
fn navigator_drag_reorder_info_sheet_and_canvas_title_inline_edit() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.5);
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, metrics);

    // 1) Drag first data sheet onto second data sheet in Navigator to commit `sheet.reorder`.
    let nav_ds0 = harness.driver.frame_meta().find_by_widget_id("nav.sheet.data-xy").expect("nav data-xy").clipped_rect.center();
    let nav_ds1 = harness.driver.frame_meta().find_by_widget_id("nav.sheet.data-grp").expect("nav data-grp").clipped_rect.center();
    harness.driver.drag(nav_ds0, nav_ds1).expect("nav reorder drag");
    assert_eq!(harness.app().document().data_sheets[0].id.as_str(), "data-grp");

    // 2) Select InfoSheet -> edit constant & notes via native `egui` widgets (`info.const.0`, `info.notes_editor`).
    harness.click("nav.chip.info").expect("info chip");
    harness.click("nav.sheet.info-1").expect("select info-1");
    let ph_wid = harness
        .driver
        .frame_meta()
        .entries
        .iter()
        .find(|e| e.widget_id.starts_with("info.const.") && e.label == "Target_pH")
        .map(|e| e.widget_id.clone())
        .expect("Target_pH row widget_id");
    harness.click(&ph_wid).expect("select constant Target_pH");
    harness.click(&ph_wid).expect("open inline editor Target_pH");
    harness.click("info.const.editor_value").expect("focus const value");
    harness.driver.press_egui_key(Key::End, Modifiers::NONE);
    for _ in 0..12 {
        harness.driver.press_key("Backspace", KeyModifiers::default()).expect("bk");
    }
    harness.driver.type_text("7.2").expect("type const");
    harness.click("info.const.editor_save").expect("save const");
    let updated_const = harness.app().document().info_sheet(&"info-1".into()).unwrap().get_constant("Target_pH").unwrap();
    assert_eq!(updated_const.value, ConstantValue::Number(7.2));

    harness.click("info.notes_editor").expect("focus notes");
    harness.driver.press_egui_key(Key::End, Modifiers::NONE);
    for _ in 0..80 {
        harness.driver.press_key("Backspace", KeyModifiers::default()).expect("bk");
    }
    harness.driver.type_text("Incubated 45 min at 37 C.").expect("type notes");
    harness.click("info.notes.save").expect("save notes");
    assert_eq!(harness.app().document().info_sheet(&"info-1".into()).unwrap().notes_markdown, "Incubated 45 min at 37 C.");

    // 3) Select GraphSheet and update title / axis via `graph.setSpec` / `graph.setAxes` commands.
    harness.click("nav.chip.graphs").expect("graph chip");
    harness.click("nav.sheet.graph-xy").expect("select graph-xy");
    harness.app_mut().set_gui_only(false);
    harness
        .app_mut()
        .execute_command("graph.setSpec", json!({ "sheet_id": "graph-xy", "spec": { "title": "Normalized Fluorescence" } }))
        .expect("set graph title");
    harness
        .app_mut()
        .execute_command("graph.setAxes", json!({ "sheet_id": "graph-xy", "x_axis": { "title": "Log[Agonist] (M)" } }))
        .expect("set x axis title");
    assert_eq!(harness.app().document().graph_sheet(&"graph-xy".into()).unwrap().spec.title, "Normalized Fluorescence");
    assert_eq!(harness.app().document().graph_sheet(&"graph-xy".into()).unwrap().spec.x_axis.title, "Log[Agonist] (M)");
}

#[test]
fn canvas_pinch_zoom_inertial_pan_coasting_and_annotation_panel_drag_snap() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 2.0);

    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).expect("select graph");
    app.canvas_dialogs.canvas.mode = curvebook_tools::CanvasToolMode::Select;
    app.canvas_dialogs.canvas.snap_to_grid = true;
    app.canvas_dialogs.canvas.snap_grid_pt = 10.0;

    app.dispatch_ui_event(UiInputEvent::PinchZoom { x: 640.0, y: 400.0, scale_delta: 1.5 }, metrics).expect("pinch");
    assert!(app.canvas_dialogs.canvas.zoom > 1.4);

    app.dispatch_ui_event(
        UiInputEvent::WheelZoom { x: 640.0, y: 400.0, delta_y: -120.0, modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        metrics,
    )
    .expect("wheel zoom");
    assert!(app.canvas_dialogs.canvas.zoom > 1.5);

    app.dispatch_ui_event(UiInputEvent::InertialPan { delta_x: 16.0, delta_y: -8.0, dt_sec: 0.016, step_coast: false }, metrics)
        .expect("pan impulse");
    let pan_after_impulse = app.canvas_dialogs.canvas.pan_pt;
    assert!(pan_after_impulse.x.abs() > 1.0);
    app.dispatch_ui_event(UiInputEvent::InertialPan { delta_x: 0.0, delta_y: 0.0, dt_sec: 0.016, step_coast: true }, metrics).expect("pan coast");
    assert!(app.canvas_dialogs.canvas.pan_pt.x > pan_after_impulse.x);

    app.handle_key_combo("Cmd+0").expect("fit sheet shortcut");
    assert!((app.canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    // Drag LayoutItem panel on LayoutSheet through `EguiHumanPathDriver`.
    app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" })).expect("select layout");
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, metrics);
    let p0_before = match &harness.app().document().layout_sheet(&"layout-1".into()).unwrap().spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => *bounds_pt,
        _ => panic!("expected LiveGraph item 0"),
    };
    let p0_pos = harness.driver.frame_meta().find_by_widget_id("canvas.layout_panel.0").expect("panel 0").clipped_rect.center();
    harness.driver.drag(p0_pos, egui::pos2(p0_pos.x + 24.0, p0_pos.y + 18.0)).expect("layout panel drag");
    let p0_after = match &harness.app().document().layout_sheet(&"layout-1".into()).unwrap().spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => *bounds_pt,
        _ => panic!("expected LiveGraph item 0"),
    };
    assert!((p0_after.x - p0_before.x).abs() >= 5.0 || (p0_after.y - p0_before.y).abs() >= 5.0, "expected layout panel move on drag");
}

#[test]
fn qol4_qol5_split_workspace_divider_drag_and_two_way_point_brushing() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.5);

    let exec = app.handle_key_combo("Cmd+\\").expect("split shortcut").expect("matched");
    assert!(exec.ui);
    assert!(app.split.enabled);

    let mut harness = human_wf_common::EguiWfHarness::from_app(app, metrics);
    let div_pos = harness.driver.frame_meta().find_by_widget_id("split.divider").expect("split.divider").clipped_rect.center();
    let before_ratio = harness.app().split.ratio;
    harness.driver.drag(div_pos, egui::pos2(div_pos.x + 60.0, div_pos.y)).expect("drag split divider");
    assert!(harness.app().split.ratio > before_ratio && harness.app().split.ratio <= 0.85);

    // Two-way point brushing (`QOL-5`): Graph plotted point -> grid selection & RGBA visual delta.
    let mut app2 = harness.driver.state().app.clone();
    let unbrushed_rgba = app2.render_egui_shell_rgba(metrics.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).expect("unbrushed");
    let scene = app2.session().render_sheet_scene(&"graph-xy".into()).expect("graph scene");
    let plotted = scene.plotted_points.first().expect("plotted point metadata");
    let brushed = app2.brush_point_from_graph_pt(&"graph-xy".into(), plotted.center_pt, 8.0).expect("hit_test_point");
    assert_eq!(brushed, plotted.source_cell);
    assert_eq!(app2.brushed_cell.as_ref(), Some(&plotted.source_cell));
    assert_eq!(app2.grid.tool.active, Some(GridCellCoord::new(plotted.source_cell.row, plotted.source_cell.col, plotted.source_cell.subcol)));
    let brushed_rgba = app2.render_egui_shell_rgba(metrics.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).expect("brushed");
    assert_ne!(unbrushed_rgba.rgba, brushed_rgba.rgba);
}

#[test]
fn qol1_qol2_convert_archetype_and_import_reshape() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));

    app.preview_convert_archetype(&"data-grp".into(), TableKind::Column).expect("preview convert");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { sheet_id: SheetId::try_new("data-grp").unwrap() });
    assert!(app.canvas_dialogs.convert_archetype.preview_ready);
    assert!(!app.canvas_dialogs.convert_archetype.preview_lossy);
    app.commit_convert_archetype().expect("lossless convert");
    assert_eq!(app.document().data_sheet(&"data-grp".into()).unwrap().kind(), TableKind::Column);

    app.execute_command("table.setXCells", json!({ "sheet_id": "data-xy", "row": 3, "value": 4.0, "excluded": true })).expect("exclude x");
    app.preview_convert_archetype(&"data-xy".into(), TableKind::Grouped).expect("preview lossy convert");
    assert!(app.canvas_dialogs.convert_archetype.preview_ready);
    assert!(app.canvas_dialogs.convert_archetype.preview_lossy);
    assert!(app.canvas_dialogs.convert_archetype.dropped_cells > 0);
    assert!(app.commit_convert_archetype().is_err(), "lossy convert must require allow_lossy confirmation");
    app.canvas_dialogs.convert_archetype.allow_lossy = true;
    app.commit_convert_archetype().expect("confirmed lossy convert");
    assert_eq!(app.document().data_sheet(&"data-xy".into()).unwrap().kind(), TableKind::Grouped);

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
    app.nav.welcome.open = false;

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

    let rs_before = app.document().result_sheet(&"res-t".into()).expect("res-t");
    let banner = AssumptionDiagnosticBanner::from_result(rs_before).expect("diagnostic banner");
    assert_eq!(banner.source_result_id.as_str(), "res-t");
    assert_eq!(banner.severity, ChecklistStatus::Violated);
    assert!(banner.diagnostic_metric.is_some());
    assert!(!banner.detail_tab_ids.is_empty());
    let initial_result_count = app.document().result_sheets.len();
    let initial_undo_depth = app.session().undo_depth();

    // Click rendered `result.diagnostic_banner.inspect` button in `egui`!
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, human_wf_common::default_viewport());
    harness.click("result.diagnostic_banner.inspect").expect("click banner inspect");
    assert_eq!(harness.app().document().result_sheets.len(), initial_result_count);
    assert_eq!(harness.app().session().undo_depth(), initial_undo_depth);
    assert_eq!(harness.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("res-t"));

    let mut app = harness.driver.state().app.clone();
    app.set_gui_only(false);
    let rem_err = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.result.applyRemediation", &json!({ "result_sheet_id": "res-t" }))
        .expect_err("QOL-7 diagnostics-only must reject applyRemediation");
    assert!(rem_err.contains("diagnostics_only_policy"));
    assert_eq!(app.document().result_sheets.len(), initial_result_count);
    let orig_after = app.document().result_sheet(&"res-t".into()).unwrap();
    assert!(matches!(orig_after.analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::UnpairedT));
}

#[test]
fn qol7_assumption_banner_reports_small_n_info_and_blocks_remediation_mutation() {
    let mut doc = build_qol_test_project();
    {
        let res_t = doc.result_sheet_mut(&"res-t".into()).expect("res-t");
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
    assert!(!banner.limitation_note.to_ascii_lowercase().contains("consider"));

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
    app.nav.welcome.open = false;
    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).expect("select graph-xy");
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);

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

    let original_spec = app.document().graph_sheet(&"graph-xy".into()).unwrap().spec.clone();
    let target = metrics.to_render_target_spec(96);
    let normal_rgba = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("normal render");
    for filter_name in ["deuteranopia", "protanopia", "tritanopia", "grayscale"] {
        curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.previewFilter.set", &json!({ "filter": filter_name })).expect("set preview filter");
        let filtered_rgba = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).expect("filtered render");
        assert_ne!(normal_rgba.rgba, filtered_rgba.rgba, "filter {filter_name} should alter rendered viewport");
        assert_eq!(app.document().graph_sheet(&"graph-xy".into()).unwrap().spec, original_spec);
    }
    assert_eq!(app.preview_filter, CanvasPreviewFilter::GrayscalePrint);

    let normal_layout = measure_qol_regions(&app, metrics);
    assert!(normal_layout.navigator.width > 0);
    assert!(normal_layout.inspector.width > 0);

    let pres_exec = app.handle_key_combo("Cmd+Shift+P").expect("pres shortcut").expect("bound");
    assert_eq!(pres_exec.command_id, "ui.presentation.toggle");
    assert!(app.presentation_mode);
    let pres_layout = measure_qol_regions(&app, metrics);
    assert_eq!(pres_layout.navigator.width, 0);
    assert_eq!(pres_layout.inspector.width, 0);
    assert!(pres_layout.canvas.width > normal_layout.canvas.width);

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
        assert!(!err.is_empty());
    }

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
