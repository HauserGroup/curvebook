#![forbid(unsafe_code)]

//! `R15.6` Wave B regression tests for InfoSheet (`B7`: `R15-GUI-01`, `R15-GUI-27`, `R15-GUI-22`)
//! and Graph dialogs (`B8`: `R15-GUI-24`, `R15-GUI-15`, `R15-GUI-28`).

use curvebook_doc::{
    Cell, CellCoord, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, DiagnosticPlotKind, FramePreset, GraphKind,
    GraphSheet, InfoConstant, InfoSheet, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype, VariableRole,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, accesskit_accessible_name};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::json;

use crate::common::{ShellTestState, fill_col_values, make_shell_driver, set_textbox};

struct SeededWorkspace {
    app: UiApp,
    info_id: SheetId,
    graph_id: SheetId,
    heatmap_id: SheetId,
}

fn seed_workspace() -> Result<SeededWorkspace, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("Wave B Info & Graph Fixture")?;
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata")?;
    info.constants = vec![
        InfoConstant::new("AssayDate", ConstantValue::Date(String::from("2026-01-15")), None)?,
        InfoConstant::new("DoseConc", ConstantValue::Number(10.0), Some(String::from("uM")))?,
        InfoConstant::new("StudioDose", ConstantValue::Number(4.0), Some(String::from("mM")))?,
    ];
    let info_id = info.id.clone();
    doc.add_info_sheet(info)?;

    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    fill_col_values(&mut table.data_sets[0], "Control", &[10.0, 12.0, 14.0, 16.0]);
    fill_col_values(&mut table.data_sets[1], "Treated", &[200.0, 250.0, 300.0, 350.0]);
    let col_ds = DataSheet::new("col-1", "Assay Data", TableArchetype::Column(table))?;
    let graph_id = SheetId::try_new("graph-1")?;
    let mut graph = GraphSheet::new_uninitialized_for_data(graph_id.clone(), &col_ds)?;
    graph.title = String::from("Assay Graph");
    graph.initialized = true;
    graph.spec.frame_preset = FramePreset::OpenL;
    graph.spec.y_axis.segments[0].min = None;
    graph.spec.y_axis.segments[0].max = None;

    let mut mv = MultipleVariablesTable::new(3, 4)?;
    for v in &mut mv.variables {
        v.role = VariableRole::Continuous;
    }
    let mut mv_ds = DataSheet::new("mv-1", "Matrix Data", TableArchetype::MultipleVariables(mv))?;
    for r in 0..4 {
        let v = (r + 1) as f64;
        for (c, factor) in [(0, 1.0), (1, 2.0), (2, 0.5)] {
            mv_ds.set_cell(CellCoord::new(r, c, 0), Cell::number(v * factor))?;
        }
    }
    let heatmap_id = SheetId::try_new("graph-hm")?;
    let mut hm_graph = GraphSheet::new_uninitialized_for_data(heatmap_id.clone(), &mv_ds)?;
    hm_graph.title = String::from("Heatmap Graph");
    hm_graph.initialized = true;
    hm_graph.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);

    doc.data_sheets = vec![col_ds, mv_ds];
    doc.graph_sheets = vec![graph, hm_graph];
    doc.active_sheet_id = Some(info_id.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    Ok(SeededWorkspace { app, info_id, graph_id, heatmap_id })
}

fn open_axes_modal(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, graph_id: &SheetId) -> Result<(), Box<dyn std::error::Error>> {
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    Ok(())
}

#[test]
fn r15_gui_01_date_constant_shape_validation_and_no_day_steppers() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { app, info_id, .. } = seed_workspace()?;
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));

    driver.click_widget_id("info.const.0")?;
    driver.click_widget_id("info.const.0")?;
    driver.click_widget_id("info.const.kind.date")?;
    assert!(driver.frame_meta().entries.iter().all(|e| e.label != "-1d" && e.label != "+1d"));
    assert!(!driver.accesskit_tree().is_some_and(|t| t.nodes.iter().any(|(_, n)| matches!(accesskit_accessible_name(n), Some("-1d" | "+1d")))));

    for invalid in ["+123-01-01", "0000-12-31", "10000-01-01"] {
        set_textbox(&mut driver, "info.const.editor_value", invalid)?;
        assert!(!driver.frame_meta().find_by_widget_id("info.const.editor_save").ok_or("missing save")?.enabled);
        assert!(driver.frame_meta().find_by_widget_id("info.const.editor_error").is_some());
    }

    set_textbox(&mut driver, "info.const.editor_value", "2026-11-20")?;
    assert!(driver.frame_meta().find_by_widget_id("info.const.editor_save").ok_or("missing save")?.enabled);
    assert!(driver.frame_meta().find_by_widget_id("info.const.editor_error").is_none());
    driver.click_widget_id("info.const.editor_save")?;
    assert_eq!(
        driver.state().app.document().info_sheet(&info_id).and_then(|s| s.get_constant("AssayDate")).map(|c| &c.value),
        Some(&ConstantValue::Date(String::from("2026-11-20")))
    );
    Ok(())
}

#[test]
fn r15_gui_27_add_constant_picks_first_unused_name_without_overwriting() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, info_id, .. } = seed_workspace()?;
    let mut doc = app.document().clone();
    doc.info_sheet_mut(&info_id).ok_or("missing info")?.constants = vec![
        InfoConstant::new("Constant 1", ConstantValue::Number(11.0), None)?,
        InfoConstant::new("Constant 2", ConstantValue::Number(22.0), None)?,
        InfoConstant::new("Constant 3", ConstantValue::Number(33.0), None)?,
    ];
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    app.execute_command("info.removeConstant", json!({ "sheet_id": info_id.as_str(), "name": "Constant 1" }))?;

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));
    driver.click_widget_id("info.add_constant")?;
    let info = driver.state().app.document().info_sheet(&info_id).ok_or("missing info")?;
    assert_eq!(info.constants.len(), 3);
    assert_eq!(info.get_constant("Constant 3").map(|c| &c.value), Some(&ConstantValue::Number(33.0)));
    assert_eq!(info.get_constant("Constant 2").map(|c| &c.value), Some(&ConstantValue::Number(22.0)));
    assert!(info.get_constant("Constant 1").is_some());

    let mut doc2 = driver.state().app.document().clone();
    doc2.info_sheet_mut(&info_id).ok_or("missing info")?.constants = vec![
        InfoConstant::new("constant 1", ConstantValue::Number(11.0), None)?,
        InfoConstant::new("Constant 2", ConstantValue::Number(22.0), None)?,
        InfoConstant::new("Constant 3", ConstantValue::Number(33.0), None)?,
    ];
    *driver.state_mut().app.session_mut() = Session::from_document(doc2);
    driver.state_mut().app.sync_views();
    driver.state_mut().views.info_view.editing_key = None;
    driver.run_settled();
    driver.click_widget_id("info.add_constant")?;

    let info2 = driver.state().app.document().info_sheet(&info_id).ok_or("missing info")?;
    assert_eq!(info2.constants.len(), 4);
    assert_eq!(info2.get_constant("constant 1").map(|c| &c.value), Some(&ConstantValue::Number(11.0)));
    assert!(info2.get_constant("Constant 4").is_some());
    Ok(())
}

#[test]
fn r15_gui_22_comma_decimal_locale_in_info_studio_ticks_and_heatmap() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, info_id, graph_id, heatmap_id } = seed_workspace()?;
    assert_eq!(app.set_locale("de"), "de");
    app.sync_views();
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));

    // 1) InfoSheet inline + 2) Trailing Studio Info panel accept "1,5".
    driver.click_widget_id("info.const.1")?;
    driver.click_widget_id("info.const.1")?;
    driver.click_widget_id("info.const.kind.number")?;
    set_textbox(&mut driver, "info.const.editor_value", "1,5")?;
    assert!(driver.frame_meta().find_by_widget_id("info.const.editor_save").ok_or("missing save")?.enabled);
    driver.click_widget_id("info.const.editor_save")?;
    assert_eq!(
        driver.state().app.document().info_sheet(&info_id).and_then(|s| s.get_constant("DoseConc")).map(|c| &c.value),
        Some(&ConstantValue::Number(1.5))
    );

    driver.click_widget_id("inspector.info.const.2")?;
    driver.click_widget_id("inspector.info.kind.0")?;
    set_textbox(&mut driver, "inspector.info.value_input", "1,5")?;
    driver.click_widget_id("inspector.info.save_const")?;
    assert_eq!(
        driver.state().app.document().info_sheet(&info_id).and_then(|s| s.get_constant("StudioDose")).map(|c| &c.value),
        Some(&ConstantValue::Number(1.5))
    );

    // 3) Format Axes Additional Ticks accepts "1,5".
    open_axes_modal(&mut driver, &graph_id)?;
    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    set_textbox(&mut driver, "axes.add_tick.value", "1,5")?;
    set_textbox(&mut driver, "axes.add_tick.label", "DE Cutoff")?;
    for wid in ["axes.add_tick.save", "axes.add_tick.close", "modal.ok"] {
        driver.click_widget_id(wid)?;
    }
    let gs = driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?;
    assert!(gs.spec.y_axis.additional_ticks.iter().any(|t| (t.value - 1.5).abs() < 1e-6 && t.label.as_deref() == Some("DE Cutoff")));

    // 4) Heatmap colour range accepts "0,5".."1,5" and blocks invalid bounds.
    let snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&snap, &heatmap_id));
    driver.state_mut().app.sync_views();
    driver.run_settled();

    driver.click_widget_id("format_graph.heatmap.auto_range")?;
    set_textbox(&mut driver, "format_graph.heatmap.min", "not-a-number")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.range_error").is_some());
    set_textbox(&mut driver, "format_graph.heatmap.min", "2,5")?;
    set_textbox(&mut driver, "format_graph.heatmap.max", "1,5")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.range_error").is_some());
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. }));

    set_textbox(&mut driver, "format_graph.heatmap.min", "0,5")?;
    set_textbox(&mut driver, "format_graph.heatmap.max", "1,5")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.range_error").is_none());
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let enc = driver.state().app.document().graph_sheet(&heatmap_id).and_then(|h| h.spec.multivariate.as_ref()).ok_or("missing enc")?;
    assert!(
        matches!(enc.color, ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. } if (lo - 0.5).abs() < 1e-6 && (hi - 1.5).abs() < 1e-6)
    );
    Ok(())
}

#[test]
fn r15_gui_24_format_axes_resyncs_on_reopen_and_preserves_newer_frame_preset() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { app, graph_id, .. } = seed_workspace()?;
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));
    open_axes_modal(&mut driver, &graph_id)?;

    driver.click_widget_id("axes.frame.plain_boxed")?;
    assert_eq!(driver.state().views.graph_modals.format_axes_frame_preset, FramePreset::PlainBoxed);
    assert!(driver.state().views.graph_modals.format_axes_frame_preset_dirty);
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset), Some(FramePreset::OpenL));

    driver.state_mut().app.execute_command("graph.setAxes", json!({ "sheet_id": graph_id.as_str(), "frame_preset": "OffsetRangeTufte" }))?;
    open_axes_modal(&mut driver, &graph_id)?;
    assert_eq!(driver.state().views.graph_modals.format_axes_frame_preset, FramePreset::OffsetRangeTufte);
    assert!(!driver.state().views.graph_modals.format_axes_frame_preset_dirty);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset), Some(FramePreset::OffsetRangeTufte));

    open_axes_modal(&mut driver, &graph_id)?;
    driver.click_widget_id("axes.frame.origin")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset), Some(FramePreset::OriginCrossing));
    Ok(())
}

#[test]
fn r15_gui_15_format_axes_auto_bounds_and_validation_feedback() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { app, graph_id, .. } = seed_workspace()?;
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));
    open_axes_modal(&mut driver, &graph_id)?;
    driver.click_widget_id("edit.tab.axes.y")?;
    assert!(driver.state().views.graph_modals.axis_min_draft.is_empty() && driver.state().views.graph_modals.axis_max_draft.is_empty());

    set_textbox(&mut driver, "axes.seg.0.min", "150")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.bounds_error").is_none());
    driver.click_widget_id("modal.ok")?;
    let seg = &driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.segments[0];
    assert_eq!((seg.min, seg.max), (Some(150.0), None));

    open_axes_modal(&mut driver, &graph_id)?;
    driver.click_widget_id("edit.tab.axes.y")?;
    set_textbox(&mut driver, "axes.seg.0.max", "100")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.bounds_error").is_some());
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    set_textbox(&mut driver, "axes.seg.0.max", "1e12")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.bounds_error").is_none());
    driver.click_widget_id("modal.ok")?;
    let seg2 = &driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.segments[0];
    assert_eq!((seg2.min, seg2.max), (Some(150.0), Some(1.0e12)));
    Ok(())
}

#[test]
fn r15_gui_28_additional_ticks_staged_until_ok_and_discarded_on_cancel() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { app, graph_id, .. } = seed_workspace()?;
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));

    for (label, finish_wid, expected_len) in [("CanceledTick", "modal.cancel", 0usize), ("CommittedTick", "modal.ok", 1usize)] {
        open_axes_modal(&mut driver, &graph_id)?;
        driver.click_widget_id("edit.tab.axes.y")?;
        driver.click_widget_id("axes.additional_ticks.open")?;
        set_textbox(&mut driver, "axes.add_tick.value", "42.5")?;
        set_textbox(&mut driver, "axes.add_tick.label", label)?;
        driver.click_widget_id("axes.add_tick.save")?;
        assert!(driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks.is_empty());
        driver.click_widget_id("axes.add_tick.close")?;
        driver.click_widget_id(finish_wid)?;
        let ticks = &driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks;
        assert_eq!(ticks.len(), expected_len);
        if expected_len == 1 {
            assert!((ticks[0].value - 42.5).abs() < 1e-6 && ticks[0].label.as_deref() == Some("CommittedTick"));
        }
    }

    open_axes_modal(&mut driver, &graph_id)?;
    for wid in ["edit.tab.axes.y", "axes.additional_ticks.open", "axes.add_tick.delete.0", "axes.add_tick.close", "modal.cancel"] {
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks.len(), 1);
    Ok(())
}
