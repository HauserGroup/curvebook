use std::collections::BTreeMap;

use lumencraft_format::{CURRENT_PREFERENCES_VERSION, PreferencesFile};
use lumencraft_ui_egui::serde_json::json;
use lumencraft_ui_egui::{CanvasPreviewFilter, KeyModifiers, UiInputEvent};
use lumencraft_web::{
    AUTOSAVE_KEY_PREFIX, DirectorySyncPlan, FileSystemAccessBridge, PREFERENCES_STORAGE_KEY, QUOTA_BUDGET_BYTES, TabAccessMode, TouchPoint,
    WebAdapterError, WebBridgeCommand, WebBridgeEvent, WebBridgeSession, WebLockCoordinator, WebStorageBackend, WebStorageManager, WebWorkspaceHost,
};

fn build_populated_xy_session(doc_id: &str, tab_id: &str) -> Result<(WebBridgeSession, String, String), String> {
    let mut session = WebBridgeSession::with_ids(doc_id, tab_id);
    let created = session.host.execute_command(
        "sheet.createData",
        json!({
            "kind": "xy",
            "title": "Dose Response Alpha",
            "auto_family": true,
        }),
    )?;
    let data_id = created["payload"]["sheet_id"].as_str().ok_or("missing data sheet_id")?.to_owned();
    let graph_id = created["payload"]["graph_sheet_id"].as_str().ok_or("missing auto graph_sheet_id")?.to_owned();

    session.host.execute_command("table.setXCells", json!({ "sheet_id": data_id, "values": [1.0, 2.0, 4.0, 8.0] }))?;
    for (row, val) in [12.5, 24.0, 49.5, 96.0].into_iter().enumerate() {
        session.host.execute_command("table.setCell", json!({ "sheet_id": data_id, "row": row, "col": 0, "value": val }))?;
    }
    session.host.execute_command("sheet.select", json!({ "sheet_id": graph_id }))?;

    Ok((session, data_id, graph_id))
}

#[test]
fn preferences_local_storage_roundtrip_and_forward_version_refusal() {
    let mut storage = WebStorageManager::memory();
    assert_eq!(storage.quota_budget_bytes(), QUOTA_BUDGET_BYTES);
    assert_eq!(storage.load_preferences().expect("empty load ok"), None);

    let mut prefs = PreferencesFile {
        version: CURRENT_PREFERENCES_VERSION,
        locale: Some(String::from("de-DE")),
        theme: Some(String::from("dark")),
        shortcuts: BTreeMap::from([(String::from("view.zoomIn"), String::from("Ctrl+="))]),
        ..PreferencesFile::default()
    };
    prefs.extra.insert(String::from("grid_rulers"), json!(true));

    let _encoded = storage.save_preferences(&prefs).expect("v1 preferences save ok");
    let loaded = storage.load_preferences().expect("reload ok").expect("stored preferences exist");
    assert_eq!(loaded, prefs);
    assert!(storage.backend().keys().contains(&String::from(PREFERENCES_STORAGE_KEY)));

    // Simulate newer LumenCraft build having written forward schema preferences.
    let forward_payload = r#"{"schema_version":99,"version":99,"locale":"ja-JP","theme":"solar"}"#;
    storage.backend_mut().set_item(PREFERENCES_STORAGE_KEY, forward_payload).expect("write simulated forward version");

    let load_err = storage.load_preferences().expect_err("must reject forward schema version on load");
    assert_eq!(load_err, WebAdapterError::ForwardVersionPreferences { found: 99, supported: CURRENT_PREFERENCES_VERSION });

    let save_err = storage.save_preferences(&prefs).expect_err("must refuse overwriting forward schema");
    assert_eq!(save_err, WebAdapterError::ForwardVersionPreferences { found: 99, supported: CURRENT_PREFERENCES_VERSION });
    assert_eq!(storage.backend().get_item(PREFERENCES_STORAGE_KEY).as_deref(), Some(forward_payload));

    // Also reject saving a struct whose own version exceeds supported CURRENT_PREFERENCES_VERSION.
    storage.backend_mut().remove_item(PREFERENCES_STORAGE_KEY);
    let newer_struct = PreferencesFile { version: CURRENT_PREFERENCES_VERSION + 2, ..PreferencesFile::default() };
    let struct_err = storage.save_preferences(&newer_struct).expect_err("must reject future version struct");
    assert_eq!(
        struct_err,
        WebAdapterError::ForwardVersionPreferences { found: CURRENT_PREFERENCES_VERSION + 2, supported: CURRENT_PREFERENCES_VERSION }
    );
}

#[test]
fn autosave_evicts_lru_and_oldest_within_five_mib_budget_without_dropping_preferences() {
    let mut storage = WebStorageManager::memory();
    let prefs = PreferencesFile { version: CURRENT_PREFERENCES_VERSION, theme: Some(String::from("high_contrast")), ..PreferencesFile::default() };
    let _ = storage.save_preferences(&prefs).expect("save preferences");

    let payload_1_2_mb = "X".repeat(1_200_000);
    for (idx, doc) in ["doc-a", "doc-b", "doc-c", "doc-d"].into_iter().enumerate() {
        let outcome = storage.save_autosave_snapshot(doc, &payload_1_2_mb, 1_000 + idx as u64 * 100).expect("1.2 MB snapshot fits inside 5 MiB");
        assert!(outcome.evicted_keys.is_empty());
    }
    assert!(storage.used_bytes() > 4_800_000 && storage.used_bytes() <= QUOTA_BUDGET_BYTES);

    // Reading doc-a marks it warmer than doc-b (true LRU behavior).
    let loaded_a = storage.load_autosave_snapshot("doc-a", 2_000).expect("load doc-a");
    assert_eq!(loaded_a.as_deref().map(str::len), Some(1_200_000));

    // Adding doc-e (~1.2 MB) pushes total past 5 MiB and must evict doc-b (oldest unused), keeping preferences intact.
    let outcome_e = storage.save_autosave_snapshot("doc-e", &payload_1_2_mb, 2_500).expect("eviction frees budget for doc-e");
    assert_eq!(outcome_e.evicted_keys, vec![format!("{AUTOSAVE_KEY_PREFIX}doc-b")]);
    assert!(storage.used_bytes() <= QUOTA_BUDGET_BYTES);

    assert_eq!(storage.load_autosave_snapshot("doc-b", 2_600).expect("query doc-b"), None);
    assert!(storage.load_autosave_snapshot("doc-a", 2_600).expect("doc-a retained").is_some());
    assert!(storage.load_autosave_snapshot("doc-e", 2_600).expect("doc-e retained").is_some());
    assert_eq!(storage.load_preferences().expect("preferences preserved"), Some(prefs));

    // Single snapshot exceeding 5 MiB fails cleanly without wiping warm entries.
    let oversized = "Z".repeat(QUOTA_BUDGET_BYTES + 128);
    let quota_err = storage.save_autosave_snapshot("huge-doc", &oversized, 3_000).expect_err("oversized must fail");
    assert!(matches!(quota_err, WebAdapterError::QuotaExceeded { .. }));
    assert!(storage.load_autosave_snapshot("doc-a", 3_100).expect("doc-a untouched on QuotaExceeded").is_some());
    assert!(storage.clear_autosave_snapshot("doc-c").expect("clear doc-c"));
}

#[test]
fn lock_coordinator_two_tab_contention_heartbeat_stale_takeover_and_promotion() {
    let mut coord = WebLockCoordinator::new(10_000);

    let acq_a = coord.acquire_document_lock("study-1", "tab-a", 1_000).expect("tab-a acquires lock");
    assert_eq!(acq_a.mode, TabAccessMode::ExclusiveReadWrite);
    assert!(!acq_a.stale_takeover);

    // Heartbeat keeps lease alive past initial TTL window.
    assert!(coord.renew_heartbeat("study-1", "tab-a", 8_000).expect("heartbeat ok"));

    let acq_b = coord.acquire_document_lock("study-1", "tab-b", 12_000).expect("tab-b sees contention");
    assert_eq!(acq_b.mode, TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("tab-a") });
    assert!(!acq_b.stale_takeover);
    assert_eq!(coord.contenders("study-1"), vec![String::from("tab-b")]);

    // Explicit release by Tab A promotes Tab B cleanly.
    let rel_a = coord.release_document_lock("study-1", "tab-a", 12_500).expect("tab-a release");
    assert!(rel_a.was_holder);
    assert_eq!(rel_a.promoted_tab_id.as_deref(), Some("tab-b"));
    assert_eq!(coord.access_mode("study-1", "tab-b", 12_500), TabAccessMode::ExclusiveReadWrite);
    assert_eq!(coord.active_holder("study-1", 12_500).as_deref(), Some("tab-b"));

    // Tab C enters while Tab B is live, then reclaims via stale lease takeover after TTL expires.
    let acq_c1 = coord.acquire_document_lock("study-1", "tab-c", 14_000).expect("tab-c contended");
    assert_eq!(acq_c1.mode, TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("tab-b") });

    let acq_c2 = coord.acquire_document_lock("study-1", "tab-c", 23_000).expect("tab-c stale takeover");
    assert_eq!(acq_c2.mode, TabAccessMode::ExclusiveReadWrite);
    assert!(acq_c2.stale_takeover);
    assert!(!coord.renew_heartbeat("study-1", "tab-b", 23_100).expect("old holder renewal returns false"));
}

fn assert_contended_rejects_mutations(tab_b: &mut WebBridgeSession, data_id: &str) {
    let mutating_events = [
        WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.createData"), params: json!({ "kind": "xy", "title": "Blocked Sheet" }) },
        WebBridgeEvent::DispatchControlLine {
            line: json!({ "id": "r1", "method": "sheet.rename", "params": { "sheet_id": data_id, "title": "Blocked" } }).to_string(),
        },
        WebBridgeEvent::UiInput { event: UiInputEvent::TextInput { text: String::from("999") } },
        WebBridgeEvent::SyncDirectory { root_name: None },
        WebBridgeEvent::SaveAutosave { now_ms: 2_000 },
    ];
    for ev in mutating_events {
        let err = tab_b.handle_event(ev).err();
        assert!(matches!(err, Some(WebAdapterError::ReadOnlyContended { ref holder_tab_id, .. }) if holder_tab_id == "tab-a"));
    }
}

fn assert_contended_allows_viewport_and_exports(tab_b: &mut WebBridgeSession) -> Result<(), String> {
    let zoom_before = tab_b.host.app().canvas_dialogs.pan_zoom.zoom;
    let pan_before = tab_b.host.app().canvas_dialogs.pan_zoom.pan_pt;
    let _ = tab_b
        .handle_event(WebBridgeEvent::TouchStart {
            points: vec![TouchPoint { id: 1, x: 200.0, y: 200.0 }, TouchPoint { id: 2, x: 300.0, y: 200.0 }],
            dt_sec: 0.016,
        })
        .map_err(|e| e.to_string())?;
    let move_cmds = tab_b
        .handle_event(WebBridgeEvent::TouchMove {
            points: vec![TouchPoint { id: 1, x: 180.0, y: 220.0 }, TouchPoint { id: 2, x: 380.0, y: 220.0 }],
            dt_sec: 0.016,
        })
        .map_err(|e| e.to_string())?;
    assert!(matches!(move_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    assert!(tab_b.host.app().canvas_dialogs.pan_zoom.zoom > zoom_before);
    assert_ne!(tab_b.host.app().canvas_dialogs.pan_zoom.pan_pt, pan_before);

    let filter_cmds =
        tab_b.handle_event(WebBridgeEvent::SetPreviewFilter { filter: CanvasPreviewFilter::Deuteranopia }).map_err(|e| e.to_string())?;
    assert!(matches!(filter_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));

    let inspect_cmds = tab_b
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("doc.inspect"), params: json!({}) })
        .map_err(|e| e.to_string())?;
    assert!(matches!(inspect_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { command_result: Some(_), .. })));

    let clip_cmds = tab_b.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: None, dpi: 150 }).map_err(|e| e.to_string())?;
    assert!(matches!(clip_cmds.first(), Some(WebBridgeCommand::WriteClipboardItems { .. })));

    let print_cmds = tab_b.handle_event(WebBridgeEvent::PrintCurrentSheet { sheet_id: None, scale_percent: 100.0 }).map_err(|e| e.to_string())?;
    assert!(matches!(print_cmds.first(), Some(WebBridgeCommand::TriggerPrintHtml { .. })));

    assert!(tab_b.export_lumencraft_bytes().map_err(|e| e.to_string())?.starts_with(b"PK"));
    assert!(tab_b.export_directory_tree().map_err(|e| e.to_string())?.contains_key("document.json"));
    Ok(())
}

#[test]
fn web_bridge_read_only_contended_rejects_mutations_but_allows_pan_zoom_and_export() {
    let mut coord = WebLockCoordinator::new(15_000);
    let (mut tab_a, data_id, _graph_id) = build_populated_xy_session("study-shared", "tab-a").expect("populated session");

    let (acq_a, cmds_a) = tab_a.acquire_lock_with(&mut coord, 1_000).expect("tab-a acquire");
    assert_eq!(acq_a.mode, TabAccessMode::ExclusiveReadWrite);
    assert!(matches!(cmds_a.first(), Some(WebBridgeCommand::SetReadOnlyBanner { read_only: false, .. })));

    let archive_bytes = tab_a.export_lumencraft_bytes().expect("export archive from tab-a");

    let mut tab_b = WebBridgeSession::with_ids("study-shared", "tab-b");
    tab_b
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("study.lumencraft"), bytes: archive_bytes })
        .expect("tab-b loads initial file before joining lock");

    let (acq_b, cmds_b) = tab_b.acquire_lock_with(&mut coord, 1_200).expect("tab-b acquire");
    assert_eq!(acq_b.mode, TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("tab-a") });
    assert!(matches!(
        cmds_b.first(),
        Some(WebBridgeCommand::SetReadOnlyBanner {
            read_only: true,
            holder_tab_id: Some(holder),
            message: Some(_),
            ..
        }) if holder == "tab-a"
    ));

    assert_contended_rejects_mutations(&mut tab_b, &data_id);
    assert_contended_allows_viewport_and_exports(&mut tab_b).expect("read-only viewport & exports allowed");

    // Tab A releases lock -> Tab B promotes to ExclusiveReadWrite and clears read-only banner.
    let (rel_a, _) = tab_a.release_lock_with(&mut coord, 3_000).expect("tab-a release");
    assert_eq!(rel_a.promoted_tab_id.as_deref(), Some("tab-b"));

    let (mode_b, promo_cmds) = tab_b.sync_lock_from(&coord, 3_000);
    assert_eq!(mode_b, TabAccessMode::ExclusiveReadWrite);
    assert!(matches!(promo_cmds.first(), Some(WebBridgeCommand::SetReadOnlyBanner { read_only: false, holder_tab_id: None, message: None, .. })));

    tab_b
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("sheet.createData"),
            params: json!({ "kind": "column", "title": "Unlocked After Promotion", "auto_family": false }),
        })
        .expect("mutations succeed once promoted");
}

#[test]
fn directory_bridge_roundtrip_and_incremental_sync_plan() {
    let (mut session, sheet_alpha_id, _graph_id) = build_populated_xy_session("dir-doc", "tab-dir").expect("populated dir session");
    let second = session
        .host
        .execute_command(
            "sheet.createData",
            json!({
                "kind": "column",
                "title": "Control Group Beta",
                "auto_family": false,
            }),
        )
        .expect("create second data sheet");
    let sheet_beta_id = second["payload"]["sheet_id"].as_str().expect("beta sheet_id").to_owned();
    session
        .host
        .execute_command("table.setCell", json!({ "sheet_id": sheet_beta_id, "row": 0, "col": 0, "value": 7.5 }))
        .expect("populate beta cell");

    // Initial directory sync writes document.json, manifest.json, and both CSVs.
    let initial_cmds =
        session.handle_event(WebBridgeEvent::SyncDirectory { root_name: Some(String::from("assay.lumencraft.d")) }).expect("initial directory sync");
    let Some(WebBridgeCommand::ApplyDirectorySync { root_name, plan: initial_plan }) = initial_cmds.into_iter().next() else {
        panic!("expected ApplyDirectorySync command");
    };
    assert_eq!(root_name, "assay.lumencraft.d");
    assert_eq!(initial_plan.unchanged_count, 0);
    assert!(initial_plan.removals.is_empty());
    let expected_alpha_csv = format!("sheets/{sheet_alpha_id}.csv");
    let expected_beta_csv = format!("sheets/{sheet_beta_id}.csv");
    assert_eq!(initial_plan.writes.len(), 4);
    assert!(initial_plan.written_paths().contains(&"document.json"));
    assert!(initial_plan.written_paths().contains(&"manifest.json"));
    assert!(initial_plan.written_paths().contains(&expected_alpha_csv.as_str()));
    assert!(initial_plan.written_paths().contains(&expected_beta_csv.as_str()));

    // No edits -> zero writes, zero removals, all 4 unchanged.
    let noop_plan = session.fs.sync_directory_from_host(&mut session.host, None).expect("noop sync");
    assert!(noop_plan.is_empty_changes());
    assert_eq!(noop_plan.unchanged_count, 4);

    // Import exported .lumencraft.d tree into a fresh host via directory import AND HTML5 drop simulation.
    let exported_tree: BTreeMap<String, Vec<u8>> = initial_plan.writes.iter().cloned().collect();
    let mut reopened_host = WebWorkspaceHost::new();
    let mut reopened_bridge = FileSystemAccessBridge::new();
    let summary = reopened_bridge
        .import_directory_tree(&mut reopened_host, "assay.lumencraft.d", exported_tree.clone())
        .expect("directory tree roundtrip opens");
    assert_eq!(summary.data_sheet_count, 2);

    let dropped_files: Vec<(String, Vec<u8>)> =
        exported_tree.iter().map(|(rel, bytes)| (format!("assay.lumencraft.d/{rel}"), bytes.clone())).collect();
    let mut drop_host = WebWorkspaceHost::new();
    let mut drop_bridge = FileSystemAccessBridge::new();
    let drop_summary = drop_bridge.import_dropped_files(&mut drop_host, dropped_files).expect("HTML5 folder drop succeeds");
    assert_eq!(drop_summary.data_sheet_count, 2);

    // Edit ONLY sheet alpha -> incremental DirectorySyncPlan writes ONLY alpha CSV + document.json.
    session
        .host
        .execute_command("table.setCell", json!({ "sheet_id": sheet_alpha_id, "row": 1, "col": 0, "value": 99.25 }))
        .expect("edit cell in sheet alpha");
    let inc_plan = session.fs.sync_directory_from_host(&mut session.host, None).expect("incremental sync");
    let mut written_paths = inc_plan.written_paths();
    written_paths.sort_unstable();
    assert_eq!(written_paths, vec!["document.json", expected_alpha_csv.as_str()]);
    assert!(inc_plan.removals.is_empty());
    assert_eq!(inc_plan.unchanged_count, 2);

    // Delete sheet beta -> incremental sync schedules sheets/<beta>.csv for removal.
    session.host.execute_command("sheet.delete", json!({ "sheet_id": sheet_beta_id, "cascade": true })).expect("delete sheet beta");
    let del_plan: DirectorySyncPlan = session.fs.sync_directory_from_host(&mut session.host, None).expect("delete sync");
    assert_eq!(del_plan.removals, vec![expected_beta_csv]);
}

#[test]
fn two_finger_touch_pinch_zoom_and_inertial_pan_coasting() {
    let (mut session, _data_id, _graph_id) = build_populated_xy_session("touch-doc", "tab-touch").expect("populated touch session");
    session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 2.0 })
        .expect("resize viewport");

    let initial_zoom = session.host.app().canvas_dialogs.pan_zoom.zoom;
    let initial_pan = session.host.app().canvas_dialogs.pan_zoom.pan_pt;

    // Staggered two-finger touchdown (finger 1 lands slightly before finger 2).
    let _ = session
        .handle_event(WebBridgeEvent::TouchStart { points: vec![TouchPoint { id: 10, x: 400.0, y: 300.0 }], dt_sec: 0.016 })
        .expect("finger 1 down");
    let _ = session
        .handle_event(WebBridgeEvent::TouchStart { points: vec![TouchPoint { id: 11, x: 500.0, y: 300.0 }], dt_sec: 0.016 })
        .expect("finger 2 down");
    assert!(session.touch.is_two_finger_gesture());
    assert_eq!(session.touch.active_count(), 2);

    // Spread apart (scale > 1) and translate right/down (dx=+40, dy=+25).
    let move_cmds = session
        .handle_event(WebBridgeEvent::TouchMove {
            points: vec![TouchPoint { id: 10, x: 390.0, y: 325.0 }, TouchPoint { id: 11, x: 590.0, y: 325.0 }],
            dt_sec: 0.016,
        })
        .expect("two-finger pinch + pan");
    assert!(matches!(move_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { zoom, .. }) if *zoom > initial_zoom));

    let after_move_zoom = session.host.app().canvas_dialogs.pan_zoom.zoom;
    let after_move_pan = session.host.app().canvas_dialogs.pan_zoom.pan_pt;
    let move_vel = session.host.app().canvas_dialogs.pan_zoom.vel_pt_per_sec;
    assert!(after_move_zoom > initial_zoom);
    assert_ne!(after_move_pan, initial_pan);
    assert!(move_vel[0].abs() > 0.0 || move_vel[1].abs() > 0.0);

    // Release fingers -> TouchEnd emits coast step; subsequent CoastStep continues gliding & decaying velocity.
    let _ = session.handle_event(WebBridgeEvent::TouchEnd { released_ids: vec![10, 11], dt_sec: 0.016 }).expect("fingers lifted");
    assert!(!session.touch.is_two_finger_gesture());
    let pan_after_lift = session.host.app().canvas_dialogs.pan_zoom.pan_pt;
    assert_ne!(pan_after_lift, after_move_pan);

    let _ = session.handle_event(WebBridgeEvent::CoastStep { dt_sec: 0.016 }).expect("animation frame coast step");
    let pan_after_coast = session.host.app().canvas_dialogs.pan_zoom.pan_pt;
    let vel_after_coast = session.host.app().canvas_dialogs.pan_zoom.vel_pt_per_sec;
    assert_ne!(pan_after_coast, pan_after_lift);
    assert!(vel_after_coast[0].hypot(vel_after_coast[1]) < move_vel[0].hypot(move_vel[1]));
}

#[test]
fn multi_flavor_clipboard_print_and_json_bridge_roundtrip() {
    let (mut session, _data_id, _graph_id) = build_populated_xy_session("clip-doc", "tab-clip").expect("populated clip session");

    let copy_req_json = WebBridgeEvent::CopyClipboard { sheet_id: None, dpi: 150 }.to_json_string().expect("serialize CopyClipboard event");
    let copy_resp_json = session.handle_event_json(&copy_req_json).expect("handle CopyClipboard JSON");
    let copy_cmds = WebBridgeSession::commands_from_json_str(&copy_resp_json).expect("decode CopyClipboard commands");

    let Some(WebBridgeCommand::WriteClipboardItems { payload }) = copy_cmds.into_iter().next() else {
        panic!("expected WriteClipboardItems command");
    };
    assert!(payload.pdf_bytes.starts_with(b"%PDF-"));
    assert!(payload.svg.contains("<svg"));
    assert!(payload.png_bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]));
    assert!(payload.html.contains("<html") || payload.html.contains("<svg"));
    let tsv = payload.tsv.as_deref().expect("TSV flavor present");
    assert!(tsv.contains("49.5") || tsv.contains("96"));
    for flavor in ["pdf", "svg", "html", "png", "tsv"] {
        assert!(payload.flavors.iter().any(|f| f == flavor), "missing flavor {flavor}");
    }

    let print_req_json =
        WebBridgeEvent::PrintCurrentSheet { sheet_id: None, scale_percent: 100.0 }.to_json_string().expect("serialize PrintCurrentSheet event");
    let print_resp_json = session.handle_event_json(&print_req_json).expect("handle PrintCurrentSheet JSON");
    let print_cmds = WebBridgeSession::commands_from_json_str(&print_resp_json).expect("decode PrintCurrentSheet commands");
    let Some(WebBridgeCommand::TriggerPrintHtml { title, svg_pages, html, pdf_bytes, scale_percent, page_sizes_pt }) = print_cmds.into_iter().next()
    else {
        panic!("expected TriggerPrintHtml command");
    };
    assert!(!title.is_empty());
    assert_eq!(svg_pages.len(), 1);
    assert!(svg_pages[0].contains("<svg"));
    assert!(html.contains("@page") && html.contains("lumencraft-print-page"));
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    assert!((scale_percent - 100.0).abs() < 1e-9);
    assert_eq!(page_sizes_pt.len(), 1);

    // Event JSON roundtrip checks across lock/view/ui variants.
    let roundtrip_events = [
        WebBridgeEvent::ResizeViewport { logical_width: 1440.0, logical_height: 900.0, scale_factor: 2.0 },
        WebBridgeEvent::SetPreviewFilter { filter: CanvasPreviewFilter::Tritanopia },
        WebBridgeEvent::SetPresentationMode { enabled: true },
        WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } },
        WebBridgeEvent::AcquireLock { doc_id: String::from("clip-doc"), tab_id: String::from("tab-clip"), now_ms: 5_000 },
    ];
    for ev in roundtrip_events {
        let encoded = ev.to_json_string().expect("encode event");
        let decoded = WebBridgeEvent::from_json_str(&encoded).expect("decode event");
        assert_eq!(decoded, ev);
    }
}
