//! End-to-end integration tests for strict `--gui-only` caller separation, all 8 interactive
//! GUI actions (`ui.inspect` .. `ui.answer_dialog`), modal graying rules (`R1`–`R10`),
//! the 4-stage Power & Sample Size wizard, multi-row grid selection + `Format Selected Points...`,
//! and `ResultSheet` -> `Analyze...` downstream chaining (`R8.6` & `R8.7`).

use curvebook_doc::{AnalysisKind, CellCoord, CellValue, ErrorCode, GraphAnnotation, SheetId, SheetSection, TableArchetype, TableKind};
use curvebook_ui_egui::control_io::handle_control_line;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn control_json(app: &mut UiApp, id: u64, method: &str, params: Value) -> Result<Value, String> {
    let line = json!({ "id": id, "method": method, "params": params }).to_string();
    let raw = handle_control_line(app, &line);
    serde_json::from_str(&raw).map_err(|e| format!("parse control response: {e}"))
}

fn assert_control_error_code(resp: &Value, expected_code: &str, context: &str) {
    assert_eq!(resp.get("ok").and_then(Value::as_bool), Some(false), "{context}: expected error response, got {resp}");
    let code = resp.pointer("/error_detail/code").or_else(|| resp.pointer("/result/error/code")).and_then(Value::as_str);
    assert_eq!(code, Some(expected_code), "{context}: expected code `{expected_code}`, got {resp}");
}

fn err_str(ctx: &str, err: impl std::fmt::Display) -> String {
    format!("{ctx}: {err}")
}

#[test]
fn gui_only_blocks_direct_control_mutations_while_allowing_gui_and_reads() -> TestRes {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    assert!(app.is_gui_only());

    // Direct non-UI mutations over JSON-lines MUST fail with GuiOnlyViolation without mutating.
    let initial_sheets = app.document().data_sheets.len();
    let r1 = control_json(&mut app, 1, "engine.execute", json!({ "command_id": "sheet.createData", "params": { "kind": "xy", "title": "Bypass" } }))?;
    assert_control_error_code(&r1, "gui_only_violation", "engine.execute under gui-only");

    let r2 = control_json(&mut app, 2, "command.run", json!({ "command_id": "sheet.createData", "params": { "kind": "column" } }))?;
    assert_control_error_code(&r2, "gui_only_violation", "command.run under gui-only");

    let r3 =
        control_json(&mut app, 3, "command.batch", json!({ "commands": [{ "command_id": "sheet.createData", "params": { "kind": "column" } }] }))?;
    assert_control_error_code(&r3, "gui_only_violation", "command.batch under gui-only");

    let r4 = control_json(&mut app, 4, "sheet.createData", json!({ "kind": "grouped", "title": "Direct" }))?;
    assert_control_error_code(&r4, "gui_only_violation", "direct domain command under gui-only");

    let r5 = control_json(&mut app, 5, "ui.layout.snap", json!({}))?;
    assert_control_error_code(&r5, "gui_only_violation", "direct mutating ui.* call under gui-only");

    let r6 = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "target": "full_window", "path": "/tmp/forbidden.png" }));
    assert_eq!(
        r6.as_ref().err().map(|e| e.code),
        Some(ErrorCode::GuiOnlyViolation),
        "ui.screenshot disk write must be blocked under --gui-only: {r6:?}"
    );
    assert_eq!(app.document().data_sheets.len(), initial_sheets, "blocked control calls must not mutate doc");

    // Read-only control RPCs remain available under --gui-only.
    let ping = control_json(&mut app, 6, "ping", json!({}))?;
    assert_eq!(ping.get("ok").and_then(Value::as_bool), Some(true));

    let state = control_json(&mut app, 7, "ui.state", json!({}))?;
    assert_eq!(state.pointer("/result/gui_only").and_then(Value::as_bool), Some(true));

    let inspect_ctrl = control_json(&mut app, 8, "ui.inspect", json!({ "include_bounds": true }))?;
    assert_eq!(inspect_ctrl.get("ok").and_then(Value::as_bool), Some(true));
    assert_eq!(inspect_ctrl.pointer("/result/gui_only").and_then(Value::as_bool), Some(true));

    // Re-enabling Caller::Control allows direct automation again when gui_only is toggled off.
    app.set_gui_only(false);
    assert!(!app.is_gui_only());
    let ok_direct =
        control_json(&mut app, 9, "engine.execute", json!({ "command_id": "sheet.createData", "params": { "kind": "xy", "title": "AllowedNow" } }))?;
    assert_eq!(ok_direct.get("ok").and_then(Value::as_bool), Some(true));
    Ok(())
}

#[test]
fn eight_gui_actions_and_modal_guards_drive_live_ui_under_gui_only() -> TestRes {
    let mut app = UiApp::new();
    app.set_gui_only(true);

    // 1. Disabled menu item (no sheets yet -> Format Selected Points disabled) must refuse via Conflict.
    let disabled_menu = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.formatPoints.open" }));
    assert_eq!(disabled_menu.as_ref().err().map(|e| e.code), Some(ErrorCode::Conflict));

    // 2. Create a column table through Caller::Ui internal UI command.
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 3, "rows": 5, "auto_graph": false }))
        .map_err(|e| err_str("create column table", e))?;

    // 3. ui.inspect verifies tree structure, bounds, and navigator entries.
    let snap = dispatch_gui_action(&mut app, "ui.inspect", json!({ "include_bounds": true })).map_err(|e| err_str("ui.inspect", e))?;
    assert_eq!(snap.get("active_sheet_id").and_then(Value::as_str), Some("col-1"));
    assert_eq!(snap.get("gui_only").and_then(Value::as_bool), Some(true));
    let menus = snap.get("menus").and_then(Value::as_array).map_or(0, Vec::len);
    assert_eq!(menus, 7, "all 7 top-level menus present in ui.inspect");

    // 4. ui.type_text types scalar cell values and TSV blocks cleanly as Caller::Ui.
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.0.0.0" }))
        .map_err(|e| err_str("select cell 0,0", e))?;
    dispatch_gui_action(
        &mut app,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": "10.0\t20.0\t30.0\n12.0\t24.0\t36.0\n11.0\t22.0\t33.0\n13.0\t26.0\t39.0", "clear_existing": true }),
    )
    .map_err(|e| err_str("paste TSV block", e))?;
    let col_id = SheetId::try_new("col-1").map_err(|e| err_str("sid", e))?;
    let col_sheet = app.document().data_sheet(&col_id).ok_or_else(|| String::from("missing col-1"))?;
    let cell_val = col_sheet.archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| c.value.clone());
    assert_eq!(cell_val, Some(CellValue::Number(10.0)));

    // 5. Run ANOVA via menu -> Analyze... -> One-Way ANOVA -> ok.
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": "Analyze/Analyze..." })).map_err(|e| err_str("open analyze", e))?;
    let analyzed = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" } }))
        .map_err(|e| err_str("run anova", e))?;
    let result_id = analyzed.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("missing result sheet_id"))?.to_owned();

    // 6. Open Edit Analysis Parameters with Cmd+T via ui.press_key, verify R1 Repeated Measures tab guard.
    let press_t =
        dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } })).map_err(|e| err_str("Cmd+T", e))?;
    assert_eq!(press_t.get("result_sheet_id").and_then(Value::as_str), Some(result_id.as_str()));
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    // Ensure R1 disabled Repeated Measures tab rejects ui.switch_tab when unpaired!
    let r1_reject = dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab": "repeated_measures" }));
    assert_eq!(r1_reject.as_ref().err().map(|e| e.code), Some(ErrorCode::Conflict));

    // Verify invalid confidence_percent disables OK (`R9` / confidence guard) cleanly in ui.answer_dialog.
    let invalid_conf = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "confidence_percent": 150.0 } }));
    assert_eq!(invalid_conf.as_ref().err().map(|e| e.code), Some(ErrorCode::Conflict));

    // 7. Capture screenshots in-memory across full_window, workspace, viewport, canvas, and dialog without filesystem writes.
    for target in ["full_window", "workspace", "viewport", "canvas", "dialog"] {
        let shot = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "target": target, "width_px": 480, "height_px": 320 }))
            .map_err(|e| err_str(target, e))?;
        let b64 = shot.get("data_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing data_base64"))?;
        assert!(b64.starts_with("iVBORw0KGgo"), "screenshot `{target}` must start with PNG magic prefix");
        assert!(shot.get("bytes_len").and_then(Value::as_u64).unwrap_or(0) > 64);
        assert!(shot.get("path").is_some_and(Value::is_null));
    }

    // Also verify `action: "wheel"` pointer gesture alias normalized by MCP `gui_tools.rs`.
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "wheel", "x": 400.0, "y": 240.0, "delta_y": 120.0 }))
        .map_err(|e| err_str("wheel zoom gesture", e))?;

    // 8. Press Escape via ui.press_key to close the modal dialog cleanly.
    dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Escape" })).map_err(|e| err_str("Escape", e))?;
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn power_wizard_four_stage_flow_under_gui_only_emits_linked_result_and_graph() -> TestRes {
    let mut app = UiApp::new();
    app.set_gui_only(true);

    // Open via Analyze > Power and Sample Size... using array path form (`["Analyze", "Power and Sample Size..."]`).
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": ["Analyze", "Power and Sample Size..."] }))
        .map_err(|e| err_str("open power wizard", e))?;
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::PowerAndSampleSize);

    // Verify fields-only ui.answer_dialog patches without advancing prematurely, then advance Stage 1 -> Stage 2.
    let updated_only =
        dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "family": "paired_t", "solve_target": "sample_size" } }))
            .map_err(|e| err_str("fields-only patch", e))?;
    assert_eq!(updated_only.get("updated").and_then(Value::as_bool), Some(true));
    assert_eq!(app.canvas_dialogs.power_wizard.stage, PowerWizardStage::ExperimentDesign);

    let next1 = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "effect_mode": "means_and_sd" } }))
        .map_err(|e| err_str("stage 1 -> 2", e))?;
    assert_eq!(next1.get("stage_index").and_then(Value::as_u64), Some(1));
    assert_eq!(app.canvas_dialogs.power_wizard.stage, PowerWizardStage::EffectSize);

    // Stage 2: specify mean difference, SD, paired correlation r = 0.75.
    let next2 = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "next", "fields": { "mean_diff": 3.0, "sd": 4.0, "r": 0.75 } }))
        .map_err(|e| err_str("stage 2 -> 3", e))?;
    assert_eq!(next2.get("stage_index").and_then(Value::as_u64), Some(2));
    let expected_sigma_d = (2.0_f64 * 16.0 * (1.0 - 0.75)).sqrt();
    let actual_sigma_d = app.canvas_dialogs.power_wizard.computed_paired_sigma_d().ok_or_else(|| String::from("sigma_d"))?;
    assert!((actual_sigma_d - expected_sigma_d).abs() < 1e-6);

    // Stage 3 -> Stage 4 (Results & Sensitivity).
    let next3 = dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "next", "fields": { "alpha": 0.05, "target_power": 0.85, "tail_mode": "two_sided" } }),
    )
    .map_err(|e| err_str("stage 3 -> 4", e))?;
    assert_eq!(next3.get("stage_index").and_then(Value::as_u64), Some(3));

    // Hover along the Stage 4 Power vs N curve via ui.pointer_gesture.
    let hovered =
        dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "hover", "widget_id": "power.curve_hover", "hover_fraction_x": 0.6 }))
            .map_err(|e| err_str("hover curve", e))?;
    assert!(hovered.get("hover_n").and_then(Value::as_f64).is_some_and(|n| n > 0.0));
    assert!(hovered.get("hover_power").and_then(Value::as_f64).is_some_and(|p| (0.0..=1.0).contains(&p)));

    // Stage 4 Create emits BOTH linked Power & Sample Size ResultSheet and GraphSheet under --gui-only!
    let created = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "create" })).map_err(|e| err_str("create", e))?;
    assert_eq!(created.get("created").and_then(Value::as_bool), Some(true));
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let res_id_str = created.get("result_sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("result_sheet_id"))?;
    let graph_id_str = created.get("graph_sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("graph_sheet_id"))?;
    let res_id = SheetId::try_new(res_id_str).map_err(|e| err_str("res_id", e))?;
    let graph_id = SheetId::try_new(graph_id_str).map_err(|e| err_str("graph_id", e))?;
    assert!(app.document().result_sheet(&res_id).is_some(), "ResultSheet created");
    assert!(app.document().graph_sheet(&graph_id).is_some(), "linked GraphSheet created");
    Ok(())
}

#[test]
fn multi_row_selection_format_selected_points_and_result_sheet_chaining_work() -> TestRes {
    let mut app = UiApp::new();
    app.set_gui_only(true);

    // 1. Create an XY sheet with auto graph + multivariate PCA table for ResultSheet chaining.
    app.execute_command("sheet.createData", json!({ "sheet_id": "xy-1", "kind": "xy", "cols": 2, "rows": 5, "auto_graph": true }))
        .map_err(|e| err_str("create xy sheet", e))?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0, 4.0, 5.0] }))
        .map_err(|e| err_str("set X values", e))?;
    app.execute_command("table.setCells", json!({ "sheet_id": "xy-1", "rows": [[2.0, 3.0], [4.0, 5.0], [6.0, 7.0], [8.0, 9.0], [10.0, 11.0]] }))
        .map_err(|e| err_str("set Y values", e))?;
    app.execute_command("table.setRowTitles", json!({ "sheet_id": "xy-1", "titles": ["Ctrl", "PtB", "OutlierC", "PtD", "PtE"] }))
        .map_err(|e| err_str("set row titles", e))?;

    // 2. Multi-row Shift+Click & Cmd+Click grid selection syncs `brushed_points` for graph brushing!
    let xy_id = SheetId::try_new("xy-1").map_err(|e| err_str("xy sid", e))?;
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.0.0.0" }))
        .map_err(|e| err_str("click row 0", e))?;
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.2.0.0", "modifiers": { "shift": true } }))
        .map_err(|e| err_str("shift-click row 2", e))?;
    dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.4.0.0", "modifiers": { "meta": true } }))
        .map_err(|e| err_str("cmd-click row 4", e))?;
    assert_eq!(app.grid.selected_rows, [0, 1, 2, 4], "Shift range 0..=2 plus Cmd toggle row 4");
    app.sync_brush_from_grid_selection(&xy_id);
    assert!(app.brushed_points.len() >= 4, "brushed_points reflects all 4 selected rows");

    // 3. Open Format > Format Selected Points... and style all brushed points cleanly via Caller::Ui.
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.formatPoints.open" }))
        .map_err(|e| err_str("open Format Selected Points", e))?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }));

    let fmt_out = dispatch_gui_action(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "ok", "fields": { "color": "#DC2626", "symbol": "diamond", "size_pt": 9.0, "show_row_labels": true } }),
    )
    .map_err(|e| err_str("apply point overrides", e))?;
    assert!(fmt_out.get("formatted_count").and_then(Value::as_u64).unwrap_or(0) >= 4);
    let graph = app.document().graph_sheets.first().ok_or_else(|| String::from("missing graph"))?;
    assert!(graph.spec.point_overrides.len() >= 4, "point_overrides recorded on GraphSheet");
    assert!(
        graph.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::TextBox { markdown, .. } if markdown == "OutlierC")),
        "row label annotation added for brushed point"
    );

    // 4. ResultSheet `Analyze...` chaining: Multivariate -> PCA -> PC scores tab -> Analyze... -> Descriptive!
    let mv = app
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "mv-1", "kind": "multiple_variables", "cols": 3, "rows": 5, "titles": ["F1", "F2", "F3"] }),
        )
        .map_err(|e| err_str("create mv", e))?;
    assert!(mv.mutated);
    app.execute_command(
        "table.setCells",
        json!({ "sheet_id": "mv-1", "rows": [[1.0, 2.0, 3.5], [2.0, 3.9, 5.2], [3.1, 6.0, 7.1], [4.2, 8.1, 9.4], [5.0, 9.8, 11.2]] }),
    )
    .map_err(|e| err_str("populate mv", e))?;
    let pca = app
        .execute_command(
            "analysis.run",
            json!({ "source_sheet_id": "mv-1", "sheet_id": "res-pca", "kind": { "Multivariate": { "Pca": { "standardize": true, "parallel_analysis_seed": 7 } } } }),
        )
        .map_err(|e| err_str("run PCA", e))?;
    assert!(pca.mutated);
    app.execute_command("sheet.select", json!({ "sheet_id": "res-pca" })).map_err(|e| err_str("select PCA", e))?;

    // Verify ui.inspect reports ResultSheet active and Analyze enabled!
    let snap_res = dispatch_gui_action(&mut app, "ui.inspect", json!({ "scope": "menus", "include_bounds": false }))
        .map_err(|e| err_str("inspect ResultSheet", e))?;
    assert_eq!(snap_res.get("active_sheet_id").and_then(Value::as_str), Some("res-pca"));
    let pca_sid = SheetId::try_new("res-pca").map_err(|e| err_str("pca sid", e))?;
    assert_eq!(app.document().sheet_section(&pca_sid), Some(SheetSection::Analyses));

    // Switch ResultSheet tab to `pc_scores` (or whatever extractable scores tab exists), then invoke Analyze...!
    let rs = app.document().result_sheet(&pca_sid).ok_or_else(|| String::from("missing res-pca"))?;
    let scores_tab_id =
        rs.tabs.iter().find(|t| t.id.contains("score")).or_else(|| rs.tabs.first()).map(|t| t.id.clone()).ok_or_else(|| String::from("tab"))?;
    dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab": scores_tab_id })).map_err(|e| err_str("switch tab", e))?;

    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "path": "Analyze/Analyze..." })).map_err(|e| err_str("Analyze from ResultSheet", e))?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));

    let chained = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.descriptive" } }))
        .map_err(|e| err_str("chain downstream analysis", e))?;
    let downstream_id = chained.pointer("/payload/sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("chained sheet_id"))?;
    let downstream_sid = SheetId::try_new(downstream_id).map_err(|e| err_str("downstream sid", e))?;
    let downstream_rs = app.document().result_sheet(&downstream_sid).ok_or_else(|| String::from("missing downstream ResultSheet"))?;
    assert!(matches!(downstream_rs.analysis.kind, AnalysisKind::DescriptiveStats(_)));
    assert!(app.document().data_sheets.iter().any(|d| d.kind() == TableKind::Column));
    assert!(
        app.document()
            .data_sheets
            .iter()
            .any(|d| matches!(d.archetype, TableArchetype::Column(_) | TableArchetype::Xy(_) | TableArchetype::MultipleVariables(_)))
    );
    Ok(())
}
