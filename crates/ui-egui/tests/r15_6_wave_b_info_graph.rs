#![forbid(unsafe_code)]

//! `R15.6` Wave B regression tests for InfoSheet (`B7`: `R15-GUI-01`, `R15-GUI-27`, `R15-GUI-22`)
//! and Graph dialogs (`B8`: `R15-GUI-24`, `R15-GUI-15`, `R15-GUI-28`).
//!
//! Every test drives the rendered `egui` widgets through [`EguiHumanPathDriver`].

use curvebook_doc::{
    Cell, CellCoord, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, DiagnosticPlotKind, FramePreset, GraphKind,
    GraphSheet, InfoConstant, InfoSheet, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype, VariableRole,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, accesskit_accessible_name, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use egui::{Key, Modifiers};
use serde_json::json;

struct ShellState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl ShellState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

fn make_driver<'a>(app: UiApp, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, ShellState> {
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellState::new(app),
        metrics,
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn replace_input_text(driver: &mut EguiHumanPathDriver<'_, ShellState>, widget_id: &str, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    driver.click_widget_id(widget_id)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.press_egui_key(Key::Backspace, Modifiers::NONE);
    if !text.is_empty() {
        driver.type_text(text)?;
    }
    Ok(())
}

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
    table.data_sets[0].title = String::from("Control");
    table.data_sets[1].title = String::from("Treated");
    for (r, (a, b)) in [(10.0, 200.0), (12.0, 250.0), (14.0, 300.0), (16.0, 350.0)].iter().enumerate() {
        table.data_sets[0].cells[r][0] = Cell::number(*a);
        table.data_sets[1].cells[r][0] = Cell::number(*b);
    }
    let col_ds = DataSheet::new("col-1", "Assay Data", TableArchetype::Column(table))?;
    let graph_id = SheetId::try_new("graph-1")?;
    let mut graph = GraphSheet::new_uninitialized_for_data(graph_id.clone(), &col_ds)?;
    graph.title = String::from("Assay Graph");
    graph.initialized = true;
    graph.spec.frame_preset = FramePreset::OpenL;
    graph.spec.y_axis.segments[0].min = None;
    graph.spec.y_axis.segments[0].max = None;

    let mut mv = MultipleVariablesTable::new(3, 4)?;
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].role = VariableRole::Continuous;
    let mut mv_ds = DataSheet::new("mv-1", "Matrix Data", TableArchetype::MultipleVariables(mv))?;
    for r in 0..4 {
        let v = (r + 1) as f64;
        mv_ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(v))?;
        mv_ds.set_cell(CellCoord::new(r, 1, 0), Cell::number(v * 2.0))?;
        mv_ds.set_cell(CellCoord::new(r, 2, 0), Cell::number(v * 0.5))?;
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
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    app.sync_views();
    Ok(SeededWorkspace { app, info_id, graph_id, heatmap_id })
}

#[test]
fn r15_gui_01_date_constant_shape_validation_and_no_day_steppers() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { app, info_id, .. } = seed_workspace()?;
    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));

    // Open inline editor for AssayDate (row 0).
    driver.click_widget_id("info.const.0")?;
    driver.click_widget_id("info.const.0")?;
    driver.click_widget_id("info.const.kind.date")?;

    // Verify no -1d / +1d calendar stepper controls exist anywhere in rendered UI or AccessKit tree.
    assert!(
        !driver.frame_meta().entries.iter().any(|e| e.label == "-1d" || e.label == "+1d"),
        "date stepper buttons must not be rendered in InfoSheet"
    );
    let has_stepper_ak =
        driver.accesskit_tree().is_some_and(|tree| tree.nodes.iter().any(|(_, node)| matches!(accesskit_accessible_name(node), Some("-1d" | "+1d"))));
    assert!(!has_stepper_ak, "date stepper controls must not exist in AccessKit tree");

    // Reject malformed and out-of-range dates.
    for invalid in ["+123-01-01", "0000-12-31", "10000-01-01"] {
        replace_input_text(&mut driver, "info.const.editor_value", invalid)?;
        let save_btn = driver.frame_meta().find_by_widget_id("info.const.editor_save").ok_or("missing save button")?;
        assert!(!save_btn.enabled, "Save must be disabled for invalid date `{invalid}`");
        assert!(driver.frame_meta().find_by_widget_id("info.const.editor_error").is_some(), "expected validation error for `{invalid}`");
    }

    // Accept valid ISO shape YYYY-MM-DD and commit.
    replace_input_text(&mut driver, "info.const.editor_value", "2026-11-20")?;
    let save_btn = driver.frame_meta().find_by_widget_id("info.const.editor_save").ok_or("missing save button")?;
    assert!(save_btn.enabled, "Save must be enabled for valid ISO date");
    assert!(driver.frame_meta().find_by_widget_id("info.const.editor_error").is_none());
    driver.click_widget_id("info.const.editor_save")?;

    let updated = driver.state().app.document().info_sheet(&info_id).and_then(|s| s.get_constant("AssayDate")).map(|c| c.value.clone());
    assert_eq!(updated, Some(ConstantValue::Date(String::from("2026-11-20"))));
    Ok(())
}

#[test]
fn r15_gui_27_add_constant_picks_first_unused_name_without_overwriting() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, info_id, .. } = seed_workspace()?;
    {
        let mut doc = app.document().clone();
        let info = doc.info_sheet_mut(&info_id).ok_or("missing info sheet")?;
        info.constants = vec![
            InfoConstant::new("Constant 1", ConstantValue::Number(11.0), None)?,
            InfoConstant::new("Constant 2", ConstantValue::Number(22.0), None)?,
            InfoConstant::new("Constant 3", ConstantValue::Number(33.0), None)?,
        ];
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    // Delete Constant 1 via engine so remaining constants are Constant 2 and Constant 3 (len = 2).
    app.execute_command("info.removeConstant", json!({ "sheet_id": info_id.as_str(), "name": "Constant 1" }))?;

    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));
    driver.click_widget_id("info.add_constant")?;

    let info = driver.state().app.document().info_sheet(&info_id).ok_or("missing info sheet")?;
    assert_eq!(info.constants.len(), 3, "adding constant must append a new row rather than overwriting Constant 3");
    assert_eq!(info.get_constant("Constant 3").map(|c| &c.value), Some(&ConstantValue::Number(33.0)));
    assert_eq!(info.get_constant("Constant 2").map(|c| &c.value), Some(&ConstantValue::Number(22.0)));
    assert!(info.get_constant("Constant 1").is_some(), "first unused name Constant 1 must be chosen");

    // Case-insensitive collision check: with lowercase "constant 1", "Constant 2", "Constant 3", clicking "+" must pick "Constant 4".
    {
        let mut doc = driver.state().app.document().clone();
        let info_mut = doc.info_sheet_mut(&info_id).ok_or("missing info sheet")?;
        info_mut.constants = vec![
            InfoConstant::new("constant 1", ConstantValue::Number(11.0), None)?,
            InfoConstant::new("Constant 2", ConstantValue::Number(22.0), None)?,
            InfoConstant::new("Constant 3", ConstantValue::Number(33.0), None)?,
        ];
        *driver.state_mut().app.session_mut() = Session::from_document(doc);
        driver.state_mut().app.sync_views();
        driver.state_mut().views.info_view.editing_key = None;
    }
    driver.run_settled();
    driver.click_widget_id("info.add_constant")?;

    let info2 = driver.state().app.document().info_sheet(&info_id).ok_or("missing info sheet")?;
    assert_eq!(info2.constants.len(), 4);
    assert_eq!(info2.get_constant("constant 1").map(|c| &c.value), Some(&ConstantValue::Number(11.0)));
    assert!(info2.get_constant("Constant 4").is_some(), "must skip case-insensitive collision on Constant 1..3");
    Ok(())
}

#[test]
fn r15_gui_22_comma_decimal_locale_in_info_studio_ticks_and_heatmap() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, info_id, graph_id, heatmap_id } = seed_workspace()?;
    assert_eq!(app.set_locale("de"), "de");
    app.sync_views();

    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));

    // 1) InfoSheet inline Number constant accepts "1,5" in `de` locale -> 1.5.
    driver.click_widget_id("info.const.1")?;
    driver.click_widget_id("info.const.1")?;
    driver.click_widget_id("info.const.kind.number")?;
    replace_input_text(&mut driver, "info.const.editor_value", "1,5")?;
    let save_btn = driver.frame_meta().find_by_widget_id("info.const.editor_save").ok_or("missing inline save")?;
    assert!(save_btn.enabled, "comma-decimal `1,5` must be valid in German locale");
    driver.click_widget_id("info.const.editor_save")?;
    let dose_val = driver.state().app.document().info_sheet(&info_id).and_then(|s| s.get_constant("DoseConc")).map(|c| c.value.clone());
    assert_eq!(dose_val, Some(ConstantValue::Number(1.5)));

    // 2) Trailing Studio Info panel accepts "1,5" in `de` locale -> 1.5.
    driver.click_widget_id("inspector.info.const.2")?;
    driver.click_widget_id("inspector.info.kind.0")?;
    replace_input_text(&mut driver, "inspector.info.value_input", "1,5")?;
    driver.click_widget_id("inspector.info.save_const")?;
    let studio_val = driver.state().app.document().info_sheet(&info_id).and_then(|s| s.get_constant("StudioDose")).map(|c| c.value.clone());
    assert_eq!(studio_val, Some(ConstantValue::Number(1.5)));

    // 3) Format Axes Additional Ticks accepts "1,5" in `de` locale -> 1.5.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    replace_input_text(&mut driver, "axes.add_tick.value", "1,5")?;
    replace_input_text(&mut driver, "axes.add_tick.label", "DE Cutoff")?;
    driver.click_widget_id("axes.add_tick.save")?;
    driver.click_widget_id("axes.add_tick.close")?;
    driver.click_widget_id("modal.ok")?;
    let gs = driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?;
    assert!(
        gs.spec.y_axis.additional_ticks.iter().any(|t| (t.value - 1.5).abs() < 1e-6 && t.label.as_deref() == Some("DE Cutoff")),
        "expected additional tick at 1.5, got {:?}",
        gs.spec.y_axis.additional_ticks
    );

    // 4) Heatmap colour range accepts "0,5".."1,5" in `de` locale and surfaces validation error on min >= max.
    let doc_snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc_snap, &heatmap_id));
    driver.state_mut().app.sync_views();
    driver.run_settled();

    driver.click_widget_id("format_graph.heatmap.auto_range")?;
    replace_input_text(&mut driver, "format_graph.heatmap.min", "not-a-number")?;
    assert!(
        driver.frame_meta().find_by_widget_id("format_graph.heatmap.range_error").is_some(),
        "expected validation error for non-numeric heatmap bound"
    );
    replace_input_text(&mut driver, "format_graph.heatmap.min", "2,5")?;
    replace_input_text(&mut driver, "format_graph.heatmap.max", "1,5")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.range_error").is_some(), "expected validation error when min >= max");
    driver.click_widget_id("modal.ok")?;
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. }),
        "invalid heatmap range must block commit rather than silently falling back to auto"
    );

    replace_input_text(&mut driver, "format_graph.heatmap.min", "0,5")?;
    replace_input_text(&mut driver, "format_graph.heatmap.max", "1,5")?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.heatmap.range_error").is_none());
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let hm = driver.state().app.document().graph_sheet(&heatmap_id).ok_or("missing heatmap")?;
    let enc = hm.spec.multivariate.as_ref().ok_or("missing heatmap encoding")?;
    assert!(
        matches!(enc.color, ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. } if (lo - 0.5).abs() < 1e-6 && (hi - 1.5).abs() < 1e-6),
        "expected domain_clamp Some((0.5, 1.5)), got {:?}",
        enc.color
    );
    Ok(())
}

#[test]
fn r15_gui_24_format_axes_resyncs_on_reopen_and_preserves_newer_frame_preset() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, graph_id, .. } = seed_workspace()?;
    let _ =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }))?;
    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));

    // Change frame preset to PlainBoxed, then Cancel.
    driver.click_widget_id("axes.frame.plain_boxed")?;
    assert_eq!(driver.state().views.graph_modals.format_axes_frame_preset, FramePreset::PlainBoxed);
    assert!(driver.state().views.graph_modals.format_axes_frame_preset_dirty);
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset), Some(FramePreset::OpenL));

    // Change graph frame externally (simulating Studio Rail / Undo).
    driver.state_mut().app.execute_command("graph.setAxes", json!({ "sheet_id": graph_id.as_str(), "frame_preset": "OffsetRangeTufte" }))?;
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset), Some(FramePreset::OffsetRangeTufte));

    // Reopen Format Axes on the same graph and press OK without touching Frame preset.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    assert_eq!(driver.state().views.graph_modals.format_axes_frame_preset, FramePreset::OffsetRangeTufte);
    assert!(!driver.state().views.graph_modals.format_axes_frame_preset_dirty);

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(
        driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset),
        Some(FramePreset::OffsetRangeTufte),
        "reopening Format Axes and pressing OK without touching frame must preserve OffsetRangeTufte"
    );

    // Explicit frame change on reopen still commits.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    driver.click_widget_id("axes.frame.origin")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.document().graph_sheet(&graph_id).map(|g| g.spec.frame_preset), Some(FramePreset::OriginCrossing));
    Ok(())
}

#[test]
fn r15_gui_15_format_axes_auto_bounds_and_validation_feedback() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, graph_id, .. } = seed_workspace()?;
    let _ =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }))?;
    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));
    driver.click_widget_id("edit.tab.axes.y")?;

    // Auto bounds start empty (Auto), not fake 0 / 100 placeholders.
    assert!(driver.state().views.graph_modals.axis_min_draft.is_empty());
    assert!(driver.state().views.graph_modals.axis_max_draft.is_empty());

    // Setting min = 150 while max is Auto (None) must succeed without being dropped against fake 100.
    replace_input_text(&mut driver, "axes.seg.0.min", "150")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.bounds_error").is_none());
    driver.click_widget_id("modal.ok")?;

    let seg = &driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.segments[0];
    assert_eq!(seg.min, Some(150.0));
    assert_eq!(seg.max, None);

    // Reopen and enter min >= max (150 >= 100) -> visible validation error and commit blocked.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    driver.click_widget_id("edit.tab.axes.y")?;
    replace_input_text(&mut driver, "axes.seg.0.max", "100")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.bounds_error").is_some(), "expected visible validation message when min >= max");
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    // Enter a large scientific max (> 1e9) to confirm removal of the artificial ±1e9 cap.
    replace_input_text(&mut driver, "axes.seg.0.max", "1e12")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.bounds_error").is_none());
    driver.click_widget_id("modal.ok")?;
    let seg2 = &driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.segments[0];
    assert_eq!(seg2.min, Some(150.0));
    assert_eq!(seg2.max, Some(1.0e12));
    Ok(())
}

#[test]
fn r15_gui_28_additional_ticks_staged_until_ok_and_discarded_on_cancel() -> Result<(), Box<dyn std::error::Error>> {
    let SeededWorkspace { mut app, graph_id, .. } = seed_workspace()?;
    let _ =
        dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }))?;
    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 760.0, 1.0));
    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;

    replace_input_text(&mut driver, "axes.add_tick.value", "42.5")?;
    replace_input_text(&mut driver, "axes.add_tick.label", "CanceledTick")?;
    driver.click_widget_id("axes.add_tick.save")?;

    // Tick is staged in dialog draft only — NOT written to the document yet.
    assert!(
        driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks.is_empty(),
        "adding a tick inside Additional Ticks must not mutate graph before OK"
    );

    driver.click_widget_id("axes.add_tick.close")?;
    driver.click_widget_id("modal.cancel")?;
    assert!(
        driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks.is_empty(),
        "cancelling Format Axes must discard staged additional ticks"
    );

    // Reopen, add tick, and press OK -> graph receives the new additional tick.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;

    replace_input_text(&mut driver, "axes.add_tick.value", "42.5")?;
    replace_input_text(&mut driver, "axes.add_tick.label", "CommittedTick")?;
    driver.click_widget_id("axes.add_tick.save")?;
    assert!(
        driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks.is_empty(),
        "staged tick must still wait for OK"
    );
    driver.click_widget_id("axes.add_tick.close")?;
    driver.click_widget_id("modal.ok")?;

    let ticks = &driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks;
    assert!(
        ticks.iter().any(|t| (t.value - 42.5).abs() < 1e-6 && t.label.as_deref() == Some("CommittedTick")),
        "expected CommittedTick at 42.5 after OK, got {ticks:?}"
    );

    // Deleting an existing tick is also staged: Cancel keeps the tick intact.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.graph.axes.open", "params": { "sheet_id": graph_id.as_str() } }),
    )?;
    driver.run_settled();
    driver.click_widget_id("edit.tab.axes.y")?;
    driver.click_widget_id("axes.additional_ticks.open")?;
    driver.click_widget_id("axes.add_tick.delete.0")?;
    driver.click_widget_id("axes.add_tick.close")?;
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(
        driver.state().app.document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.y_axis.additional_ticks.len(),
        1,
        "cancelling after deleting a tick must keep the existing tick"
    );
    Ok(())
}
