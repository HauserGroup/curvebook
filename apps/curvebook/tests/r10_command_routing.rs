#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook::desktop_host::{
    DesktopCommandRoute, DesktopDialogOpener, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, resolve_desktop_command_route,
};
use curvebook::gui_window::{
    CurveBookNativeApp, DEFAULT_TEMPLATE_FILE_NAME, NATIVE_OPEN_FILE_EXTENSIONS, NATIVE_TEMPLATE_FILE_EXTENSIONS, build_egui_font_definitions,
    execute_native_import_at_path, menu_action_triggers_native_file_dialog, native_import_replaces_project_file_path,
};
use curvebook_doc::{CellCoord, FigureCanvasPreset, ProjectDocument, SheetId};
use curvebook_engine::{Session, cmd};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector};
use curvebook_ui_egui::interaction::{InlineEditorTarget, KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::menus::{TOOLBAR_BUTTONS, UI_COMMANDS, menu_bar_tooltips};
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{CanvasPreviewFilter, StubPlatformClipboardAdapter, UiApp, dispatch_gui_action};
use eframe::egui;
use serde_json::{Value, json};

fn unique_temp_file(name: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("curvebook-r10-{}-{nanos}-{name}", std::process::id()))
}

fn build_populated_routing_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let _ = metrics;

    // 1. XY table with linked InfoSheet + GraphSheet + uninitialized GraphSheet.
    let _ = app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "data-xy",
            "title": "Dose Response",
            "kind": "xy",
            "cols": 2,
            "rows": 5,
            "auto_info": true,
            "auto_graph": false
        }),
    )?;
    let _ = app.execute_command("table.setXCells", json!({ "sheet_id": "data-xy", "values": [1.0, 2.0, 4.0, 8.0, 16.0] }))?;
    let _ = app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "data-xy",
            "rows": [
                [10.0, 11.0],
                [20.0, 22.0],
                [40.0, 39.0],
                [78.0, 81.0],
                [95.0, 96.0]
            ]
        }),
    )?;

    // 2. Column table for group/column commands and uninitialized graph target.
    let _ = app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "data-col",
            "title": "Column Data",
            "kind": "column",
            "cols": 2,
            "rows": 4,
            "auto_info": false,
            "auto_graph": false
        }),
    )?;
    let _ = app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "data-col",
            "rows": [[1.0, 2.0], [1.5, 2.5], [1.2, 2.7], [1.4, 2.4]]
        }),
    )?;

    // 3. MultipleVariables table + clustering ResultSheet so even MV/Cluster commands succeed.
    let _ = app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "data-mv",
            "title": "MV Features",
            "kind": "multiple_variables",
            "variables": [
                { "title": "F1", "role": "continuous" },
                { "title": "F2", "role": "continuous" },
                { "title": "F3", "role": "continuous" }
            ],
            "rows": 6,
            "auto_info": false,
            "auto_graph": false
        }),
    )?;
    let _ = app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "data-mv",
            "rows": [
                [1.0, 1.0, 2.0],
                [1.1, 0.9, 2.1],
                [0.9, 1.1, 1.9],
                [10.0, 10.0, 8.0],
                [10.2, 9.8, 8.2],
                [9.9, 10.1, 7.9]
            ]
        }),
    )?;

    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "data-mv" }))?;
    let _ = curvebook_ui_egui::dispatch_ui_command(&mut app, "ui.analyze.open", &json!({ "sheet_id": "data-mv" }))
        .map_err(|e| format!("failed to open Analyze dialog for fixture: {e}"))?;
    let _ =
        dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.clustering", "k_means_k": 2 } }))?;
    app.canvas_dialogs.close_dialog();

    // 4. Graph sheets (one initialized on data-xy, one uninitialized on data-col).
    let _ = app.execute_command(
        "graph.create",
        json!({
            "data_sheet_id": "data-xy",
            "sheet_id": "graph-xy",
            "title": "XY Curve",
            "kind": "xy",
            "initialized": true
        }),
    )?;
    let _ = app.execute_command(
        "graph.create",
        json!({
            "data_sheet_id": "data-col",
            "sheet_id": "graph-uninit",
            "title": "Uninitialized Column Graph",
            "initialized": false
        }),
    )?;

    // 5. Layout sheet with a panel so layout.alignOrigins/exportEmf/composer all have valid targets.
    let _ = app.execute_command("layout.create", json!({ "sheet_id": "layout-1", "title": "Figure 1" }))?;
    let _ = app.execute_command("layout.addPanel", json!({ "sheet_id": "layout-1", "graph_sheet_id": "graph-xy" }))?;

    // 5b. Seed embedded result table and image objects so R13 table/image formatting and reorder routes have targets.
    let result_id = app.document().result_sheets.first().map(|r| r.id.as_str().to_owned()).ok_or("expected populated result sheet")?;
    let _ = app.execute_command("edit.pasteResultTable", json!({ "source_sheet_id": result_id, "target_sheet_id": "graph-xy" }))?;
    let tiny_png = curvebook_render::png::encode_png(&curvebook_render::RasterImage { width_px: 2, height_px: 2, dpi: 96, rgba: vec![255u8; 16] })?;
    let tiny_png_b64 = encode_base64(&tiny_png);
    let _ = app.execute_command("graph.insertImage", json!({ "sheet_id": "graph-xy", "bytes_base64": tiny_png_b64.clone() }))?;
    let _ = app.execute_command("layout.insertImage", json!({ "sheet_id": "layout-1", "bytes_base64": tiny_png_b64 }))?;

    // 6. Focus back onto data-xy so grid cell selection exists cleanly.
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "data-xy" }))?;
    app.grid.select_single_sync(curvebook_doc::CellCoord::new(0, 0, 0).into());
    app.sync_views();
    Ok(app)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = usize::from(*chunk.first().unwrap_or(&0));
        let b1 = usize::from(*chunk.get(1).unwrap_or(&0));
        let b2 = usize::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let encode = |shift: usize| char::from(TABLE.get((triple >> shift) & 0x3F).copied().unwrap_or(b'A'));
        out.push(encode(18));
        out.push(encode(12));
        out.push(if chunk.len() > 1 { encode(6) } else { '=' });
        out.push(if chunk.len() > 2 { encode(0) } else { '=' });
    }
    out
}

fn spawn_window_from_session(session: Session) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        ui_backend: curvebook_ui_egui::egui_shell::UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: metrics,
        clipboard_adapter: Some(Arc::new(StubPlatformClipboardAdapter::new())),
        ..DesktopHostConfig::default()
    });
    let wid = rt.open_window_with_session(session, metrics, false)?;
    Ok((rt, wid))
}

fn collect_all_desktop_command_ids(sample_app: &UiApp) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for spec in cmd::registry().specs() {
        ids.insert(spec.id.to_owned());
    }
    for spec in UI_COMMANDS {
        ids.insert(spec.id.to_owned());
    }
    for btn in TOOLBAR_BUTTONS {
        ids.insert(btn.id.to_owned());
        ids.insert(btn.command_id.to_owned());
    }
    for sec in menu_bar_tooltips(sample_app) {
        for item in sec.items {
            ids.insert(item.command_id.to_owned());
        }
    }
    for alias in [
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
    ] {
        ids.insert(alias.to_owned());
    }
    ids
}

#[test]
fn registry_sweep_routes_every_engine_and_ui_command_without_null_params() -> Result<(), Box<dyn std::error::Error>> {
    let base_app = build_populated_routing_app()?;
    let base_doc = base_app.document().clone();
    let all_ids = collect_all_desktop_command_ids(&base_app);

    assert!(all_ids.len() >= cmd::registry().len() + UI_COMMANDS.len());

    // 1. Route resolution sweep on BOTH empty and populated documents must NEVER produce Value::Null.
    let empty_app = UiApp::new();
    for cmd_id in &all_ids {
        if let Ok(DesktopCommandRoute::EngineCommand { params, .. } | DesktopCommandRoute::UiCommand { params, .. }) =
            resolve_desktop_command_route(&empty_app, None, cmd_id)
        {
            assert!(!params.is_null(), "command `{cmd_id}` resolved to Value::Null on empty doc");
        }

        let route = resolve_desktop_command_route(&base_app, None, cmd_id)
            .map_err(|e| format!("command `{cmd_id}` failed route resolution on populated doc: {e}"))?;
        match &route {
            DesktopCommandRoute::EngineCommand { engine_command_id, params } => {
                assert!(!params.is_null(), "engine route `{cmd_id}` -> `{engine_command_id}` dispatched Value::Null");
                assert!(params.is_object(), "engine route `{cmd_id}` -> `{engine_command_id}` params must be JSON object, got {params}");
                let spec = cmd::registry().get(engine_command_id).ok_or_else(|| format!("unknown routed engine command `{engine_command_id}`"))?;
                let schema = (spec.input_schema)();
                if let Some(req) = schema.get("required").and_then(Value::as_array) {
                    for key_val in req {
                        let req_key = key_val.as_str().ok_or("non-string required key")?;
                        assert!(
                            params.get(req_key).is_some_and(|v| !v.is_null()),
                            "engine route `{cmd_id}` -> `{engine_command_id}` missing required `{req_key}` in {params}"
                        );
                    }
                }
            }
            DesktopCommandRoute::UiCommand { ui_command_id, params } => {
                assert!(!params.is_null(), "ui route `{cmd_id}` -> `{ui_command_id}` dispatched Value::Null");
            }
            DesktopCommandRoute::HostAction { .. } | DesktopCommandRoute::UiDialogOpener { .. } => {}
        }
    }

    // 2. End-to-end MenuAction execution sweep on fresh populated windows: EVERY command succeeds.
    for cmd_id in &all_ids {
        let mut fresh_session = Session::from_document(base_doc.clone());
        // Seed one edit so edit.undo / edit.redo also exercise non-empty undo state cleanly.
        let _ = fresh_session.execute("table.setCell", json!({ "sheet_id": "data-xy", "row": 0, "col": 0, "value": 10.5 }));
        let (mut rt, wid) = spawn_window_from_session(fresh_session)?;
        {
            let win = rt.window_mut(wid)?;
            win.app_mut().grid.select_single_sync(CellCoord::new(0, 0, 0).into());
            // For commands requiring clipboard data during edit.paste, seed clipboard text.
            if cmd_id == "edit.paste" {
                rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("edit.copy") })?;
            }
        }
        // Special case: table.setVariableRole / recodeLevels / setCategoricalOrder / setCalculatedVariable open for MV sheet
        // which `resolve_mv_sheet_id` finds automatically even while data-xy is active!
        let outcome = rt
            .dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: cmd_id.clone() })
            .map_err(|e| format!("MenuAction `{cmd_id}` failed on populated window: {e}"))?;
        assert!(outcome.handled, "MenuAction `{cmd_id}` was unexpectedly marked unhandled: {outcome:?}");
    }

    // 3. Empty document dispatch safety check: no command may return engine's `expected object, got null` error.
    for cmd_id in &all_ids {
        let (mut rt, wid) = spawn_window_from_session(Session::from_document(ProjectDocument::default()))?;
        if let Err(err) = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: cmd_id.clone() }) {
            assert!(!err.contains("expected object, got null"), "empty-doc MenuAction `{cmd_id}` leaked Value::Null error: {err}");
        }
    }

    Ok(())
}

fn run_menu_action_on_fresh(base_doc: &ProjectDocument, active_sheet: &str, cmd_id: &str) -> Result<(UiApp, Value), Box<dyn std::error::Error>> {
    let mut session = Session::from_document(base_doc.clone());
    let _ = session.execute("sheet.select", json!({ "sheet_id": active_sheet }));
    let (mut rt, wid) = spawn_window_from_session(session)?;
    rt.window_mut(wid)?.app_mut().grid.select_single_sync(CellCoord::new(0, 0, 0).into());
    let out = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: cmd_id.to_owned() })?;
    let app = rt.window(wid)?.app().clone();
    Ok((app, out.command_result.unwrap_or(Value::Null)))
}

fn check_analysis_and_graph_openers(base_doc: &ProjectDocument) -> Result<(), Box<dyn std::error::Error>> {
    for analyze_cmd in ["analysis.add", "analysis.run", "stats.run", "transform.run", "fit.run", "stats.pvalueStack", "sheet.extractResultTab"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", analyze_cmd)?;
        assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }), "{analyze_cmd} did not open AnalyzeSheet");
    }

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "analysis.updateParams")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "analysis.retargetData")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeDataAnalyzed { .. }));

    let (app, _) = run_menu_action_on_fresh(base_doc, "graph-xy", "graph.setKind")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }));

    for fmt_cmd in ["graph.setSpec", "graph.setSeries"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "graph-xy", fmt_cmd)?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. })
                || app.canvas_dialogs.active_inspector == ActiveInspector::FormatGraph,
            "{fmt_cmd} did not open FormatGraph"
        );
    }

    for axes_cmd in ["graph.setAxes", "graph.setAdditionalTicks", "toolbar.axes"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "graph-xy", axes_cmd)?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. })
                || app.canvas_dialogs.active_inspector == ActiveInspector::FormatAxes,
            "{axes_cmd} did not open FormatAxes"
        );
    }

    for pairwise_cmd in ["graph.addComparisonBrackets", "graph.setCompactLetterDisplay"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "graph-xy", pairwise_cmd)?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::PairwiseComparisons { .. })
                || app.canvas_dialogs.active_inspector == ActiveInspector::PairwiseComparisons,
            "{pairwise_cmd} did not open PairwiseComparisons"
        );
    }

    let (app, _) = run_menu_action_on_fresh(base_doc, "graph-xy", "graph.formatSelectedPoints")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatSelectedPoints { .. }));

    let (app, _) = run_menu_action_on_fresh(base_doc, "graph-xy", "graph.applyStyleFrom")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. }));
    Ok(())
}

fn check_table_layout_and_info_openers(base_doc: &ProjectDocument) -> Result<(), Box<dyn std::error::Error>> {
    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "sheet.createData")?;
    assert!(app.nav.welcome.open);

    for reshape_cmd in ["table.importPlateMap", "table.importTidyLong"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", reshape_cmd)?;
        assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::ImportReshape);
    }

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "table.excludeCellsWithReason")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }));

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "table.randomizeGroups")?;
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::RandomizeGroups);

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "table.randomSampleRows")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::RandomSampleRows { .. }));

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "table.simulateDistributions")?;
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::SimulateDistributions);

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "table.convertArchetype")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ConvertArchetype { .. }));

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-mv", "table.setCalculatedVariable")?;
    assert!(app.grid.formula_bar.active);

    for var_cmd in ["table.setVariableRole", "table.recodeLevels", "table.setCategoricalOrder"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "data-mv", var_cmd)?;
        assert!(app.grid.variables_dialog.open, "{var_cmd} did not open VariablesDialog");
    }

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "table.pasteLink")?;
    assert!(app.grid.import_wizard.open && app.grid.import_wizard.is_live_link);

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "sheet.clonePipeline")?;
    assert!(app.nav.welcome.open);

    for layout_cmd in ["layout.setSpec", "layout.addPanel"] {
        let (app, _) = run_menu_action_on_fresh(base_doc, "layout-1", layout_cmd)?;
        assert!(
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::PageLayoutComposer { .. })
                || app.canvas_dialogs.active_inspector == ActiveInspector::PageLayout,
            "{layout_cmd} did not open PageLayoutComposer"
        );
    }

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "info.setNote")?;
    assert!(app.inline_editor.active && matches!(app.inline_editor.target, Some(InlineEditorTarget::InfoNotes { .. })));

    let (app, _) = run_menu_action_on_fresh(base_doc, "data-xy", "info.setConstant")?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ConstantHookPicker { .. }));
    Ok(())
}

#[test]
fn parameterized_commands_open_expected_dialogs_and_wizards() -> Result<(), Box<dyn std::error::Error>> {
    let base_app = build_populated_routing_app()?;
    let base_doc = base_app.document().clone();
    check_analysis_and_graph_openers(&base_doc)?;
    check_table_layout_and_info_openers(&base_doc)?;

    assert_eq!(
        resolve_desktop_command_route(&base_app, None, "table.simulateXy")?,
        DesktopCommandRoute::EngineCommand { engine_command_id: String::from("table.simulateXy"), params: json!({}) }
    );
    assert_eq!(
        resolve_desktop_command_route(&base_app, None, "sheet.rename")?,
        DesktopCommandRoute::UiDialogOpener {
            kind: DesktopDialogOpener::CommandPalette,
            target_sheet_id: Some(String::from("data-xy")),
            query: Some(String::from("sheet.rename")),
        }
    );

    Ok(())
}

#[test]
fn submenu_variants_and_qol7_diagnostics_route_and_persist_preferences() -> Result<(), Box<dyn std::error::Error>> {
    let base_doc = build_populated_routing_app()?.document().clone();
    let (mut rt, wid) = spawn_window_from_session(Session::from_document(base_doc))?;
    let persist_calls = Arc::new(AtomicUsize::new(0));
    let persist_calls_clone = Arc::clone(&persist_calls);
    rt.set_preferences_persist_hook(move |_app| {
        persist_calls_clone.fetch_add(1, Ordering::Relaxed);
    });

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.theme.set:high_contrast") })?;
    assert_eq!(rt.window(wid)?.app().theme.mode, UiThemeMode::HighContrast);

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.previewFilter.set:deuteranopia") })?;
    assert_eq!(rt.window(wid)?.app().preview_filter, CanvasPreviewFilter::Deuteranopia);

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.locale.set:de") })?;
    assert_eq!(rt.window(wid)?.app().locale, "de");
    assert!(persist_calls.load(Ordering::Relaxed) >= 2);

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.canvas.applyPreset:DoubleColumn180mm") })?;
    let graph = rt.window(wid)?.app().document().graph_sheet(&SheetId::new("graph-xy")).ok_or("missing graph-xy")?;
    let expected_size = FigureCanvasPreset::DoubleColumn180mm.dimensions_pt();
    assert!((graph.spec.canvas_size_pt.width - expected_size.width).abs() < 1e-3);
    assert!((graph.spec.canvas_size_pt.height - expected_size.height).abs() < 1e-3);

    let doc_before_remediation = rt.window(wid)?.app().document().clone();
    let rem_out = rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.result.applyRemediation") })?;
    assert!(rem_out.handled);
    assert_eq!(rt.window(wid)?.app().document(), &doc_before_remediation);
    let rem_res = rem_out.command_result.ok_or("missing remediation command result")?;
    assert_eq!(rem_res["mutated"], json!(false));
    assert!(rem_res["result_sheet_id"].as_str().is_some());
    Ok(())
}

#[test]
fn keyboard_shortcuts_route_context_and_dialogs_correctly() -> Result<(), Box<dyn std::error::Error>> {
    let base_doc = build_populated_routing_app()?.document().clone();
    let (mut rt, wid) = spawn_window_from_session(Session::from_document(base_doc))?;
    let meta = KeyModifiers { meta: true, ..KeyModifiers::default() };
    let ctrl = KeyModifiers { ctrl: true, ..KeyModifiers::default() };

    // 1. Cmd+E toggles exclusion on selected cell bounds cleanly.
    rt.window_mut(wid)?.app_mut().grid.tool.select_single(CellCoord::new(0, 0, 0).into());
    rt.window_mut(wid)?.app_mut().grid.tool.extend_selection(CellCoord::new(1, 0, 0).into());
    let excl_out =
        rt.dispatch(DesktopHostEvent::KeyboardInput { window_id: wid, key: String::from("E"), pressed: true, modifiers: meta, text: None })?;
    assert!(excl_out.handled);
    assert_eq!(excl_out.command_id.as_deref(), Some("table.excludeCells"));
    let ds = rt.window(wid)?.app().document().data_sheet(&SheetId::new("data-xy")).ok_or("missing data-xy")?;
    assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded));
    assert!(ds.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| c.excluded));

    // 2. Bind Ctrl+L -> `sheet.freeze` via ShortcutRegistry and trigger freeze cleanly without Value::Null.
    rt.window_mut(wid)?.app_mut().shortcuts.set_shortcut("sheet.freeze", "Ctrl+L").map_err(|e| format!("failed to bind Ctrl+L: {e}"))?;
    let freeze_out =
        rt.dispatch(DesktopHostEvent::KeyboardInput { window_id: wid, key: String::from("L"), pressed: true, modifiers: ctrl, text: None })?;
    assert!(freeze_out.handled);
    assert_eq!(freeze_out.command_id.as_deref(), Some("sheet.freeze"));
    assert!(rt.window(wid)?.app().document().data_sheet(&SheetId::new("data-xy")).is_some_and(|s| s.frozen));

    // 3. Cmd+T on DataSheet opens WelcomeWizard; on ResultSheet opens EditAnalysisParameters.
    let t_data =
        rt.dispatch(DesktopHostEvent::KeyboardInput { window_id: wid, key: String::from("T"), pressed: true, modifiers: meta, text: None })?;
    assert!(t_data.handled);
    assert!(rt.window(wid)?.app().nav.welcome.open);
    rt.window_mut(wid)?.app_mut().nav.welcome.open = false;

    let rs_id = rt.window(wid)?.app().document().result_sheets.first().ok_or("missing result")?.id.clone();
    let _ = rt.window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": rs_id.as_str() }))?;
    let t_res =
        rt.dispatch(DesktopHostEvent::KeyboardInput { window_id: wid, key: String::from("T"), pressed: true, modifiers: meta, text: None })?;
    assert!(t_res.handled);
    assert!(matches!(rt.window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    rt.window_mut(wid)?.app_mut().canvas_dialogs.close_dialog();

    // 4. Cmd+S saves to associated file_path cleanly when set.
    let save_path = unique_temp_file("shortcut-save.curvebook");
    rt.window_mut(wid)?.app_mut().session_mut().set_file_path(Some(save_path.clone()), false);
    let save_out =
        rt.dispatch(DesktopHostEvent::KeyboardInput { window_id: wid, key: String::from("S"), pressed: true, modifiers: meta, text: None })?;
    assert!(save_out.handled);
    assert!(save_path.exists());
    let _ = std::fs::remove_file(&save_path);

    Ok(())
}

#[test]
fn mouse_wheel_scrolls_data_grid_and_ignores_non_canvas_without_zooming() -> Result<(), Box<dyn std::error::Error>> {
    // Build a large table so max_scroll_y_px > 0.
    let mut session = Session::from_document(ProjectDocument::default());
    let _ = session.execute(
        "sheet.createData",
        json!({ "sheet_id": "big-ds", "title": "Large Table", "kind": "column", "cols": 2, "rows": 80, "auto_graph": true }),
    )?;
    let _ = session.execute("sheet.select", json!({ "sheet_id": "big-ds" }))?;

    let (mut rt, wid) = spawn_window_from_session(session)?;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let snap = curvebook_ui_egui::inspect_ui_tree_for_metrics(rt.window(wid)?.app(), metrics, None, true);

    // 1. Wheel over DataGrid scrolls grid without altering canvas zoom.
    let cell_bounds = snap.find_widget("grid.cell.0.0.0").and_then(|n| n.bounds).ok_or("missing cell bounds")?;
    let cell_pt = metrics.physical_to_logical_px([cell_bounds[0] + 12.0, cell_bounds[1] + 10.0]);
    let zoom_before = rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    let grid_wheel = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: cell_pt[0],
        y_pt: cell_pt[1],
        delta_x: 0.0,
        delta_y: -48.0,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(grid_wheel.handled);
    assert_eq!(grid_wheel.action, "grid.scroll");
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_before).abs() < 1e-9);
    assert!(rt.window(wid)?.app().grid.tool.viewport.scroll_y_px > 0.0);

    // 2. Wheel over Navigator ignores zoom cleanly.
    let nav_bounds = snap.navigator_sheets.as_deref().and_then(|ns| ns.first()).and_then(|n| n.bounds).ok_or("missing nav entry bounds")?;
    let nav_pt = metrics.physical_to_logical_px([nav_bounds[0] + 12.0, nav_bounds[1] + 8.0]);
    let nav_wheel = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: nav_pt[0],
        y_pt: nav_pt[1],
        delta_x: 0.0,
        delta_y: -48.0,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(!nav_wheel.handled);
    assert_eq!(nav_wheel.action, "wheel.non_canvas_ignored");
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_before).abs() < 1e-9);

    // 3. Unmodified vertical wheel over GraphSheet viewport pans cleanly without zooming (`B4`).
    let graph_id = rt.window(wid)?.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
    let _ = rt.window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
    let gsnap = curvebook_ui_egui::inspect_ui_tree_for_metrics(rt.window(wid)?.app(), metrics, Some("canvas"), true);
    let vp_bounds = gsnap.find_widget("canvas.viewport").and_then(|n| n.bounds).ok_or("missing canvas.viewport")?;
    let paper_pt = metrics.physical_to_logical_px([vp_bounds[0] + vp_bounds[2] * 0.5, vp_bounds[1] + vp_bounds[3] * 0.5]);
    let pan_before = rt.window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    let pan_wheel = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: paper_pt[0],
        y_pt: paper_pt[1],
        delta_x: 0.0,
        delta_y: 48.0,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(pan_wheel.handled);
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_before).abs() < 1e-9);
    assert_ne!(rt.window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan_before);

    // 4. Ctrl+Wheel over GraphSheet ScenePaper zooms canvas cleanly (`B4`).
    let zoom_wheel = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: paper_pt[0],
        y_pt: paper_pt[1],
        delta_x: 0.0,
        delta_y: 48.0,
        modifiers: KeyModifiers { ctrl: true, ..KeyModifiers::default() },
    })?;
    assert!(zoom_wheel.handled);
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_before).abs() > 1e-3);

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
        assert!(menu_action_triggers_native_file_dialog(cmd_id), "missing native file dialog interception for `{cmd_id}`");
    }
    assert!(!menu_action_triggers_native_file_dialog("edit.undo"));
    assert!(!menu_action_triggers_native_file_dialog("ui.analyze.open"));
}

#[test]
fn native_open_template_filters_and_foreign_import_file_path_clearing() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(NATIVE_OPEN_FILE_EXTENSIONS, ["curvebook", "cbtemplate", "prism", "pzfx", "xlsx", "csv", "tsv"]);
    assert!(!NATIVE_OPEN_FILE_EXTENSIONS.contains(&"json"));
    assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(&"xlsx"));
    assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(&"cbtemplate"));
    assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(&"prism"));

    assert_eq!(NATIVE_TEMPLATE_FILE_EXTENSIONS, ["cbtemplate"]);
    assert_eq!(DEFAULT_TEMPLATE_FILE_NAME, "untitled.cbtemplate");

    assert!(native_import_replaces_project_file_path("project.instantiateTemplate"));
    assert!(native_import_replaces_project_file_path("table.importPrism"));
    assert!(native_import_replaces_project_file_path("table.importPzfx"));
    assert!(native_import_replaces_project_file_path("table.importXlsx"));
    assert!(!native_import_replaces_project_file_path("table.importCsv"));

    // Save a `.curvebook` project + `.cbtemplate` file, then verify CSV preserves `file_path`
    // while `instantiateTemplate` and foreign project replacement detach any prior archive path.
    let base_doc = build_populated_routing_app()?.document().clone();
    let (mut rt, wid) = spawn_window_from_session(Session::from_document(base_doc))?;
    let project_path = unique_temp_file("origin.curvebook");
    let template_path = unique_temp_file("origin.cbtemplate");
    let csv_path = unique_temp_file("append.csv");
    std::fs::write(&csv_path, "X,Y\n1,2\n3,4\n")?;

    {
        let win = rt.window_mut(wid)?;
        win.execute_and_journal("project.save", json!({ "path": project_path.to_string_lossy(), "unpacked": false }))?;
        win.app_mut().session_mut().set_file_path(Some(project_path.clone()), true);
        win.execute_and_journal("project.saveTemplate", json!({ "path": template_path.to_string_lossy() }))?;
        assert!(template_path.exists());

        execute_native_import_at_path(win, "table.importCsv", &csv_path)?;
        assert_eq!(win.app().session().file_path(), Some(project_path.as_path()));

        execute_native_import_at_path(win, "project.instantiateTemplate", &template_path)?;
        assert_eq!(win.app().session().file_path(), None);
        assert!(win.app().session().is_dirty());
    }

    let _ = std::fs::remove_file(&project_path);
    let _ = std::fs::remove_file(&template_path);
    let _ = std::fs::remove_file(&csv_path);
    Ok(())
}

#[test]
fn welcome_open_file_dialog_hook_invoked_and_escape_closes_welcome() -> Result<(), Box<dyn std::error::Error>> {
    // Prepare a real saved `.curvebook` archive file so the native open hook returns a valid path.
    let base_doc = build_populated_routing_app()?.document().clone();
    let (mut save_rt, save_wid) = spawn_window_from_session(Session::from_document(base_doc))?;
    let archive_path = unique_temp_file("welcome-open.curvebook");
    save_rt.window_mut(save_wid)?.execute_and_journal("project.save", json!({ "path": archive_path.to_string_lossy(), "unpacked": false }))?;
    drop(save_rt);

    let (mut rt, wid) = spawn_window_from_session(Session::new())?;
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls_cb = Arc::clone(&hook_calls);
    let archive_cb = archive_path.clone();
    rt.set_open_file_dialog_hook(move |_win_id| {
        hook_calls_cb.fetch_add(1, Ordering::SeqCst);
        Ok(Some(archive_cb.clone()))
    });

    // Empty session shows Welcome Launcher; clicking `welcome.action.open_existing` -> `welcome.btn.open_file`
    // emits `project.open` and routes through `desktop_host::open_file_dialog_hook`, opening the project cleanly.
    let _ = metrics;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, wid, None);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let step = |app_host: &mut CurveBookNativeApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| app_host.step_ui(ui));
        out.textures_delta.clear();
    };
    let click_wid = |app_host: &mut CurveBookNativeApp, widget_id: &str| -> Result<(), Box<dyn std::error::Error>> {
        step(app_host, Vec::new());
        step(app_host, Vec::new());
        let pos = app_host.inspect_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing `{widget_id}`"))?.clipped_rect.center();
        step(
            app_host,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
        );
        step(app_host, Vec::new());
        Ok(())
    };

    click_wid(&mut native, "welcome.action.open_existing")?;
    if native.inspect_meta().find_by_widget_id("welcome.btn.open_file").is_some() {
        click_wid(&mut native, "welcome.btn.open_file")?;
    }
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
    assert!(native.runtime().window(wid)?.app().document().total_sheet_count() >= 1);

    // Re-open Welcome overlay via `nav.welcome.open` and verify Escape dismisses it cleanly.
    native.runtime_mut().window_mut(wid)?.app_mut().nav.welcome.open = true;
    step(&mut native, Vec::new());
    step(
        &mut native,
        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
    );
    step(&mut native, Vec::new());
    assert!(!native.runtime().window(wid)?.app().nav.welcome.open);

    // Also verify `File > Open...` menu command invokes the same open file dialog hook.
    click_wid(&mut native, "menu.file")?;
    click_wid(&mut native, "menu.project.open")?;
    assert_eq!(hook_calls.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_file(&archive_path);
    Ok(())
}

#[test]
fn r10_23_drawn_file_menu_routes_open_save_save_as_and_exports_through_adapter() -> Result<(), Box<dyn std::error::Error>> {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime, NativeFileDialogKind, StubNativeFileDialogAdapter};
    use curvebook_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, ProjectDocument, TableArchetype};
    use curvebook_format::{ZipWriter, open_project_file, save_project_file};
    use curvebook_ui_egui::interaction::ViewportMetrics;

    fn click_drawn_file_submenu_item(
        native: &mut CurveBookNativeApp,
        ctx: &egui::Context,
        submenu_id: Option<&str>,
        item_id: &str,
    ) -> Result<(), String> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
        let step = |app_host: &mut CurveBookNativeApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| app_host.step_ui(ui));
            out.textures_delta.clear();
        };
        step(native, Vec::new());
        step(native, Vec::new());
        let file_pos = native.inspect_meta().find_by_widget_id("menu.file").ok_or("missing menu.file button")?.clipped_rect.center();
        step(
            native,
            vec![
                egui::Event::PointerMoved(file_pos),
                egui::Event::PointerButton { pos: file_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: file_pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
        );
        step(native, Vec::new());
        if let Some(sub_id) = submenu_id {
            let sub_pos = native
                .inspect_meta()
                .find_by_widget_id(sub_id)
                .ok_or_else(|| format!("missing `{sub_id}` inside popup.menu.file"))?
                .clipped_rect
                .center();
            step(native, vec![egui::Event::PointerMoved(sub_pos)]);
            step(native, Vec::new());
        }
        let item_pos = native
            .inspect_meta()
            .find_by_widget_id(item_id)
            .ok_or_else(|| format!("missing `{item_id}` inside popup.menu.file"))?
            .clipped_rect
            .center();
        step(
            native,
            vec![
                egui::Event::PointerMoved(item_pos),
                egui::Event::PointerButton { pos: item_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: item_pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
        );
        step(native, Vec::new());
        Ok(())
    }

    fn click_drawn_file_menu_item(native: &mut CurveBookNativeApp, ctx: &egui::Context, item_id: &str) -> Result<(), String> {
        click_drawn_file_submenu_item(native, ctx, None, item_id)
    }

    let tmp_dir = std::env::temp_dir().join(format!("curvebook-r1023-file-dlg-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)?;

    // 1. Prepare real .curvebook, .pzfx, .prism, and .csv files on disk.
    let cb_path = tmp_dir.join("sample.curvebook");
    {
        let mut doc = ProjectDocument::default();
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
        let mut ds = DataSheet::new("ds-cb", "Native Archive Sheet", TableArchetype::Column(col))?;
        ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(42.5))?;
        doc.add_data_sheet(ds)?;
        save_project_file(&cb_path, &doc)?;
    }
    let pzfx_path = tmp_dir.join("cohort.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile SchemaVersion="2">
  <Title>PZFX Dish</Title>
  <TableSequence>
    <Table ID="t-pzfx" TableType="OneWay">
      <Title>PZFX Imported Columns</Title>
      <YColumn><Title>Ctrl</Title><Subcolumn><d>11.0</d><d>13.0</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#,
    )?;
    let zip_archive_path = tmp_dir.join("kinetics.prism");
    {
        let mut writer = ZipWriter::new();
        writer.add_file(
            "manifest.json",
            serde_json::to_vec(&json!({
                "title": "Archive Container Assay",
                "sheets": [{
                    "id": "sheet-kin",
                    "title": "Archive Zip Kinetics",
                    "kind": "XY",
                    "subcolumn_mode": "Single",
                    "csv_path": "data/kin.csv"
                }]
            }))?,
        )?;
        writer.add_file("data/kin.csv", b"Time,Resp\n0,2.5\n5,8.5\n".to_vec())?;
        fs::write(&zip_archive_path, writer.finish()?)?;
    }
    let csv_path = tmp_dir.join("extra.csv");
    fs::write(&csv_path, "GroupA,GroupB\n10.25,20.75\n")?;

    let save_path = tmp_dir.join("saved.curvebook");
    let save_as_path = tmp_dir.join("saved-as.curvebook");
    let svg_path = tmp_dir.join("out.svg");
    let pdf_path = tmp_dir.join("out.pdf");
    let png_path = tmp_dir.join("out.png");

    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    adapter.push_response("project.open", Some(cb_path.clone()));
    adapter.push_response("project.open", Some(pzfx_path.clone()));
    adapter.push_response("project.open", Some(zip_archive_path.clone()));
    adapter.push_response("project.open", Some(csv_path.clone()));
    adapter.push_response("project.save", Some(save_path.clone()));
    adapter.push_response("file.save_as", Some(save_as_path.clone()));
    adapter.push_response("export.svg", Some(svg_path.clone()));
    adapter.push_response("export.pdf", Some(pdf_path.clone()));
    adapter.push_response("export.png", Some(png_path.clone()));

    let metrics = ViewportMetrics::new(1280.0, 820.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        ui_backend: curvebook_ui_egui::egui_shell::UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: metrics,
        clipboard_adapter: Some(Arc::new(StubPlatformClipboardAdapter::new())),
        ..DesktopHostConfig::default()
    });
    rt.set_file_dialog_adapter(adapter.clone());
    let wid = rt.open_window(None, metrics)?;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, wid, None);

    // 2. Click File > Open... for .curvebook, .pzfx, .prism, and .csv.
    for (expected_title, ext) in
        [("Native Archive Sheet", ".curvebook"), ("PZFX Imported Columns", ".pzfx"), ("Archive Zip Kinetics", ".prism"), ("extra", ".csv")]
    {
        click_drawn_file_menu_item(&mut native, &ctx, "menu.project.open")?;
        let cur_wid = native.runtime().focused_window_id().unwrap_or(wid);
        let doc = native.runtime().window(cur_wid)?.app().document();
        assert!(
            doc.data_sheets.iter().any(|ds| ds.title == expected_title),
            "expected `{expected_title}` after File > Open ({ext}), got {:?}",
            doc.data_sheets.iter().map(|ds| &ds.title).collect::<Vec<_>>()
        );
    }

    // 3. Create a GraphSheet so Export SVG/PDF/PNG are enabled, then test Save, direct Save, Save As, Exports.
    let active_wid = native.runtime().focused_window_id().unwrap_or(wid);
    {
        let win = native.runtime_mut().window_mut(active_wid)?;
        let mut doc = win.app().document().clone();
        let src_id = doc.data_sheets.first().ok_or("missing data sheet")?.id.clone();
        let ds_ref = doc.data_sheet(&src_id).ok_or("missing source")?.clone();
        let mut graph = GraphSheet::new_uninitialized_for_data("g-export", &ds_ref)?;
        graph.initialized = true;
        graph.title = String::from("Export Plot");
        graph.spec.kind = GraphKind::Column;
        doc.add_graph_sheet(graph)?;
        let _ = doc.select_in_navigator(&"g-export".into());
        win.app_mut().session_mut().set_file_path(None, true);
        let bytes = curvebook_format::save_project_bytes(&doc)?;
        let tmp_seed = tmp_dir.join("seed-export.curvebook");
        fs::write(&tmp_seed, &bytes)?;
        let _ = win.execute_and_journal("project.open", json!({ "path": tmp_seed.to_string_lossy() }))?;
        win.app_mut().session_mut().set_file_path(None, false);
        win.app_mut().sync_views();
    }

    let reqs_before_save = adapter.requests().len();
    click_drawn_file_menu_item(&mut native, &ctx, "menu.project.save")?;
    assert!(save_path.is_file(), "File > Save must write real archive to disk");
    assert_eq!(native.runtime().window(active_wid)?.app().session().file_path(), Some(Path::new(&save_path)));
    assert_eq!(adapter.requests().len(), reqs_before_save + 1);

    // Clicking File > Save again with existing file_path writes directly with no extra dialog.
    click_drawn_file_menu_item(&mut native, &ctx, "menu.project.save")?;
    assert_eq!(adapter.requests().len(), reqs_before_save + 1);

    // Clicking File > Save As... always opens SaveFile dialog and switches file_path.
    click_drawn_file_menu_item(&mut native, &ctx, "menu.file.save_as")?;
    assert!(save_as_path.is_file(), "File > Save As... must write archive to picked path");
    assert_eq!(native.runtime().window(active_wid)?.app().session().file_path(), Some(Path::new(&save_as_path)));
    assert!(!open_project_file(&save_as_path)?.data_sheets.is_empty());

    // Clicking File > Export > SVG / PDF / PNG opens SaveFile dialogs and writes valid artifacts.
    click_drawn_file_submenu_item(&mut native, &ctx, Some("menu.file.export"), "menu.export.svg")?;
    assert!(fs::read_to_string(&svg_path)?.contains("<svg"));

    click_drawn_file_submenu_item(&mut native, &ctx, Some("menu.file.export"), "menu.export.pdf")?;
    assert!(fs::read(&pdf_path)?.starts_with(b"%PDF"));

    click_drawn_file_submenu_item(&mut native, &ctx, Some("menu.file.export"), "menu.export.png")?;
    assert!(fs::read(&png_path)?.starts_with(&[0x89, b'P', b'N', b'G']));

    let recorded = adapter.requests();
    assert_eq!(recorded.len(), 9);
    assert!(recorded[..4].iter().all(|r| r.kind == NativeFileDialogKind::OpenFile && r.extensions.iter().any(|e| e == "pzfx")));
    assert_eq!(recorded[4].kind, NativeFileDialogKind::SaveFile);
    assert_eq!(recorded[5].command_id, "file.save_as");
    assert_eq!(recorded[6].extensions, vec!["svg"]);
    assert_eq!(recorded[7].extensions, vec!["pdf"]);
    assert_eq!(recorded[8].extensions, vec!["png"]);

    let _ = fs::remove_dir_all(&tmp_dir);
    Ok(())
}

#[test]
fn r13_7_native_rfd_image_picker_inserts_png_on_graph_and_layout() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook::desktop_host::{NativeFileDialogKind, StubNativeFileDialogAdapter};
    use curvebook_doc::{GraphAnnotation, LayoutItem};
    use curvebook_render::{RasterImage, png::encode_png};

    let png_path = unique_temp_file("r13-7-insert.png");
    let png_bytes = encode_png(&RasterImage { width_px: 16, height_px: 8, dpi: 96, rgba: [40, 120, 220, 255].repeat(16 * 8) })?;
    std::fs::write(&png_path, png_bytes)?;

    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    adapter.push_response("graph.insertImage", Some(png_path.clone()));
    adapter.push_response("layout.insertImage", Some(png_path.clone()));

    let metrics = ViewportMetrics::new(1280.0, 820.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        ui_backend: curvebook_ui_egui::egui_shell::UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: metrics,
        clipboard_adapter: Some(Arc::new(StubPlatformClipboardAdapter::new())),
        ..DesktopHostConfig::default()
    });
    rt.set_file_dialog_adapter(adapter.clone());
    let wid = rt.open_window(None, metrics)?;

    rt.window_mut(wid)?.app_mut().execute_command(
        "sheet.createData",
        json!({ "sheet_id": "data-xy", "title": "Image Host Data", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": false }),
    )?;
    rt.window_mut(wid)?.app_mut().execute_command("graph.create", json!({ "sheet_id": "g-img", "data_sheet_id": "data-xy", "kind": "xy" }))?;
    rt.window_mut(wid)?.app_mut().execute_command("layout.create", json!({ "sheet_id": "lay-img", "title": "Img Layout" }))?;

    rt.window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "g-img" }))?;
    let g_out = rt.dispatch_native_file_command(wid, "graph.insertImage")?.ok_or("expected graph.insertImage native dialog")?;
    assert!(g_out.handled);

    rt.window_mut(wid)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "lay-img" }))?;
    let l_out = rt.dispatch_native_file_command(wid, "layout.insertImage")?.ok_or("expected layout.insertImage native dialog")?;
    assert!(l_out.handled);

    let reqs = adapter.requests();
    assert_eq!(reqs.len(), 2);
    for req in &reqs {
        assert_eq!(req.kind, NativeFileDialogKind::OpenFile);
        assert_eq!(req.extensions, vec!["png", "jpg", "jpeg", "tif", "tiff"]);
    }

    let doc = rt.window(wid)?.app().document();
    let gs = doc.graph_sheet(&SheetId::new("g-img")).ok_or("missing g-img")?;
    assert!(gs.spec.annotations.iter().any(|a| matches!(a, GraphAnnotation::EmbeddedImage { .. })), "expected EmbeddedImage on GraphSheet");
    let ls = doc.layout_sheet(&SheetId::new("lay-img")).ok_or("missing lay-img")?;
    assert!(ls.spec.items.iter().any(|it| matches!(it, LayoutItem::EmbeddedImage { .. })), "expected EmbeddedImage on LayoutSheet");

    let _ = std::fs::remove_file(&png_path);
    Ok(())
}
