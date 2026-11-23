use std::collections::BTreeMap;

use curvebook_format::{CURRENT_PREFERENCES_VERSION, PreferencesFile};
use curvebook_ui_egui::serde_json::json;
use curvebook_ui_egui::{CanvasPreviewFilter, KeyModifiers, UiInputEvent};
use curvebook_web::{
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
    let data_id = created.get("payload").and_then(|p| p.get("sheet_id")).and_then(|v| v.as_str()).ok_or("missing data sheet_id")?.to_owned();
    let graph_id =
        created.get("payload").and_then(|p| p.get("graph_sheet_id")).and_then(|v| v.as_str()).ok_or("missing auto graph_sheet_id")?.to_owned();

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

    // Legacy `lumencraft.v1.*` storage entries are neither migrated nor modified by CurveBook.
    let legacy_prefs = r#"{"version":1,"theme":"light"}"#;
    storage.backend_mut().set_item("lumencraft.v1.preferences", legacy_prefs).expect("seed legacy prefs");
    storage.backend_mut().set_item("lumencraft.v1.autosave.legacy-doc", "legacy-snapshot").expect("seed legacy autosave");
    assert_eq!(storage.load_preferences().expect("legacy preferences ignored"), None);
    assert_eq!(storage.load_autosave_snapshot("legacy-doc", 500).expect("legacy autosave ignored"), None);

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
    assert_eq!(PREFERENCES_STORAGE_KEY, "curvebook.v1.preferences");
    assert_eq!(AUTOSAVE_KEY_PREFIX, "curvebook.v1.autosave.");
    assert_eq!(storage.backend().get_item("lumencraft.v1.preferences").as_deref(), Some(legacy_prefs));
    assert_eq!(storage.backend().get_item("lumencraft.v1.autosave.legacy-doc").as_deref(), Some("legacy-snapshot"));

    // Simulate newer CurveBook build having written forward schema preferences.
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

    assert!(tab_b.export_curvebook_bytes().map_err(|e| e.to_string())?.starts_with(b"PK"));
    assert!(tab_b.export_directory_tree().map_err(|e| e.to_string())?.contains_key("document.json"));
    Ok(())
}

#[test]
fn web_bridge_read_only_contended_rejects_mutations_but_allows_pan_zoom_and_export() {
    let mut coord = WebLockCoordinator::new(15_000);
    let (mut tab_a, data_id, _graph_id) = build_populated_xy_session("study-shared", "tab-a").expect("populated session");

    let archive_bytes = tab_a.export_curvebook_bytes().expect("export archive from tab-a");
    tab_a
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("study.curvebook"), bytes: archive_bytes.clone() })
        .expect("tab-a opens shared file");

    let (acq_a, cmds_a) = tab_a.acquire_lock_with(&mut coord, 1_000).expect("tab-a acquire");
    assert_eq!(acq_a.mode, TabAccessMode::ExclusiveReadWrite);
    assert!(matches!(cmds_a.first(), Some(WebBridgeCommand::SetReadOnlyBanner { read_only: false, .. })));

    let mut tab_b = WebBridgeSession::with_ids("untitled-tab-b", "tab-b");
    tab_b
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("study.curvebook"), bytes: archive_bytes })
        .expect("tab-b loads initial file before joining lock");
    assert_eq!(tab_b.doc_id(), tab_a.doc_id(), "opening identical file must converge on the same derived doc_id");

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

    // Initial directory sync writes document.json, manifest.json, and both CSVs, leaving dirty=true until confirmed.
    let initial_cmds =
        session.handle_event(WebBridgeEvent::SyncDirectory { root_name: Some(String::from("assay.curvebook.d")) }).expect("initial directory sync");
    let Some(WebBridgeCommand::ApplyDirectorySync { root_name, plan: initial_plan }) = initial_cmds.into_iter().next() else {
        panic!("expected ApplyDirectorySync command");
    };
    assert_eq!(root_name, "assay.curvebook.d");
    assert_eq!(initial_plan.unchanged_count, 0);
    assert!(initial_plan.removals.is_empty());
    let expected_alpha_csv = format!("sheets/{sheet_alpha_id}.csv");
    let expected_beta_csv = format!("sheets/{sheet_beta_id}.csv");
    assert_eq!(initial_plan.writes.len(), 4);
    assert!(initial_plan.written_paths().contains(&"document.json"));
    assert!(initial_plan.written_paths().contains(&"manifest.json"));
    assert!(initial_plan.written_paths().contains(&expected_alpha_csv.as_str()));
    assert!(initial_plan.written_paths().contains(&expected_beta_csv.as_str()));
    assert!(session.host.inspect_summary().dirty, "workspace must remain dirty before directory confirmation");

    // Browser confirms initial writes -> dirty clears and noop sync becomes empty.
    let initial_written: Vec<String> = initial_plan.written_paths().into_iter().map(str::to_owned).collect();
    session
        .handle_event(WebBridgeEvent::ConfirmDirectorySync {
            root_name: root_name.clone(),
            written_paths: initial_written,
            removed_paths: Vec::new(),
            succeeded: true,
            error: None,
        })
        .expect("confirm initial directory sync");
    assert!(!session.host.inspect_summary().dirty);

    // No edits -> zero writes, zero removals, all 4 unchanged.
    let noop_plan = session.fs.sync_directory_from_host(&mut session.host, None).expect("noop sync");
    assert!(noop_plan.is_empty_changes());
    assert_eq!(noop_plan.unchanged_count, 4);
    let _ = session.fs.confirm_directory_sync(&mut session.host, "assay.curvebook.d", &[], &[], true).expect("confirm noop directory sync");

    // Import exported .curvebook.d tree into a fresh host via directory import AND HTML5 drop simulation.
    let exported_tree: BTreeMap<String, Vec<u8>> = initial_plan.writes.iter().cloned().collect();
    let mut reopened_host = WebWorkspaceHost::new();
    let mut reopened_bridge = FileSystemAccessBridge::new();
    let summary = reopened_bridge
        .import_directory_tree(&mut reopened_host, "assay.curvebook.d", exported_tree.clone())
        .expect("directory tree roundtrip opens");
    assert_eq!(summary.data_sheet_count, 2);

    let dropped_files: Vec<(String, Vec<u8>)> =
        exported_tree.iter().map(|(rel, bytes)| (format!("assay.curvebook.d/{rel}"), bytes.clone())).collect();
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
    let mut written_paths: Vec<String> = inc_plan.written_paths().into_iter().map(str::to_owned).collect();
    written_paths.sort_unstable();
    assert_eq!(written_paths, vec![String::from("document.json"), expected_alpha_csv]);
    assert!(inc_plan.removals.is_empty());
    assert_eq!(inc_plan.unchanged_count, 2);
    assert!(session.fs.confirm_directory_sync(&mut session.host, "assay.curvebook.d", &written_paths, &[], true).expect("confirm incremental sync"));

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
    assert!(html.contains("@page") && html.contains("curvebook-print-page"));
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    assert!((scale_percent - 100.0).abs() < 1e-9);
    assert_eq!(page_sizes_pt.len(), 1);

    // Event JSON roundtrip checks across lock/view/ui variants.
    let roundtrip_events = [
        WebBridgeEvent::ResizeViewport { logical_width: 1440.0, logical_height: 900.0, scale_factor: 2.0 },
        WebBridgeEvent::SetPreviewFilter { filter: CanvasPreviewFilter::Tritanopia },
        WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } },
        WebBridgeEvent::AcquireLock { doc_id: String::from("clip-doc"), tab_id: String::from("tab-clip"), now_ms: 5_000 },
    ];
    for ev in roundtrip_events {
        let encoded = ev.to_json_string().expect("encode event");
        let decoded = WebBridgeEvent::from_json_str(&encoded).expect("decode event");
        assert_eq!(decoded, ev);
    }
}

#[test]
fn r2_s11_web_edge_gaps_paste_storage_locks_print_and_schema() {
    // 1. R2-S11-03: non-integer `schema_version` must be rejected rather than falling back to 0.
    let mut json_host = curvebook_web::WebWorkspaceHost::new();
    let bad_ver = br#"{"schema_version":"future","title":"Bad"}"#;
    let err = json_host.open_project_json_bytes(bad_ver).expect_err("non-integer schema_version must error");
    assert!(err.contains("schema_version"), "unexpected error: {err}");

    // 2. R2-S11-06: PasteClipboard must route into active inline editor BEFORE DataSheet,
    // and when not inline editing, pass locale `decimal_separator` to `table.paste`.
    let (mut session, data_id, _graph_id) = build_populated_xy_session("web-r2-doc", "tab-r2").expect("populated session");
    session.host.app_mut().inline_editor.active = true;
    session.host.app_mut().inline_editor.buffer = String::from("abc");
    session.host.app_mut().inline_editor.caret_byte = 3;
    let inline_cmds = session.handle_event(WebBridgeEvent::PasteClipboard { text: String::from("XYZ") }).expect("inline paste");
    assert!(matches!(inline_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    assert_eq!(session.host.app().inline_editor.buffer, "abcXYZ");

    session.host.app_mut().inline_editor.active = false;
    let _ = session.host.execute_command("sheet.select", json!({ "sheet_id": data_id })).expect("select data sheet");
    let _ = curvebook_ui_egui::dispatch_ui_command(session.host.app_mut(), "ui.locale.set", &json!({ "locale": "de-DE" }))
        .expect("set German comma decimal locale");
    let paste_cmds = session.handle_event(WebBridgeEvent::PasteClipboard { text: String::from("3,5") }).expect("comma decimal paste");
    assert!(matches!(paste_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    let ds = session.host.app().document().data_sheets.iter().find(|s| s.id.as_str() == data_id).expect("data sheet present");
    let pasted_csv = curvebook_format::export_datasheet_to_csv(ds, &curvebook_format::CsvOptions::csv()).expect("export pasted csv");
    assert!(pasted_csv.contains("3.5"), "comma-decimal paste under de-DE must parse 3.5; csv:\n{pasted_csv}");

    // 3. R2-S11-05: UI theme/locale/shortcut changes emit PersistLocalStorage automatically, and
    // HydrateLocalStorage restores theme, locale, shortcuts, AND blank-session autosave snapshots.
    let theme_cmds = session
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.theme.set"), params: json!({ "theme": "dark" }) })
        .expect("set dark theme");
    let persisted = theme_cmds.iter().find(|cmd| matches!(cmd, WebBridgeCommand::PersistLocalStorage { .. }));
    assert!(persisted.is_some(), "changing theme must emit PersistLocalStorage automatically");
    let Some(WebBridgeCommand::PersistLocalStorage { writes, .. }) = persisted else {
        panic!("missing PersistLocalStorage");
    };
    let autosave_cmds = session.handle_event(WebBridgeEvent::SaveAutosave { now_ms: 9_999 }).expect("save autosave");
    let Some(WebBridgeCommand::PersistLocalStorage { writes: doc_writes, .. }) = autosave_cmds.into_iter().next() else {
        panic!("expected autosave PersistLocalStorage");
    };
    let mut hydrated_entries = writes.clone();
    hydrated_entries.extend(doc_writes);

    let mut fresh = WebBridgeSession::with_ids("web-r2-doc", "tab-r2-fresh");
    let hyd_cmds = fresh.handle_event(WebBridgeEvent::HydrateLocalStorage { entries: hydrated_entries }).expect("hydrate theme + autosave");
    assert!(matches!(hyd_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    assert_eq!(fresh.host.app().theme.mode, curvebook_ui_egui::theme::UiThemeMode::Dark);
    assert_eq!(fresh.host.app().locale, "de");
    let restored_ds = fresh.host.app().document().data_sheets.iter().find(|s| s.id.as_str() == data_id).expect("restored autosave data sheet");
    let restored_csv = curvebook_format::export_datasheet_to_csv(restored_ds, &curvebook_format::CsvOptions::csv()).expect("export restored csv");
    assert!(restored_csv.contains("3.5"), "restored autosave must preserve pasted value 3.5; csv:\n{restored_csv}");

    // 4. R2-S11-04 & R2-S11-13 in index.html: verify release gate (!holdsCurrentWebLock), localStorage
    // lock fallback when navigator.locks is absent, debounced autosave trigger, and DOM print zoom scaling.
    let html = include_str!("../index.html");
    assert!(html.contains("data.type === \"release\" && !holdsCurrentWebLock"));
    assert!(html.contains("tryAcquireFallbackStorageLock"));
    assert!(html.contains("scheduleAutosave"));
    assert!(html.contains("section.style.zoom = scaleFactor.toFixed(4)"));
}

struct QuotaRecordingBackend {
    items: BTreeMap<String, String>,
    ops: Vec<String>,
    fail_key_prefix: Option<String>,
    max_total_bytes: Option<usize>,
}

impl QuotaRecordingBackend {
    fn new(max_total_bytes: Option<usize>, fail_key_prefix: Option<&str>) -> Self {
        Self { items: BTreeMap::new(), ops: Vec::new(), fail_key_prefix: fail_key_prefix.map(str::to_owned), max_total_bytes }
    }
}

impl WebStorageBackend for QuotaRecordingBackend {
    fn get_item(&self, key: &str) -> Option<String> {
        self.items.get(key).cloned()
    }

    fn set_item(&mut self, key: &str, value: &str) -> Result<(), WebAdapterError> {
        self.ops.push(format!("set:{key}"));
        if self.fail_key_prefix.as_deref().is_some_and(|pfx| key.starts_with(pfx)) {
            return Err(WebAdapterError::QuotaExceeded { required_bytes: key.len() + value.len(), budget_bytes: 1024 });
        }
        let prev = self.items.get(key).map_or(0, |v| key.len() + v.len());
        let next = self.used_bytes().saturating_sub(prev).saturating_add(key.len() + value.len());
        if let Some(limit) = self.max_total_bytes
            && next > limit
        {
            return Err(WebAdapterError::QuotaExceeded { required_bytes: next, budget_bytes: limit });
        }
        self.items.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove_item(&mut self, key: &str) -> Option<String> {
        self.ops.push(format!("remove:{key}"));
        self.items.remove(key)
    }

    fn keys(&self) -> Vec<String> {
        self.items.keys().cloned().collect()
    }

    fn used_bytes(&self) -> usize {
        self.items.iter().map(|(k, v)| k.len() + v.len()).sum()
    }
}

#[test]
fn rv80_autosave_writes_before_eviction_and_rolls_back_on_quota_failure() {
    // 1. When backend accepts the new write, set_item runs BEFORE any remove_item eviction.
    let backend = QuotaRecordingBackend::new(None, None);
    let mut mgr = WebStorageManager::with_budget(backend, 150);
    let _ = mgr.save_autosave_snapshot("doc-1", &"A".repeat(40), 100).expect("save doc-1");
    let _ = mgr.save_autosave_snapshot("doc-2", &"B".repeat(40), 200).expect("save doc-2");
    mgr.backend_mut().ops.clear();

    let out = mgr.save_autosave_snapshot("doc-3", &"C".repeat(40), 300).expect("save doc-3 evicts doc-1");
    assert_eq!(out.evicted_keys, vec![format!("{AUTOSAVE_KEY_PREFIX}doc-1")]);
    let set_pos = mgr.backend().ops.iter().position(|op| op == &format!("set:{AUTOSAVE_KEY_PREFIX}doc-3")).expect("set doc-3 logged");
    let rm_pos = mgr.backend().ops.iter().position(|op| op == &format!("remove:{AUTOSAVE_KEY_PREFIX}doc-1")).expect("remove doc-1 logged");
    assert!(set_pos < rm_pos, "new autosave must be written BEFORE older snapshot eviction; ops={:?}", mgr.backend().ops);

    // 2. When backend storage rejects the new write after evicting older entries (e.g. external origin
    // usage leaves less physical room than the manager's logical budget), all previously stored
    // recovery snapshots MUST be rolled back intact (`RV-80`).
    let mut failing_mgr = WebStorageManager::with_budget(QuotaRecordingBackend::new(Some(100), None), 1024);
    let _ = failing_mgr.save_autosave_snapshot("doc-keep-1", "recovery-one", 100).expect("seed keep-1");
    let _ = failing_mgr.save_autosave_snapshot("doc-keep-2", "recovery-two", 200).expect("seed keep-2");
    let err = failing_mgr.save_autosave_snapshot("doc-fail", &"D".repeat(90), 300).expect_err("storage failure must surface QuotaExceeded");
    assert!(matches!(err, WebAdapterError::QuotaExceeded { .. }), "unexpected error: {err}");
    assert_eq!(failing_mgr.load_autosave_snapshot("doc-keep-1", 400).expect("read keep-1").as_deref(), Some("recovery-one"));
    assert_eq!(failing_mgr.load_autosave_snapshot("doc-keep-2", 400).expect("read keep-2").as_deref(), Some("recovery-two"));

    // 2b. Also verify explicit backend key write errors preserve existing entries.
    let mut prefix_fail_mgr = WebStorageManager::with_budget(QuotaRecordingBackend::new(None, Some(&format!("{AUTOSAVE_KEY_PREFIX}doc-err"))), 1024);
    let _ = prefix_fail_mgr.save_autosave_snapshot("doc-safe", "safe-recovery", 100).expect("seed safe");
    assert!(prefix_fail_mgr.save_autosave_snapshot("doc-err", "payload", 200).is_err());
    assert_eq!(prefix_fail_mgr.load_autosave_snapshot("doc-safe", 300).expect("read safe").as_deref(), Some("safe-recovery"));

    // 3. index.html `applyLocalStorageCommand` executes writes BEFORE removals and restores backups on failure.
    let html = include_str!("../index.html");
    let (_, after_fn) = html.split_once("function applyLocalStorageCommand(cmd)").expect("applyLocalStorageCommand exists");
    let fn_slice: String = after_fn.chars().take(2600).collect();
    assert!(fn_slice.contains("writeAll()"));
    assert!(fn_slice.contains("rollbackStorage()"));
    assert!(fn_slice.contains("removedDuringRetry"));
}

#[test]
fn rv81_web_flushes_autosave_on_visibility_pagehide_and_prompts_beforeunload_when_dirty() {
    let (mut session, data_id, _graph_id) = build_populated_xy_session("web-rv81-doc", "tab-rv81").expect("populated session");
    let clean_or_dirty = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("table.setCell"),
            params: json!({ "sheet_id": data_id, "row": 0, "col": 0, "value": 99.5 }),
        })
        .expect("mutate cell");
    let dirty_flag = clean_or_dirty.iter().find_map(|cmd| match cmd {
        WebBridgeCommand::RenderRgbaFrame { chrome, .. } => Some(chrome.dirty),
        _ => None,
    });
    assert_eq!(dirty_flag, Some(true), "RenderRgbaFrame.chrome.dirty must reflect dirty session");

    let html = include_str!("../index.html");
    assert!(html.contains("function flushPendingAutosave()"));
    assert!(html.contains("document.addEventListener(\"visibilitychange\""));
    assert!(html.contains("document.visibilityState === \"hidden\""));
    assert!(html.contains("window.addEventListener(\"pagehide\""));
    assert!(html.contains("window.addEventListener(\"beforeunload\""));
    assert!(html.contains("if (documentDirty || hasUnflushedAutosave)"));
    assert!(html.contains("ev.preventDefault()"));
    assert!(html.contains("ev.returnValue = \"\""));
}

#[test]
fn r8_web_duplicate_normalized_import_path_and_dpi_validation() {
    // 1. Duplicate normalized paths inside ImportDirectoryEntries must error cleanly instead of silent overwrite.
    let mut session = WebBridgeSession::with_ids("web-r8-dir", "tab-r8");
    let dup_err = session
        .handle_event(WebBridgeEvent::ImportDirectoryEntries {
            root_name: String::from("study.curvebook.d"),
            entries: BTreeMap::from([
                (String::from("./sub/data.csv"), b"x,y\n1,2\n".to_vec()),
                (String::from("sub/data.csv"), b"x,y\n3,4\n".to_vec()),
            ]),
        })
        .expect_err("duplicate normalized directory entry path must return error");
    assert!(matches!(dup_err, WebAdapterError::Validation(ref msg) if msg.contains("duplicate normalized directory")), "got {dup_err}");

    let dup_root_err = session
        .handle_event(WebBridgeEvent::ImportDirectoryEntries {
            root_name: String::from("root"),
            entries: BTreeMap::from([(String::from("root/manifest.json"), b"{}".to_vec()), (String::from("manifest.json"), b"{}".to_vec())]),
        })
        .expect_err("root-prefixed duplicate path must fail");
    assert!(matches!(dup_root_err, WebAdapterError::Validation(_)));

    // 2. Out-of-range / invalid `dpi` must return explicit validation error instead of defaulting/clamping.
    for bad_ev in [
        r#"{"kind":"render_frame","dpi":0}"#,
        r#"{"kind":"render_frame","dpi":71}"#,
        r#"{"kind":"render_frame","dpi":1201}"#,
        r#"{"kind":"render_frame","dpi":"300"}"#,
        r#"{"kind":"copy_clipboard","dpi":99999}"#,
        r#"{"kind":"export_file","format":"png","dpi":-10}"#,
        r#"{"kind":"export_file","format":"tiff","dpi":5000}"#,
    ] {
        let err = WebBridgeEvent::from_json_str(bad_ev).expect_err("out-of-range/invalid dpi JSON must fail");
        assert!(matches!(err, WebAdapterError::Validation(_)), "expected Validation for {bad_ev}, got {err}");
    }
    let direct_err = session.handle_event(WebBridgeEvent::RenderFrame { dpi: 5000 }).expect_err("direct out-of-range RenderFrame dpi must fail");
    assert!(matches!(direct_err, WebAdapterError::Validation(_)));

    let export_err = session
        .handle_event(WebBridgeEvent::ExportFile { format: curvebook_web::WebExportFormat::Png, sheet_id: None, dpi: 2400 })
        .expect_err("out-of-range ExportFile dpi must fail");
    assert!(matches!(export_err, WebAdapterError::Validation(_)));

    let clip_err = curvebook_web::WebClipboardPayload::from_json_value(&json!({ "dpi": 15 })).expect_err("out-of-range clipboard dpi must fail");
    assert!(matches!(clip_err, WebAdapterError::Validation(_)));
}

#[test]
fn audit_123_1_browser_lock_grant_replaces_cached_lease_within_ttl_and_clears_banner() {
    let (mut session, data_id, _graph_id) = build_populated_xy_session("doc-lock-handoff", "tab-b").expect("populated session");

    // Browser initially observes tab-a holding the lock at t=1000ms -> tab-b enters read-only mode.
    let obs_cmds = session
        .handle_event(WebBridgeEvent::ObserveExternalLock {
            doc_id: String::from("doc-lock-handoff"),
            holder_tab_id: String::from("tab-a"),
            now_ms: 1_000,
        })
        .expect("observe external lock");
    assert_eq!(session.access_mode(), &TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("tab-a") });
    assert!(matches!(
        obs_cmds.first(),
        Some(WebBridgeCommand::SetReadOnlyBanner { read_only: true, holder_tab_id: Some(holder), .. }) if holder == "tab-a"
    ));

    // Mutating commands are refused while tab-a's cached lease is observed.
    let err = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("table.setCell"),
            params: json!({ "sheet_id": data_id, "row": 0, "col": 0, "value": 42.0 }),
        })
        .expect_err("contended tab must block edits");
    assert!(matches!(err, WebAdapterError::ReadOnlyContended { .. }));

    // 2 seconds later (well within the 15s lease TTL), the browser's Web Lock request resolves for tab-b.
    // AcquireLock must immediately replace the cached lease and clear the read-only banner.
    let acq_cmds = session
        .handle_event(WebBridgeEvent::AcquireLock { doc_id: String::from("doc-lock-handoff"), tab_id: String::from("tab-b"), now_ms: 3_000 })
        .expect("browser lock grant");
    assert_eq!(session.access_mode(), &TabAccessMode::ExclusiveReadWrite);
    assert!(matches!(acq_cmds.first(), Some(WebBridgeCommand::SetReadOnlyBanner { read_only: false, holder_tab_id: None, .. })));

    // Editing and autosave immediately work for tab-b.
    session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("table.setCell"),
            params: json!({ "sheet_id": data_id, "row": 0, "col": 0, "value": 42.5 }),
        })
        .expect("promoted tab edits");
    let autosave_cmds = session.handle_event(WebBridgeEvent::SaveAutosave { now_ms: 3_500 }).expect("promoted tab saves autosave");
    assert!(matches!(autosave_cmds.first(), Some(WebBridgeCommand::PersistLocalStorage { .. })));

    // Shared in-memory lock coordinator directly enforces lease contention, while grant_browser_lock supersedes it.
    let mut locks = WebLockCoordinator::default();
    let tab_a = locks.acquire_document_lock("doc-shared", "tab-a", 1_000).expect("tab-a lock");
    assert_eq!(tab_a.mode, TabAccessMode::ExclusiveReadWrite);
    let contended = locks.acquire_document_lock("doc-shared", "tab-b", 3_000).expect("tab-b contended");
    assert_eq!(contended.mode, TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("tab-a") });
    let granted = locks.grant_browser_lock("doc-shared", "tab-b", 3_100).expect("browser Web Lock grant");
    assert_eq!(granted.mode, TabAccessMode::ExclusiveReadWrite);

    // index.html enforces release-before-broadcast, waiting navigator.locks takeover, fallback retries, and stale holder guard.
    let html = include_str!("../index.html");
    let (_, after_rel) = html.split_once("releaseCurrentWebLock();").expect("release resolves web lock before broadcast");
    assert!(after_rel.contains("type: \"release\""));
    assert!(html.contains("holdGrantedWebLock"));
    assert!(html.contains("navigator.locks.request(lockName, { signal: abortCtrl.signal }"));
    assert!(html.contains("if (!holdsCurrentWebLock)"));
    assert!(html.contains("window.addEventListener(\"storage\""));
}

#[test]
fn audit_123_2_directory_sync_only_advances_on_confirmation_and_zip_export_preserves_dirty() {
    let (mut session, data_id, _graph_id) = build_populated_xy_session("doc-dir-confirm", "tab-dir").expect("populated session");
    let sheet_csv = format!("sheets/{data_id}.csv");

    // 1. Export CurveBookDirectory ZIP and export_virtual_entries must NOT clear dirty or advance synced tree.
    assert!(session.host.inspect_summary().dirty);
    let zip_cmds = session
        .handle_event(WebBridgeEvent::ExportFile { format: curvebook_web::WebExportFormat::CurveBookDirectory, sheet_id: None, dpi: 300 })
        .expect("export directory zip");
    assert!(matches!(zip_cmds.first(), Some(WebBridgeCommand::SaveOrDownloadFile { .. })));
    assert!(session.host.inspect_summary().dirty, "ZIP export must leave document dirty");
    let _entries = session.host.export_virtual_entries().expect("export virtual entries");
    assert!(session.host.inspect_summary().dirty, "export_virtual_entries must leave document dirty");

    // 2. SyncDirectory emits all files, but dirty stays true until browser confirms.
    let cmds_1 = session.handle_event(WebBridgeEvent::SyncDirectory { root_name: Some(String::from("study.curvebook.d")) }).expect("sync emit 1");
    let Some(WebBridgeCommand::ApplyDirectorySync { root_name, plan: plan_1 }) = cmds_1.into_iter().next() else {
        panic!("expected ApplyDirectorySync");
    };
    assert_eq!(root_name, "study.curvebook.d");
    assert!(plan_1.writes.len() >= 3);
    assert!(session.host.inspect_summary().dirty);

    // 3. Simulated complete failure (e.g. missing directory handle) leaves dirty=true and next SyncDirectory re-emits everything.
    session
        .handle_event(WebBridgeEvent::ConfirmDirectorySync {
            root_name: root_name.clone(),
            written_paths: Vec::new(),
            removed_paths: Vec::new(),
            succeeded: false,
            error: Some(String::from("no active directory handle")),
        })
        .expect("confirm failure");
    assert!(session.host.inspect_summary().dirty);

    let cmds_retry_1 = session.handle_event(WebBridgeEvent::SyncDirectory { root_name: None }).expect("retry after total failure");
    let Some(WebBridgeCommand::ApplyDirectorySync { plan: retry_plan_1, .. }) = cmds_retry_1.into_iter().next() else {
        panic!("expected ApplyDirectorySync retry 1");
    };
    assert_eq!(retry_plan_1.writes.len(), plan_1.writes.len());
    assert_eq!(retry_plan_1.unchanged_count, 0);

    // 4. Partial write: only `manifest.json` written, remaining writes failed -> dirty stays true and retry writes only remaining files.
    session
        .handle_event(WebBridgeEvent::ConfirmDirectorySync {
            root_name: root_name.clone(),
            written_paths: vec![String::from("manifest.json")],
            removed_paths: Vec::new(),
            succeeded: false,
            error: Some(String::from("disk full during sheet write")),
        })
        .expect("confirm partial failure");
    assert!(session.host.inspect_summary().dirty, "partial failure must keep dirty=true");

    let cmds_retry_2 = session.handle_event(WebBridgeEvent::SyncDirectory { root_name: None }).expect("retry after partial failure");
    let Some(WebBridgeCommand::ApplyDirectorySync { plan: retry_plan_2, .. }) = cmds_retry_2.into_iter().next() else {
        panic!("expected ApplyDirectorySync retry 2");
    };
    assert_eq!(retry_plan_2.unchanged_count, 1, "already-confirmed manifest.json must be unchanged");
    assert!(!retry_plan_2.written_paths().contains(&"manifest.json"));
    assert!(retry_plan_2.written_paths().contains(&"document.json"));
    assert!(retry_plan_2.written_paths().contains(&sheet_csv.as_str()));

    // 5. Full confirmation finally clears dirty and subsequent SyncDirectory has 0 changes (even after another ZIP download).
    let remaining_paths: Vec<String> = retry_plan_2.written_paths().into_iter().map(str::to_owned).collect();
    session
        .handle_event(WebBridgeEvent::ConfirmDirectorySync {
            root_name,
            written_paths: remaining_paths,
            removed_paths: Vec::new(),
            succeeded: true,
            error: None,
        })
        .expect("confirm full retry");
    assert!(!session.host.inspect_summary().dirty, "full confirmation must clear dirty");

    // Mutate cell -> ZIP export still keeps dirty=true -> SyncDirectory still emits delta for edited document/csv.
    session.host.execute_command("table.setCell", json!({ "sheet_id": data_id, "row": 0, "col": 0, "value": 88.5 })).expect("edit cell after sync");
    let _zip_after = session
        .handle_event(WebBridgeEvent::ExportFile { format: curvebook_web::WebExportFormat::CurveBookDirectory, sheet_id: None, dpi: 300 })
        .expect("zip after edit");
    assert!(session.host.inspect_summary().dirty, "ZIP download after edit must not clear dirty");
    let cmds_delta = session.handle_event(WebBridgeEvent::SyncDirectory { root_name: None }).expect("delta after zip");
    let Some(WebBridgeCommand::ApplyDirectorySync { plan: delta_plan, .. }) = cmds_delta.into_iter().next() else {
        panic!("expected delta ApplyDirectorySync");
    };
    assert!(delta_plan.written_paths().contains(&"document.json"));
    assert!(delta_plan.written_paths().contains(&sheet_csv.as_str()));

    // Verify index.html emits confirm_directory_sync after writes and in the missing-handle/error paths.
    let html = include_str!("../index.html");
    assert!(html.contains("confirm_directory_sync"));
}

#[test]
fn audit_123_3_preferences_near_quota_emit_evictions_and_reconcile_from_browser_outcome() {
    // 1. Direct SavePreferences near quota must emit evicted autosave keys in `removals`.
    let budget = 400;
    let mut session = WebBridgeSession::with_ids("doc-pref-active", "tab-pref");
    session.storage = WebStorageManager::with_budget(curvebook_web::MemoryWebStorage::default(), budget);
    let _ = session.storage.save_autosave_snapshot("doc-old-1", &"x".repeat(120), 100).expect("seed old 1");
    let _ = session.storage.save_autosave_snapshot("doc-old-2", &"y".repeat(120), 200).expect("seed old 2");
    assert_eq!(session.storage.list_autosave_doc_ids().len(), 2, "both seeded autosaves must fit before prefs save");

    let prefs = PreferencesFile {
        version: CURRENT_PREFERENCES_VERSION,
        locale: Some(String::from("fr-FR")),
        theme: Some(String::from("dark")),
        shortcuts: BTreeMap::from([(String::from("file.export"), String::from("Ctrl+Shift+E"))]),
        ..PreferencesFile::default()
    };
    let cmds = session.handle_event(WebBridgeEvent::SavePreferences { preferences: prefs.clone() }).expect("save prefs near quota");
    let Some(WebBridgeCommand::PersistLocalStorage { writes, removals }) = cmds.into_iter().next() else {
        panic!("expected PersistLocalStorage");
    };
    assert!(writes.iter().any(|(k, _)| k == PREFERENCES_STORAGE_KEY));
    assert!(!removals.is_empty(), "SavePreferences near quota must emit evicted autosave keys in removals");
    let old_1_key = format!("{AUTOSAVE_KEY_PREFIX}doc-old-1");
    assert!(removals.contains(&old_1_key));

    // If the browser reports quota failure/rollback, the mirror restores doc-old-1 AND previous preferences state.
    session
        .handle_event(WebBridgeEvent::ConfirmLocalStorage { writes: Vec::new(), removals: Vec::new(), succeeded: false })
        .expect("confirm storage rollback");
    assert_eq!(session.storage.load_preferences().expect("prefs rolled back"), None);
    assert!(
        session.storage.backend().get_item(&old_1_key).is_some(),
        "rolled-back autosave must remain available in mirror for future saves without altering LRU"
    );

    // Retry SavePreferences -> succeeds cleanly -> ConfirmLocalStorage commits eviction.
    let retry_cmds = session.handle_event(WebBridgeEvent::SavePreferences { preferences: prefs.clone() }).expect("retry save prefs");
    let Some(WebBridgeCommand::PersistLocalStorage { writes: w2, removals: r2 }) = retry_cmds.into_iter().next() else {
        panic!("expected retry PersistLocalStorage");
    };
    assert!(r2.contains(&old_1_key), "retry should again pick oldest doc-old-1 for eviction");
    let write_keys: Vec<String> = w2.into_iter().map(|(k, _)| k).collect();
    session
        .handle_event(WebBridgeEvent::ConfirmLocalStorage { writes: write_keys, removals: r2.clone(), succeeded: true })
        .expect("confirm retry success");
    assert_eq!(session.storage.load_preferences().expect("saved prefs"), Some(prefs));
    assert_eq!(session.storage.load_autosave_snapshot("doc-old-1", 400).expect("evicted snapshot gone"), None);

    // 2. UI-triggered preference change (ExecuteCommand ui.locale.set) near quota also emits removals.
    let mut ui_session = WebBridgeSession::with_ids("doc-pref-ui", "tab-pref-ui");
    ui_session.storage = WebStorageManager::with_budget(curvebook_web::MemoryWebStorage::default(), 250);
    let _ = ui_session.storage.save_autosave_snapshot("doc-ui-old", &"z".repeat(130), 500).expect("seed old ui");
    assert!(
        ui_session.storage.list_autosave_doc_ids().contains(&String::from("doc-ui-old")),
        "doc-ui-old must fit just below quota before UI preference update"
    );
    let ui_cmds = ui_session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("ui.locale.set"),
            params: json!({ "locale": "zh-Hant-TW-custom-extended-locale" }),
        })
        .expect("ui.locale.set emits preferences persist near quota");
    let ui_persist = ui_cmds.into_iter().find_map(|cmd| match cmd {
        WebBridgeCommand::PersistLocalStorage { writes, removals } => Some((writes, removals)),
        _ => None,
    });
    let (ui_writes, ui_removals) = ui_persist.expect("UI preference change must emit PersistLocalStorage");
    assert!(ui_writes.iter().any(|(k, _)| k == PREFERENCES_STORAGE_KEY));
    assert!(ui_removals.contains(&format!("{AUTOSAVE_KEY_PREFIX}doc-ui-old")), "UI preference change near quota must emit evicted removals");

    // 3. Partial browser outcome (e.g. write succeeded without needing removal) restores unused eviction candidate.
    let ui_write_keys: Vec<String> = ui_writes.into_iter().map(|(k, _)| k).collect();
    ui_session
        .handle_event(WebBridgeEvent::ConfirmLocalStorage { writes: ui_write_keys, removals: Vec::new(), succeeded: true })
        .expect("confirm without removals restores mirror candidate");
    assert!(
        ui_session.storage.list_autosave_doc_ids().contains(&String::from("doc-ui-old")),
        "unremoved eviction candidate doc-ui-old should remain in mirror"
    );

    // Verify index.html sends confirm_local_storage.
    let html = include_str!("../index.html");
    assert!(html.contains("confirm_local_storage"));
}

#[test]
fn audit_123_4_explicit_sheet_targeting_validation_and_clipboard_isolation() {
    let (mut session, data_id, graph_id) = build_populated_xy_session("doc-sheet-target", "tab-target").expect("populated session");

    // Create a second independent DataSheet (`data_2`) and GraphSheet (`graph_2`) plus a ResultSheet (`res_1`).
    let created_2 = session
        .host
        .execute_command("sheet.createData", json!({ "kind": "xy", "title": "Second Sheet", "auto_family": true }))
        .expect("create second xy sheet");
    let data_2_id = created_2["payload"]["sheet_id"].as_str().expect("second data_id").to_owned();
    let graph_2_id = created_2["payload"]["graph_sheet_id"].as_str().expect("second graph_id").to_owned();
    let info_only_id = created_2["payload"]["info_sheet_id"].as_str().expect("second info_id").to_owned();
    session.host.execute_command("table.setXCells", json!({ "sheet_id": data_2_id, "values": [10.0, 20.0] })).expect("set x2");
    session.host.execute_command("table.setCell", json!({ "sheet_id": data_2_id, "row": 0, "col": 0, "value": 555.0 })).expect("set y2");
    session.host.execute_command("info.setNote", json!({ "sheet_id": data_2_id, "text": "Study Notes Line" })).expect("set info note");
    session.host.execute_command("layout.create", json!({ "sheet_id": "layout-only", "title": "Summary Layout" })).expect("create layout sheet");
    session
        .host
        .execute_command("analysis.run", json!({ "source_sheet_id": data_id, "sheet_id": "res-only", "kind": "linear_regression" }))
        .expect("create result sheet");

    // 1. Export CSV with nonexistent or GraphSheet ID returns WebAdapterError::Validation (no silent fallback).
    let bad_csv = session
        .handle_event(WebBridgeEvent::ExportFile { format: curvebook_web::WebExportFormat::Csv, sheet_id: Some(graph_id.clone()), dpi: 300 })
        .expect_err("CSV export targeting GraphSheet must fail");
    assert!(matches!(bad_csv, WebAdapterError::Validation(_)));

    let missing_csv = session
        .handle_event(WebBridgeEvent::ExportFile {
            format: curvebook_web::WebExportFormat::Csv,
            sheet_id: Some(String::from("no-such-sheet")),
            dpi: 300,
        })
        .expect_err("CSV export targeting unknown sheet must fail");
    assert!(matches!(missing_csv, WebAdapterError::Validation(_)));

    // Export figure (SVG / PNG) targeting DataSheet or unknown sheet returns WebAdapterError::Validation.
    for fmt in [curvebook_web::WebExportFormat::Svg, curvebook_web::WebExportFormat::Png] {
        let wrong_kind = session
            .handle_event(WebBridgeEvent::ExportFile { format: fmt, sheet_id: Some(data_id.clone()), dpi: 300 })
            .expect_err("figure export targeting DataSheet must fail");
        assert!(matches!(wrong_kind, WebAdapterError::Validation(_)));

        let missing_fig = session
            .handle_event(WebBridgeEvent::ExportFile { format: fmt, sheet_id: Some(String::from("nonexistent")), dpi: 300 })
            .expect_err("figure export targeting missing sheet must fail");
        assert!(matches!(missing_fig, WebAdapterError::Validation(_)));
    }

    // Valid explicit target succeeds even when another sheet is active.
    session.host.execute_command("sheet.select", json!({ "sheet_id": graph_2_id })).expect("select graph_2");
    let valid_csv = session
        .handle_event(WebBridgeEvent::ExportFile { format: curvebook_web::WebExportFormat::Csv, sheet_id: Some(data_id.clone()), dpi: 300 })
        .expect("explicit DataSheet CSV succeeds");
    let Some(WebBridgeCommand::SaveOrDownloadFile { bytes: csv_bytes, .. }) = valid_csv.into_iter().next() else {
        panic!("expected CSV SaveOrDownloadFile");
    };
    let csv_text = String::from_utf8(csv_bytes).expect("utf8 csv");
    assert!(csv_text.contains("49.5"));
    assert!(!csv_text.contains("555"));

    // 2. CopyClipboard validation & isolation.
    let bad_clip = session
        .handle_event(WebBridgeEvent::CopyClipboard { sheet_id: Some(String::from("bogus-sheet")), dpi: 300 })
        .expect_err("CopyClipboard with unknown sheet_id must return Clipboard error");
    assert!(matches!(bad_clip, WebAdapterError::Clipboard(_)));

    // Explicit DataSheet vs active DataSheet produce IDENTICAL payloads (including active selection bounds, and NO unrelated graph PNG).
    session.host.execute_command("sheet.select", json!({ "sheet_id": data_id })).expect("select data_id");
    session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("ArrowDown"), modifiers: KeyModifiers::default() } })
        .expect("select single grid cell via ArrowDown");
    assert!(session.host.app().grid.tool.selection_bounds().is_some(), "ArrowDown should arm grid selection bounds");
    let active_data_cmds = session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: None, dpi: 300 }).expect("copy active data");
    let explicit_data_cmds =
        session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: Some(data_id.clone()), dpi: 300 }).expect("copy explicit data");
    let Some(WebBridgeCommand::WriteClipboardItems { payload: active_data_clip }) = active_data_cmds.into_iter().next() else {
        panic!("expected WriteClipboardItems");
    };
    let Some(WebBridgeCommand::WriteClipboardItems { payload: explicit_data_clip }) = explicit_data_cmds.into_iter().next() else {
        panic!("expected WriteClipboardItems");
    };
    assert_eq!(active_data_clip, explicit_data_clip, "Some(active_id) and None must align identically");
    assert!(active_data_clip.tsv.is_some());
    assert!(active_data_clip.svg.is_empty(), "DataSheet clipboard must not pull unrelated GraphSheet SVG");
    assert!(active_data_clip.png_bytes.is_empty(), "DataSheet clipboard must not pull unrelated GraphSheet PNG");

    // LayoutSheet, ResultSheet, and InfoSheet payloads reflect ONLY that sheet (never unrelated DataSheet/GraphSheet).
    let layout_cmds =
        session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: Some(String::from("layout-only")), dpi: 150 }).expect("copy layout");
    let Some(WebBridgeCommand::WriteClipboardItems { payload: layout_clip }) = layout_cmds.into_iter().next() else {
        panic!("expected layout WriteClipboardItems");
    };
    assert!(layout_clip.tsv.is_none(), "LayoutSheet clipboard must not include unrelated DataSheet TSV");
    assert!(layout_clip.plain_text.is_none());
    assert!(!layout_clip.svg.is_empty());

    let result_cmds =
        session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: Some(String::from("res-only")), dpi: 150 }).expect("copy result");
    let Some(WebBridgeCommand::WriteClipboardItems { payload: result_clip }) = result_cmds.into_iter().next() else {
        panic!("expected result WriteClipboardItems");
    };
    assert!(result_clip.tsv.is_some());
    assert!(result_clip.svg.is_empty());
    assert!(result_clip.png_bytes.is_empty());

    let info_cmds = session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: Some(info_only_id), dpi: 150 }).expect("copy info");
    let Some(WebBridgeCommand::WriteClipboardItems { payload: info_clip }) = info_cmds.into_iter().next() else {
        panic!("expected info WriteClipboardItems");
    };
    assert!(info_clip.tsv.as_deref().is_some_and(|t| t.contains("Study Notes Line")));
    assert!(info_clip.svg.is_empty());
    assert!(info_clip.png_bytes.is_empty());

    // Orphan GraphSheet (missing primary_source_sheet_id) does NOT fall back to an unrelated DataSheet for TSV.
    let doc_bytes = session
        .host
        .snapshot_virtual_entries_with_root(Some("tmp.curvebook.d"))
        .expect("snapshot entries")
        .into_iter()
        .find(|(path, _)| path == "document.json")
        .map(|(_, bytes)| bytes)
        .expect("document.json");
    let mut doc_json: curvebook_ui_egui::serde_json::Value = curvebook_ui_egui::serde_json::from_slice(&doc_bytes).expect("parse document.json");
    let mut cloned_graph = doc_json["graph_sheets"][0].clone();
    cloned_graph["id"] = json!("graph-orphan");
    cloned_graph["title"] = json!("Orphan Graph");
    cloned_graph["spec"]["primary_source_sheet_id"] = json!("missing-source-id");
    doc_json["graph_sheets"].as_array_mut().expect("graph_sheets array").push(cloned_graph);
    let updated_doc_bytes = curvebook_ui_egui::serde_json::to_vec(&doc_json).expect("encode updated document.json");
    session.host.open_project_json_bytes(&updated_doc_bytes).expect("load document with orphan graph");

    let orphan_cmds =
        session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: Some(String::from("graph-orphan")), dpi: 150 }).expect("copy orphan graph");
    let Some(WebBridgeCommand::WriteClipboardItems { payload: orphan_clip }) = orphan_cmds.into_iter().next() else {
        panic!("expected WriteClipboardItems");
    };
    assert!(orphan_clip.tsv.is_none(), "orphan GraphSheet must not borrow unrelated DataSheet TSV");
    assert!(!orphan_clip.svg.is_empty());
}

#[test]
fn audit_123_5_dropped_files_and_json_directory_import_reject_duplicate_paths() {
    let mut session = WebBridgeSession::with_ids("web-dup-drop", "tab-dup");
    let mut host_copy = WebWorkspaceHost::new();
    let _ = host_copy.execute_command("sheet.createData", json!({ "kind": "xy", "title": "Clean" })).expect("create sheet");
    let valid_entries: BTreeMap<String, Vec<u8>> =
        host_copy.snapshot_virtual_entries_with_root(Some("assay.curvebook.d")).expect("snapshot entries").into_iter().collect();
    let doc_bytes = valid_entries.get("document.json").expect("doc bytes").clone();

    // 1. ImportDroppedFiles rejects duplicate normalized relative paths (`./assay.curvebook.d/...` vs `assay.curvebook.d/...`, backslashes, or root-stripped vs unstripped).
    let dup_drop_err = session
        .handle_event(WebBridgeEvent::ImportDroppedFiles {
            files: vec![
                (String::from("./assay.curvebook.d/document.json"), doc_bytes.clone()),
                (String::from("assay.curvebook.d/document.json"), doc_bytes.clone()),
            ],
        })
        .expect_err("dropped folder with duplicate normalized paths must fail");
    assert!(matches!(dup_drop_err, WebAdapterError::Validation(ref msg) if msg.contains("duplicate normalized")), "got {dup_drop_err}");

    let dup_slash_err = session
        .handle_event(WebBridgeEvent::ImportDroppedFiles {
            files: vec![
                (String::from("assay.curvebook.d\\document.json"), doc_bytes.clone()),
                (String::from("assay.curvebook.d/document.json"), doc_bytes),
            ],
        })
        .expect_err("dropped folder with backslash/forward-slash duplicate must fail");
    assert!(matches!(dup_slash_err, WebAdapterError::Validation(ref msg) if msg.contains("duplicate normalized")), "got {dup_slash_err}");

    // Also test ImportDroppedFiles with `./manifest.json` vs `manifest.json` in a flat batch.
    let dup_flat_err = session
        .handle_event(WebBridgeEvent::ImportDroppedFiles {
            files: vec![(String::from("./manifest.json"), b"{}".to_vec()), (String::from("manifest.json"), b"{}".to_vec())],
        })
        .expect_err("flat duplicate normalized paths must fail");
    assert!(matches!(dup_flat_err, WebAdapterError::Validation(_)));

    // 2. Raw JSON `import_directory_entries` with duplicate exact or normalized paths is rejected before map construction.
    let raw_dup_exact = r#"{"kind":"import_directory_entries","root_name":"study.curvebook.d","entries":[{"path":"document.json","bytes":[123,125]},{"path":"document.json","bytes":[91,93]}]}"#;
    let err_raw_exact = session.handle_event_json(raw_dup_exact).expect_err("raw JSON duplicate array entries must fail");
    assert!(matches!(err_raw_exact, WebAdapterError::Validation(ref msg) if msg.contains("duplicate")), "got {err_raw_exact}");

    let raw_dup_norm = r#"{"kind":"import_directory_entries","root_name":"study.curvebook.d","entries":[{"path":"./manifest.json","bytes":[123,125]},{"path":"manifest.json","bytes":[123,125]}]}"#;
    let err_raw_norm = session.handle_event_json(raw_dup_norm).expect_err("raw JSON duplicate normalized paths must fail");
    assert!(matches!(err_raw_norm, WebAdapterError::Validation(ref msg) if msg.contains("duplicate normalized")), "got {err_raw_norm}");

    let raw_dup_root = r#"{"kind":"import_directory_entries","root_name":"study.curvebook.d","entries":[{"path":"study.curvebook.d/manifest.json","bytes":[123,125]},{"path":"manifest.json","bytes":[123,125]}]}"#;
    let err_raw_root = session.handle_event_json(raw_dup_root).expect_err("raw JSON root-prefixed duplicate must fail");
    assert!(matches!(err_raw_root, WebAdapterError::Validation(ref msg) if msg.contains("duplicate normalized")), "got {err_raw_root}");
}

#[test]
fn audit_123_6_pinch_after_one_finger_press_cancels_armed_pointer_and_drags() {
    let (mut session, data_id, _graph_id) = build_populated_xy_session("touch-pinch-cancel", "tab-pinch").expect("populated touch session");
    session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 2.0 })
        .expect("resize viewport");
    session
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": data_id }) })
        .expect("select data sheet");

    // 1. One-finger press arms primary pointer and enters active drag states on the app.
    let first_finger = vec![TouchPoint { id: 1, x: 420.0, y: 260.0 }];
    session.handle_event(WebBridgeEvent::TouchStart { points: first_finger, dt_sec: 0.016 }).expect("1-finger touch start");
    assert!(session.touch.has_primary_pointer_down());
    assert!(session.host.app().interaction.pointer_down, "single-finger press should arm pointer_down");

    // Simulate an in-flight drag armed during the 1-finger press (e.g., sheet drag / split divider / canvas drag).
    session.host.app_mut().interaction.dragging_grid_sheet = Some(data_id.clone().into());
    session.host.app_mut().interaction.dragging_split_divider = true;
    session.host.app_mut().split.dragging_divider = true;
    session.host.app_mut().interaction.pressed_widget_id = Some(String::from("toolbar.save"));
    session.host.app_mut().canvas_dialogs.canvas.drag_start_pt = Some(curvebook_ui_egui::PointPt { x: 100.0, y: 100.0 });

    // 2. Second finger lands (transition to 2-finger pinch) -> must cancel armed primary pointer + all drag states.
    let two_fingers = vec![TouchPoint { id: 1, x: 420.0, y: 260.0 }, TouchPoint { id: 2, x: 520.0, y: 260.0 }];
    let pinch_start_cmds = session.handle_event(WebBridgeEvent::TouchStart { points: two_fingers, dt_sec: 0.016 }).expect("2-finger touch start");
    assert!(session.touch.is_two_finger_gesture());
    assert!(!session.touch.has_primary_pointer_down());
    assert!(!session.host.app().interaction.pointer_down, "2-finger pinch must clear pointer_down");
    assert!(session.host.app().interaction.dragging_grid_sheet.is_none());
    assert!(!session.host.app().interaction.dragging_split_divider);
    assert!(!session.host.app().split.dragging_divider);
    assert!(session.host.app().interaction.pressed_widget_id.is_none());
    assert!(session.host.app().canvas_dialogs.canvas.drag_start_pt.is_none());
    // Must not fire toolbar.save click or mutate selection/dirty via spurious click.
    assert!(!pinch_start_cmds.iter().any(|c| matches!(c, WebBridgeCommand::ApplyDirectorySync { .. })));

    // 3. Pinch move zooms viewport without re-arming pointer_down.
    let spread = vec![TouchPoint { id: 1, x: 380.0, y: 260.0 }, TouchPoint { id: 2, x: 560.0, y: 260.0 }];
    session.handle_event(WebBridgeEvent::TouchMove { points: spread, dt_sec: 0.016 }).expect("pinch spread");
    assert!(!session.host.app().interaction.pointer_down);

    // 4. Lift BOTH fingers (even sequentially) -> zero active touches, zero primary pointer down, zero drags.
    session.handle_event(WebBridgeEvent::TouchEnd { released_ids: vec![2], dt_sec: 0.016 }).expect("lift second finger");
    session.handle_event(WebBridgeEvent::TouchEnd { released_ids: vec![1], dt_sec: 0.016 }).expect("lift first finger");
    assert_eq!(session.touch.active_count(), 0);
    assert!(!session.touch.is_two_finger_gesture());
    assert!(!session.touch.has_primary_pointer_down());
    assert!(!session.host.app().interaction.pointer_down);
    assert!(session.host.app().interaction.dragging_grid_sheet.is_none());
    assert!(!session.host.app().interaction.dragging_split_divider);

    // 5. Verify direct TouchGestureRecognizer cancellation unit sequence:
    let mut rec = curvebook_web::TouchGestureRecognizer::new();
    let down_ev = rec.touch_start(&[TouchPoint { id: 10, x: 200.0, y: 200.0 }], 0.0);
    assert!(matches!(down_ev.as_slice(), [UiInputEvent::PointerDown { .. }]));
    let pinch_ev = rec.touch_start(&[TouchPoint { id: 10, x: 200.0, y: 200.0 }, TouchPoint { id: 11, x: 260.0, y: 200.0 }], 0.016);
    assert!(pinch_ev.iter().any(|ev| matches!(ev, UiInputEvent::PointerUp { x, y, .. } if *x < 0.0 && *y < 0.0)));
    assert!(!rec.has_primary_pointer_down());
}
