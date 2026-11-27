#![forbid(unsafe_code)]

//! `R15.7` Post-Cutover GUI Audit Addendum Regression Tests (`R15-GUI-29`..`R15-GUI-40`).

use std::path::Path;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, CurveSamplePoint, EquationDisplayMode, FittedCurveOverlay, GraphAnnotation, NonlinearFitSpec,
    RectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
};
use curvebook_engine::{Session, cmd as engine_cmd};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::egui_shell::{LayoutMetrics, Tokens};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, DESKTOP_HOST_ROUTED_COMMANDS, EguiSurfaceScanner, UI_COMMANDS, UiApp, format_shortcut_for_os, tr};
use serde_json::json;

use crate::common::{ShellTestState, make_shell_driver_with_state, set_textbox, widget_center};

fn make_driver(app: UiApp, width: f64, height: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(width, height, 1.0), tokens)
}

fn seed_xy_data_and_graph_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    for (cmd_id, params) in [
        ("sheet.createData", json!({ "sheet_id": "ds-xy", "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 3, "auto_graph": false })),
        ("table.setRowTitles", json!({ "sheet_id": "ds-xy", "titles": ["Control", "Low Dose", "High Dose"] })),
        ("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [1.0, 2.0, 4.0] })),
        ("table.setCells", json!({ "sheet_id": "ds-xy", "rows": [[10.0, 12.0], [25.0, 28.0], [50.0, 52.0]] })),
        ("graph.create", json!({ "sheet_id": "gs-1", "data_sheet_id": "ds-xy", "title": "Dose Curve" })),
        ("sheet.select", json!({ "sheet_id": "ds-xy" })),
    ] {
        app.execute_command(cmd_id, params).map_err(|e| e.to_string())?;
    }
    app.sync_views();
    Ok(app)
}

fn collect_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn extract_push_literals(src: &str, needle: &str) -> Vec<String> {
    let clean: String = src
        .lines()
        .map(|line| match line.split_once("//") {
            Some((head, _)) if !head.contains('"') => head,
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    let mut tail = clean.as_str();
    while let Some((_, after)) = tail.split_once(needle) {
        if let Some(rest) = after.trim_start().strip_prefix('"')
            && let Some((lit, _)) = rest.split_once('"')
        {
            found.push(lit.to_owned());
        }
        tail = after;
    }
    found
}

fn verify_shell_push_literals(needle: &str, is_known: impl Fn(&str) -> bool) -> Result<(), String> {
    let shell_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/egui_shell");
    let mut files = Vec::new();
    collect_rs_files(&shell_dir, &mut files)?;
    files.sort();
    let mut total = 0usize;
    let mut unknown = Vec::new();
    for file in files {
        let content = std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        for cmd in extract_push_literals(&content, needle) {
            total += 1;
            if !is_known(cmd.as_str()) {
                unknown.push(format!("{}: `{cmd}`", file.display()));
            }
        }
    }
    assert!(total >= 20, "expected >= 20 `{needle}` calls, got {total}");
    assert!(unknown.is_empty(), "unregistered `{needle}` literals: {unknown:?}");
    Ok(())
}

#[test]
fn static_guard_all_egui_shell_push_engine_literals_exist_in_registry() -> Result<(), String> {
    let reg = engine_cmd::registry();
    verify_shell_push_literals("push_engine(", |cmd| reg.get(cmd).is_some())
}

#[test]
fn static_guard_all_egui_shell_push_ui_literals_exist_in_catalog() -> Result<(), String> {
    verify_shell_push_literals("push_ui(", |cmd| UI_COMMANDS.iter().any(|u| u.id == cmd) || DESKTOP_HOST_ROUTED_COMMANDS.contains(&cmd))
}

#[test]
fn r15_gui_29_grid_row_context_menu_routes_registered_graph_commands() -> Result<(), String> {
    let mut driver = make_driver(seed_xy_data_and_graph_app()?, 1280.0, 800.0);
    let row_pos = widget_center(&driver, "grid.row_num.1")?;
    driver.click_at_button(row_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("grid.ctx.show_row_titles").is_some());
    assert!(!driver.frame_meta().find_by_widget_id("grid.ctx.reset_style").ok_or("missing reset_style")?.enabled);

    driver.click_widget_id("grid.ctx.show_row_titles")?;
    assert_eq!(driver.state().last_intent_error, None);
    let gs = driver.state().app.document().graph_sheet(&"gs-1".into()).ok_or("missing gs-1")?;
    assert!(gs.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::TextBox { markdown, .. } if markdown == "Low Dose")));

    driver.click_at_button(row_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.style_points")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }));
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    driver
        .state_mut()
        .app
        .execute_command(
            "graph.formatSelectedPoints",
            json!({ "sheet_id": "gs-1", "points": [{ "series_index": 0, "point_index": 1 }], "size_pt": 11.0 }),
        )
        .map_err(|e| e.to_string())?;
    driver.state_mut().app.sync_views();
    driver.run_settled();

    let row_pos = widget_center(&driver, "grid.row_num.1")?;
    driver.click_at_button(row_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.reset_style")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(driver.state().app.document().graph_sheet(&"gs-1".into()).ok_or("missing gs-1")?.spec.point_overrides.is_empty());
    Ok(())
}

#[test]
fn r15_gui_30_inline_menus_synthesize_parameters_and_open_dialogs() -> Result<(), String> {
    let mut driver = make_driver(seed_xy_data_and_graph_app()?, 1440.0, 800.0);
    for (menu, item, check) in [
        ("menu.analyze", "menu.ui.analyze.open", (|d: &ActiveDialog| matches!(d, ActiveDialog::AnalyzeSheet { .. })) as fn(&ActiveDialog) -> bool),
        ("menu.graph", "menu.ui.graph.axes.open", |d| matches!(d, ActiveDialog::FormatAxes { .. })),
        ("menu.data", "menu.ui.table.convertArchetype.open", |d| matches!(d, ActiveDialog::ConvertArchetype { .. })),
    ] {
        driver.click_widget_id(menu)?;
        driver.click_widget_id(item)?;
        assert_eq!(driver.state().last_intent_error, None);
        assert!(check(&driver.state().app.canvas_dialogs.active_dialog), "{item} opened {:?}", driver.state().app.canvas_dialogs.active_dialog);
        driver.click_widget_id("modal.cancel")?;
        assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    let initial_data_count = driver.state().app.document().data_sheets.len();
    driver.click_widget_id("menu.data")?;
    driver.click_widget_id("menu.sheet.duplicate")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.document().data_sheets.len(), initial_data_count + 1);
    Ok(())
}

#[test]
fn r15_gui_31_checkpoints_modal_creates_restores_and_deletes_checkpoints() -> Result<(), String> {
    let mut driver = make_driver(seed_xy_data_and_graph_app()?, 1440.0, 800.0);
    driver.click_widget_id("toolbar.command_palette")?;
    driver.type_text("checkpoint")?;
    driver.click_widget_id("palette.item.0")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Checkpoints);

    set_textbox(&mut driver, "checkpoints.name", "Baseline XY")?;
    set_textbox(&mut driver, "checkpoints.note", "Before editing cell 0")?;
    driver.click_widget_id("checkpoints.create")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.document().checkpoints.len(), 1);
    assert_eq!(driver.state().app.document().checkpoints[0].name, "Baseline XY");
    assert_eq!(driver.state().app.document().checkpoints[0].note, "Before editing cell 0");

    driver.click_widget_id("modal.cancel")?;
    driver
        .state_mut()
        .app
        .execute_command("table.setCell", json!({ "sheet_id": "ds-xy", "row": 0, "col": 0, "value": 999.0 }))
        .map_err(|e| e.to_string())?;
    driver.state_mut().app.sync_views();
    driver.run_settled();

    driver.click_widget_id("menu.file")?;
    driver.hover_widget_id("menu.file.checkpoints")?;
    driver.click_widget_id("menu.project.listCheckpoints")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Checkpoints);

    driver.click_widget_id("checkpoints.restore.0")?;
    driver.click_widget_id("checkpoints.restore.confirm.0")?;
    assert_eq!(driver.state().last_intent_error, None);
    let val = driver.state().app.document().data_sheet(&"ds-xy".into()).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).map(|c| &c.value);
    assert_eq!(val, Some(&CellValue::Number(10.0)));

    driver.click_widget_id("checkpoints.delete.0")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(driver.state().app.document().checkpoints.is_empty());
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn r15_gui_32_column_cell_right_click_cut_and_canvas_copy_image() -> Result<(), String> {
    let mut app = UiApp::new();
    for (cmd_id, params) in [
        ("sheet.createData", json!({ "sheet_id": "ds-col", "title": "Column Data", "kind": "column", "cols": 2, "rows": 3 })),
        ("table.setCells", json!({ "sheet_id": "ds-col", "rows": [[42.0, 55.0], [10.0, 20.0]] })),
        ("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col", "title": "Col Graph" })),
        ("layout.create", json!({ "sheet_id": "ls-col", "title": "Layout 1" })),
        ("sheet.select", json!({ "sheet_id": "ds-col" })),
    ] {
        app.execute_command(cmd_id, params).map_err(|e| e.to_string())?;
    }
    app.sync_views();
    let mut driver = make_driver(app, 1280.0, 800.0);

    let cell_pos = widget_center(&driver, "grid.cell.0.0.0")?;
    driver.click_at_button(cell_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.cut")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(
        driver
            .state()
            .app
            .document()
            .data_sheet(&"ds-col".into())
            .and_then(|d| d.cell(CellCoord::new(0, 0, 0)))
            .is_some_and(curvebook_doc::Cell::is_empty)
    );
    assert_eq!(driver.state().app.interaction.last_clipboard_text.as_deref(), Some("42"));

    for (pre_clicks, ctx_wid) in
        [(&["nav.sheet.gs-col"][..], "canvas.graph_ctx.copy_image"), (&["nav.chip.layouts", "nav.sheet.ls-col"][..], "canvas.layout_ctx.copy_image")]
    {
        for wid in pre_clicks {
            if wid.starts_with("nav.chip.") {
                crate::common::select_nav_kind_filter(&mut driver, wid)?;
            } else {
                driver.click_widget_id(wid)?;
            }
        }
        let c = widget_center(&driver, "canvas.viewport")?;
        driver.click_at_button(c, PointerButton::Secondary, KeyModifiers::default())?;
        driver.click_widget_id(ctx_wid)?;
        assert_eq!(driver.state().last_intent_error, None);
        assert!(driver.state().app.interaction.last_clipboard_bundle.is_some());
    }
    Ok(())
}

#[test]
fn r15_gui_33_about_modal_closes_via_button_and_escape_without_internal_clutter() -> Result<(), String> {
    let mut driver = make_driver(seed_xy_data_and_graph_app()?, 1440.0, 800.0);
    assert!(driver.frame_meta().find_by_widget_id("toolbar.help.about").is_none());
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);

    let mut scan_app = driver.state().app.clone();
    let scanner = EguiSurfaceScanner::new();
    let _ = scanner.scan(&mut scan_app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    let cap = scanner.scan(&mut scan_app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    let joined: String = cap.text_runs.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" | ");
    assert!(!joined.contains("Active Beta locales") && !joined.contains("Registered keyboard shortcuts") && joined.contains("MIT OR Apache-2.0"));

    assert!(driver.frame_meta().find_by_widget_id("dialog.button.ok").is_none());
    driver.click_widget_id("dialog.about.copy_version")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("dialog.about.copy_version").ok_or("missing copy")?.label, "Copied");
    driver.click_widget_id("dialog.about.close")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    let dlg_rect = driver.frame_meta().find_by_widget_id("dialog.about").ok_or("missing dialog.about")?.clipped_rect;
    let outside = egui::pos2((dlg_rect.left() - 12.0).max(4.0), dlg_rect.center().y);
    driver.click_at(outside)?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn r15_gui_36_dock_splitters_resize_clamp_double_click_reset_and_hide_when_collapsed() -> Result<(), String> {
    let mut driver = make_driver(seed_xy_data_and_graph_app()?, 1440.0, 800.0);
    let m = LayoutMetrics::CANONICAL;

    let c = widget_center(&driver, "nav.dock_splitter")?;
    driver.drag(c, egui::Pos2::new(c.x + 60.0, c.y))?;
    assert!((driver.state().views.leading_width_pt.ok_or("leading_width")? - 300.0).abs() <= 4.0);
    assert!(!driver.state().views.trailing_collapsed_rail);

    let c = widget_center(&driver, "nav.dock_splitter")?;
    driver.drag(c, egui::Pos2::new(c.x + 500.0, c.y))?;
    assert!((driver.state().views.leading_width_pt.ok_or("clamped")? - m.navigator_max_w).abs() <= 0.5);

    let c = widget_center(&driver, "nav.dock_splitter")?;
    driver.click_at(c)?;
    driver.click_at(c)?;
    assert!((driver.state().views.leading_width_pt.ok_or("reset lead")? - m.navigator_default_w).abs() <= 0.5);

    let c = widget_center(&driver, "inspector.dock_splitter")?;
    driver.drag(c, egui::Pos2::new(c.x - 60.0, c.y))?;
    assert!((driver.state().views.trailing_width_pt.ok_or("trail")? - 340.0).abs() <= 4.0);
    let c = widget_center(&driver, "inspector.dock_splitter")?;
    driver.click_at(c)?;
    driver.click_at(c)?;
    assert!((driver.state().views.trailing_width_pt.ok_or("reset trail")? - m.inspector_default_w).abs() <= 0.5);

    driver.state_mut().app.egui_views.leading_collapsed_rail = true;
    driver.state_mut().app.egui_views.trailing_collapsed_rail = true;
    driver.state_mut().views = driver.state().app.egui_views.clone();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("nav.dock_splitter").is_none());
    assert!(driver.frame_meta().find_by_widget_id("inspector.dock_splitter").is_none());
    Ok(())
}

#[test]
fn r15_gui_38_navigator_right_click_targets_clicked_sheet_and_confirms_cascade_delete() -> Result<(), String> {
    let mut app = seed_xy_data_and_graph_app()?;
    app.execute_command(
        "analysis.run",
        json!({
            "source_sheet_id": "ds-xy", "sheet_id": "res-desc", "title": "Dose Descriptive",
            "kind": { "DescriptiveStats": { "aggregation": "PerSubcolumn", "custom_upper_percentile": null, "run_normality_battery": false, "hypothetical_value": null } }
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-1" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut driver = make_driver(app, 1440.0, 800.0);
    let res_pos = widget_center(&driver, "nav.sheet.res-desc")?;
    driver.click_at_button(res_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("gs-1"));
    for wid in ["nav.ctx.rename", "nav.ctx.duplicate", "nav.ctx.freeze", "nav.ctx.edit_params", "nav.ctx.rerun", "nav.ctx.retarget", "nav.ctx.delete"]
    {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing {wid}");
    }
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.delete").is_none());

    let ds_pos = widget_center(&driver, "nav.sheet.ds-xy")?;
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    for wid in ["nav.ctx.analyze", "nav.ctx.new_graph", "nav.ctx.convert_archetype", "nav.ctx.duplicate_family"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing {wid}");
    }
    driver.click_widget_id("nav.ctx.freeze")?;
    assert!(driver.state().app.document().data_sheet(&"ds-xy".into()).is_some_and(|d| d.frozen));
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("gs-1"));

    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.rename").is_some_and(|n| !n.enabled));
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.delete").is_some_and(|n| !n.enabled));
    driver.click_widget_id("nav.ctx.freeze")?;
    assert!(driver.state().app.document().data_sheet(&"ds-xy".into()).is_some_and(|d| !d.frozen));

    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("nav.ctx.delete")?;
    let confirm = driver.state().views.leading_dock.delete_confirm.clone().ok_or("expected delete_confirm")?;
    let dep_ids: Vec<&str> = confirm.dependents.iter().map(|(id, _)| id.as_str()).collect();
    assert!(dep_ids.contains(&"res-desc") && dep_ids.contains(&"gs-1"));
    driver.click_widget_id("nav.delete_confirm.cancel")?;
    assert!(driver.state().views.leading_dock.delete_confirm.is_none() && driver.state().app.document().data_sheet(&"ds-xy".into()).is_some());

    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("nav.ctx.delete")?;
    driver.click_widget_id("nav.delete_confirm.confirm")?;
    assert_eq!(driver.state().last_intent_error, None);
    let doc = driver.state().app.document();
    assert!(doc.data_sheet(&"ds-xy".into()).is_none() && doc.result_sheet(&"res-desc".into()).is_none() && doc.graph_sheet(&"gs-1".into()).is_none());
    Ok(())
}

#[test]
fn r15_gui_39_command_search_uses_localized_label_and_platform_shortcut_formatting() -> Result<(), String> {
    assert_eq!(
        (format_shortcut_for_os("Cmd+K", true), format_shortcut_for_os("Cmd+K", false), tr("en", "toolbar.palette")),
        ("⌘K".into(), "Ctrl+K".into(), "Commands")
    );
    for &locale in BETA_ACTIVE_LOCALES {
        let mut app = seed_xy_data_and_graph_app()?;
        app.locale = String::from(locale);
        app.sync_views();
        let driver = make_driver(app, 1440.0, 800.0);
        let pal_node = driver.frame_meta().find_by_widget_id("toolbar.command_palette").ok_or("missing toolbar.command_palette")?;
        assert!(!pal_node.label.contains("Cmd+K") && pal_node.label == tr(locale, "toolbar.palette"));
    }
    let mut narrow_app = seed_xy_data_and_graph_app()?;
    let cap = EguiSurfaceScanner::new().scan(&mut narrow_app, ViewportMetrics::new(720.0, 600.0, 1.0));
    assert_eq!(cap.a11y_entries.iter().find(|n| n.widget_id == "toolbar.command_palette").ok_or("missing narrow palette")?.label, "Commands");
    assert!(!cap.text_runs.iter().any(|(_, t)| t.contains("Cmd+K")));
    Ok(())
}

#[test]
fn r15_gui_40_live_tex_equation_does_not_double_paint_egui_overlay_text() -> Result<(), String> {
    let mut app = seed_xy_data_and_graph_app()?;
    let mut nl_rs = ResultSheet::new(
        AnalysisSpec::new("res-nl", "ds-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_inhibitor_vs_response_3pl")))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    nl_rs.status = ResultStatus::UpToDate;
    nl_rs.scalars = [("Bottom", 5.0), ("Top", 95.0), ("LogIC50", -6.2)]
        .into_iter()
        .map(|(k, value)| ResultScalarEntry { key: k.to_owned(), data_set_index: Some(0), value, units: None })
        .collect();
    nl_rs.fitted_curves = vec![FittedCurveOverlay {
        equation_id: String::from("log_inhibitor_vs_response_3pl"),
        converged: true,
        ambiguous: false,
        data_set_index: 0,
        points: vec![CurveSamplePoint { x: 0.0, y: 95.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }],
    }];
    let mut doc = app.document().clone();
    doc.result_sheets.push(nl_rs);
    doc.graph_sheets[0].spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-nl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(120.0, 28.0, 140.0, 48.0),
        font_size_pt: 11.0,
        bordered: true,
    });
    *app.session_mut() = Session::from_document(doc);
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-1" })).map_err(|e| e.to_string())?;
    app.egui_views.canvas_view.synchronous_raster = true;
    app.sync_views();

    let mut scan_app = app.clone();
    let driver = make_driver(app, 1440.0, 800.0);
    let eq_node = driver.frame_meta().find_by_widget_id("canvas.tex_equation.0").ok_or("missing canvas.tex_equation.0")?;
    assert!(eq_node.clipped_rect.width() >= 16.0 && eq_node.clipped_rect.height() >= 12.0 && !eq_node.label.trim().is_empty());
    let cap = EguiSurfaceScanner::new().scan(&mut scan_app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    assert!(cap.text_runs.iter().all(|(owner, _)| owner != "canvas.tex_equation.0"));
    Ok(())
}
