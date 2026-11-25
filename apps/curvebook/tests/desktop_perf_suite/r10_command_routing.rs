#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use curvebook::desktop_host::{
    DesktopCommandRoute, DesktopDialogOpener, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogKind,
    StubNativeFileDialogAdapter, resolve_desktop_command_route,
};
use curvebook::gui_window::{
    CurveBookNativeApp, DEFAULT_TEMPLATE_FILE_NAME, NATIVE_OPEN_FILE_EXTENSIONS, NATIVE_TEMPLATE_FILE_EXTENSIONS, execute_native_import_at_path,
    menu_action_triggers_native_file_dialog, native_import_replaces_project_file_path,
};
use curvebook_doc::{CellCoord, FigureCanvasPreset, GraphAnnotation, GraphKind, GraphSheet, LayoutItem, ProjectDocument, SheetId};
use curvebook_engine::{Session, cmd};
use curvebook_format::{open_project_file, save_project_file};
use curvebook_render::{RasterImage, png::encode_png};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector};
use curvebook_ui_egui::interaction::{InlineEditorTarget, ViewportMetrics};
use curvebook_ui_egui::menus::{TOOLBAR_BUTTONS, UI_COMMANDS, menu_bar_tooltips};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{CanvasPreviewFilter, StubPlatformClipboardAdapter, UiApp, dispatch_gui_action};
use eframe::egui;
use serde_json::{Value, json};

use crate::common::{NativeHarness, TempDirGuard, build_col_doc, build_zip_entries, encode_base64};

fn build_populated_routing_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    for (cmd_id, params) in [
        (
            "sheet.createData",
            json!({ "sheet_id": "data-xy", "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 5, "auto_info": true, "auto_graph": false }),
        ),
        ("table.setXCells", json!({ "sheet_id": "data-xy", "values": [1.0, 2.0, 4.0, 8.0, 16.0] })),
        ("table.setCells", json!({ "sheet_id": "data-xy", "rows": [[10.0, 11.0], [20.0, 22.0], [40.0, 39.0], [78.0, 81.0], [95.0, 96.0]] })),
        (
            "sheet.createData",
            json!({ "sheet_id": "data-col", "title": "Column Data", "kind": "column", "cols": 2, "rows": 4, "auto_info": false, "auto_graph": false }),
        ),
        ("table.setCells", json!({ "sheet_id": "data-col", "rows": [[1.0, 2.0], [1.5, 2.5], [1.2, 2.7], [1.4, 2.4]] })),
        (
            "sheet.createData",
            json!({ "sheet_id": "data-mv", "title": "MV Features", "kind": "multiple_variables", "variables": [{ "title": "F1", "role": "continuous" }, { "title": "F2", "role": "continuous" }, { "title": "F3", "role": "continuous" }], "rows": 6, "auto_info": false, "auto_graph": false }),
        ),
        (
            "table.setCells",
            json!({ "sheet_id": "data-mv", "rows": [[1.0, 1.0, 2.0], [1.1, 0.9, 2.1], [0.9, 1.1, 1.9], [10.0, 10.0, 8.0], [10.2, 9.8, 8.2], [9.9, 10.1, 7.9]] }),
        ),
        ("sheet.select", json!({ "sheet_id": "data-mv" })),
    ] {
        let _ = app.execute_command(cmd_id, params)?;
    }

    let _ = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.analyze.open", &json!({ "sheet_id": "data-mv" }))
        .map_err(|e| format!("open Analyze failed: {e}"))?;
    let _ =
        dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.clustering", "k_means_k": 2 } }))?;
    app.canvas_dialogs.close_dialog();

    let result_id = app.document().result_sheets.first().map(|r| r.id.as_str().to_owned()).ok_or("expected result sheet")?;
    let tiny_png_b64 = encode_base64(&encode_png(&RasterImage { width_px: 2, height_px: 2, dpi: 96, rgba: vec![255u8; 16] })?);
    for (cmd_id, params) in [
        ("graph.create", json!({ "data_sheet_id": "data-xy", "sheet_id": "graph-xy", "title": "XY Curve", "kind": "xy", "initialized": true })),
        (
            "graph.create",
            json!({ "data_sheet_id": "data-col", "sheet_id": "graph-uninit", "title": "Uninitialized Column Graph", "initialized": false }),
        ),
        ("layout.create", json!({ "sheet_id": "layout-1", "title": "Figure 1" })),
        ("layout.addPanel", json!({ "sheet_id": "layout-1", "graph_sheet_id": "graph-xy" })),
        ("edit.pasteResultTable", json!({ "source_sheet_id": result_id, "target_sheet_id": "graph-xy" })),
        ("graph.insertImage", json!({ "sheet_id": "graph-xy", "bytes_base64": tiny_png_b64.clone() })),
        ("layout.insertImage", json!({ "sheet_id": "layout-1", "bytes_base64": tiny_png_b64 })),
        ("sheet.select", json!({ "sheet_id": "data-xy" })),
    ] {
        let _ = app.execute_command(cmd_id, params)?;
    }

    app.grid.select_single_sync(CellCoord::new(0, 0, 0).into());
    app.sync_views();
    Ok(app)
}

fn spawn_window_from_session(session: Session) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        initial_viewport: metrics,
        clipboard_adapter: Some(Arc::new(StubPlatformClipboardAdapter::new())),
        ..DesktopHostConfig::default()
    });
    let wid = rt.open_window_with_session(session, metrics, false)?;
    Ok((rt, wid))
}

fn collect_all_desktop_command_ids(sample_app: &UiApp) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    ids.extend(cmd::registry().specs().map(|s| s.id.to_owned()));
    ids.extend(UI_COMMANDS.iter().map(|s| s.id.to_owned()));
    for btn in TOOLBAR_BUTTONS {
        ids.insert(btn.id.to_owned());
        ids.insert(btn.command_id.to_owned());
    }
    ids.extend(menu_bar_tooltips(sample_app).into_iter().flat_map(|s| s.items.into_iter().map(|i| i.command_id.to_owned())));
    ids.extend(
        [
            "window.new",
            "window.close",
            "file.newWindow",
            "file.close",
            "file.open",
            "file.save",
            "file.save_as",
            "project.saveAs",
            "file.export_svg",
            "file.export_pdf",
            "file.export_png",
            "file.print",
            "edit.copy",
            "edit.paste",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    ids
}

#[test]
fn registry_sweep_routes_every_engine_and_ui_command_without_null_params() -> Result<(), Box<dyn std::error::Error>> {
    let base_app = build_populated_routing_app()?;
    let base_doc = base_app.document().clone();
    let all_ids = collect_all_desktop_command_ids(&base_app);
    assert!(all_ids.len() >= cmd::registry().len() + UI_COMMANDS.len());

    // 1. Route resolution sweep on empty & populated docs never emits Value::Null.
    let empty_app = UiApp::new();
    for cmd_id in &all_ids {
        if let Ok(DesktopCommandRoute::EngineCommand { params, .. } | DesktopCommandRoute::UiCommand { params, .. }) =
            resolve_desktop_command_route(&empty_app, None, cmd_id)
        {
            assert!(!params.is_null(), "command `{cmd_id}` resolved to Null on empty doc");
        }
        let route = resolve_desktop_command_route(&base_app, None, cmd_id).map_err(|e| format!("`{cmd_id}` failed route: {e}"))?;
        match &route {
            DesktopCommandRoute::EngineCommand { engine_command_id, params } => {
                assert!(params.is_object(), "`{cmd_id}` -> `{engine_command_id}` params must be object, got {params}");
                let spec = cmd::registry().get(engine_command_id).ok_or_else(|| format!("unknown `{engine_command_id}`"))?;
                if let Some(req) = (spec.input_schema)().get("required").and_then(Value::as_array) {
                    for key_val in req {
                        let k = key_val.as_str().ok_or("non-string required key")?;
                        assert!(params.get(k).is_some_and(|v| !v.is_null()), "`{cmd_id}` missing `{k}` in {params}");
                    }
                }
            }
            DesktopCommandRoute::UiCommand { ui_command_id, params } => assert!(!params.is_null(), "`{cmd_id}` -> `{ui_command_id}` null"),
            DesktopCommandRoute::HostAction { .. } | DesktopCommandRoute::UiDialogOpener { .. } => {}
        }
    }

    // 2. End-to-end MenuAction execution sweep on fresh populated windows.
    for cmd_id in &all_ids {
        let mut fresh_session = Session::from_document(base_doc.clone());
        let _ = fresh_session.execute("table.setCell", json!({ "sheet_id": "data-xy", "row": 0, "col": 0, "value": 10.5 }));
        let (mut rt, wid) = spawn_window_from_session(fresh_session)?;
        rt.window_mut(wid)?.app_mut().grid.select_single_sync(CellCoord::new(0, 0, 0).into());
        if cmd_id == "edit.paste" {
            rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("edit.copy") })?;
        }
        let outcome = rt
            .dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: cmd_id.clone() })
            .map_err(|e| format!("`{cmd_id}` failed: {e}"))?;
        assert!(outcome.handled, "MenuAction `{cmd_id}` unhandled");
    }

    // 3. Empty-doc dispatch never leaks `expected object, got null`.
    for cmd_id in &all_ids {
        let (mut rt, wid) = spawn_window_from_session(Session::from_document(ProjectDocument::default()))?;
        if let Err(err) = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: cmd_id.clone() }) {
            assert!(!err.contains("expected object, got null"), "empty-doc `{cmd_id}` leaked null error: {err}");
        }
    }
    Ok(())
}

fn run_menu_action_on_fresh(base_doc: &ProjectDocument, active_sheet: &str, cmd_id: &str) -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut session = Session::from_document(base_doc.clone());
    let _ = session.execute("sheet.select", json!({ "sheet_id": active_sheet }));
    let (mut rt, wid) = spawn_window_from_session(session)?;
    rt.window_mut(wid)?.app_mut().grid.select_single_sync(CellCoord::new(0, 0, 0).into());
    let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: cmd_id.to_owned() })?;
    Ok(rt.window(wid)?.app().clone())
}

#[test]
fn parameterized_commands_open_expected_dialogs_and_wizards() -> Result<(), Box<dyn std::error::Error>> {
    let base_app = build_populated_routing_app()?;
    let base_doc = base_app.document().clone();

    type DialogCase = (&'static str, &'static [&'static str], fn(&UiApp) -> bool);
    let cases: &[DialogCase] = &[
        ("data-xy", &["analysis.add", "analysis.run", "stats.run", "transform.run", "fit.run", "stats.pvalueStack", "sheet.extractResultTab"], |a| {
            matches!(a.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. })
        }),
        ("data-xy", &["analysis.updateParams"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. })),
        ("data-xy", &["analysis.retargetData"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::ChangeDataAnalyzed { .. })),
        ("graph-xy", &["graph.setKind"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. })),
        ("graph-xy", &["graph.setSpec", "graph.setSeries"], |a| {
            matches!(a.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. })
                || a.canvas_dialogs.active_inspector == ActiveInspector::FormatGraph
        }),
        ("graph-xy", &["graph.setAxes", "graph.setAdditionalTicks", "toolbar.axes"], |a| {
            matches!(a.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. })
                || a.canvas_dialogs.active_inspector == ActiveInspector::FormatAxes
        }),
        ("graph-xy", &["graph.addComparisonBrackets", "graph.setCompactLetterDisplay"], |a| {
            matches!(a.canvas_dialogs.active_dialog, ActiveDialog::PairwiseComparisons { .. })
                || a.canvas_dialogs.active_inspector == ActiveInspector::PairwiseComparisons
        }),
        ("graph-xy", &["graph.formatSelectedPoints"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. })),
        ("graph-xy", &["graph.applyStyleFrom"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. })),
        ("data-xy", &["sheet.createData", "sheet.clonePipeline"], |a| a.nav.welcome.open),
        ("data-xy", &["table.importPlateMap", "table.importTidyLong"], |a| a.canvas_dialogs.active_dialog == ActiveDialog::ImportReshape),
        ("data-xy", &["table.excludeCellsWithReason"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. })),
        ("data-xy", &["table.randomizeGroups"], |a| a.canvas_dialogs.active_dialog == ActiveDialog::RandomizeGroups),
        ("data-xy", &["table.randomSampleRows"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::RandomSampleRows { .. })),
        ("data-xy", &["table.simulateDistributions"], |a| a.canvas_dialogs.active_dialog == ActiveDialog::SimulateDistributions),
        ("data-xy", &["table.convertArchetype"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. })),
        ("data-mv", &["table.setCalculatedVariable"], |a| a.grid.formula_bar.active),
        ("data-mv", &["table.setVariableRole", "table.recodeLevels", "table.setCategoricalOrder"], |a| a.grid.variables_dialog.open),
        ("data-xy", &["table.pasteLink"], |a| a.grid.import_wizard.open && a.grid.import_wizard.is_live_link),
        ("layout-1", &["layout.setSpec", "layout.addPanel"], |a| {
            matches!(a.canvas_dialogs.active_dialog, ActiveDialog::PageLayoutComposer { .. })
                || a.canvas_dialogs.active_inspector == ActiveInspector::PageLayout
        }),
        ("data-xy", &["info.setNote"], |a| a.inline_editor.active && matches!(a.inline_editor.target, Some(InlineEditorTarget::InfoNotes { .. }))),
        ("data-xy", &["info.setConstant"], |a| matches!(a.canvas_dialogs.active_dialog, ActiveDialog::ConstantHookPicker { .. })),
    ];
    for &(sheet, cmds, check) in cases {
        for &cmd_id in cmds {
            let app = run_menu_action_on_fresh(&base_doc, sheet, cmd_id)?;
            assert!(check(&app), "`{cmd_id}` on `{sheet}` did not open expected dialog/wizard");
        }
    }

    assert_eq!(
        resolve_desktop_command_route(&base_app, None, "table.simulateXy")?,
        DesktopCommandRoute::EngineCommand { engine_command_id: String::from("table.simulateXy"), params: json!({}) }
    );
    assert_eq!(
        resolve_desktop_command_route(&base_app, None, "sheet.rename")?,
        DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: Some(String::from("data-xy")),
            query: Some(String::from("sheet.rename"))
        }
    );
    Ok(())
}

#[test]
fn submenu_variants_and_qol7_diagnostics_route_and_persist_preferences() -> Result<(), Box<dyn std::error::Error>> {
    let (mut rt, wid) = spawn_window_from_session(Session::from_document(build_populated_routing_app()?.document().clone()))?;
    let persist_calls = Arc::new(AtomicUsize::new(0));
    let persist_cb = Arc::clone(&persist_calls);
    rt.set_preferences_persist_hook(move |_| {
        persist_cb.fetch_add(1, Ordering::Relaxed);
    });

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.theme.set:high_contrast") })?;
    assert_eq!(rt.window(wid)?.app().theme.mode, UiThemeMode::HighContrast);
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.previewFilter.set:deuteranopia") })?;
    assert_eq!(rt.window(wid)?.app().preview_filter, CanvasPreviewFilter::Deuteranopia);
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.locale.set:de") })?;
    assert!(rt.window(wid)?.app().locale == "de" && persist_calls.load(Ordering::Relaxed) >= 2);

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.canvas.applyPreset:DoubleColumn180mm") })?;
    let g_size = rt.window(wid)?.app().document().graph_sheet(&SheetId::new("graph-xy")).ok_or("missing graph-xy")?.spec.canvas_size_pt;
    let exp = FigureCanvasPreset::DoubleColumn180mm.dimensions_pt();
    assert!((g_size.width - exp.width).abs() < 1e-3 && (g_size.height - exp.height).abs() < 1e-3);

    let before = rt.window(wid)?.app().document().clone();
    let rem = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.result.applyRemediation") })?;
    let rem_res = rem.command_result.ok_or("missing remediation result")?;
    assert!(
        rem.handled && rt.window(wid)?.app().document() == &before && rem_res["mutated"] == json!(false) && rem_res["result_sheet_id"].is_string()
    );
    Ok(())
}

#[test]
fn keyboard_shortcuts_route_context_and_dialogs_correctly() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, wid) = spawn_window_from_session(Session::from_document(build_populated_routing_app()?.document().clone()))?;
    let cmd_mods = egui::Modifiers { command: true, mac_cmd: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..Default::default() };
    let ctrl_mods = egui::Modifiers { ctrl: true, ..Default::default() };
    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(1);

    // 1. Cmd+E toggles exclusion on selected range.
    h.app.runtime_mut().window_mut(wid)?.app_mut().grid.tool.select_single(CellCoord::new(0, 0, 0).into());
    h.app.runtime_mut().window_mut(wid)?.app_mut().grid.tool.extend_selection(CellCoord::new(1, 0, 0).into());
    let _ = h.key_down(egui::Key::E, cmd_mods);
    let ds = h.app.runtime().window(wid)?.app().document().data_sheet(&SheetId::new("data-xy")).ok_or("missing data-xy")?;
    assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded) && ds.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| c.excluded));

    // 2. Ctrl+L -> sheet.freeze.
    h.app.runtime_mut().window_mut(wid)?.app_mut().shortcuts.set_shortcut("sheet.freeze", "Ctrl+L")?;
    let _ = h.key_down(egui::Key::L, ctrl_mods);
    assert!(h.app.runtime().window(wid)?.app().document().data_sheet(&SheetId::new("data-xy")).is_some_and(|s| s.frozen));

    // 3. Cmd+T on DataSheet -> Welcome; on ResultSheet -> EditAnalysisParameters.
    let _ = h.key_down(egui::Key::T, cmd_mods);
    assert!(h.app.runtime().window(wid)?.app().nav.welcome.open);
    h.app.runtime_mut().window_mut(wid)?.app_mut().nav.welcome.open = false;

    let rs_id = h.app.runtime().window(wid)?.app().document().result_sheets.first().ok_or("missing result")?.id.clone();
    let _ = h.app.runtime_mut().window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() }))?;
    let _ = h.key_down(egui::Key::T, cmd_mods);
    assert!(matches!(h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    h.app.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.close_dialog();

    // 4. Cmd+S saves to associated file_path.
    let tmp = TempDirGuard::new("r10-shortcut-save")?;
    let save_path = tmp.child("shortcut-save.curvebook");
    h.app.runtime_mut().window_mut(wid)?.app_mut().session_mut().set_file_path(Some(save_path.clone()), false);
    let _ = h.key_down(egui::Key::S, cmd_mods);
    assert!(save_path.exists());
    Ok(())
}

#[test]
fn mouse_wheel_scrolls_data_grid_and_ignores_non_canvas_without_zooming() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::from_document(ProjectDocument::default());
    let _ = session.execute(
        "sheet.createData",
        json!({ "sheet_id": "big-ds", "title": "Large Table", "kind": "column", "cols": 2, "rows": 80, "auto_graph": true }),
    )?;
    let _ = session.execute("sheet.select", json!({ "sheet_id": "big-ds" }))?;

    let (rt, wid) = spawn_window_from_session(session)?;
    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(2);

    let zoom0 = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    let cell_pt = h.app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing cell")?.clipped_rect.center();
    h.send_wheel(cell_pt, egui::vec2(0.0, -48.0), egui::Modifiers::NONE);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    assert!(h.app.runtime().window(wid)?.app().egui_views.column_grid.scroll_offset_pt.y > 0.0);

    let nav_pt = h.app.inspect_meta().find_by_widget_id("nav.sheet.big-ds").ok_or("missing nav")?.clipped_rect.center();
    let pan_nav = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    h.send_wheel(nav_pt, egui::vec2(0.0, -48.0), egui::Modifiers::NONE);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    assert_eq!(h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan_nav);

    let gid = h.app.runtime().window(wid)?.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
    let _ = h.app.runtime_mut().window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": gid.as_str() }))?;
    h.settle(2);
    let paper_pt = h.app.inspect_meta().find_by_widget_id("canvas.viewport").ok_or("missing canvas.viewport")?.clipped_rect.center();
    let pan_before = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    h.send_wheel(paper_pt, egui::vec2(0.0, 48.0), egui::Modifiers::NONE);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    assert_ne!(h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan_before);

    h.send_wheel(paper_pt, egui::vec2(0.0, 48.0), egui::Modifiers { ctrl: true, command: true, ..Default::default() });
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() > 1e-3);
    Ok(())
}

#[test]
fn native_file_dialog_routing_covers_all_import_export_template_commands() {
    for cmd_id in [
        "project.open",
        "file.open",
        "project.save",
        "file.save",
        "project.saveAs",
        "file.save_as",
        "project.saveTemplate",
        "project.instantiateTemplate",
        "export.svg",
        "export.pdf",
        "export.png",
        "export.tiff",
        "graph.exportSvg",
        "graph.exportPdf",
        "graph.exportPng",
        "graph.exportEmf",
        "layout.exportEmf",
        "graph.insertImage",
        "layout.insertImage",
        "table.importCsv",
        "table.importPrism",
        "table.importPzfx",
        "table.importXlsx",
        "table.exportCsv",
        "analysis.exportJson",
        "analysis.exportCsv",
        "result.generateReport",
        "file.export_svg",
        "file.export_pdf",
        "file.export_png",
    ] {
        assert!(menu_action_triggers_native_file_dialog(cmd_id), "missing native dialog for `{cmd_id}`");
    }
    assert!(!menu_action_triggers_native_file_dialog("edit.undo") && !menu_action_triggers_native_file_dialog("ui.analyze.open"));
}

#[test]
fn native_open_template_filters_and_foreign_import_file_path_clearing() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(NATIVE_OPEN_FILE_EXTENSIONS, ["curvebook", "cbtemplate", "prism", "pzfx", "xlsx", "csv", "tsv"]);
    assert_eq!((NATIVE_TEMPLATE_FILE_EXTENSIONS, DEFAULT_TEMPLATE_FILE_NAME), (["cbtemplate"].as_slice(), "untitled.cbtemplate"));
    for cmd in ["project.instantiateTemplate", "table.importPrism", "table.importPzfx", "table.importXlsx"] {
        assert!(native_import_replaces_project_file_path(cmd));
    }
    assert!(!native_import_replaces_project_file_path("table.importCsv"));

    let tmp = TempDirGuard::new("r10-filters")?;
    let (project_path, template_path, csv_path) = (tmp.child("origin.curvebook"), tmp.child("origin.cbtemplate"), tmp.child("append.csv"));
    fs::write(&csv_path, "X,Y\n1,2\n3,4\n")?;

    let (mut rt, wid) = spawn_window_from_session(Session::from_document(build_populated_routing_app()?.document().clone()))?;
    let win = rt.window_mut(wid)?;
    win.execute_and_journal("project.save", json!({ "path": project_path.to_string_lossy(), "unpacked": false }))?;
    win.app_mut().session_mut().set_file_path(Some(project_path.clone()), true);
    win.execute_and_journal("project.saveTemplate", json!({ "path": template_path.to_string_lossy() }))?;
    assert!(template_path.exists());

    execute_native_import_at_path(win, "table.importCsv", &csv_path, false)?;
    assert_eq!(win.app().session().file_path(), Some(project_path.as_path()));
    execute_native_import_at_path(win, "project.instantiateTemplate", &template_path, false)?;
    assert!(win.app().session().file_path().is_none() && win.app().session().is_dirty());
    Ok(())
}

#[test]
fn welcome_open_file_dialog_hook_invoked_and_escape_closes_welcome() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r10-welcome-open")?;
    let archive_path = tmp.child("welcome-open.curvebook");
    save_project_file(&archive_path, build_populated_routing_app()?.document())?;

    let (mut rt, wid) = spawn_window_from_session(Session::new())?;
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let (hook_cb, archive_cb) = (Arc::clone(&hook_calls), archive_path);
    rt.set_open_file_dialog_hook(move |_| {
        hook_cb.fetch_add(1, Ordering::SeqCst);
        Ok(Some(archive_cb.clone()))
    });

    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(2);
    let _ = h.click_widget("welcome.action.open_existing")?;
    if h.app.inspect_meta().find_by_widget_id("welcome.btn.open_file").is_some() {
        let _ = h.click_widget("welcome.btn.open_file")?;
    }
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
    assert!(h.app.runtime().window(wid)?.app().document().total_sheet_count() >= 1);

    h.app.runtime_mut().window_mut(wid)?.app_mut().nav.welcome.open = true;
    h.settle(1);
    let _ = h.key_down(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!h.app.runtime().window(wid)?.app().nav.welcome.open);

    h.open_menu_item("menu.file", None, "menu.project.open")?;
    assert_eq!(hook_calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn r10_23_drawn_file_menu_routes_open_save_save_as_and_exports_through_adapter() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r1023-file-dlg")?;
    let (cb_path, pzfx_path, zip_path, csv_path) =
        (tmp.child("sample.curvebook"), tmp.child("cohort.pzfx"), tmp.child("kinetics.prism"), tmp.child("extra.csv"));
    save_project_file(&cb_path, &build_col_doc("ds-cb", "Native Archive Sheet", &[("A", &[42.5])])?)?;
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile SchemaVersion="2"><Title>PZFX Dish</Title><TableSequence><Table ID="t-pzfx" TableType="OneWay"><Title>PZFX Imported Columns</Title><YColumn><Title>Ctrl</Title><Subcolumn><d>11.0</d><d>13.0</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#,
    )?;
    let manifest = serde_json::to_string(
        &json!({ "title": "Archive Container Assay", "sheets": [{ "id": "sheet-kin", "title": "Archive Zip Kinetics", "kind": "XY", "subcolumn_mode": "Single", "csv_path": "data/kin.csv" }] }),
    )?;
    fs::write(&zip_path, build_zip_entries(&[("manifest.json", &manifest), ("data/kin.csv", "Time,Resp\n0,2.5\n5,8.5\n")])?)?;
    fs::write(&csv_path, "GroupA,GroupB\n10.25,20.75\n")?;

    let (save_path, save_as_path, svg_path, pdf_path, png_path) =
        (tmp.child("saved.curvebook"), tmp.child("saved-as.curvebook"), tmp.child("out.svg"), tmp.child("out.pdf"), tmp.child("out.png"));
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    for (cmd, p) in [
        ("project.open", &cb_path),
        ("project.open", &pzfx_path),
        ("project.open", &zip_path),
        ("project.open", &csv_path),
        ("project.save", &save_path),
        ("file.save_as", &save_as_path),
        ("export.svg", &svg_path),
        ("export.pdf", &pdf_path),
        ("export.png", &png_path),
    ] {
        adapter.push_response(cmd, Some(p.clone()));
    }

    let metrics = ViewportMetrics::new(1280.0, 820.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        initial_viewport: metrics,
        clipboard_adapter: Some(Arc::new(StubPlatformClipboardAdapter::new())),
        ..DesktopHostConfig::default()
    });
    rt.set_file_dialog_adapter(adapter.clone());
    let wid = rt.open_window(None, metrics)?;
    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, wid, None), egui::vec2(1280.0, 820.0));
    h.settle(2);

    for expected_title in ["Native Archive Sheet", "PZFX Imported Columns", "Archive Zip Kinetics", "extra"] {
        h.open_menu_item("menu.file", None, "menu.project.open")?;
        let cur = h.app.runtime().focused_window_id().unwrap_or(wid);
        assert!(h.app.runtime().window(cur)?.app().document().data_sheets.iter().any(|ds| ds.title == expected_title), "missing `{expected_title}`");
    }

    // Add GraphSheet to enable exports, then test Save, direct Save, Save As, and Exports.
    let active_wid = h.app.runtime().focused_window_id().unwrap_or(wid);
    {
        let win = h.app.runtime_mut().window_mut(active_wid)?;
        let mut doc = win.app().document().clone();
        let ds_ref = doc.data_sheets.first().ok_or("missing data sheet")?.clone();
        let mut graph = GraphSheet::new_uninitialized_for_data("g-export", &ds_ref)?;
        graph.initialized = true;
        graph.title = String::from("Export Plot");
        graph.spec.kind = GraphKind::Column;
        doc.add_graph_sheet(graph)?;
        let _ = doc.select_in_navigator(&"g-export".into());
        let tmp_seed = tmp.child("seed-export.curvebook");
        save_project_file(&tmp_seed, &doc)?;
        let _ = win.execute_and_journal("project.open", json!({ "path": tmp_seed.to_string_lossy() }))?;
        win.app_mut().session_mut().set_file_path(None, false);
        win.app_mut().sync_views();
    }

    let reqs_before = adapter.requests().len();
    h.open_menu_item("menu.file", None, "menu.project.save")?;
    assert!(
        save_path.is_file()
            && h.app.runtime().window(active_wid)?.app().session().file_path() == Some(Path::new(&save_path))
            && adapter.requests().len() == reqs_before + 1
    );
    h.open_menu_item("menu.file", None, "menu.project.save")?;
    assert_eq!(adapter.requests().len(), reqs_before + 1);

    h.open_menu_item("menu.file", None, "menu.file.save_as")?;
    assert!(
        save_as_path.is_file()
            && h.app.runtime().window(active_wid)?.app().session().file_path() == Some(Path::new(&save_as_path))
            && !open_project_file(&save_as_path)?.data_sheets.is_empty()
    );

    h.open_menu_item("menu.file", Some("menu.file.export"), "menu.export.svg")?;
    assert!(fs::read_to_string(&svg_path)?.contains("<svg"));
    h.open_menu_item("menu.file", Some("menu.file.export"), "menu.export.pdf")?;
    assert!(fs::read(&pdf_path)?.starts_with(b"%PDF"));
    h.open_menu_item("menu.file", Some("menu.file.export"), "menu.export.png")?;
    assert!(fs::read(&png_path)?.starts_with(b"\x89PNG"));

    let recorded = adapter.requests();
    assert_eq!(recorded.len(), 9);
    assert!(recorded[..4].iter().all(|r| r.kind == NativeFileDialogKind::OpenFile && r.extensions.iter().any(|e| e == "pzfx")));
    assert_eq!((recorded[4].kind, recorded[5].command_id.as_str()), (NativeFileDialogKind::SaveFile, "file.save_as"));
    assert!(recorded[6].extensions == ["svg"] && recorded[7].extensions == ["pdf"] && recorded[8].extensions == ["png"]);
    Ok(())
}

#[test]
fn r13_7_native_rfd_image_picker_inserts_png_on_graph_and_layout() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r13-7-insert")?;
    let png_path = tmp.child("r13-7-insert.png");
    fs::write(&png_path, encode_png(&RasterImage { width_px: 16, height_px: 8, dpi: 96, rgba: [40, 120, 220, 255].repeat(16 * 8) })?)?;

    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    adapter.push_response("graph.insertImage", Some(png_path.clone()));
    adapter.push_response("layout.insertImage", Some(png_path));

    let (mut rt, wid) = spawn_window_from_session(Session::new())?;
    rt.set_file_dialog_adapter(adapter.clone());
    for (cmd_id, params) in [
        ("sheet.createData", json!({ "sheet_id": "data-xy", "title": "Image Host Data", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": false })),
        ("graph.create", json!({ "sheet_id": "g-img", "data_sheet_id": "data-xy", "kind": "xy" })),
        ("layout.create", json!({ "sheet_id": "lay-img", "title": "Img Layout" })),
        ("sheet.select", json!({ "sheet_id": "g-img" })),
    ] {
        rt.window_mut(wid)?.app_mut().execute_command(cmd_id, params)?;
    }
    assert!(rt.dispatch_native_file_command(wid, "graph.insertImage")?.ok_or("missing graph dialog")?.handled);
    rt.window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "lay-img" }))?;
    assert!(rt.dispatch_native_file_command(wid, "layout.insertImage")?.ok_or("missing layout dialog")?.handled);

    let reqs = adapter.requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs.iter().all(|r| r.kind == NativeFileDialogKind::OpenFile && r.extensions == ["png", "jpg", "jpeg", "tif", "tiff"]));
    let doc = rt.window(wid)?.app().document();
    assert!(
        doc.graph_sheet(&SheetId::new("g-img"))
            .ok_or("missing g-img")?
            .spec
            .annotations
            .iter()
            .any(|a| matches!(a, GraphAnnotation::EmbeddedImage { .. }))
    );
    assert!(
        doc.layout_sheet(&SheetId::new("lay-img"))
            .ok_or("missing lay-img")?
            .spec
            .items
            .iter()
            .any(|it| matches!(it, LayoutItem::EmbeddedImage { .. }))
    );
    Ok(())
}
