//! Comprehensive integration tests covering HiDPI `1.0x`/`1.5x`/`2.0x`/`3.0x` coordinate
//! transforms, LTR/RTL interactive pointer & inline-editing event dispatch (`G6`, `M7`),
//! `QOL-1`..`QOL-10` workflows, `UI_COMMANDS`, and `G11` EMF clipboard bundles on pure `egui`.
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet,
    GraphAnnotation, GraphKind, GraphSheet, GroupedTable, InfoConstant, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec,
    ProjectDocument, RectPt, RemediationKind, ResultCell, ResultSheet, ResultStatus, ResultTabKind, SheetId, SubcolumnMode, TableArchetype,
    TableKind, TwoGroupMethod, TwoGroupSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::CanvasPreviewFilter;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner};
use curvebook_ui_egui::{
    ClipboardBundle, GridViewState, KeyModifiers, ScreenshotRegionTarget, UiApp, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder,
    dispatch_ui_command, write_os_clipboard_bundle,
};
use egui::{Key, Modifiers};
use serde_json::{Value, json};

use crate::common::{fill_col_values, measure_egui_regions, widget_center};
use crate::helpers::{call_egui_control, make_result_tab};

fn build_qol_test_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("QOL Interactive Suite").unwrap();

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).unwrap();
    fill_col_values(&mut xy.data_sets[0], "Vehicle", &[10.0, 14.0, 18.0, 21.0]);
    fill_col_values(&mut xy.data_sets[1], "Drug", &[22.0, 28.0, 35.0, 41.0]);
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
    }
    doc.add_data_sheet(DataSheet::new("data-xy", "Dose Curve", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut ds_grp =
        DataSheet::new("data-grp", "Cohort Assay", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap()))
            .unwrap();
    for r in 0..3 {
        ds_grp.set_cell(CellCoord::new(r, 0, 0), Cell::number(5.0 + r as f64)).unwrap();
        ds_grp.set_cell(CellCoord::new(r, 0, 1), Cell::number(5.5 + r as f64)).unwrap();
        ds_grp.set_cell(CellCoord::new(r, 1, 0), Cell::number(11.0 + r as f64)).unwrap();
    }
    doc.add_data_sheet(ds_grp).unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap();
    fill_col_values(&mut col.data_sets[0], "Ctrl", &[10.0, 11.0, 10.5, 10.2, 10.8]);
    fill_col_values(&mut col.data_sets[1], "Treated", &[20.0, 35.0, 18.0, 42.0, 29.0]);
    doc.add_data_sheet(DataSheet::new("data-col", "Two Group Assay", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Protocol").unwrap();
    info.constants.push(InfoConstant::new("Target_pH", ConstantValue::Number(7.4), Some("pH".into())).unwrap());
    info.notes_markdown = "Initial buffer protocol.".into();
    doc.add_info_sheet(info).unwrap();

    let mut spec = AnalysisSpec::new("res-t", "data-col", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    spec.selected_data_sets = vec![0, 1];
    let mut res = ResultSheet::new(spec).unwrap();
    res.status = ResultStatus::UpToDate;
    res.tabs = vec![
        make_result_tab(
            "summary",
            "Tabular Results",
            ResultTabKind::MainSummary,
            &["Metric", "Value"],
            &["1"],
            vec![vec![ResultCell::Text("p value".into()), ResultCell::Number { value: 0.04, prefix: None, units: None }]],
        ),
        make_result_tab(
            "diagnostics",
            "Diagnostics",
            ResultTabKind::Diagnostics,
            &["Check", "Statistic"],
            &["1"],
            vec![vec![ResultCell::Text("Levene's test".into()), ResultCell::Text("F = 8.42, p = 0.008".into())]],
        ),
    ];
    res.checklist = vec![ChecklistItem {
        assumption: "Equal variances (Levene's test)".into(),
        status: ChecklistStatus::Violated,
        diagnostic_metric: Some("F = 8.42, p = 0.008".into()),
        explanation: "Group variance spread dispersion differed across samples; consider Welch's t test.".into(),
        remediation: Some(RemediationKind::WelchT),
    }];
    doc.add_result_sheet(res).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("graph-xy", doc.data_sheet(&"data-xy".into()).unwrap()).unwrap();
    graph.initialized = true;
    graph.title = "Response Graph".into();
    graph.spec.kind = GraphKind::Xy;
    graph.spec.title = "Response Curve".into();
    graph.spec.x_axis.title = "Dose".into();
    graph.spec.y_axis.title = "Signal".into();
    graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(140.0, 90.0, 90.0, 32.0),
        markdown: "EC50 Note".into(),
        bordered: true,
    });
    doc.add_graph_sheet(graph).unwrap();

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: "panel-a".into(),
        graph_sheet_id: SheetId::new("graph-xy"),
        bounds_pt: RectPt::new(40.0, 40.0, 180.0, 120.0),
        scale_percent: 100.0,
        badge_override: Some("A".into()),
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: "callout-b".into(),
        markdown: "Figure Legend".into(),
        bounds_pt: RectPt::new(260.0, 60.0, 140.0, 60.0),
        font_size_pt: 9.0,
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Composite Figure", layout_spec).unwrap()).unwrap();
    doc.active_sheet_id = Some(SheetId::new("data-xy"));
    doc
}

fn end_backspace_and_type(h: &mut crate::human_wf_common::EguiWfHarness, wid: &str, backspaces: usize, text: &str) {
    h.click(wid).unwrap();
    h.driver.press_egui_key(Key::End, Modifiers::NONE);
    for _ in 0..backspaces {
        h.driver.press_key("Backspace", KeyModifiers::default()).unwrap();
    }
    h.driver.type_text(text).unwrap();
}

#[test]
fn hidpi_scales_and_rtl_coordinate_roundtrip_drag_select_and_inline_edit() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    assert_eq!(app.set_locale("de"), "de");

    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1024.0, 640.0, 1.0));
    for scale in [1.0_f64, 1.5, 2.0, 3.0] {
        let m = ViewportMetrics::new(1024.0, 640.0, scale);
        assert_eq!(
            (m.physical_width_px(), m.physical_height_px(), m.to_render_target_spec(96).dpi),
            ((1024.0 * scale).round() as u32, (640.0 * scale).round() as u32, (96.0 * scale).round() as u16)
        );
        let back = m.physical_to_logical_px(m.logical_to_physical_px([317.25, 241.5]));
        assert!((back[0] - 317.25).abs() < 1e-9 && (back[1] - 241.5).abs() < 1e-9);

        h.driver.set_viewport(m);
        let frame = h.app_mut().render_egui_shell_rgba(m.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).unwrap();
        assert_eq!((frame.width_px, frame.height_px), (m.physical_width_px(), m.physical_height_px()));

        h.type_replace_cell("grid.cell.0.0.0", &format!("12,{scale:.0}")).unwrap();
        let expected_num: f64 = format!("12.{scale:.0}").parse().unwrap();
        assert_eq!(
            h.app().document().data_sheet(&"data-xy".into()).unwrap().cell(CellCoord::new(0, 0, 0)).unwrap().value,
            CellValue::Number(expected_num)
        );
    }

    let metrics_2x = ViewportMetrics::new(1024.0, 640.0, 2.0);
    h.driver.set_viewport(metrics_2x);
    h.dbl_click_widget("grid.cell.1.1.0").unwrap();
    h.driver.press_egui_key(Key::A, Modifiers::COMMAND);
    h.driver.type_text("SHOULD_NOT_SAVE").unwrap();
    h.driver.press_key("Escape", KeyModifiers::default()).unwrap();
    assert_eq!(h.app().document().data_sheet(&"data-xy".into()).unwrap().cell(CellCoord::new(1, 1, 0)).unwrap().active_number(), Some(28.0));

    assert_eq!(h.app_mut().set_locale("en"), "en");
    h.driver.run_settled();
    let ltr = h.app_mut().render_egui_shell_rgba(metrics_2x.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).unwrap();
    assert_eq!(h.app_mut().set_locale("ar"), "ar");
    h.driver.run_settled();
    assert_ne!(ltr.rgba, h.app_mut().render_egui_shell_rgba(metrics_2x.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).unwrap().rgba);

    let (p0, p2) = (widget_center(&h.driver, "grid.cell.0.0.0").unwrap(), widget_center(&h.driver, "grid.cell.2.1.0").unwrap());
    h.driver.drag(p0, p2).unwrap();
    assert_eq!((h.app().grid.tool.anchor, h.app().grid.tool.active), (Some(GridCellCoord::new(0, 0, 0)), Some(GridCellCoord::new(2, 1, 0))));
}

#[test]
fn navigator_drag_reorder_info_sheet_and_canvas_title_inline_edit() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1280.0, 800.0, 1.5));

    h.driver.drag(widget_center(&h.driver, "nav.sheet.data-xy").unwrap(), widget_center(&h.driver, "nav.sheet.data-grp").unwrap()).unwrap();
    assert_eq!(h.app().document().data_sheets[0].id.as_str(), "data-grp");

    h.click("nav.chip.info").unwrap();
    h.click("nav.sheet.info-1").unwrap();
    let ph_wid =
        h.driver.frame_meta().entries.iter().find(|e| e.widget_id.starts_with("info.const.") && e.label == "Target_pH").unwrap().widget_id.clone();
    h.click(&ph_wid).unwrap();
    h.click(&ph_wid).unwrap();
    end_backspace_and_type(&mut h, "info.const.editor_value", 12, "7.2");
    h.click("info.const.editor_save").unwrap();
    assert_eq!(h.app().document().info_sheet(&"info-1".into()).unwrap().get_constant("Target_pH").unwrap().value, ConstantValue::Number(7.2));

    end_backspace_and_type(&mut h, "info.notes_editor", 80, "Incubated 45 min at 37 C.");
    h.click("info.notes.save").unwrap();
    assert_eq!(h.app().document().info_sheet(&"info-1".into()).unwrap().notes_markdown, "Incubated 45 min at 37 C.");

    h.click("nav.chip.graphs").unwrap();
    h.click("nav.sheet.graph-xy").unwrap();
    h.app_mut().execute_command("graph.setSpec", json!({ "sheet_id": "graph-xy", "spec": { "title": "Normalized Fluorescence" } })).unwrap();
    h.app_mut().execute_command("graph.setAxes", json!({ "sheet_id": "graph-xy", "x_axis": { "title": "Log[Agonist] (M)" } })).unwrap();
    let gs = h.app().document().graph_sheet(&"graph-xy".into()).unwrap();
    assert_eq!((gs.spec.title.as_str(), gs.spec.x_axis.title.as_str()), ("Normalized Fluorescence", "Log[Agonist] (M)"));
}

#[test]
fn canvas_pinch_zoom_inertial_pan_coasting_and_annotation_panel_drag_snap() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 2.0);
    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).unwrap();
    app.canvas_dialogs.canvas.mode = curvebook_tools::CanvasToolMode::Select;
    app.canvas_dialogs.canvas.snap_to_grid = true;
    app.canvas_dialogs.canvas.snap_grid_pt = 10.0;

    app.dispatch_ui_event(UiInputEvent::PinchZoom { x: 640.0, y: 400.0, scale_delta: 1.5 }, metrics).unwrap();
    assert!(app.canvas_dialogs.canvas.zoom > 1.4);
    app.dispatch_ui_event(
        UiInputEvent::WheelZoom { x: 640.0, y: 400.0, delta_y: -120.0, modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        metrics,
    )
    .unwrap();
    assert!(app.canvas_dialogs.canvas.zoom > 1.5);
    app.dispatch_ui_event(UiInputEvent::InertialPan { delta_x: 16.0, delta_y: -8.0, dt_sec: 0.016, step_coast: false }, metrics).unwrap();
    let pan0 = app.canvas_dialogs.canvas.pan_pt;
    assert!(pan0.x.abs() > 1.0);
    app.dispatch_ui_event(UiInputEvent::InertialPan { delta_x: 0.0, delta_y: 0.0, dt_sec: 0.016, step_coast: true }, metrics).unwrap();
    assert!(app.canvas_dialogs.canvas.pan_pt.x > pan0.x);
    app.handle_key_combo("Cmd+0").unwrap();
    assert!((app.canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    app.execute_command("sheet.select", json!({ "sheet_id": "layout-1" })).unwrap();
    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, metrics);
    let panel0 = |a: &UiApp| match a.document().layout_sheet(&"layout-1".into()).unwrap().spec.items[0] {
        LayoutItem::LiveGraph { bounds_pt, .. } => bounds_pt,
        _ => panic!(),
    };
    let before = panel0(h.app());
    let pos = widget_center(&h.driver, "canvas.layout_panel.0").unwrap();
    h.driver.drag(pos, egui::pos2(pos.x + 24.0, pos.y + 18.0)).unwrap();
    let after = panel0(h.app());
    assert!((after.x - before.x).abs() >= 5.0 || (after.y - before.y).abs() >= 5.0);
}

#[test]
fn qol4_qol5_split_workspace_divider_drag_and_two_way_point_brushing() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.5);
    assert!(app.handle_key_combo("Cmd+\\").unwrap().unwrap().ui && app.split.enabled);

    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, metrics);
    let div_pos = widget_center(&h.driver, "split.divider").unwrap();
    let before = h.app().split.ratio;
    h.driver.drag(div_pos, egui::pos2(div_pos.x + 60.0, div_pos.y)).unwrap();
    assert!(h.app().split.ratio > before && h.app().split.ratio <= 0.85);

    let mut app2 = h.driver.state().app.clone();
    let unbrushed = app2.render_egui_shell_rgba(metrics.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).unwrap();
    let plotted = app2.session().render_sheet_scene(&"graph-xy".into()).unwrap().plotted_points[0].clone();
    let brushed = app2.brush_point_from_graph_pt(&"graph-xy".into(), plotted.center_pt, 8.0).unwrap();
    assert_eq!((brushed.clone(), app2.brushed_cell.as_ref()), (plotted.source_cell.clone(), Some(&plotted.source_cell)));
    assert_eq!(app2.grid.tool.active, Some(GridCellCoord::new(plotted.source_cell.row, plotted.source_cell.col, plotted.source_cell.subcol)));
    assert_ne!(unbrushed.rgba, app2.render_egui_shell_rgba(metrics.to_render_target_spec(96), ScreenshotRegionTarget::FullWindow).unwrap().rgba);
}

#[test]
fn qol1_qol2_convert_archetype_and_import_reshape() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.preview_convert_archetype(&"data-grp".into(), TableKind::Column).unwrap();
    assert!(
        app.canvas_dialogs.active_dialog == ActiveDialog::ConvertArchetype { sheet_id: SheetId::new("data-grp") }
            && app.canvas_dialogs.convert_archetype.preview_ready
            && !app.canvas_dialogs.convert_archetype.preview_lossy
    );
    app.commit_convert_archetype().unwrap();
    assert_eq!(app.document().data_sheet(&"data-grp".into()).unwrap().kind(), TableKind::Column);

    app.execute_command("table.setXCells", json!({ "sheet_id": "data-xy", "row": 3, "value": 4.0, "excluded": true })).unwrap();
    app.preview_convert_archetype(&"data-xy".into(), TableKind::Grouped).unwrap();
    assert!(
        app.canvas_dialogs.convert_archetype.preview_ready
            && app.canvas_dialogs.convert_archetype.preview_lossy
            && app.canvas_dialogs.convert_archetype.dropped_cells > 0
    );
    assert!(app.commit_convert_archetype().is_err());
    app.canvas_dialogs.convert_archetype.allow_lossy = true;
    app.commit_convert_archetype().unwrap();
    assert_eq!(app.document().data_sheet(&"data-xy".into()).unwrap().kind(), TableKind::Grouped);

    app.grid.import_reshape.mode = curvebook_ui_egui::ImportReshapeMode::Plate96;
    app.grid.import_reshape.title = "Plate 96 Screening".into();
    app.grid.import_reshape.plate_values = vec![Some(10.0); 96];
    let mut roles = vec![json!("Blank"); 96];
    for (idx, slot) in roles.iter_mut().enumerate().skip(12) {
        *slot = json!({ "group_name": "CompoundA", "dilution_x": 10.0 * ((idx % 12) as f64 + 1.0), "replicate": idx / 12 });
    }
    app.grid.import_reshape.plate_roles = roles;
    let p96 = app.grid.import_reshape.build_import_intent().unwrap();
    assert_eq!(p96.command_id, "table.importPlateMap");
    let out = app.execute_command(&p96.command_id, p96.params).unwrap();
    assert!(app.document().data_sheets.iter().any(|s| s.id.as_str() == out.payload["sheet_id"].as_str().unwrap()));

    app.grid.import_reshape.mode = curvebook_ui_egui::ImportReshapeMode::Plate384;
    app.grid.import_reshape.title = "Plate 384 Check".into();
    app.grid.import_reshape.plate_values = vec![Some(2.0); 384];
    app.grid.import_reshape.plate_roles = vec![json!({ "group_name": "Ctrl", "dilution_x": 1.0, "replicate": 1 }); 384];
    assert_eq!(app.grid.import_reshape.build_import_intent().unwrap().params["geometry"], "Wells384");

    app.grid.import_reshape.mode = curvebook_ui_egui::ImportReshapeMode::TidyLongPivot;
    app.grid.import_reshape.title = "Tidy Assay".into();
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.grid.import_reshape.x_col = None;
    app.grid.import_reshape.tidy_csv = "Group,Value\nCtrl,10.2\nCtrl,11.0\nTreated,22.5\nTreated,24.0\n".into();
    let tidy = app.grid.import_reshape.build_import_intent().unwrap();
    assert_eq!(tidy.command_id, "table.importTidyLong");
    app.execute_command(&tidy.command_id, tidy.params).unwrap();
}

#[test]
fn qol3_qol6_qol7_exclude_reason_report_and_remediation_sibling() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    app.execute_command("sheet.select", json!({ "sheet_id": "data-xy" })).unwrap();
    app.grid.tool.select_single(GridCellCoord::new(0, 1, 0));
    assert_eq!(app.handle_key_combo("Cmd+Shift+E").unwrap().unwrap().command_id, "ui.grid.excludeWithReason");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { sheet_id: SheetId::new("data-xy") });
    app.canvas_dialogs.exclude_with_reason.reason_tag = "Bubble".into();
    app.canvas_dialogs.exclude_with_reason.note = "Bubble in cuvette".into();
    let excl = app.canvas_dialogs.exclude_with_reason.commit_intent().unwrap();
    app.execute_command(&excl.command_id, excl.params).unwrap();
    app.canvas_dialogs.close_dialog();

    let ds = app.document().data_sheet(&"data-xy".into()).unwrap();
    let tip = GridViewState::cell_exclusion_tooltip(ds, GridCellCoord::new(0, 1, 0)).unwrap();
    assert!(ds.cell(CellCoord::new(0, 1, 0)).unwrap().excluded && tip.contains("Bubble") && tip.contains("Bubble in cuvette"));

    app.execute_command("sheet.select", json!({ "sheet_id": "res-t" })).unwrap();
    for fmt in ["plain", "markdown", "latex"] {
        let rep = dispatch_ui_command(
            &mut app,
            "ui.result.copyMethodsReport",
            &json!({ "result_sheet_id": "res-t", "format": fmt, "skip_os_clipboard": true }),
        )
        .unwrap()
        .unwrap();
        assert!(!rep["report"].as_str().unwrap().is_empty());
    }

    let banner = AssumptionDiagnosticBanner::from_result(app.document().result_sheet(&"res-t".into()).unwrap()).unwrap();
    assert!(
        banner.source_result_id.as_str() == "res-t"
            && banner.severity == ChecklistStatus::Violated
            && banner.diagnostic_metric.is_some()
            && !banner.detail_tab_ids.is_empty()
    );
    let (init_res, init_undo) = (app.document().result_sheets.len(), app.session().undo_depth());

    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, crate::human_wf_common::default_viewport());
    h.click("result.diagnostic_banner.inspect").unwrap();
    assert_eq!(
        (h.app().document().result_sheets.len(), h.app().session().undo_depth(), h.app().document().active_sheet_id.as_ref().map(SheetId::as_str)),
        (init_res, init_undo, Some("res-t"))
    );

    let mut app = h.driver.state().app.clone();
    assert!(
        dispatch_ui_command(&mut app, "ui.result.applyRemediation", &json!({ "result_sheet_id": "res-t" }))
            .unwrap_err()
            .contains("diagnostics_only_policy")
    );
    assert_eq!(app.document().result_sheets.len(), init_res);
    assert!(
        matches!(app.document().result_sheet(&"res-t".into()).unwrap().analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::UnpairedT)
    );
}

#[test]
fn qol7_assumption_banner_reports_small_n_info_and_blocks_remediation_mutation() {
    let mut doc = build_qol_test_project();
    doc.result_sheet_mut(&"res-t".into()).unwrap().checklist = vec![ChecklistItem {
        assumption: "Shapiro-Wilk normality check".into(),
        status: ChecklistStatus::Info,
        diagnostic_metric: Some("n = 4".into()),
        explanation: "Inconclusive normality power due to small sample size (n = 4); consider Mann-Whitney U.".into(),
        remediation: Some(RemediationKind::MannWhitneyU),
    }];
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.execute_command("sheet.select", json!({ "sheet_id": "res-t" })).unwrap();
    let banner = AssumptionDiagnosticBanner::from_result(app.document().result_sheet(&"res-t".into()).unwrap()).unwrap();
    assert_eq!((banner.severity, banner.diagnostic_metric.as_deref()), (ChecklistStatus::Info, Some("n = 4")));
    assert!(!banner.limitation_note.to_ascii_lowercase().contains("consider"));

    let inspected = dispatch_ui_command(&mut app, "ui.result.inspectDiagnostics", &json!({ "result_sheet_id": "res-t" })).unwrap().unwrap();
    assert_eq!((inspected["mutated"].as_bool(), inspected["tab_id"].as_str()), (Some(false), Some("diagnostics")));
    let count = app.document().result_sheets.len();
    assert!(
        dispatch_ui_command(&mut app, "ui.result.applyRemediation", &json!({ "result_sheet_id": "res-t" }))
            .unwrap_err()
            .contains("diagnostics_only_policy")
    );
    assert_eq!(app.document().result_sheets.len(), count);
}

#[test]
fn qol8_qol9_qol10_presets_preflight_cvd_non_mutating_presentation_and_virtual_workspace() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    app.nav.welcome.open = false;
    app.execute_command("sheet.select", json!({ "sheet_id": "graph-xy" })).unwrap();
    let metrics = ViewportMetrics::new(1024.0, 640.0, 1.0);

    assert_eq!(
        dispatch_ui_command(
            &mut app,
            "ui.canvas.applyPreset",
            &json!({ "sheet_id": "graph-xy", "preset": "DoubleColumn180mm", "resize_mode": "KeepTypographyPt" })
        )
        .unwrap()
        .unwrap()["preset"],
        "DoubleColumn180mm"
    );
    assert!(
        dispatch_ui_command(&mut app, "ui.canvas.preflight", &json!({ "sheet_id": "graph-xy" })).unwrap().unwrap()["issue_count"].as_u64().is_some()
    );

    let orig_spec = app.document().graph_sheet(&"graph-xy".into()).unwrap().spec.clone();
    let target = metrics.to_render_target_spec(96);
    let normal = app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap();
    for filter in ["deuteranopia", "protanopia", "tritanopia", "grayscale"] {
        dispatch_ui_command(&mut app, "ui.previewFilter.set", &json!({ "filter": filter })).unwrap();
        assert_ne!(normal.rgba, app.render_egui_shell_rgba(target, ScreenshotRegionTarget::FullWindow).unwrap().rgba);
        assert_eq!(app.document().graph_sheet(&"graph-xy".into()).unwrap().spec, orig_spec);
    }
    assert_eq!(app.preview_filter, CanvasPreviewFilter::GrayscalePrint);
    let layout = measure_egui_regions(&app, metrics);
    assert!(layout.navigator.width > 0 && layout.canvas.width > 0 && layout.inspector.width > 0);

    assert!(VirtualWorkspaceFolder::normalize_rel_path("../etc/passwd").is_err() && VirtualWorkspaceFolder::normalize_rel_path("/abs/path").is_err());
    let archive_len = app.save_to_virtual_workspace("experiment.curvebook").unwrap();
    assert!(archive_len > 0);
    let mut reloaded = UiApp::new();
    reloaded.virtual_workspace = app.virtual_workspace.clone();
    reloaded.load_from_virtual_workspace("experiment.curvebook").unwrap();
    assert_eq!(reloaded.document().title, app.document().title);

    let rdoc = reloaded.document().clone();
    assert!(reloaded.virtual_workspace.save_unpacked_snapshot(&rdoc).unwrap() > archive_len);
    assert!(reloaded.virtual_workspace.contains("document.json") && reloaded.virtual_workspace.contains("manifest.json"));
    assert_eq!(reloaded.virtual_workspace.open_unpacked_snapshot().unwrap().data_sheets.len(), app.document().data_sheets.len());
}

#[test]
fn g11_emf_clipboard_bundle_and_control_ui_event_dispatch() {
    let mut app = UiApp::from_session(Session::from_document(build_qol_test_project()));
    let bundle = ClipboardBundle::from_sheet(app.session(), &"graph-xy".into(), 144).unwrap();
    let emf = bundle.emf_bytes.as_ref().unwrap();
    assert!(
        emf.len() > 88
            && u32::from_le_bytes([emf[0], emf[1], emf[2], emf[3]]) == 1
            && u32::from_le_bytes([emf[40], emf[41], emf[42], emf[43]]) == 0x464D_4520
    );
    assert!(bundle.available_flavors().contains(&"emf"));
    if let Err(err) = write_os_clipboard_bundle(&bundle) {
        assert!(!err.is_empty());
    }

    let reply = call_egui_control(
        &mut app,
        &json!({ "id": "ev1", "method": "command.run", "command": "ui.event.dispatch", "params": { "event": { "PinchZoom": { "x": 400.0, "y": 300.0, "scale_delta": 1.25 } }, "metrics": { "logical_width": 1024.0, "logical_height": 640.0, "scale_factor": 2.0 } } }),
    ).unwrap();
    assert_eq!(
        (&reply["ok"], &reply["result"]["handled"], &reply["result"]["action"]),
        (&Value::Bool(true), &Value::Bool(true), &json!("canvas.pinch_zoom"))
    );
}
