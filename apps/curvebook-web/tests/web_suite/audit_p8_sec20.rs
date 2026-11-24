use std::collections::BTreeMap;

use curvebook_engine::ExternalPathPolicy;
use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_ZIP_ENTRIES};
use curvebook_text::TextDirection;
use curvebook_ui_egui::dialogs_canvas::ExportFigureFormat;
use curvebook_ui_egui::serde_json::{self, Value, json};
use curvebook_ui_egui::{ActiveDialog, KeyModifiers, PointerButton, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder};
use curvebook_web::text_bridge::CanvasMeasureBackend;
use curvebook_web::{
    BrowserCanvasTextMeasurer, MeasuredTextMetrics, MemoryWebStorage, UiTextStyle, WebAdapterError, WebBridgeCommand, WebBridgeEvent,
    WebBridgeSession, WebChromeBridge, WebChromeDispatch, WebStorageBackend, WebStorageManager, WebTextBridge, WebTextMeasurer, WebWorkspaceHost,
};

fn build_populated_web_session(doc_id: &str, tab_id: &str) -> Result<(WebBridgeSession, String, String), Box<dyn std::error::Error>> {
    let mut host = WebWorkspaceHost::new();
    let _ = host.execute_command("sheet.createData", json!({ "kind": "xy", "title": "XY Data" }))?;
    let ds_id = host.app().document().data_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing created data sheet")?;
    for (row, x, y) in [(0usize, 1.0_f64, 2.0_f64), (1, 2.0, 4.0), (2, 3.0, 6.0)] {
        let _ = host.execute_command("table.setCell", json!({ "sheet_id": ds_id, "row": row, "col": 0, "subcol": 0, "value": x }))?;
        let _ = host.execute_command("table.setCell", json!({ "sheet_id": ds_id, "row": row, "col": 1, "subcol": 0, "value": y }))?;
    }
    let _graph_out = host.execute_command("graph.create", json!({ "data_sheet_id": ds_id, "title": "Main Graph" }))?;
    let graph_id = host.app().document().graph_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing created graph sheet_id")?;
    let mut session = WebBridgeSession::with_ids(doc_id, tab_id);
    session.host = host;
    let _ = session.handle_event(WebBridgeEvent::AcquireLock { doc_id: doc_id.to_owned(), tab_id: tab_id.to_owned(), now_ms: 10_000 })?;
    Ok((session, ds_id, graph_id))
}

#[test]
fn fa_sec20_04_missing_dialogs_export_options_palette_focus_and_modal_keys() -> Result<(), Box<dyn std::error::Error>> {
    // 1. ExportFigure preserves CMYK-Proof & transparent_bg and executes export once (no duplicate un-parameterized export)
    let (mut session, ds_id, graph_id) = build_populated_web_session("sec20-04-doc", "tab-sec20-04")?;
    let sheet_id = session
        .host
        .app()
        .document()
        .graph_sheets
        .iter()
        .find(|s| s.id.as_str() == graph_id)
        .map(|s| s.id.clone())
        .ok_or("missing created graph SheetId")?;
    {
        let app = session.host.app_mut();
        app.canvas_dialogs.open_export_figure(&sheet_id);
        app.canvas_dialogs.export_figure.format = ExportFigureFormat::Pdf;
        app.canvas_dialogs.export_figure.color_space = String::from("CMYK-Proof");
        app.canvas_dialogs.export_figure.transparent_bg = true;
    }
    let cmds = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("modal.ok"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    let render_opts = cmds.iter().find_map(|c| match c {
        WebBridgeCommand::RenderRgbaFrame { command_result: Some(res), .. } => res.get("export_options").cloned(),
        _ => None,
    });
    let opts = render_opts.ok_or("expected export_options on RenderRgbaFrame command_result")?;
    assert_eq!(opts.get("color_space").and_then(Value::as_str), Some("CMYK-Proof"));
    assert_eq!(opts.get("transparent_bg").and_then(Value::as_bool), Some(true));
    let pdf_downloads: Vec<&[u8]> = cmds
        .iter()
        .filter_map(|c| match c {
            WebBridgeCommand::SaveOrDownloadFile { bytes, .. } => Some(bytes.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(pdf_downloads.len(), 1);
    assert!(pdf_downloads[0].starts_with(b"%PDF"));

    // 2. CommandPalette confirm clears focus_widget_id & query buffer
    {
        let app = session.host.app_mut();
        app.canvas_dialogs.open_command_palette();
        app.canvas_dialogs.command_palette.query = String::from("New Data");
        app.focus_widget_id = Some(String::from("palette.query"));
    }
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Enter"), modifiers: KeyModifiers::default() } })?;
    assert_eq!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(session.host.app().focus_widget_id.is_none());

    // 3. Ported 6 dialog commit arms: Checkpoints, SimulateDistributions, RandomSampleRows, RandomizeGroups, ConstantHookPicker, ImportReshape
    let before_cp = session.host.app().document().checkpoints.len();
    {
        let doc = session.host.app().document().clone();
        let app = session.host.app_mut();
        app.canvas_dialogs.open_checkpoints(&doc);
        app.canvas_dialogs.checkpoints.name_draft = String::from("Web CP 1");
        app.canvas_dialogs.checkpoints.note_draft = String::from("Created in modal");
    }
    let _ = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("modal.ok"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(session.host.app().document().checkpoints.len(), before_cp + 1);
    assert_eq!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::Checkpoints);
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    // Checkpoints modal accepts non-Confirm keyboard typing via checkpoints.name focus
    {
        let doc = session.host.app().document().clone();
        let app = session.host.app_mut();
        app.canvas_dialogs.open_checkpoints(&doc);
        app.canvas_dialogs.checkpoints.name_draft.clear();
        app.focus_widget_id = Some(String::from("checkpoints.name"));
    }
    for key in ["A", "B", "Enter"] {
        let _ = session
            .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: key.to_owned(), modifiers: KeyModifiers::default() } })?;
    }
    assert_eq!(session.host.app().document().checkpoints.last().map(|c| c.name.as_str()), Some("AB"));
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    // FormatAxes accepts keyboard typing on axes.title
    let _ = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("toolbar.axes"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    session.host.app_mut().focus_widget_id = Some(String::from("axes.title"));
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Z"), modifiers: KeyModifiers::default() } })?;
    assert!(
        session.host.app().interaction.text_field_buffers.get("axes.title").is_some_and(|b| b.text.ends_with('Z')),
        "FormatAxes keyboard typing must update axes.title buffer"
    );
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    // SimulateDistributions, RandomSampleRows, RandomizeGroups, ConstantHookPicker, ImportReshape
    let before_sheets = session.host.app().document().data_sheets.len();
    session.host.app_mut().canvas_dialogs.open_simulate_distributions();
    let _ = curvebook_web::chrome_bridge::commit_active_modal_dialog(session.host.app_mut())?;
    assert!(session.host.app().document().data_sheets.len() > before_sheets);

    let ds_sheet =
        session.host.app().document().data_sheets.iter().find(|s| s.id.as_str() == ds_id).map(|s| s.id.clone()).ok_or("missing ds_id SheetId")?;
    {
        let doc = session.host.app().document().clone();
        assert!(session.host.app_mut().canvas_dialogs.open_random_sample_rows(&doc, Some(&ds_sheet)));
    }
    let _ = curvebook_web::chrome_bridge::commit_active_modal_dialog(session.host.app_mut())?;

    {
        let doc = session.host.app().document().clone();
        session.host.app_mut().canvas_dialogs.open_randomize_groups(&doc);
    }
    let _ = curvebook_web::chrome_bridge::commit_active_modal_dialog(session.host.app_mut())?;

    // ConstantHookPicker on GraphSheet textbox -> selects constant hook & commits annotation
    if let Some(info_id) = session.host.app().document().info_sheets.first().map(|s| s.id.as_str().to_owned()) {
        let _ = session.host.execute_command("info.setConstant", json!({ "sheet_id": info_id, "name": "ConcMax", "value": 42.0 }))?;
    }
    {
        let doc = session.host.app().document().clone();
        let app = session.host.app_mut();
        app.canvas_dialogs.open_hook_picker(&doc, &sheet_id, Some(String::from("Callout")));
        assert!(!app.canvas_dialogs.hook_picker.options.is_empty());
        app.canvas_dialogs.hook_picker.selected_index = Some(0);
    }
    let _ = curvebook_web::chrome_bridge::commit_active_modal_dialog(session.host.app_mut())?;

    // ImportReshape commit
    {
        let app = session.host.app_mut();
        app.grid.import_paste.source_origin = String::from("external_csv");
        app.grid.import_paste.raw_source_text = String::from("X,Y\n10,20\n30,40\n");
        app.canvas_dialogs.open_import_reshape();
    }
    let before_import_sheets = session.host.app().document().data_sheets.len();
    let _ = curvebook_web::chrome_bridge::commit_active_modal_dialog(session.host.app_mut())?;
    assert_eq!(session.host.app().document().data_sheets.len(), before_import_sheets + 1);

    // Wizard StepBack/StepNext + Analyze SelectDialogItem + FocusModalInput
    session.host.app_mut().canvas_dialogs.open_power_wizard();
    let _ = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("power.next"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(session.host.app().canvas_dialogs.power_wizard.stage, curvebook_ui_egui::PowerWizardStage::EffectSize);
    let _ = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("power.back"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(session.host.app().canvas_dialogs.power_wizard.stage, curvebook_ui_egui::PowerWizardStage::ExperimentDesign);
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    // SelectDialogItem + FocusModalInput on CommandPalette
    session.host.app_mut().canvas_dialogs.open_command_palette();
    session.host.app_mut().focus_widget_id = None;
    let _ = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("palette.query"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(session.host.app().focus_widget_id.as_deref(), Some("palette.query"));
    let _ = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("palette.item.1"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(session.host.app().canvas_dialogs.command_palette.selected_index, 1);
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    Ok(())
}

#[test]
fn fa_sec20_05_dropped_directory_preserves_nested_sheets_and_index_guards() -> Result<(), Box<dyn std::error::Error>> {
    let mut seed_host = WebWorkspaceHost::new();
    let _ = seed_host.execute_command("sheet.createData", json!({ "kind": "column", "title": "Dropped Table" }))?;
    let mut doc = seed_host.app().document().clone();
    doc.title = String::from("Dropped Snapshot");
    let mut folder = VirtualWorkspaceFolder::new("drop.curvebook.d");
    let _ = folder.save_unpacked_snapshot(&doc)?;
    let dropped_files: Vec<(String, Vec<u8>)> = folder.files.iter().map(|(rel, bytes)| (format!("drop.curvebook.d/{rel}"), bytes.clone())).collect();

    let mut session = WebBridgeSession::with_ids("empty", "tab-drop");
    let _ = session.handle_event(WebBridgeEvent::ImportDroppedFiles { files: dropped_files })?;
    let files = session.host.app().virtual_workspace.list_files();
    assert!(files.iter().any(|p| p.starts_with("sheets/")), "nested `sheets/` prefix must not be stripped: {files:?}");

    let html = super::common::INDEX_HTML;
    let (_, after_start) = html.split_once("async function collectDroppedEntry").ok_or("missing collectDroppedEntry")?;
    let slice: String = after_start.chars().take(900).collect();
    assert!(slice.contains("try {") && slice.contains("} catch (err) {") && slice.contains("reject(err)"));
    Ok(())
}

#[test]
fn fa_sec20_06_full_a11y_tree_schema_matches_dom_mirror() -> Result<(), Box<dyn std::error::Error>> {
    let mut host = WebWorkspaceHost::new();
    let _ = curvebook_ui_egui::dispatch_ui_command(host.app_mut(), "ui.locale.set", &json!({ "locale": "ar-SA" }))?;
    let mut bridge = WebChromeBridge::new();
    bridge.open_menu_key = Some(String::from("menu.file"));
    let doc = host.app().document().clone();
    host.app_mut().canvas_dialogs.open_checkpoints(&doc);
    host.app_mut().focus_widget_id = Some(String::from("checkpoints.name"));

    let layout = bridge.build_layout(host.app(), ViewportMetrics::new(1024.0, 640.0, 1.0));
    let tree = layout.snapshot.a11y_tree.ok_or("missing a11y_tree")?;
    assert_eq!(tree.get("dir").and_then(Value::as_str), Some("rtl"));
    assert_eq!(tree.get("locale").and_then(Value::as_str), Some("ar"));
    assert_eq!(tree.get("focused_dom_id").and_then(Value::as_str), Some("cb-chrome-checkpoints-name"));
    assert!(tree.get("live_announcement").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    let nodes = tree.get("nodes").and_then(Value::as_array).ok_or("missing a11y nodes array")?;
    let cp_node = nodes
        .iter()
        .find(|n| n.get("dom_id").and_then(Value::as_str) == Some("cb-chrome-checkpoints-name"))
        .ok_or("missing checkpoints.name a11y node")?;
    assert_eq!(cp_node.get("tag_name").and_then(Value::as_str), Some("input"));
    assert_eq!(cp_node.get("parent_dom_id").and_then(Value::as_str), Some("cb-chrome-modal-dialog"));
    assert!(cp_node.get("value_text").and_then(Value::as_str).is_some());
    assert!(cp_node.get("attributes").and_then(Value::as_object).is_some());
    Ok(())
}

#[test]
fn fa_sec20_07_browser_shortcut_prevention_and_unhandled_printable_keydown_routes_text_input() -> Result<(), Box<dyn std::error::Error>> {
    let html = super::common::INDEX_HTML;
    let (_, after_fn) = html.split_once("function shouldPreventBrowserShortcut").ok_or("missing shouldPreventBrowserShortcut")?;
    let fn_slice: String = after_fn.chars().take(900).collect();
    assert!(fn_slice.contains("\"F6\""));
    assert!(fn_slice.contains("[\"a\", \"f\", \"g\"]"));
    assert!(fn_slice.contains("\"enter\""));
    assert!(html.contains("TextInput"));

    let (mut session, ds_id, _graph_id) = build_populated_web_session("sec20-07-doc", "tab-sec20-07")?;
    let _ = session.host.execute_command("sheet.select", json!({ "sheet_id": ds_id }))?;
    // 1. Frozen sheet guard catches unhandled printable KeyDown -> TextInput fallback
    let _ = session.host.execute_command("sheet.freeze", json!({ "sheet_id": ds_id }))?;
    let frozen_cmds = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("7"), modifiers: KeyModifiers::default() } })?;
    assert!(
        frozen_cmds.iter().any(|c| matches!(c, WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. } if act == "grid_core.frozen_read_only")),
        "unhandled printable KeyDown must route TextInput into frozen-sheet guard: {frozen_cmds:?}"
    );
    let _ = session.host.execute_command("sheet.unfreeze", json!({ "sheet_id": ds_id }))?;

    // 2. Focused modal/textbox input receives printable KeyDown -> TextInput fallback
    let doc = session.host.app().document().clone();
    session.host.app_mut().canvas_dialogs.open_checkpoints(&doc);
    session.host.app_mut().focus_widget_id = Some(String::from("checkpoints.name"));
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("7"), modifiers: KeyModifiers::default() } })?;
    assert!(session.host.app().canvas_dialogs.checkpoints.name_draft.ends_with('7'));
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    // Contended read-only rejects direct typing via KeyDown -> ReadOnlyContended
    let _ = session.handle_event(WebBridgeEvent::ObserveExternalLock {
        doc_id: String::from("sec20-07-doc"),
        holder_tab_id: String::from("other-tab"),
        now_ms: 20_000,
    })?;
    let err = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("9"), modifiers: KeyModifiers::default() } })
        .expect_err("read-only tab must block direct cell typing");
    assert!(matches!(err, WebAdapterError::ReadOnlyContended { .. }));
    Ok(())
}

#[test]
fn fa_sec20_08_atomic_shortcuts_now_ms_autosave_and_index_local_storage_guards() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Atomic import_overrides_json: mixed valid + invalid combo leaves shortcuts unchanged
    let mut session = WebBridgeSession::with_ids("sec20-08-doc", "tab-sec20-08");
    let invalid_prefs = json!({
        "schema_version": 1,
        "shortcuts": {
            "project.save": "Cmd+S",
            "file.new": "Invalid++Combo!!"
        }
    });
    let cmds = session.handle_event(WebBridgeEvent::HydrateLocalStorageAt {
        entries: vec![(curvebook_web::PREFERENCES_STORAGE_KEY.to_owned(), invalid_prefs.to_string())],
        now_ms: 50_000,
    })?;
    assert!(cmds.iter().any(|c| matches!(c, WebBridgeCommand::NotifyStatus { code, .. } if code == "invalid_shortcut_overrides")));

    // 2. Hydrating autosave records client now_ms recency instead of 0 (so newer hydrated docs are not evicted before older docs)
    let mut seed_doc_host = WebWorkspaceHost::new();
    let _ = seed_doc_host.execute_command("sheet.createData", json!({ "kind": "column", "title": "Autosave Data" }))?;
    let mut doc = seed_doc_host.app().document().clone();
    doc.title = String::from("Hydrated Newer Doc");
    let valid_doc_json = serde_json::to_string(&doc)?;
    let mut mgr = WebStorageManager::new(MemoryWebStorage::new());
    let _ = mgr.save_autosave_snapshot("doc-hydrated", &valid_doc_json, 1_000)?;
    let stored: Vec<(String, String)> = mgr.backend().keys().into_iter().filter_map(|k| mgr.backend().get_item(&k).map(|v| (k, v))).collect();
    let mut lru_session = WebBridgeSession::with_ids("doc-hydrated", "tab-lru");
    let _ = lru_session.handle_event(WebBridgeEvent::HydrateLocalStorageAt { entries: stored, now_ms: 900_000 })?;
    assert_eq!(lru_session.host.app().document().title, "Hydrated Newer Doc");
    // Saving a 3rd ~2 MB payload triggers 5 MiB LRU eviction; doc-older (500_000) must be evicted before doc-hydrated (900_000).
    let padded_payload = "Z".repeat(2_000_000);
    let mut evict_mgr = WebStorageManager::new(MemoryWebStorage::new());
    let _ = evict_mgr.save_autosave_snapshot("doc-older", &padded_payload, 500_000)?;
    let _ = evict_mgr.save_autosave_snapshot("doc-hydrated", &padded_payload, 1_000)?;
    let _ = evict_mgr.load_autosave_snapshot("doc-hydrated", 900_000)?;
    let _ = evict_mgr.save_autosave_snapshot("doc-newest", &padded_payload, 950_000)?;
    let remaining = evict_mgr.list_autosave_doc_ids();
    assert!(remaining.contains(&String::from("doc-hydrated")), "hydrated doc touched at now_ms=900_000 must outlive 500_000: {remaining:?}");
    assert!(!remaining.contains(&String::from("doc-older")), "older doc at 500_000 must be evicted first: {remaining:?}");

    // 3. index.html checks: tryGetLocalStorage, now_ms: Date.now(), clearInterval(fallbackLockTimerId), fail-closed observe_external_lock
    let html = super::common::INDEX_HTML;
    assert!(html.contains("tryGetLocalStorage"));
    assert!(html.contains("tryReadLocalStorageItem"));
    assert!(!html.contains("if (!window.localStorage)"), "bare window.localStorage check outside try/catch must not exist");
    assert!(!html.contains("!holdsCurrentWebLock || !window.localStorage"), "heartbeat must use tryGetLocalStorage()");
    assert!(html.contains("emitEvent({ kind: \"hydrate_local_storage\", entries: stored, now_ms: Date.now() })"));
    let (_, after_lock) = html.split_once("function tryAcquireFallbackStorageLock").ok_or("missing tryAcquireFallbackStorageLock")?;
    let lock_slice: String = after_lock.chars().take(4000).collect();
    assert!(lock_slice.contains("clearInterval(fallbackLockTimerId)"));
    assert!(lock_slice.contains("holder_tab_id: \"storage-unavailable\""));
    Ok(())
}

#[test]
fn fa_sec20_09_axes_disabled_clamped_wheel_context_items_and_chrome_text_ops() -> Result<(), Box<dyn std::error::Error>> {
    // Empty workspace has no graph sheet -> toolbar.axes must be disabled
    let empty_app = curvebook_ui_egui::UiApp::new();
    let mut bridge = WebChromeBridge::new();
    let empty_layout = bridge.build_layout(&empty_app, ViewportMetrics::new(800.0, 600.0, 1.0));
    let axes_btn = empty_layout.find_node("toolbar.axes").ok_or("missing toolbar.axes")?;
    assert!(!axes_btn.enabled, "toolbar.axes must be disabled without a GraphSheet");

    // Dropdown clamps to small vh and scrolls on wheel
    let small_vp = ViewportMetrics::new(800.0, 240.0, 1.0);
    bridge.open_menu_key = Some(String::from("menu.file"));
    let pre_scroll = bridge.build_layout(&empty_app, small_vp);
    let popup = pre_scroll.find_node("popup.menu.file").ok_or("missing popup.menu.file")?;
    assert!(popup.rect_px.height <= 240 - 32);
    let wheel_disp = bridge.route_wheel(&mut curvebook_ui_egui::UiApp::new(), &pre_scroll, [40.0, 60.0], [0.0, 28.0], KeyModifiers::default());
    assert!(matches!(wheel_disp, WebChromeDispatch::HandledRedraw { ref action, .. } if action == "popup.wheel_scroll"));

    // Secondary right-click opens context menu with clickable items & consumes Secondary PointerUp
    let (mut session, _ds_id, _graph_id) = build_populated_web_session("sec20-09-doc", "tab-sec20-09")?;
    let down_cmds = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 200.0, y: 200.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?;
    assert!(!down_cmds.is_empty());
    let up_cmds = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerUp { x: 200.0, y: 200.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?;
    let consumed =
        up_cmds.iter().any(|c| matches!(c, WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. } if act == "context_menu.pointer_up_consumed"));
    assert!(consumed, "Secondary PointerUp while context menu open must be consumed");

    // Projected frame text_ops include menu/toolbar/context menu labels (not just status.bar)
    let ctx_layout = session.chrome.build_layout(session.host.app(), ViewportMetrics::new(1024.0, 640.0, 1.0));
    assert!(ctx_layout.nodes.iter().any(|n| n.id.starts_with("context_menu.")));
    let frame = WebTextBridge::try_compose_chrome_frame(1024, 640, 96, [24, 28, 36, 255], &ctx_layout)?;
    assert!(frame.text_ops.iter().any(|op| op.node_id.as_deref() == Some("menu.file")));
    assert!(frame.text_ops.iter().any(|op| op.node_id.as_deref().is_some_and(|id| id.starts_with("context_menu."))));
    Ok(())
}

#[derive(Debug, Default)]
struct KerningMockBackend;

impl CanvasMeasureBackend for KerningMockBackend {
    fn measure_uncached(&self, _style: &UiTextStyle, _font_css: &str, text: &str, _dpi: u16) -> MeasuredTextMetrics {
        let width_px = match text {
            "A" => 10.0,
            "AV" => 16.0,
            "AW" => 19.0,
            "AVI" => 20.0,
            "V" => 10.0,
            "W" => 12.0,
            "I" => 4.0,
            _ => {
                #[allow(clippy::cast_precision_loss)]
                {
                    (text.chars().count() as f64) * 7.5
                }
            }
        };
        MeasuredTextMetrics { width_px, ascent_px: 10.0, descent_px: 3.0, line_height_px: 14.0 }
    }
}

#[test]
fn fa_sec20_10_shape_line_measures_cumulative_kerning_and_rtl() {
    let measurer = BrowserCanvasTextMeasurer::with_backend(KerningMockBackend, 64);
    let style = UiTextStyle::ui_sans(13.0);
    let shaped_av = measurer.shape_line(&style, "AVI", 96);
    assert_eq!(shaped_av.glyphs.len(), 3);
    assert!((shaped_av.glyphs[0].advance_px - 10.0).abs() < 1e-6);
    assert!((shaped_av.glyphs[1].advance_px - 6.0).abs() < 1e-6, "kerning pair AV must contract V advance to 6.0");
    assert!((shaped_av.glyphs[2].advance_px - 4.0).abs() < 1e-6);

    let shaped_rtl = measurer.shape_line(&style, "مرحبا 42", 96);
    assert_eq!(shaped_rtl.direction, TextDirection::Rtl);
    assert!(shaped_rtl.glyphs.windows(2).all(|w| w[1].x_px >= w[0].x_px));
    assert!((shaped_rtl.glyphs.iter().map(|g| g.advance_px).sum::<f64>() - shaped_rtl.metrics.width_px).abs() < 4.0);

    let mono = BrowserCanvasTextMeasurer::new().shape_line(&UiTextStyle::ui_mono(12.0), "1234", 96);
    assert_eq!(mono.glyphs.len(), 4);
}

#[test]
fn fa_x1_02_enforces_caps_structural_limits_and_preserves_security_policies() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Pre-allocation caps on raw JSON, "bytes" arrays, and batch entry counts
    let mut session = WebBridgeSession::with_ids("x1-02-doc", "tab-x1-02");
    let oversized_json = " ".repeat(MAX_ARCHIVE_FILE_BYTES * 2 + 1);
    assert!(session.handle_event_json(&oversized_json).is_err());

    let too_many_entries = json!({
        "kind": "import_directory_entries",
        "root_name": "big.curvebook.d",
        "entries": vec![json!({ "path": "x.txt", "bytes_base64": "" }); MAX_ZIP_ENTRIES + 1],
    });
    assert!(session.handle_event_json(&too_many_entries.to_string()).is_err());

    // 2. Host byte caps and ExternalPathPolicy + gui_only preservation
    let mut host = WebWorkspaceHost::new();
    host.app_mut().set_gui_only(true);
    assert_eq!(*host.app().session().path_policy(), ExternalPathPolicy::RejectExternal);

    let oversized_json_doc = vec![b' '; MAX_ARCHIVE_FILE_BYTES + 1];
    assert!(host.open_project_json_bytes(&oversized_json_doc).is_err());

    let oversized_csv = vec![b'a'; MAX_CSV_FILE_BYTES + 1];
    assert!(host.open_delimited_bytes(&oversized_csv, false, Some("Big"), None).is_err());

    // open_delimited_bytes must preserve RejectExternal & gui_only
    let _ = host.open_delimited_bytes(b"X,Y\n1,2\n3,4\n", false, Some("Preserved"), None)?;
    assert!(host.app().is_gui_only());
    assert_eq!(*host.app().session().path_policy(), ExternalPathPolicy::RejectExternal);

    // open_project_json_bytes must preserve RejectExternal & gui_only
    let doc_json = serde_json::to_vec(host.app().document())?;
    let _ = host.open_project_json_bytes(&doc_json)?;
    assert!(host.app().is_gui_only());
    assert_eq!(*host.app().session().path_policy(), ExternalPathPolicy::RejectExternal);

    // 3. index.html pre-read size & batch guards before arrayBuffer()
    let html = super::common::INDEX_HTML;
    assert!(html.contains("MAX_ARCHIVE_BYTES"));
    assert!(html.contains("MAX_CSV_BYTES"));
    assert!(html.contains("assertFileWithinImportBudget"));
    let _unused: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    Ok(())
}
