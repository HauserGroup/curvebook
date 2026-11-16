//! Round 2 S10 & companion `interaction-gui_actions.md` regression tests (`R0.19`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, ChecklistItem, ChecklistStatus, ErrorCode, GraphAnnotation, GraphSheet, GraphicalDataObject,
    InfoSheet, OneWayAnovaSpec, OneWayMethod, PValueStackSpec, PowerAnalysisSpec, ResultSheet, SheetId, SheetSection,
};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::control_io::{dispatch_gui_action, handle_control_line};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AxisTarget};
use curvebook_ui_egui::interaction::InlineEditorTarget;
use curvebook_ui_egui::menus::tooltips::menu_bar_tooltips;
use curvebook_ui_egui::modal_predicates::evaluate_dialog_widget;
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::{
    HumanPathDriver, KeyModifiers, LayoutTree, LocalePreset, PointerButton, UiApp, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder,
    WidgetAction, WorkspacePixelRect, dispatch_ui_event, execute_widget_action, inspect_ui_tree, inspect_ui_tree_for_metrics,
};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn ok_doc_and_app() -> Result<(UiApp, SheetId, SheetId, SheetId, SheetId), String> {
    let mut app = UiApp::new();
    let d1_out = app
        .execute_command("sheet.createData", json!({ "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 4, "auto_graph": true }))
        .map_err(|e| format!("create xy sheet: {e}"))?;
    let ds_id = SheetId::try_new(d1_out.payload["sheet_id"].as_str().ok_or_else(|| String::from("missing ds_id"))?).map_err(|e| format!("{e}"))?;
    let gs_id = SheetId::try_new(
        d1_out.payload["auto_graph_sheet_id"]
            .as_str()
            .or_else(|| app.document().graph_sheets.first().map(|g| g.id.as_str()))
            .ok_or_else(|| String::from("missing auto graph sheet"))?,
    )
    .map_err(|e| format!("{e}"))?;
    app.execute_command("table.paste", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "start_col": 0, "text": "2\t4\n3\t6\n5\t9\n7\t11" }))
        .map_err(|e| format!("paste xy y: {e}"))?;
    app.execute_command("table.setXCells", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "values": [1.0, 2.0, 3.0, 4.0] }))
        .map_err(|e| format!("set xy x: {e}"))?;

    let d2_out = app
        .execute_command("sheet.createData", json!({ "title": "Groups", "kind": "column", "cols": 2, "rows": 4, "auto_graph": false }))
        .map_err(|e| format!("create col sheet: {e}"))?;
    let ds2_id = SheetId::try_new(d2_out.payload["sheet_id"].as_str().ok_or_else(|| String::from("missing ds2_id"))?).map_err(|e| format!("{e}"))?;
    app.execute_command("table.paste", json!({ "sheet_id": ds2_id.as_str(), "start_row": 0, "start_col": 0, "text": "1\t3\n2\t4\n5\t6\n4\t7" }))
        .map_err(|e| format!("paste col: {e}"))?;

    let lay_out = app.execute_command("layout.create", json!({ "title": "Figure 1" })).map_err(|e| format!("create layout: {e}"))?;
    let ls_id = SheetId::try_new(lay_out.payload["sheet_id"].as_str().ok_or_else(|| String::from("missing ls_id"))?).map_err(|e| format!("{e}"))?;

    {
        let mut doc = app.document().clone();
        doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;
        app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
    }
    app.execute_command("info.setConstant", json!({ "sheet_id": "info-1", "name": "EC50", "value": 12.5 }))
        .map_err(|e| format!("set info constant: {e}"))?;

    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| format!("select ds_id: {e}"))?;
    app.sync_views();
    Ok((app, ds_id, ds2_id, gs_id, ls_id))
}

#[test]
fn r2_s10_01_04_gui_only_two_tier_policy_and_hidden_internal_menus() -> TestRes {
    let (mut app, ds_id, _, _, _) = ok_doc_and_app()?;
    app.set_gui_only(true);

    // Tier B (GuiOnlyPolicy::Strict): ui.screenshot with disk path rejected.
    let shot_err = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "path": "forbidden.png" }))
        .err()
        .ok_or_else(|| String::from("expected GuiOnlyViolation on ui.screenshot path"))?;
    if shot_err.code != ErrorCode::GuiOnlyViolation {
        return Err(format!("expected GuiOnlyViolation on screenshot path, got {:?}", shot_err.code));
    }

    // Tier B: blocked non-menu commands, direct cell mutation, OS helpers, and unconstrained paths.
    for (cmd, params) in [
        ("table.setCell", json!({ "sheet_id": ds_id.as_str(), "row": 0, "col": 0, "value": 99 })),
        ("ui.screenshot", json!({})),
        ("ui.event.dispatch", json!({ "event": { "kind": "text", "text": "x" } })),
        ("ui.shortcut.set", json!({ "command_id": "edit.undo", "shortcut": "Cmd+Z" })),
        ("ui.shortcut.reset", json!({})),
        ("ui.brush.point", json!({ "sheet_id": ds_id.as_str(), "row": 0, "col": 0 })),
        ("app.quit", json!({})),
        ("ui.result.copyMethodsReport", json!({})),
        ("file.print", json!({})),
        ("ui.print.open", json!({ "submit_to_os": true })),
        ("project.save", json!({ "path": "unconstrained.json" })),
    ] {
        let res = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": cmd, "params": params }));
        let err = res.err().ok_or_else(|| format!("expected `{cmd}` under ui.invoke_menu --gui-only to fail"))?;
        if err.code != ErrorCode::GuiOnlyViolation {
            return Err(format!("expected GuiOnlyViolation for `{cmd}`, got {:?}: {}", err.code, err.message()));
        }
    }

    // menu_bar_tooltips hides internal non-menu commands under --gui-only.
    let tips = menu_bar_tooltips(&app);
    for hidden_id in ["ui.screenshot", "ui.event.dispatch", "ui.shortcut.set", "ui.shortcut.reset", "ui.brush.point", "app.quit"] {
        if tips.iter().any(|sec| sec.items.iter().any(|t| t.command_id == hidden_id)) {
            return Err(format!("menu_bar_tooltips leaked internal command `{hidden_id}` under --gui-only"));
        }
    }

    // R2-S10-04: control port under --gui-only rejects doc.inspect, render.svg, render.pdf, app.quit,
    // and ui.result.inspectDiagnostics.
    for method in ["doc.inspect", "render.svg", "render.pdf", "app.quit", "ui.result.inspectDiagnostics"] {
        let raw = handle_control_line(&mut app, &json!({ "id": 1, "method": method, "params": {} }).to_string());
        let val: Value = serde_json::from_str(&raw).map_err(|e| format!("parse control reply: {e}"))?;
        if val["ok"].as_bool() != Some(false) || val["error_detail"]["code"].as_str() != Some("gui_only_violation") {
            return Err(format!("expected gui_only_violation on control `{method}`, got {val}"));
        }
    }

    // Tier A (HumanPathDriver): rejects non-empty params on ui.invoke_menu.
    let mut driver = HumanPathDriver::new(app, ViewportMetrics::default());
    let err = dispatch_gui_action(driver.app_mut(), "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "search": "anova" } }))
        .err()
        .ok_or_else(|| String::from("expected HumanPathDriver to reject non-empty params"))?;
    if err.code != ErrorCode::GuiOnlyViolation {
        return Err(format!("expected GuiOnlyViolation for human_path_only params, got {:?}", err.code));
    }

    Ok(())
}

#[test]
fn r2_s10_02_19_exhaustive_modal_contract_and_fail_closed() -> TestRes {
    let (mut app, ds_id, ds2_id, gs_id, ls_id) = ok_doc_and_app()?;

    // Add ResultSheets covering ANOVA, PowerAnalysis, and PValueStack.
    let rs_spec = AnalysisSpec::new(
        "rs-1",
        ds2_id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, compute_effect_sizes: true, ..OneWayAnovaSpec::default() }),
    )
    .map_err(|e| format!("{e}"))?;
    let rs = ResultSheet::new(rs_spec).map_err(|e| format!("{e}"))?;
    let rs_id = rs.id.clone();
    let power_spec =
        AnalysisSpec::new("rs-pwr-1", ds2_id.clone(), AnalysisKind::PowerAnalysis(PowerAnalysisSpec::default())).map_err(|e| format!("{e}"))?;
    let pstack_spec =
        AnalysisSpec::new("rs-ps-1", ds2_id.clone(), AnalysisKind::PValueStack(PValueStackSpec::default())).map_err(|e| format!("{e}"))?;
    {
        let mut doc = app.document().clone();
        doc.add_result_sheet(rs).map_err(|e| format!("{e}"))?;
        doc.add_result_sheet(ResultSheet::new(power_spec).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;
        doc.add_result_sheet(ResultSheet::new(pstack_spec).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;
        app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| format!("{e}"))?;
    }

    let assert_contract = |app: &mut UiApp, expected_slug: &str, valid_patch: Option<(&str, Value)>| -> TestRes {
        let snap = inspect_ui_tree(app, Some("dialog"), false);
        let dlg = snap.dialog.ok_or_else(|| format!("expected open dialog `{expected_slug}`"))?;
        if dlg.dialog_kind != expected_slug {
            return Err(format!("expected dialog_kind `{expected_slug}`, got `{}`", dlg.dialog_kind));
        }

        let mismatch = dispatch_gui_action(app, "ui.answer_dialog", json!({ "dialog_id": "non_matching_modal_slug", "action": "ok" }))
            .err()
            .ok_or_else(|| format!("expected dialog_id mismatch Conflict on `{expected_slug}`"))?;
        if mismatch.code != ErrorCode::Conflict {
            return Err(format!("expected Conflict on dialog_id mismatch, got {:?}", mismatch.code));
        }

        let unknown =
            dispatch_gui_action(app, "ui.answer_dialog", json!({ "dialog_id": expected_slug, "fields": { "__r2_s10_unknown_field__": 42 } }));
        if unknown.is_ok() {
            return Err(format!("dialog `{expected_slug}` accepted unknown field without failing closed"));
        }

        if let Some((wid, val)) = valid_patch {
            if !dlg.controls.iter().any(|c| c.widget_id == wid) {
                return Err(format!(
                    "widget `{wid}` missing from ui.inspect controls on `{expected_slug}`: {:?}",
                    dlg.controls.iter().map(|c| c.widget_id.as_str()).collect::<Vec<_>>()
                ));
            }
            let mut map = serde_json::Map::new();
            map.insert(wid.to_owned(), val);
            dispatch_gui_action(app, "ui.answer_dialog", json!({ "dialog_id": expected_slug, "fields": Value::Object(map) }))
                .map_err(|e| format!("failed patching `{wid}` on `{expected_slug}`: {e}"))?;
        }
        app.canvas_dialogs.close_dialog();
        Ok(())
    };

    let doc = app.document().clone();

    // 1. AnalyzeSheet
    app.canvas_dialogs.open_analyze_dialog(&doc, &ds2_id);
    assert_contract(&mut app, "analyze_sheet", Some(("analyze.search", json!("anova"))))?;

    // 2. DuplicateAnalysisPrompt
    app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: ds2_id.clone(),
        pending_entry_id: String::from("stats.one_way_anova"),
        existing_result_ids: vec![rs_id.clone()],
    };
    assert_contract(&mut app, "duplicate_analysis_prompt", Some(("duplicate.choice", json!("modify_existing"))))?;

    // 3. EditAnalysisParameters (ANOVA, PowerAnalysis, PValueStack)
    app.canvas_dialogs.open_edit_parameters(&doc, &rs_id);
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "p_value_format": "custom" } }))
        .map_err(|e| format!("enable custom p value format: {e}"))?;
    assert_contract(&mut app, "edit_analysis_parameters", Some(("edit.pvalue.digits", json!(3))))?;

    let pwr_id = SheetId::try_new("rs-pwr-1").map_err(|e| format!("{e}"))?;
    app.canvas_dialogs.open_edit_parameters(&doc, &pwr_id);
    assert_contract(&mut app, "edit_analysis_parameters", Some(("edit.power.alpha", json!(0.01))))?;

    let ps_id = SheetId::try_new("rs-ps-1").map_err(|e| format!("{e}"))?;
    app.canvas_dialogs.open_edit_parameters(&doc, &ps_id);
    assert_contract(&mut app, "edit_analysis_parameters", Some(("edit.pvalue_stack.threshold", json!(0.02))))?;

    // 4. PowerAndSampleSize
    app.canvas_dialogs.open_power_wizard();
    assert_contract(&mut app, "power_and_sample_size", Some(("power.alpha", json!(0.01))))?;

    // 5. FormatSelectedPoints
    app.canvas_dialogs.open_format_selected_points(&doc, &gs_id, vec![(0, 0)]);
    assert_contract(&mut app, "format_selected_points", Some(("format_points.show_row_labels", json!(true))))?;

    // 6. FormatGraph
    app.canvas_dialogs.open_format_graph(&doc, &gs_id);
    assert_contract(&mut app, "format_graph", Some(("format_graph.show_legend", json!(false))))?;

    // 7. FormatAxes
    let gs = doc.graph_sheet(&gs_id).ok_or_else(|| String::from("missing graph"))?;
    app.canvas_dialogs.format_axes.sync_from_graph(gs, AxisTarget::Y);
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gs_id.clone() };
    assert_contract(&mut app, "format_axes", Some(("axes.title", json!("Y Axis"))))?;

    // 8. PairwiseComparisons
    app.canvas_dialogs.open_pairwise_comparisons(&doc, &gs_id);
    assert_contract(&mut app, "pairwise_comparisons", Some(("pairwise.hide_non_significant", json!(false))))?;

    // 9. BubblePlotInspector
    app.canvas_dialogs.open_bubble_plot(&doc, &gs_id);
    assert_contract(&mut app, "bubble_plot_inspector", Some(("bubble.show_convex_hulls", json!(true))))?;

    // 10. ApplyStyleFrom
    app.canvas_dialogs.open_apply_style_from(&doc, &gs_id);
    assert_contract(&mut app, "apply_style_from", Some(("apply_style.clone_palette", json!(false))))?;

    // 11. PageLayoutComposer
    app.canvas_dialogs.open_page_layout_composer(&doc, &ls_id);
    assert_contract(&mut app, "page_layout_composer", Some(("layout.rows", json!(2))))?;

    // 12. ExportFigure
    app.canvas_dialogs.open_export_figure(&gs_id);
    assert_contract(&mut app, "export_figure", Some(("export.dpi", json!(600))))?;

    // 13. ConvertArchetype
    app.canvas_dialogs.open_convert_archetype(&doc, &ds2_id);
    assert_contract(&mut app, "convert_archetype", Some(("convert.allow_lossy", json!(true))))?;

    // 14. ImportReshape
    app.canvas_dialogs.open_import_reshape();
    assert_contract(&mut app, "import_reshape", Some(("import.title", json!("Reshaped"))))?;

    // 15. ExcludeWithReason
    app.canvas_dialogs.open_exclude_with_reason(ds_id.clone(), vec![GridCellCoord::new(0, 0, 0)]);
    assert_contract(&mut app, "exclude_with_reason", Some(("exclude.note", json!("Outlier"))))?;

    // 16. RandomizeGroups
    app.canvas_dialogs.open_randomize_groups(&doc);
    assert_contract(&mut app, "randomize_groups", Some(("randomize.seed", json!(7))))?;

    // 17. RandomSampleRows
    app.canvas_dialogs.open_random_sample_rows(&doc, Some(&ds_id));
    assert_contract(&mut app, "random_sample_rows", Some(("sample.sample_size", json!(3))))?;

    // 18. SimulateDistributions
    app.canvas_dialogs.open_simulate_distributions();
    assert_contract(&mut app, "simulate_distributions", Some(("simulate.n_rows", json!(25))))?;

    // 19. InsertModelEquation
    app.canvas_dialogs.open_insert_model_equation(&doc, &gs_id);
    assert_contract(&mut app, "insert_model_equation", Some(("equation.bordered", json!(false))))?;

    // 20. Print
    app.open_print_dialog_for(Some(&gs_id));
    assert_contract(&mut app, "print", Some(("print.copies", json!(2))))?;

    // 21. ChangeDataAnalyzed
    app.canvas_dialogs.open_change_data_analyzed(&doc, &rs_id);
    assert_contract(&mut app, "change_data_analyzed", Some(("change_data.selected_data_sets", json!([0]))))?;

    // 22. ChangeGraphType
    app.execute_command("sheet.select", json!({ "sheet_id": gs_id.as_str() })).map_err(|e| format!("{e}"))?;
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open" }))
        .map_err(|e| format!("open change graph type: {e}"))?;
    assert_contract(&mut app, "change_graph_type", Some(("graph_type.summary_measure", json!("median"))))?;

    // 23. ConstantHookPicker
    app.canvas_dialogs.open_hook_picker(&doc, &rs_id, Some(String::from("Bottom")));
    assert_contract(&mut app, "constant_hook_picker", Some(("hook.search_query", json!("EC50"))))?;

    // 24. CommandPalette
    app.canvas_dialogs.open_command_palette();
    assert_contract(&mut app, "command_palette", Some(("palette.query", json!("analyze"))))?;

    // 25. Read-only / non-patchable dialogs reject field patches cleanly instead of returning updated: true.
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    assert_contract(&mut app, "about", None)?;
    app.canvas_dialogs.active_dialog = ActiveDialog::ResultViewer { result_sheet_id: rs_id };
    assert_contract(&mut app, "result_viewer", None)?;

    Ok(())
}

#[test]
fn r2_s10_03_16_localized_parsing_proportions_and_power_group2_errors() -> TestRes {
    let (mut app, ds_id, _, _, _) = ok_doc_and_app()?;
    let reply = handle_control_line(&mut app, &json!({ "id": 1, "method": "ui.locale.set", "params": { "locale": "de-DE" } }).to_string());
    let parsed_reply: Value = serde_json::from_str(&reply).map_err(|e| format!("{e}"))?;
    if parsed_reply["ok"].as_bool() != Some(true) {
        return Err(format!("locale set de-DE failed: {parsed_reply}"));
    }

    // Semicolon-separated comma-decimal proportions must NOT split `"0,2"` into 0 and 2!
    let doc = app.document().clone();
    app.canvas_dialogs.open_analyze_dialog(&doc, &ds_id);
    dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({ "fields": { "selected_entry_id": "stats.contingency", "expected_proportions": "0,2; 0,3; 0,5" } }),
    )
    .map_err(|e| format!("de-DE expected_proportions patch failed: {e}"))?;
    let props = app
        .canvas_dialogs
        .analyze
        .contingency_expected_proportions
        .as_ref()
        .ok_or_else(|| String::from("expected contingency_expected_proportions"))?;
    if props.len() != 3 || (props[0] - 0.2).abs() > 1e-9 || (props[1] - 0.3).abs() > 1e-9 || (props[2] - 0.5).abs() > 1e-9 {
        return Err(format!("wrong de-DE expected_proportions: {props:?}"));
    }
    app.canvas_dialogs.close_dialog();

    // Power wizard: rejects non-numeric string for sd_group2/sample_size_group2 instead of silently resetting to None.
    app.canvas_dialogs.open_power_wizard();
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "sd_group2": "2,5", "sample_size_group2": "12,0" } }))
        .map_err(|e| format!("de-DE power group2 localized patch failed: {e}"))?;
    if app.canvas_dialogs.power_wizard.sd_group2 != Some(2.5) || app.canvas_dialogs.power_wizard.sample_size_group2 != Some(12.0) {
        return Err(format!(
            "expected Some(2.5)/Some(12.0), got {:?}/{:?}",
            app.canvas_dialogs.power_wizard.sd_group2, app.canvas_dialogs.power_wizard.sample_size_group2
        ));
    }
    for bad_key in ["sd_group2", "sample_size_group2"] {
        let mut m = serde_json::Map::new();
        m.insert(bad_key.to_owned(), json!("abc"));
        if app.canvas_dialogs.power_wizard.apply_field_patch_localized(&m, &LocalePreset::comma_semicolon("de-DE")).is_ok() {
            return Err(format!("expected non-numeric string in `{bad_key}` to return Err instead of silent None"));
        }
    }
    app.canvas_dialogs.close_dialog();

    // SimulateDistributions localized mean
    app.canvas_dialogs.open_simulate_distributions();
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "simulate.mean": "3,75" } }))
        .map_err(|e| format!("de-DE simulate.mean failed: {e}"))?;
    let col0_mean = app.canvas_dialogs.simulate_distributions.columns.first().map(|c| c.mean);
    if col0_mean != Some(3.75) {
        return Err(format!("expected simulate mean 3.75 under de-DE, got {col0_mean:?}"));
    }
    Ok(())
}

#[test]
fn r2_s10_05_06_07_08_inspect_predicates_and_format_graph() -> TestRes {
    // R2-S10-05: cap grid_selection at 500 while reporting exact count, and respect scope filtering.
    let mut big_app = UiApp::new();
    let out = big_app
        .execute_command("sheet.createData", json!({ "title": "Big", "kind": "xy", "cols": 1, "rows": 2000, "auto_graph": false }))
        .map_err(|e| format!("{e}"))?;
    let big_id = SheetId::try_new(out.payload["sheet_id"].as_str().ok_or_else(|| String::from("big id"))?).map_err(|e| format!("{e}"))?;
    {
        let doc = big_app.document().clone();
        let ds = doc.data_sheet(&big_id).ok_or_else(|| String::from("missing big ds"))?;
        big_app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
        big_app.grid.extend_shift_range(ds, GridCellCoord::new(1999, 0, 0));
        big_app.grid.refresh_selection_stats(&doc);
    }
    let full = inspect_ui_tree(&big_app, None, false);
    if full.grid_selection.selected_cells_count != 2000
        || full.grid_selection.selected_coords.len() > 500
        || full.grid_selection.selected_rows.len() > 500
    {
        return Err(format!(
            "unexpected big grid snapshot: count={}, coords={}, rows={}",
            full.grid_selection.selected_cells_count,
            full.grid_selection.selected_coords.len(),
            full.grid_selection.selected_rows.len()
        ));
    }
    let toolbar_only = inspect_ui_tree(&big_app, Some("toolbar"), false);
    if !toolbar_only.grid_selection.selected_coords.is_empty() || toolbar_only.grid_cells.is_some() {
        return Err(String::from("expected scope=toolbar to skip grid selection coords and cells"));
    }

    // R2-S10-06: Dunnett routes to parametric rule (NOT Dunn), and multiplicity is NOT contingency CI.
    let (mut app, _, ds2_id, gs_id, _) = ok_doc_and_app()?;
    let rs_spec = AnalysisSpec::new(
        "rs-1",
        ds2_id,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..OneWayAnovaSpec::default() }),
    )
    .map_err(|e| format!("{e}"))?;
    let rs = ResultSheet::new(rs_spec).map_err(|e| format!("{e}"))?;
    let rs_id = rs.id.clone();
    {
        let mut d = app.document().clone();
        d.add_result_sheet(rs).map_err(|e| format!("{e}"))?;
        app = UiApp::from_session(curvebook_engine::Session::from_document(d));
    }
    let d_snap = app.document().clone();
    app.canvas_dialogs.open_edit_parameters(&d_snap, &rs_id);
    app.canvas_dialogs.edit_params.assume_gaussian = false;
    let dunnett = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.dunnett");
    let dunn = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.dunn");
    let mult = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.multiplicity_method");
    if dunnett.enabled || !dunn.enabled || !mult.enabled {
        return Err(format!("R2-S10-06 failed: dunnett={dunnett:?}, dunn={dunn:?}, mult={mult:?}"));
    }
    app.canvas_dialogs.close_dialog();

    // R2-S10-07: toggling half_violin_with_points off writes gdo with half_violin_with_points == false.
    let doc2 = app.document().clone();
    app.canvas_dialogs.open_format_graph(&doc2, &gs_id);
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "apply", "fields": { "half_violin_with_points": true } }))
        .map_err(|e| format!("format_graph violin toggle on failed: {e}"))?;
    let doc_on = app.document().clone();
    app.canvas_dialogs.open_format_graph(&doc_on, &gs_id);
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "half_violin_with_points": false } }))
        .map_err(|e| format!("format_graph violin toggle off failed: {e}"))?;
    let still_half = app.document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).and_then(|s| match s.gdo {
        GraphicalDataObject::Violin { half_violin_with_points, .. } => Some(half_violin_with_points),
        _ => None,
    });
    if still_half != Some(false) {
        return Err(format!("expected half_violin_with_points == Some(false) after toggle off, got {still_half:?}"));
    }

    // R2-S10-08: inspect_ui_tree_for_metrics uses caller viewport metrics.
    let small =
        inspect_ui_tree_for_metrics(&app, ViewportMetrics { logical_width: 1024.0, logical_height: 640.0, scale_factor: 1.0 }, Some("toolbar"), true);
    let large = inspect_ui_tree_for_metrics(
        &app,
        ViewportMetrics { logical_width: 1920.0, logical_height: 1200.0, scale_factor: 2.0 },
        Some("toolbar"),
        true,
    );
    let sb = small.toolbar.as_ref().and_then(|t| t.last()?.bounds).ok_or_else(|| String::from("missing small bounds"))?;
    let lb = large.toolbar.as_ref().and_then(|t| t.last()?.bounds).ok_or_else(|| String::from("missing large bounds"))?;
    if sb == lb {
        return Err(format!("expected viewport metrics to change inspect bounds, both were {sb:?}"));
    }
    Ok(())
}

#[test]
fn r2_s10_10_13_14_15_17_18_line_arrow_formula_discontig_print_wizard_row_labels() -> TestRes {
    let (mut app, ds_id, _, gs_id, _) = ok_doc_and_app()?;

    // R2-S10-10: LineOrArrow drag preserves negative-slope / reverse direction endpoints.
    app.execute_command(
        "graph.addAnnotation",
        json!({ "sheet_id": gs_id.as_str(), "kind": "arrow", "start_pt": [140.0, 60.0], "end_pt": [80.0, 120.0], "arrowhead": true }),
    )
    .map_err(|e| format!("add LineOrArrow: {e}"))?;
    app.execute_command("sheet.select", json!({ "sheet_id": gs_id.as_str() })).map_err(|e| format!("{e}"))?;
    app.sync_views();
    let metrics = ViewportMetrics::default();
    let tree = app.build_and_cache_layout_tree(metrics);
    if let Some(paper) = tree.scene_papers.first() {
        let mut x0 = 0.0;
        let mut y0 = 0.0;
        for px in (paper.paper.x..paper.paper.x + paper.paper.width).step_by(4) {
            for py in (paper.paper.y..paper.paper.y + paper.paper.height).step_by(4) {
                if let Some(sp) = paper.pixel_to_scene_pt(f64::from(px), f64::from(py))
                    && (sp.x - 110.0).abs() < 6.0
                    && (sp.y - 90.0).abs() < 6.0
                {
                    let lp = metrics.physical_to_logical_px([f64::from(px), f64::from(py)]);
                    x0 = lp[0];
                    y0 = lp[1];
                    break;
                }
            }
            if x0 > 0.0 {
                break;
            }
        }
        if x0 > 0.0 {
            let _ = dispatch_ui_event(
                &mut app,
                UiInputEvent::PointerDown { x: x0, y: y0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            );
            let _ =
                dispatch_ui_event(&mut app, UiInputEvent::PointerMove { x: x0 + 20.0, y: y0 + 10.0, modifiers: KeyModifiers::default() }, metrics);
            let _ = dispatch_ui_event(
                &mut app,
                UiInputEvent::PointerUp { x: x0 + 20.0, y: y0 + 10.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
                metrics,
            );
            if let Some(GraphAnnotation::LineOrArrow { start_pt, end_pt, .. }) =
                app.document().graph_sheet(&gs_id).and_then(|g| g.spec.annotations.first())
                && (start_pt.0 <= end_pt.0 || start_pt.1 >= end_pt.1)
            {
                return Err(format!("LineOrArrow signed orientation collapsed after drag: start={start_pt:?}, end={end_pt:?}"));
            }
        }
    }

    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| format!("{e}"))?;

    // R2-S10-13: select_row_click does not reset active formula bar; inspect enables commit with target_title.
    let doc_mv = app.document().clone();
    let ds = doc_mv.data_sheet(&ds_id).ok_or_else(|| String::from("ds-1"))?;
    app.grid.formula_bar.open_for_new_column(ds, "Ratio", Some(&doc_mv));
    app.grid.formula_bar.set_input("=A + 1", None, &doc_mv);
    app.grid.select_row_click(ds, 0, false, false);
    if !app.grid.formula_bar.active || app.grid.formula_bar.input != "=A + 1" {
        return Err(String::from("select_row_click reset active formula bar"));
    }
    let fb_snap = inspect_ui_tree(&app, Some("grid"), false);
    if let Some(nodes) = fb_snap.grid_cells.as_ref()
        && let Some(commit_node) = nodes.iter().find(|n| n.widget_id == "formula_bar.commit")
        && !commit_node.enabled
    {
        return Err(format!("formula_bar.commit unexpectedly disabled for new named formula column: {:?}", commit_node.disabled_reason));
    }
    let fb_tree = app.build_and_cache_layout_tree(ViewportMetrics::default());
    let commit_widget = fb_tree.node("formula_bar.commit").ok_or_else(|| String::from("missing formula_bar.commit in LayoutTree"))?;
    if !commit_widget.state.enablement.enabled || commit_widget.action == WidgetAction::NoOp {
        return Err(format!(
            "R2-S10-13 visual formula_bar.commit unexpectedly disabled for non-empty target_title: enabled={}, reason={:?}",
            commit_widget.state.enablement.enabled, commit_widget.state.enablement.disabled_reason
        ));
    }
    app.grid.formula_bar.active = false;

    // R2-S10-14: Discontiguous Cmd+Click preserves exact 2 selected cells in visible_frame, inspect, quick_stats, exclude intents.
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    app.grid.toggle_cmd_selection(ds, GridCellCoord::new(2, 1, 0));
    app.grid.refresh_selection_stats(&doc_mv);
    if let Some(frame) = app.grid.visible_frame(app.document()) {
        let sel_coords: Vec<GridCellCoord> =
            frame.rows.iter().flat_map(|r| r.cells.iter()).filter(|c| c.visual.is_selected).map(|c| c.coord).collect();
        if sel_coords != vec![GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 1, 0)] {
            return Err(format!("expected ONLY 2 discontiguous cells selected in visible_frame, got {sel_coords:?}"));
        }
    }
    if app.grid.quick_stats.map(|q| q.selected_cells) != Some(2) {
        return Err(format!("expected quick_stats.selected_cells == 2, got {:?}", app.grid.quick_stats));
    }
    let intent_count = app.grid.emit_toggle_exclude_intents(ds, true);
    if intent_count != 2 {
        return Err(format!("expected 2 toggle exclude intents for discontiguous cells, got {intent_count}"));
    }

    // R2-S10-18: FormatSelectedPoints row labels placed at distinct coordinates near plotted points.
    app.canvas_dialogs.open_format_selected_points(&doc_mv, &gs_id, vec![(0, 0), (0, 2)]);
    dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "show_row_labels": true } }))
        .map_err(|e| format!("FormatSelectedPoints ok failed: {e}"))?;
    let text_ann_positions: Vec<(f64, f64)> = app
        .document()
        .graph_sheet(&gs_id)
        .map(|g| {
            g.spec
                .annotations
                .iter()
                .filter_map(|a| match a {
                    GraphAnnotation::TextBox { bounds_pt, .. } => Some((bounds_pt.x, bounds_pt.y)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if text_ann_positions.len() < 2 || text_ann_positions[0] == text_ann_positions[1] {
        return Err(format!("expected distinct scene coordinates for row labels, got {text_ann_positions:?}"));
    }

    // R2-S10-15: Print dialog reports spooled/submitted honestly and guards submit_to_os under --gui-only.
    app.open_print_dialog_for(Some(&gs_id));
    let pr = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok" })).map_err(|e| format!("print spool failed: {e}"))?;
    if pr["spooled"].as_bool() != Some(true) || pr["submitted_to_os"].as_bool() != Some(false) || pr["printed"].as_bool() != Some(false) {
        return Err(format!("unexpected default Print ok response: {pr}"));
    }
    app.set_gui_only(true);
    app.open_print_dialog_for(Some(&gs_id));
    let submit_err = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "submit_to_os": true } }))
        .err()
        .ok_or_else(|| String::from("expected GuiOnlyViolation for print submit_to_os under --gui-only"))?;
    if submit_err.code != ErrorCode::GuiOnlyViolation {
        return Err(format!("expected GuiOnlyViolation on submit_to_os, got {:?}", submit_err.code));
    }
    app.canvas_dialogs.close_dialog();
    app.set_gui_only(false);

    // R2-S10-17: PowerWizard switch_tab steps sequentially and fails when intermediate stage invalid.
    app.canvas_dialogs.open_power_wizard();
    app.canvas_dialogs.power_wizard.alpha = 2.0; // invalid alpha at stage 3
    if dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab": "results" })).is_ok() {
        return Err(String::from("expected PowerWizard switch_tab to fail on invalid intermediate stage"));
    }
    app.canvas_dialogs.power_wizard.alpha = 0.05;
    dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab": "results" }))
        .map_err(|e| format!("valid PowerWizard sequential switch_tab failed: {e}"))?;
    if app.canvas_dialogs.power_wizard.stage != PowerWizardStage::Results || app.canvas_dialogs.power_wizard.result.is_none() {
        return Err(String::from("expected PowerWizard to reach Results with computed result"));
    }

    Ok(())
}

#[test]
fn r2_s10_19_virtual_workspace_validates_and_inline_backspace_syncs() -> TestRes {
    let mut corrupt_vws = VirtualWorkspaceFolder::new("invalid-vws");
    corrupt_vws.write_file("document.json", b"{ \"invalid\": true ".to_vec()).map_err(|e| format!("write corrupt vws json: {e}"))?;
    if corrupt_vws.open_unpacked_snapshot().is_ok() {
        return Err(String::from("expected VirtualWorkspaceFolder to reject corrupt document.json"));
    }

    let (mut app, ds_id, _, _, _) = ok_doc_and_app()?;
    app.inline_editor.begin(InlineEditorTarget::GridCell { sheet_id: ds_id, coord: GridCellCoord::new(0, 0, 0) }, String::from("123"));
    app.grid.tool.editing_buffer = Some(String::from("123"));
    dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("Backspace"), modifiers: KeyModifiers::default() },
        ViewportMetrics::default(),
    )
    .map_err(|e| format!("inline backspace failed: {e}"))?;
    if app.grid.tool.editing_buffer.as_deref() != Some("12") {
        return Err(format!("expected synced editing_buffer == \"12\", got {:?}", app.grid.tool.editing_buffer));
    }
    Ok(())
}

#[test]
fn interaction_gui_actions_items_1_3_4_5_new1() -> TestRes {
    let (mut app, ds_id, _, gs_id, _) = ok_doc_and_app()?;

    // Item 1: shortcuts (e.g. Cmd+T) must NOT run while modal is open even if stale workspace textbox had focus.
    let rs_spec = AnalysisSpec::new("rs-1", ds_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| format!("{e}"))?;
    let rs = ResultSheet::new(rs_spec).map_err(|e| format!("{e}"))?;
    let rs_id = rs.id.clone();
    {
        let mut doc_with_res = app.document().clone();
        doc_with_res.add_result_sheet(rs).map_err(|e| format!("{e}"))?;
        app = UiApp::from_session(curvebook_engine::Session::from_document(doc_with_res));
        app.execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() })).map_err(|e| format!("{e}"))?;
    }
    app.canvas_dialogs.open_command_palette();
    app.focus_widget_id = Some(String::from("formula_bar.input"));
    let pressed = dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": ["meta"] }))
        .map_err(|e| format!("press_key Cmd+T while modal open errored: {e}"))?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
        return Err(format!("Cmd+T escaped modal CommandPalette! outcome={pressed}"));
    }
    app.canvas_dialogs.close_dialog();

    // Item 3: focused formula_bar.input Escape closes formula bar cleanly via ui.press_key.
    app.execute_command("sheet.select", json!({ "sheet_id": ds_id.as_str() })).map_err(|e| format!("{e}"))?;
    let doc_ds = app.document().clone();
    let ds = doc_ds.data_sheet(&ds_id).ok_or_else(|| String::from("ds-1"))?;
    app.grid.formula_bar.open_for_new_column(ds, "NewCol", Some(&doc_ds));
    app.focus_widget_id = Some(String::from("formula_bar.input"));
    dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Escape" })).map_err(|e| format!("formula Escape failed: {e}"))?;
    if app.grid.formula_bar.active {
        return Err(String::from("expected formula_bar Escape via ui.press_key to close formula bar"));
    }

    // Item 4: secondary right-click on empty margin clears context_menu_target & pointer_gesture never sets label string.
    app.interaction.context_menu_target = Some(String::from("grid"));
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "right_click", "x": 5.0, "y": 5.0 }))
        .map_err(|e| format!("right_click empty corner failed: {e}"))?;
    if app.interaction.context_menu_target.is_some() {
        return Err(format!("expected context_menu_target cleared on empty right-click, got {:?}", app.interaction.context_menu_target));
    }

    // Item 5: ui.type_text respects de-DE decimal separator for cell, pasted block, and X cell.
    let _ = handle_control_line(&mut app, &json!({ "id": 1, "method": "ui.locale.set", "params": { "locale": "de-DE" } }).to_string());
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.0.0.0", "text": "3,75" }))
        .map_err(|e| format!("de-DE single cell type_text failed: {e}"))?;
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "grid.xcell.0.0", "text": "1,25" }))
        .map_err(|e| format!("de-DE X cell type_text failed: {e}"))?;
    dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.1.0.0", "text": "4,5\n5,5" }))
        .map_err(|e| format!("de-DE multiline paste type_text failed: {e}"))?;
    let ds_after = app.document().data_sheet(&ds_id).ok_or_else(|| String::from("missing ds after de-DE type_text"))?;
    let num_val = |v: Option<&CellValue>| -> Option<f64> {
        match v {
            Some(CellValue::Number(x)) => Some(*x),
            _ => None,
        }
    };
    let c0 = num_val(ds_after.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value));
    let x0 = num_val(ds_after.x_or_time_cell(0, 0).map(|c| &c.value));
    let c1 = num_val(ds_after.cell(CellCoord::new(1, 0, 0)).map(|c| &c.value));
    let c2 = num_val(ds_after.cell(CellCoord::new(2, 0, 0)).map(|c| &c.value));
    if c0 != Some(3.75) || x0 != Some(1.25) || c1 != Some(4.5) || c2 != Some(5.5) {
        return Err(format!("de-DE ui.type_text mismatch: c0={c0:?}, x0={x0:?}, c1={c1:?}, c2={c2:?}"));
    }

    // NEW-1: active inline_editor auto-commits on sheet switch / navigation instead of dropping buffer!
    app.inline_editor.begin(InlineEditorTarget::GridCell { sheet_id: ds_id.clone(), coord: GridCellCoord::new(0, 1, 0) }, String::from("42,5"));
    execute_widget_action(&mut app, &WidgetAction::SwitchSheet { sheet_id: gs_id, section: SheetSection::Graphs }, ViewportMetrics::default())
        .map_err(|e| format!("SwitchSheet auto-commit failed: {e}"))?;
    let saved = num_val(app.document().data_sheet(&ds_id).and_then(|ds| ds.cell(CellCoord::new(0, 1, 0))).map(|c| &c.value));
    if saved != Some(42.5) || app.inline_editor.active {
        return Err(format!("expected NEW-1 auto-commit to save 42.5, got {saved:?}, active={}", app.inline_editor.active));
    }

    Ok(())
}

fn assert_rect_inside(outer: WorkspacePixelRect, inner: WorkspacePixelRect, label: &str) -> TestRes {
    if inner.x < outer.x || inner.y < outer.y || inner.right() > outer.right() || inner.bottom() > outer.bottom() {
        return Err(format!("{label} rect {inner:?} escaped primary split pane {outer:?}"));
    }
    Ok(())
}

#[test]
fn r2_s10_09_split_pane_preserves_primary_singleton_geometry() -> TestRes {
    let (mut app, ds_id, ds2_id, gs_id, _) = ok_doc_and_app()?;

    let rs1_spec =
        AnalysisSpec::new("rs-split-1", ds_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| format!("{e}"))?;
    let rs2_spec =
        AnalysisSpec::new("rs-split-2", ds2_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| format!("{e}"))?;
    let mut rs1 = ResultSheet::new(rs1_spec).map_err(|e| format!("{e}"))?;
    let mut rs2 = ResultSheet::new(rs2_spec).map_err(|e| format!("{e}"))?;
    let warn_item = ChecklistItem {
        assumption: String::from("Normality"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: None,
        explanation: String::from("Minor skew"),
        remediation: None,
    };
    rs1.checklist.push(warn_item.clone());
    rs2.checklist.push(warn_item);
    let rs1_id = rs1.id.clone();
    let rs2_id = rs2.id.clone();

    let info1_id = SheetId::from("info-1");
    let info2 = InfoSheet::new_preseeded("info-2", "Secondary Info").map_err(|e| format!("{e}"))?;
    let info2_id = info2.id.clone();

    let gs2_id = {
        let mut doc = app.document().clone();
        let ds2 = doc.data_sheet(&ds2_id).ok_or_else(|| String::from("missing ds2"))?.clone();
        let gs2 = GraphSheet::new_uninitialized_for_data("g-split-2", &ds2).map_err(|e| format!("{e}"))?;
        let gs2_id = gs2.id.clone();
        doc.add_graph_sheet(gs2).map_err(|e| format!("{e}"))?;
        doc.add_result_sheet(rs1).map_err(|e| format!("{e}"))?;
        doc.add_result_sheet(rs2).map_err(|e| format!("{e}"))?;
        doc.add_info_sheet(info2).map_err(|e| format!("{e}"))?;
        app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        gs2_id
    };

    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    app.split.enabled = true;

    // 1. Two DataSheets: primary grid_geometry preserved.
    app.split.primary_sheet_id = Some(ds_id.clone());
    app.split.secondary_sheet_id = Some(ds2_id.clone());
    let grid_tree = LayoutTree::build(&app, metrics);
    let grid_prim = grid_tree.split_primary.ok_or_else(|| String::from("missing split_primary for grid"))?;
    let grid_geom = grid_tree.grid_geometry.as_ref().ok_or_else(|| String::from("missing grid_geometry"))?;
    if grid_geom.sheet_id != ds_id {
        return Err(format!("secondary DataSheet overwrote primary grid_geometry: expected {ds_id:?}, got {:?}", grid_geom.sheet_id));
    }
    assert_rect_inside(grid_prim, grid_geom.sheet_rect, "grid_geometry.sheet_rect")?;

    // 2. Two GraphSheets: both scene_papers kept, primary title/axis rects preserved.
    app.split.primary_sheet_id = Some(gs_id.clone());
    app.split.secondary_sheet_id = Some(gs2_id.clone());
    let graph_tree = LayoutTree::build(&app, metrics);
    let graph_prim = graph_tree.split_primary.ok_or_else(|| String::from("missing split_primary for graph"))?;
    let paper_ids: Vec<&SheetId> = graph_tree.scene_papers.iter().map(|p| &p.sheet_id).collect();
    if paper_ids != vec![&gs_id, &gs2_id] {
        return Err(format!("expected both scene_papers [{gs_id:?}, {gs2_id:?}], got {paper_ids:?}"));
    }
    let graph_map = graph_tree.to_workspace_layout_map();
    if graph_map.scene_paper.as_ref().map(|p| &p.sheet_id) != Some(&gs_id) {
        return Err(format!("expected map.scene_paper == {gs_id:?}, got {:?}", graph_map.scene_paper.as_ref().map(|p| &p.sheet_id)));
    }
    for (label, rect_opt) in [
        ("canvas_title_rect", graph_tree.canvas_title_rect),
        ("x_axis_title_rect", graph_tree.x_axis_title_rect),
        ("y_axis_title_rect", graph_tree.y_axis_title_rect),
    ] {
        let rect = rect_opt.ok_or_else(|| format!("missing {label}"))?;
        assert_rect_inside(graph_prim, rect, label)?;
    }

    // 3. Two ResultSheets with diagnostic banners: primary banner rect & sheet ID preserved.
    app.split.primary_sheet_id = Some(rs1_id.clone());
    app.split.secondary_sheet_id = Some(rs2_id);
    let res_tree = LayoutTree::build(&app, metrics);
    let res_prim = res_tree.split_primary.ok_or_else(|| String::from("missing split_primary for result"))?;
    if res_tree.result_banner_sheet_id.as_ref() != Some(&rs1_id) {
        return Err(format!("secondary ResultSheet overwrote result_banner_sheet_id: {:?}", res_tree.result_banner_sheet_id));
    }
    let banner_rect = res_tree.result_banner_rect.ok_or_else(|| String::from("missing result_banner_rect"))?;
    assert_rect_inside(res_prim, banner_rect, "result_banner_rect")?;

    // 4. Two InfoSheets: primary info_sheet_id & notes rect preserved.
    app.split.primary_sheet_id = Some(info1_id.clone());
    app.split.secondary_sheet_id = Some(info2_id);
    let info_tree = LayoutTree::build(&app, metrics);
    let info_prim = info_tree.split_primary.ok_or_else(|| String::from("missing split_primary for info"))?;
    if info_tree.info_sheet_id.as_ref() != Some(&info1_id) {
        return Err(format!("secondary InfoSheet overwrote info_sheet_id: {:?}", info_tree.info_sheet_id));
    }
    let notes_rect = info_tree.info_notes_rect.ok_or_else(|| String::from("missing info_notes_rect"))?;
    assert_rect_inside(info_prim, notes_rect, "info_notes_rect")?;

    // 5. Disjoint categories (Data primary + Graph secondary): secondary populates empty singleton slots.
    app.split.primary_sheet_id = Some(ds_id.clone());
    app.split.secondary_sheet_id = Some(gs_id.clone());
    let mixed_tree = LayoutTree::build(&app, metrics);
    let mixed_sec = mixed_tree.split_secondary.ok_or_else(|| String::from("missing split_secondary for mixed"))?;
    if mixed_tree.grid_geometry.as_ref().map(|g| &g.sheet_id) != Some(&ds_id) {
        return Err(String::from("mixed split missing primary grid_geometry"));
    }
    let mixed_title = mixed_tree.canvas_title_rect.ok_or_else(|| String::from("mixed split missing secondary canvas_title_rect"))?;
    assert_rect_inside(mixed_sec, mixed_title, "mixed secondary canvas_title_rect")?;

    Ok(())
}
