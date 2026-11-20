#![forbid(unsafe_code)]

//! `R15.7` Post-Cutover GUI Audit Addendum Regression Tests (`R15-GUI-29`..`R15-GUI-40`,
//! excluding `R15-GUI-34` on `R15.8` and `R15-GUI-37` on `R15.9`).

use std::path::Path;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, CurveSamplePoint, EquationDisplayMode, FittedCurveOverlay, GraphAnnotation, NonlinearFitSpec,
    RectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
};
use curvebook_engine::{Session, cmd as engine_cmd};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, LayoutMetrics, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, DESKTOP_HOST_ROUTED_COMMANDS, EguiSurfaceScanner, UI_COMMANDS, UiApp, format_shortcut_for_os, tr};
use serde_json::json;

struct AddendumShellState {
    app: UiApp,
    views: EguiShellViewState,
    carried_intents: PendingUiIntents,
    last_err: Option<String>,
}

fn make_shell_driver(app: UiApp, width: f64, height: f64) -> EguiHumanPathDriver<'static, AddendumShellState> {
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        AddendumShellState { app, views, carried_intents: PendingUiIntents::new(), last_err: None },
        ViewportMetrics::new(width, height, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false) {
                st.last_err = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        }
    });
    driver.run_settled();
    driver
}

fn seed_xy_data_and_graph_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-xy",
            "title": "Dose Response",
            "kind": "xy",
            "data_sets": 2,
            "rows": 3,
            "auto_graph": false
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setRowTitles", json!({ "sheet_id": "ds-xy", "titles": ["Control", "Low Dose", "High Dose"] }))
        .map_err(|e| e.to_string())?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [1.0, 2.0, 4.0] })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-xy", "rows": [[10.0, 12.0], [25.0, 28.0], [50.0, 52.0]] }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "gs-1", "data_sheet_id": "ds-xy", "title": "Dose Curve" })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).map_err(|e| e.to_string())?;
    app.sync_views();
    Ok(app)
}

fn collect_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn strip_line_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let code = match line.find("//") {
            Some(idx) if !line[..idx].contains('"') => &line[..idx],
            _ => line,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

fn extract_push_engine_literals(src: &str) -> Vec<String> {
    let clean = strip_line_comments(src);
    let mut found = Vec::new();
    let needle = "push_engine(";
    let mut cursor = 0usize;
    while let Some(rel) = clean[cursor..].find(needle) {
        let after = &clean[cursor + rel + needle.len()..];
        let trimmed = after.trim_start();
        if let Some(rest) = trimmed.strip_prefix('"')
            && let Some(end_q) = rest.find('"')
        {
            found.push(rest[..end_q].to_owned());
        }
        cursor += rel + needle.len();
    }
    found
}

#[test]
fn static_guard_all_egui_shell_push_engine_literals_exist_in_registry() -> Result<(), String> {
    let shell_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/egui_shell");
    let mut files = Vec::new();
    collect_rs_files(&shell_dir, &mut files)?;
    files.sort();
    assert!(!files.is_empty(), "expected rust files under egui_shell/");

    let reg = engine_cmd::registry();
    let mut total_checked = 0usize;
    let mut unknown = Vec::new();
    for file in files {
        let content = std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        for cmd in extract_push_engine_literals(&content) {
            total_checked += 1;
            if reg.get(cmd.as_str()).is_none() {
                unknown.push(format!("{}: `{cmd}`", file.display()));
            }
        }
    }
    assert!(total_checked >= 20, "expected at least 20 push_engine calls, got {total_checked}");
    assert!(unknown.is_empty(), "unregistered push_engine literals in egui_shell/: {unknown:?}");
    Ok(())
}

fn widget_center<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<egui::Pos2, String> {
    driver.frame_meta().find_by_widget_id(wid).map(|n| n.clipped_rect.center()).ok_or_else(|| format!("missing widget `{wid}` in FrameInspectMeta"))
}

#[test]
fn r15_gui_29_grid_row_context_menu_routes_registered_graph_commands() -> Result<(), String> {
    let app = seed_xy_data_and_graph_app()?;
    let mut driver = make_shell_driver(app, 1280.0, 800.0);

    // 1. Right-click row 1 header (`Low Dose`) and verify Reset Style is initially disabled (no overrides).
    let row_pos = widget_center(&driver, "grid.row_num.1")?;
    driver.click_at_button(row_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("grid.ctx.show_row_titles").is_some());
    let reset_meta = driver.frame_meta().find_by_widget_id("grid.ctx.reset_style").ok_or("missing grid.ctx.reset_style in context menu")?;
    assert!(!reset_meta.enabled, "reset_style should be disabled when row 1 has no overrides");

    // 2. Click Show Row Titles on Graph -> emits registered `graph.addAnnotation` (text = "Low Dose").
    driver.click_widget_id("grid.ctx.show_row_titles")?;
    assert_eq!(driver.state().last_err, None, "graph.addAnnotation from grid context menu must not fail");
    let gs = driver.state().app.document().graph_sheet(&"gs-1".into()).ok_or("missing gs-1")?;
    assert!(
        gs.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::TextBox { markdown, .. } if markdown == "Low Dose")),
        "expected text annotation for Row 1 (`Low Dose`), got {:?}",
        gs.spec.annotations
    );

    // 3. Click Style Selected Points on Graph… -> opens FormatSelectedPoints modal cleanly.
    driver.click_at_button(row_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("grid.ctx.style_points")?;
    assert_eq!(driver.state().last_err, None);
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }),
        "expected FormatSelectedPoints dialog, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 4. Add a per-point override on row 1 and verify Reset to Dataset Style enables and clears it.
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
    assert_eq!(driver.state().last_err, None, "graph.formatSelectedPoints clear must not fail");
    let gs_after = driver.state().app.document().graph_sheet(&"gs-1".into()).ok_or("missing gs-1")?;
    assert!(gs_after.spec.point_overrides.is_empty(), "expected row 1 point override cleared, got {:?}", gs_after.spec.point_overrides);

    Ok(())
}

#[test]
fn r15_gui_30_inline_menus_synthesize_parameters_and_open_dialogs() -> Result<(), String> {
    let app = seed_xy_data_and_graph_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 800.0);

    // 1. Analyze > Analyze... (`ui.analyze.open` / `analysis.add`) opens Analyze modal with sheet_id bound.
    driver.click_widget_id("menu.analyze")?;
    driver.click_widget_id("menu.analysis.add")?;
    assert_eq!(driver.state().last_err, None);
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }),
        "menu.analysis.add must open AnalyzeSheet modal, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 2. Graph > Format Axes... (`graph.setAxes`) opens FormatAxes modal with gs-1.
    driver.click_widget_id("menu.graph")?;
    driver.click_widget_id("menu.graph.setAxes")?;
    assert_eq!(driver.state().last_err, None);
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }),
        "menu.graph.setAxes must open FormatAxes modal, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 3. Analyze/Data > Convert Table Archetype... (`table.convertArchetype`) opens ConvertArchetype modal.
    driver.click_widget_id("menu.analyze")?;
    driver.click_widget_id("menu.table.convertArchetype")?;
    assert_eq!(driver.state().last_err, None);
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }),
        "menu.table.convertArchetype must open ConvertArchetype modal, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 4. Edit `sheet.duplicate` synthesizes `sheet_id` and duplicates active sheet cleanly.
    let initial_data_count = driver.state().app.document().data_sheets.len();
    driver.click_widget_id("menu.edit")?;
    driver.click_widget_id("menu.sheet.duplicate")?;
    assert_eq!(driver.state().last_err, None, "menu.sheet.duplicate must synthesize active sheet_id");
    assert_eq!(driver.state().app.document().data_sheets.len(), initial_data_count + 1);

    Ok(())
}

fn extract_push_ui_literals(src: &str) -> Vec<String> {
    let clean = strip_line_comments(src);
    let mut found = Vec::new();
    let needle = "push_ui(\"";
    let mut cursor = 0usize;
    while let Some(rel) = clean[cursor..].find(needle) {
        let rest = &clean[cursor + rel + needle.len()..];
        if let Some(end_q) = rest.find('"') {
            found.push(rest[..end_q].to_owned());
        }
        cursor += rel + needle.len();
    }
    found
}

#[test]
fn static_guard_all_egui_shell_push_ui_literals_exist_in_catalog() -> Result<(), String> {
    let shell_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/egui_shell");
    let mut files = Vec::new();
    collect_rs_files(&shell_dir, &mut files)?;
    files.sort();

    let mut total_checked = 0usize;
    let mut unknown = Vec::new();
    for file in files {
        let content = std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        for cmd in extract_push_ui_literals(&content) {
            total_checked += 1;
            let in_ui_cmds = UI_COMMANDS.iter().any(|u| u.id == cmd);
            let in_host_cmds = DESKTOP_HOST_ROUTED_COMMANDS.contains(&cmd.as_str());
            if !in_ui_cmds && !in_host_cmds {
                unknown.push(format!("{}: `{cmd}`", file.display()));
            }
        }
    }
    assert!(total_checked >= 20, "expected at least 20 push_ui calls, got {total_checked}");
    assert!(unknown.is_empty(), "unregistered push_ui literals in egui_shell/: {unknown:?}");
    Ok(())
}

#[test]
fn r15_gui_31_checkpoints_modal_creates_restores_and_deletes_checkpoints() -> Result<(), String> {
    let app = seed_xy_data_and_graph_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 800.0);

    // 1. Open Checkpoints dialog via Command Palette (`project.listCheckpoints`).
    driver.click_widget_id("toolbar.command_palette")?;
    driver.type_text("checkpoint")?;
    driver.click_widget_id("palette.item.0")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Checkpoints);

    // 2. Replace default checkpoint name, enter a note, and create checkpoint.
    let name_pos = widget_center(&driver, "checkpoints.name")?;
    driver.click_at(name_pos)?;
    driver.press_egui_key(egui::Key::A, egui::Modifiers::COMMAND);
    driver.type_text("Baseline XY")?;

    let note_pos = widget_center(&driver, "checkpoints.note")?;
    driver.click_at(note_pos)?;
    driver.type_text("Before editing cell 0")?;

    driver.click_widget_id("checkpoints.create")?;
    assert_eq!(driver.state().last_err, None);
    assert_eq!(driver.state().app.document().checkpoints.len(), 1);
    assert_eq!(driver.state().app.document().checkpoints[0].name, "Baseline XY");
    assert_eq!(driver.state().app.document().checkpoints[0].note, "Before editing cell 0");

    // 3. Close Checkpoints modal with Close button, mutate data cell (0,0,0), reopen via File menu.
    driver.click_widget_id("modal.cancel")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    driver
        .state_mut()
        .app
        .execute_command("table.setCell", json!({ "sheet_id": "ds-xy", "row": 0, "col": 0, "value": 999.0 }))
        .map_err(|e| e.to_string())?;
    driver.state_mut().app.sync_views();
    driver.run_settled();

    driver.click_widget_id("menu.file")?;
    driver.click_widget_id("menu.project.restoreCheckpoint")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Checkpoints);

    // 4. Restore checkpoint -> cell (0,0,0) reverts from 999.0 to 10.0.
    driver.click_widget_id("checkpoints.restore.0")?;
    assert_eq!(driver.state().last_err, None);
    let ds = driver.state().app.document().data_sheet(&"ds-xy".into()).ok_or("missing ds-xy")?;
    let val = ds.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value);
    assert_eq!(val, Some(&CellValue::Number(10.0)), "expected restored value 10.0");

    // 5. Delete checkpoint -> checkpoints list becomes empty; Escape closes modal.
    driver.click_widget_id("checkpoints.delete.0")?;
    assert_eq!(driver.state().last_err, None);
    assert!(driver.state().app.document().checkpoints.is_empty());

    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn r15_gui_32_column_cell_right_click_cut_and_canvas_copy_image() -> Result<(), String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-col", "title": "Column Data", "kind": "column", "cols": 2, "rows": 3 }))
        .map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-col", "rows": [[42.0, 55.0], [10.0, 20.0]] })).map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "gs-col", "data_sheet_id": "ds-col", "title": "Col Graph" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "ls-col", "title": "Layout 1" })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-col" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 800.0);

    // 1. Right-click cell (0,0,0) in Column grid -> opens context menu -> Cut Selection clears cell & copies TSV.
    let cell_pos = widget_center(&driver, "grid.cell.0.0.0")?;
    driver.click_at_button(cell_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("grid.ctx.cut").is_some(), "expected grid.ctx.cut in Column cell context menu");
    driver.click_widget_id("grid.ctx.cut")?;
    assert_eq!(driver.state().last_err, None);
    let ds = driver.state().app.document().data_sheet(&"ds-col".into()).ok_or("missing ds-col")?;
    assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(curvebook_doc::Cell::is_empty), "Cut Selection must clear cell");
    assert_eq!(driver.state().app.interaction.last_clipboard_text.as_deref(), Some("42"));

    // 2. Switch to GraphSheet and right-click canvas -> Copy as Image item present & copies vector/image bundle.
    driver.click_widget_id("nav.sheet.gs-col")?;
    let canvas_rect = driver.frame_meta().find_by_widget_id("canvas.viewport").map(|n| n.clipped_rect).ok_or("missing canvas.viewport")?;
    driver.click_at_button(canvas_rect.center(), PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("canvas.graph_ctx.copy_image").is_some(), "expected Copy as Image in GraphSheet context menu");
    driver.click_widget_id("canvas.graph_ctx.copy_image")?;
    assert_eq!(driver.state().last_err, None);
    assert!(driver.state().app.interaction.last_clipboard_bundle.is_some(), "Copy as Image must populate clipboard bundle");

    // 3. Switch to LayoutSheet and right-click canvas -> Copy as Image item present.
    driver.click_widget_id("nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.ls-col")?;
    let layout_rect = driver.frame_meta().find_by_widget_id("canvas.viewport").map(|n| n.clipped_rect).ok_or("missing canvas.viewport")?;
    driver.click_at_button(layout_rect.center(), PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("canvas.layout_ctx.copy_image").is_some(), "expected Copy as Image in LayoutSheet context menu");
    driver.click_widget_id("canvas.layout_ctx.copy_image")?;
    assert_eq!(driver.state().last_err, None);
    Ok(())
}

#[test]
fn r15_gui_33_about_modal_closes_via_button_and_escape_without_internal_clutter() -> Result<(), String> {
    let app = seed_xy_data_and_graph_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 800.0);

    // Tier 1 standalone About button is removed; About opens via Help menu.
    assert!(driver.frame_meta().find_by_widget_id("toolbar.help.about").is_none());
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);

    // Check rendered surface text excludes internal locale/shortcut counts and shows licence info.
    let mut scan_app = driver.state().app.clone();
    let scanner = curvebook_ui_egui::EguiSurfaceScanner::new();
    let _ = scanner.scan(&mut scan_app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    let cap = scanner.scan(&mut scan_app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    let joined: String = cap.text_runs.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" | ");
    assert!(!joined.contains("Active Beta locales"), "About body should not expose internal locale count: {joined}");
    assert!(!joined.contains("Registered keyboard shortcuts"), "About body should not expose shortcut count: {joined}");
    assert!(joined.contains("MIT OR Apache-2.0"), "About body must show license information: {joined}");

    // Copy version info updates button label cleanly.
    driver.click_widget_id("dialog.about.copy_version")?;
    let copy_btn = driver.frame_meta().find_by_widget_id("dialog.about.copy_version").ok_or("missing copy button")?;
    assert_eq!(copy_btn.label, "Copied");

    // OK button closes About cleanly.
    driver.click_widget_id("dialog.button.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // Escape closes About cleanly.
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn r15_gui_36_dock_splitters_resize_clamp_double_click_reset_and_hide_when_collapsed() -> Result<(), String> {
    let app = seed_xy_data_and_graph_app()?;
    let mut driver = make_shell_driver(app, 1440.0, 800.0);
    let metrics = LayoutMetrics::CANONICAL;

    // 1. Both dock splitters are present on wide desktop viewport.
    let nav_split_c = widget_center(&driver, "nav.dock_splitter")?;
    let drag_to = egui::Pos2::new(nav_split_c.x + 60.0, nav_split_c.y);
    driver.drag(nav_split_c, drag_to)?;
    let resized_lead = driver.state().views.leading_width_pt.ok_or("expected leading_width_pt after drag")?;
    assert!((resized_lead - 300.0).abs() <= 4.0, "expected leading dock ~300 pt after +60 drag, got {resized_lead}");
    assert!(!driver.state().views.trailing_collapsed_rail, "opposite dock must not collapse");

    // 2. Extreme outward drag clamps cleanly to navigator_max_w (`380 pt`).
    let nav_split_c2 = widget_center(&driver, "nav.dock_splitter")?;
    driver.drag(nav_split_c2, egui::Pos2::new(nav_split_c2.x + 500.0, nav_split_c2.y))?;
    let clamped_lead = driver.state().views.leading_width_pt.ok_or("expected clamped leading_width_pt")?;
    assert!(
        (clamped_lead - metrics.navigator_max_w).abs() <= 0.5,
        "expected clamp at navigator_max_w {}, got {clamped_lead}",
        metrics.navigator_max_w
    );

    // 3. Double-clicking nav splitter resets width to preset default (`240 pt`).
    let nav_split_c3 = widget_center(&driver, "nav.dock_splitter")?;
    driver.click_at(nav_split_c3)?;
    driver.click_at(nav_split_c3)?;
    let reset_lead = driver.state().views.leading_width_pt.ok_or("expected reset leading_width_pt")?;
    assert!((reset_lead - metrics.navigator_default_w).abs() <= 0.5, "expected reset to 240 pt, got {reset_lead}");

    // 4. Dragging inspector splitter left widens trailing studio & double-click resets (`280 pt`).
    let insp_split_c = widget_center(&driver, "inspector.dock_splitter")?;
    driver.drag(insp_split_c, egui::Pos2::new(insp_split_c.x - 60.0, insp_split_c.y))?;
    let resized_trail = driver.state().views.trailing_width_pt.ok_or("expected trailing_width_pt after drag")?;
    assert!((resized_trail - 340.0).abs() <= 4.0, "expected trailing dock ~340 pt after -60 drag, got {resized_trail}");
    let insp_split_c2 = widget_center(&driver, "inspector.dock_splitter")?;
    driver.click_at(insp_split_c2)?;
    driver.click_at(insp_split_c2)?;
    let reset_trail = driver.state().views.trailing_width_pt.ok_or("expected reset trailing_width_pt")?;
    assert!((reset_trail - metrics.inspector_default_w).abs() <= 0.5, "expected reset to 280 pt, got {reset_trail}");

    // 5. Splitters disappear when their docks collapse to rail.
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
            "source_sheet_id": "ds-xy",
            "sheet_id": "res-desc",
            "title": "Dose Descriptive",
            "kind": {
                "DescriptiveStats": {
                    "aggregation": "PerSubcolumn",
                    "custom_upper_percentile": null,
                    "run_normality_battery": false,
                    "hypothetical_value": null
                }
            }
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-1" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1440.0, 800.0);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("gs-1"));

    // 1. Right-click ResultSheet (non-active) -> shows ResultSheet context items without switching active sheet.
    let res_pos = widget_center(&driver, "nav.sheet.res-desc")?;
    driver.click_at_button(res_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert_eq!(
        driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str),
        Some("gs-1"),
        "right-clicking non-active sheet must not change active sheet"
    );
    for wid in ["nav.ctx.rename", "nav.ctx.duplicate", "nav.ctx.freeze", "nav.ctx.edit_params", "nav.ctx.rerun", "nav.ctx.retarget", "nav.ctx.delete"]
    {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing ResultSheet ctx item {wid}");
    }
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.delete").is_none());

    // 2. Right-click DataSheet (`ds-xy`, non-active) -> Freeze sets ds-xy frozen while active remains `gs-1`.
    let ds_pos = widget_center(&driver, "nav.sheet.ds-xy")?;
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    for wid in ["nav.ctx.analyze", "nav.ctx.new_graph", "nav.ctx.convert_archetype", "nav.ctx.duplicate_family"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing DataSheet ctx item {wid}");
    }
    driver.click_widget_id("nav.ctx.freeze")?;
    assert_eq!(driver.state().last_err, None);
    assert!(driver.state().app.document().data_sheet(&"ds-xy".into()).is_some_and(|d| d.frozen));
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("gs-1"));

    // 3. Re-open context menu on frozen `ds-xy` -> Rename & Delete are disabled; unfreeze restores them.
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.rename").is_some_and(|n| !n.enabled));
    assert!(driver.frame_meta().find_by_widget_id("nav.ctx.delete").is_some_and(|n| !n.enabled));
    driver.click_widget_id("nav.ctx.freeze")?;
    assert!(driver.state().app.document().data_sheet(&"ds-xy".into()).is_some_and(|d| !d.frozen));

    // 4. Right-click `ds-xy` -> Delete… opens cascade confirmation; Cancel preserves sheets; Confirm cascade-deletes all.
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("nav.ctx.delete")?;
    let confirm = driver.state().views.leading_dock.delete_confirm.clone().ok_or("expected delete_confirm modal")?;
    let dep_ids: Vec<&str> = confirm.dependents.iter().map(|(id, _)| id.as_str()).collect();
    assert!(dep_ids.contains(&"res-desc"), "expected res-desc in downstream dependents, got {dep_ids:?}");
    assert!(dep_ids.contains(&"gs-1"), "expected gs-1 in downstream dependents, got {dep_ids:?}");
    driver.click_widget_id("nav.delete_confirm.cancel")?;
    assert!(driver.state().views.leading_dock.delete_confirm.is_none());
    assert!(driver.state().app.document().data_sheet(&"ds-xy".into()).is_some());

    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("nav.ctx.delete")?;
    driver.click_widget_id("nav.delete_confirm.confirm")?;
    assert_eq!(driver.state().last_err, None);
    assert!(driver.state().app.document().data_sheet(&"ds-xy".into()).is_none());
    assert!(driver.state().app.document().result_sheet(&"res-desc".into()).is_none());
    assert!(driver.state().app.document().graph_sheet(&"gs-1".into()).is_none());
    Ok(())
}

#[test]
fn r15_gui_39_command_search_uses_localized_label_and_platform_shortcut_formatting() -> Result<(), String> {
    assert_eq!(format_shortcut_for_os("Cmd+K", true), "⌘K");
    assert_eq!(format_shortcut_for_os("Cmd+K", false), "Ctrl+K");
    assert_eq!(tr("en", "toolbar.palette"), "Commands");

    // Wide desktop chrome across all 14 Beta locales shows localized Commands label, never `Cmd+K`.
    for &locale in BETA_ACTIVE_LOCALES {
        let mut app = seed_xy_data_and_graph_app()?;
        app.locale = String::from(locale);
        app.sync_views();
        let driver = make_shell_driver(app, 1440.0, 800.0);
        let pal_node = driver.frame_meta().find_by_widget_id("toolbar.command_palette").ok_or("missing toolbar.command_palette")?;
        assert_ne!(pal_node.label, "Cmd+K", "locale `{locale}` must not expose raw `Cmd+K` label");
        assert!(!pal_node.label.contains("Cmd+K"), "locale `{locale}` label `{}` leaked `Cmd+K`", pal_node.label);
        assert_eq!(pal_node.label, tr(locale, "toolbar.palette"));
    }

    // Narrow viewport switches Command Search to compact icon button (`<= 32 pt` wide, zero `Cmd+K` text).
    let mut narrow_app = seed_xy_data_and_graph_app()?;
    narrow_app.sync_views();
    let scanner = EguiSurfaceScanner::new();
    let cap = scanner.scan(&mut narrow_app, ViewportMetrics::new(720.0, 600.0, 1.0));
    let pal_a11y = cap.a11y_entries.iter().find(|n| n.widget_id == "toolbar.command_palette").ok_or("missing narrow palette btn")?;
    assert_eq!(pal_a11y.label, "Commands");
    assert!(!cap.text_runs.iter().any(|(_, t)| t.contains("Cmd+K")), "rendered text runs must never show raw `Cmd+K`: {:?}", cap.text_runs);
    Ok(())
}

#[test]
fn r15_gui_40_live_tex_equation_does_not_double_paint_egui_overlay_text() -> Result<(), String> {
    let mut app = seed_xy_data_and_graph_app()?;
    let nl_spec = NonlinearFitSpec::new("log_inhibitor_vs_response_3pl");
    let mut nl_rs = ResultSheet::new(AnalysisSpec::new("res-nl", "ds-xy", AnalysisKind::NonlinearRegression(nl_spec)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    nl_rs.status = ResultStatus::UpToDate;
    nl_rs.scalars = vec![
        ResultScalarEntry { key: String::from("Bottom"), data_set_index: Some(0), value: 5.0, units: None },
        ResultScalarEntry { key: String::from("Top"), data_set_index: Some(0), value: 95.0, units: None },
        ResultScalarEntry { key: String::from("LogIC50"), data_set_index: Some(0), value: -6.2, units: None },
    ];
    nl_rs.fitted_curves = vec![FittedCurveOverlay {
        equation_id: String::from("log_inhibitor_vs_response_3pl"),
        converged: true,
        ambiguous: false,
        data_set_index: 0,
        points: vec![CurveSamplePoint { x: 0.0, y: 95.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }],
    }];
    {
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
    }
    app.execute_command("sheet.select", json!({ "sheet_id": "gs-1" })).map_err(|e| e.to_string())?;
    app.egui_views.canvas_view.synchronous_raster = true;
    app.sync_views();

    let mut scan_app = app.clone();
    let driver = make_shell_driver(app, 1440.0, 800.0);
    let eq_node = driver.frame_meta().find_by_widget_id("canvas.tex_equation.0").ok_or("missing canvas.tex_equation.0")?;
    assert!(eq_node.clipped_rect.width() >= 16.0 && eq_node.clipped_rect.height() >= 12.0);
    assert!(!eq_node.label.trim().is_empty());

    // Confirm egui emits ZERO duplicate `Shape::Text` runs overlapping the SVG-rendered equation box.
    let cap = EguiSurfaceScanner::new().scan(&mut scan_app, ViewportMetrics::new(1440.0, 800.0, 1.0));
    let eq_runs: Vec<_> = cap.text_runs.iter().filter(|(owner, _)| owner == "canvas.tex_equation.0").collect();
    assert!(eq_runs.is_empty(), "egui must not double-paint TeX equation overlay text: {eq_runs:?}");
    Ok(())
}
