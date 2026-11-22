//! Regression tests for Wave 1 SEC-20 security & stability fixes (`FA-SEC20-01`..`03`).

use std::collections::BTreeMap;

use curvebook_format::PreferencesFile;
use curvebook_ui_egui::{
    KeyModifiers, PointerButton,
    serde_json::{self, json},
};
use curvebook_web::{
    AUTOSAVE_KEY_PREFIX, LayoutRectPx, MAX_FRAME_RGBA_BYTES, TabAccessMode, WebAdapterError, WebBridgeCommand, WebBridgeEvent, WebBridgeSession,
    WebClipboardPayload, WebLockCoordinator, WebRgbaLayer, WebTextBridge, ZLayer,
};

fn build_session_with_data_and_graph(doc_id: &str, tab_id: &str) -> Result<(WebBridgeSession, String, String), String> {
    let mut session = WebBridgeSession::with_ids(doc_id, tab_id);
    let _ = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    let _ = session.host.execute_command("sheet.createData", json!({ "kind": "xy", "title": "Study Data", "auto_family": true }))?;
    let data_id = session.host.app().document().data_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing data sheet")?;
    let _ = session.host.execute_command("table.setCell", json!({ "sheet_id": data_id, "row": 0, "col": 0, "subcol": 0, "raw": "1.0" }))?;
    let _ = session.host.execute_command("table.setCell", json!({ "sheet_id": data_id, "row": 0, "col": 1, "subcol": 0, "raw": "2.5" }))?;
    let graph_id = session.host.app().document().graph_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing auto graph sheet")?;
    Ok((session, data_id, graph_id))
}

fn export_seed_directory_tree(tab_id: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let (mut src, _data_id, _graph_id) = build_session_with_data_and_graph("seed", tab_id)?;
    src.export_directory_tree().map_err(|e| e.to_string())
}

fn seed_directory_session(root_name: &str, tab_id: &str) -> Result<(WebBridgeSession, BTreeMap<String, Vec<u8>>), String> {
    let tree = export_seed_directory_tree(tab_id)?;
    let mut session = WebBridgeSession::with_ids("initial", tab_id);
    let _ = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    let _ = session
        .handle_event(WebBridgeEvent::ImportDirectoryEntries { root_name: root_name.to_owned(), entries: tree.clone() })
        .map_err(|e| e.to_string())?;
    Ok((session, tree))
}

fn assert_dir_state_cleared(session: &WebBridgeSession, cmds: &[WebBridgeCommand]) {
    assert!(cmds.iter().any(|c| matches!(c, WebBridgeCommand::ClearDirectoryHandle)));
    assert!(cmds.iter().any(|c| matches!(c, WebBridgeCommand::SetDocumentLock { .. })));
    assert_eq!(session.fs.last_synced_root(), None);
    assert!(session.fs.last_synced_tree().is_empty());
    assert!(session.host.virtual_workspace().root_name.is_empty());
    assert!(session.host.virtual_workspace().files.is_empty());
}

#[test]
fn fa_sec20_01_file_open_clears_virtual_workspace_and_directory_handle() {
    let (mut session, alpha_tree) = seed_directory_session("alpha.curvebook.d", "tab-dir").expect("seed dir session");
    assert_eq!(session.fs.last_synced_root(), Some("alpha.curvebook.d"));
    assert!(!session.fs.last_synced_tree().is_empty());

    // 1. Opening CSV clears virtual_workspace, directory sync baseline, and emits ClearDirectoryHandle.
    let csv_cmds = session
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("plate.csv"), bytes: b"x,y\n1,2\n3,4\n".to_vec() })
        .expect("open delimited csv");
    assert!(matches!(csv_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    assert_dir_state_cleared(&session, &csv_cmds);

    let post_csv_sync = session.handle_event(WebBridgeEvent::SyncDirectory { root_name: None }).expect("post-csv sync plan");
    match post_csv_sync.as_slice() {
        [WebBridgeCommand::ApplyDirectorySync { root_name, plan }] => {
            assert_eq!(root_name, "workspace.curvebook.d", "must never fall back to prior alpha.curvebook.d root");
            assert!(plan.removals.is_empty(), "must never emit deletions against prior directory entries");
        }
        other => panic!("expected ApplyDirectorySync, got {other:?}"),
    }

    // 2. Re-importing directory then opening `.curvebook` archive also resets virtual_workspace & handle.
    let archive_bytes = session.export_curvebook_bytes().expect("export archive");
    session
        .handle_event(WebBridgeEvent::ImportDirectoryEntries { root_name: String::from("alpha.curvebook.d"), entries: alpha_tree })
        .expect("re-import alpha");
    let open_archive_cmds = session
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("fresh.curvebook"), bytes: archive_bytes })
        .expect("open .curvebook");
    assert_dir_state_cleared(&session, &open_archive_cmds);

    // 3. index.html enforces handle.name === rootName and wires per-tab docId & ClearDirectoryHandle.
    let html = include_str!("../index.html");
    assert!(html.contains("curvebook.v1.session_doc_id"));
    assert!(html.contains("initialDocId: activeLockDocId"));
    assert!(html.contains("case \"clear_directory_handle\":"));
    assert!(html.contains("case \"set_document_lock\":"));
    assert!(html.contains("activeDirectoryHandle.name !== rootName"));
}

#[test]
fn fa_sec20_01_project_replacement_commands_clear_virtual_workspace_and_directory_handle() {
    let (mut session, alpha_tree) = seed_directory_session("alpha.curvebook.d", "tab-dir-new").expect("seed dir session");

    let new_cmds =
        session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("project.new"), params: json!({}) }).expect("project.new");
    assert!(matches!(new_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { action: Some(a), .. }) if a == "project.new"));
    assert_dir_state_cleared(&session, &new_cmds);

    session
        .handle_event(WebBridgeEvent::ImportDirectoryEntries { root_name: String::from("alpha.curvebook.d"), entries: alpha_tree })
        .expect("re-import alpha");
    let file_new_cmds = session
        .handle_event(WebBridgeEvent::DispatchControlLine { line: json!({ "id": "n1", "method": "file.new", "params": {} }).to_string() })
        .expect("file.new control line");
    assert!(matches!(file_new_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { action: Some(a), .. }) if a == "file.new"));
    assert_dir_state_cleared(&session, &file_new_cmds);
}

#[test]
fn fa_sec20_01_control_line_flattened_and_padded_project_replacements_clear_dir_state() {
    let variants = [
        json!({ "id": "c3", "method": "command.run", "command": "project.new", "params": {} }),
        json!({ "id": "c4a", "method": "engine.execute", "params": { "command_id": "project.new", "params": {} } }),
        json!({ "id": "c4b", "method": "engine.execute", "command_id": "project.new", "params": { "params": {} } }),
        json!({ "id": "c5", "method": "command.batch", "params": {}, "commands": [{ "command": "project.new", "params": {} }] }),
        json!({ "id": "c6", "method": " project.new ", "params": {} }),
    ];
    for req in variants {
        let (mut session, _alpha_tree) = seed_directory_session("alpha.curvebook.d", "tab-dir-norm").expect("seed dir session");
        let prev_doc_id = session.doc_id().to_owned();
        let cmds = session.handle_event(WebBridgeEvent::DispatchControlLine { line: req.to_string() }).expect("replacement control line succeeds");
        assert_dir_state_cleared(&session, &cmds);
        assert_ne!(session.doc_id(), prev_doc_id, "project replacement `{req}` must rotate doc_id");
    }
}

#[test]
fn fa_sec20_01_distinct_files_and_projects_isolate_locks_and_autosaves_across_tabs() {
    let mut coord = WebLockCoordinator::new(15_000);
    let (mut src_a, _, _) = build_session_with_data_and_graph("seed-a", "tab-a").expect("src-a");
    let bytes_a = src_a.export_curvebook_bytes().expect("bytes a");

    let (mut src_b, _, _) = build_session_with_data_and_graph("seed-b", "tab-b").expect("src-b");
    let _ = src_b.host.execute_command("sheet.createData", json!({ "kind": "column", "title": "Different Dataset", "auto_family": false }));
    let bytes_b = src_b.export_curvebook_bytes().expect("bytes b");

    let mut tab_a = WebBridgeSession::with_ids("untitled-tab-a", "tab-a");
    let mut tab_b = WebBridgeSession::with_ids("untitled-tab-b", "tab-b");

    tab_a.handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("a.curvebook"), bytes: bytes_a.clone() }).expect("tab-a open a");
    tab_b.handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("b.curvebook"), bytes: bytes_b }).expect("tab-b open b");

    assert_ne!(tab_a.doc_id(), tab_b.doc_id(), "distinct files must yield distinct lock/autosave doc_ids");
    assert_ne!(tab_a.doc_id(), "default");
    assert_ne!(tab_b.doc_id(), "default");

    let (acq_a, _) = tab_a.acquire_lock_with(&mut coord, 1_000).expect("acquire a");
    let (acq_b, _) = tab_b.acquire_lock_with(&mut coord, 1_100).expect("acquire b");
    assert_eq!(acq_a.mode, TabAccessMode::ExclusiveReadWrite);
    assert_eq!(acq_b.mode, TabAccessMode::ExclusiveReadWrite);

    let save_a = tab_a.handle_event(WebBridgeEvent::SaveAutosave { now_ms: 1_200 }).expect("autosave a");
    let save_b = tab_b.handle_event(WebBridgeEvent::SaveAutosave { now_ms: 1_300 }).expect("autosave b");
    let key_a = match save_a.as_slice() {
        [WebBridgeCommand::PersistLocalStorage { writes, .. }] => writes[0].0.clone(),
        other => panic!("unexpected autosave a: {other:?}"),
    };
    let key_b = match save_b.as_slice() {
        [WebBridgeCommand::PersistLocalStorage { writes, .. }] => writes[0].0.clone(),
        other => panic!("unexpected autosave b: {other:?}"),
    };
    assert!(key_a.starts_with(AUTOSAVE_KEY_PREFIX));
    assert!(key_b.starts_with(AUTOSAVE_KEY_PREFIX));
    assert_ne!(key_a, key_b);

    // Two tabs opening the SAME file share doc_id and contend; switching tab_c to another file exits ReadOnlyContended.
    let mut tab_c = WebBridgeSession::with_ids("untitled-tab-c", "tab-c");
    tab_c.handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("a.curvebook"), bytes: bytes_a }).expect("tab-c open a");
    assert_eq!(tab_c.doc_id(), tab_a.doc_id());
    let (acq_c, _) = tab_c.acquire_lock_with(&mut coord, 1_400).expect("acquire c");
    assert!(matches!(acq_c.mode, TabAccessMode::ReadOnlyContended { ref holder_tab_id } if holder_tab_id == "tab-a"));

    let switch_cmds = tab_c
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("local.csv"), bytes: b"a,b\n1,2\n".to_vec() })
        .expect("contended tab may switch to a different file");
    assert_eq!(tab_c.access_mode(), &TabAccessMode::ExclusiveReadWrite);
    assert!(switch_cmds.iter().any(|c| matches!(c, WebBridgeCommand::SetDocumentLock { doc_id } if doc_id == tab_c.doc_id())));
    assert!(switch_cmds.iter().any(|c| matches!(c, WebBridgeCommand::SetReadOnlyBanner { read_only: false, .. })));

    let prev_id = tab_a.doc_id().to_owned();
    let _ =
        tab_a.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("project.new"), params: json!({}) }).expect("tab-a project.new");
    assert_ne!(tab_a.doc_id(), prev_id);
    assert!(tab_a.doc_id().starts_with("untitled-"));
}

#[test]
fn fa_sec20_02_contended_read_only_blocks_ui_mutations_command_batch_run_and_preferences() {
    let mut coord = WebLockCoordinator::new(15_000);
    let (mut tab_a, data_id, graph_id) = build_session_with_data_and_graph("seed-ro", "tab-a").expect("tab-a setup");
    let archive_bytes = tab_a.export_curvebook_bytes().expect("archive bytes");
    tab_a
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("shared.curvebook"), bytes: archive_bytes.clone() })
        .expect("tab-a open shared");
    let _ = tab_a.acquire_lock_with(&mut coord, 1_000).expect("tab-a lock");

    let mut tab_b = WebBridgeSession::with_ids("untitled-b", "tab-b");
    tab_b.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 }).expect("resize");
    tab_b
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("shared.curvebook"), bytes: archive_bytes })
        .expect("tab-b open shared");
    let (acq_b, _) = tab_b.acquire_lock_with(&mut coord, 1_100).expect("tab-b contend");
    assert_eq!(acq_b.mode, TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("tab-a") });

    let doc_before = serde_json::to_vec(tab_b.host.app().document()).expect("doc snapshot before");

    let blocked_commands = [
        ("graph.applyCanvasPreset", json!({ "sheet_id": graph_id, "preset": "single_column_88mm" })),
        ("layout.applyCanvasPreset", json!({ "preset": "double_column_180mm" })),
        ("ui.canvas.applyPreset:single_column_88mm", json!({ "preset": "single_column_88mm" })),
        ("ui.canvas.applyPreset", json!({ "preset": "double_column_180mm" })),
        ("ui.grid.excludeWithReason", json!({ "reason": "outlier" })),
        ("ui.result.appendClusterColumn", json!({})),
        ("ui.event.dispatch", json!({})),
        ("export.svg", json!({ "sheet_id": graph_id, "path": "/tmp/should-not-write.svg" })),
        ("analysis.exportJson", json!({ "sheet_id": data_id, "path": "/tmp/should-not-write.json" })),
    ];
    for (cmd_id, params) in blocked_commands {
        let err = tab_b
            .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from(cmd_id), params })
            .expect_err("contended ExecuteCommand must reject mutation");
        assert!(matches!(err, WebAdapterError::ReadOnlyContended { .. }), "expected ReadOnlyContended for `{cmd_id}`, got {err:?}");
    }

    let toolbar_new_err = tab_b
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.new"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .expect_err("toolbar.new must reject when contended");
    assert!(matches!(toolbar_new_err, WebAdapterError::ReadOnlyContended { .. }));

    let blocked_control_lines = [
        json!({
            "id": "b1",
            "method": "command.batch",
            "params": {
                "commands": [
                    { "command": "doc.inspect", "params": {} },
                    { "command": "sheet.rename", "params": { "sheet_id": data_id, "title": "Mutated" } }
                ]
            }
        }),
        json!({
            "id": "b2",
            "method": "command.batch",
            "params": { "commands": [{ "command": "ui.canvas.applyPreset", "params": { "preset": "single_column_88mm" } }] }
        }),
        json!({
            "id": "r1",
            "method": "command.run",
            "params": { "command_id": "sheet.rename", "params": { "sheet_id": data_id, "title": "Mutated Via Run" } }
        }),
        json!({
            "id": "r-flat-path",
            "method": "command.run",
            "command": "export.svg",
            "sheet_id": graph_id,
            "path": "/tmp/flat-evil.svg"
        }),
        json!({
            "id": "r-precedence",
            "method": "command.run",
            "command": "table.paste",
            "params": { "command": "graph.preflightCheck", "sheet_id": data_id, "text": "999" }
        }),
        json!({
            "id": "e-flat-path",
            "method": "export.svg",
            "path": "/tmp/top-evil.svg",
            "params": { "sheet_id": graph_id }
        }),
        json!({
            "id": "m-padded",
            "method": " sheet.rename ",
            "params": { "sheet_id": data_id, "title": "Padded Bypass" }
        }),
        json!({
            "id": "m-batch-top",
            "method": "command.batch",
            "params": {},
            "commands": [{ "command": "sheet.rename", "params": { "sheet_id": data_id, "title": "Batch Sibling Bypass" } }]
        }),
        json!({
            "id": "b-flat-path",
            "method": "command.batch",
            "params": { "commands": [{ "command": "export.svg", "sheet_id": graph_id, "path": "/tmp/batch-evil.svg" }] }
        }),
    ];
    for line in blocked_control_lines {
        let err = tab_b
            .handle_event(WebBridgeEvent::DispatchControlLine { line: line.to_string() })
            .expect_err("mutating command.batch/command.run must reject when contended");
        assert!(matches!(err, WebAdapterError::ReadOnlyContended { .. }), "got {err:?}");
    }

    let pref_err = tab_b
        .handle_event(WebBridgeEvent::SavePreferences { preferences: PreferencesFile::default() })
        .expect_err("SavePreferences must reject when contended");
    assert!(matches!(pref_err, WebAdapterError::ReadOnlyContended { .. }));

    // View-only UI commands work locally without persisting shared preferences to localStorage.
    let theme_cmds = tab_b
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.theme.set"), params: json!({ "theme": "dark" }) })
        .expect("local theme toggle allowed in read-only");
    assert!(!theme_cmds.iter().any(|c| matches!(c, WebBridgeCommand::PersistLocalStorage { .. })), "contended tab must never persist preferences");

    let locale_cmds = tab_b
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.locale.set"), params: json!({ "locale": "de-DE" }) })
        .expect("local locale switch allowed in read-only");
    assert!(!locale_cmds.iter().any(|c| matches!(c, WebBridgeCommand::PersistLocalStorage { .. })), "contended tab must never persist preferences");

    // Read-only `command.batch` and `command.run` remain permitted (without persisting preferences).
    let ro_batch = json!({
        "id": "ok-b",
        "method": "command.batch",
        "params": {
            "commands": [
                { "command": "graph.preflightCheck", "params": { "sheet_id": graph_id } },
                { "command": "ui.rulers.toggle", "params": { "visible": true } }
            ]
        }
    });
    let batch_cmds = tab_b.handle_event(WebBridgeEvent::DispatchControlLine { line: ro_batch.to_string() }).expect("read-only command.batch allowed");
    assert!(matches!(batch_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    assert!(!batch_cmds.iter().any(|c| matches!(c, WebBridgeCommand::PersistLocalStorage { .. })));

    let ro_run =
        json!({ "id": "ok-r", "method": "command.run", "params": { "command": "graph.preflightCheck", "params": { "sheet_id": graph_id } } });
    let run_cmds = tab_b.handle_event(WebBridgeEvent::DispatchControlLine { line: ro_run.to_string() }).expect("read-only command.run allowed");
    assert!(matches!(run_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));

    let doc_after = serde_json::to_vec(tab_b.host.app().document()).expect("doc snapshot after");
    assert_eq!(doc_before, doc_after, "no contended read-only attempt may mutate document state");
}

#[test]
fn fa_sec20_03_bounded_rgba_allocation_and_checked_json_integer_conversions() {
    let mut session = WebBridgeSession::with_ids("rgba-bounds", "tab-bounds");
    session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .expect("valid initial viewport");

    let err = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 16_384.0, logical_height: 16_384.0, scale_factor: 1.0 })
        .expect_err("oversized 16384x16384 viewport must fail cleanly");
    assert!(matches!(err, WebAdapterError::Validation(_)));
    let vp = session.host.viewport();
    assert_eq!(vp.physical_width_px(), 1280);
    assert_eq!(vp.physical_height_px(), 800);

    let frame_err = WebTextBridge::try_compose_placeholder_frame_with_popup(16_384, 16_384, 96, [255, 255, 255, 255], "too large", false, false)
        .expect_err("oversized placeholder frame must fail cleanly");
    assert!(matches!(frame_err, WebAdapterError::Validation(_)));

    let overlay_budget_err = WebTextBridge::try_compose_placeholder_frame_with_popup(4_096, 4_096, 96, [0, 0, 0, 255], "overlay budget", true, true)
        .expect_err("base + overlay exceeding MAX_FRAME_RGBA_BYTES must fail cleanly");
    assert!(matches!(overlay_budget_err, WebAdapterError::Validation(_)));

    let clamped = WebTextBridge::compose_placeholder_frame_with_popup(16_384, 16_384, 96, [255, 255, 255, 255], "clamped", false, false);
    assert!(clamped.base_rgba_bytes.len() <= MAX_FRAME_RGBA_BYTES);
    WebTextBridge::validate_layered_frame(&clamped).expect("clamped frame remains valid");

    assert!(matches!(
        LayoutRectPx::from_json_value(&json!({ "x": 4_294_967_297_i64, "y": 10, "width": 100, "height": 40 })),
        Err(WebAdapterError::Serialization(_))
    ));
    assert!(matches!(LayoutRectPx::from_json_value(&json!({ "x": 0, "y": 0, "width": -1, "height": 40 })), Err(WebAdapterError::Serialization(_))));
    assert!(matches!(LayoutRectPx::from_json_value(&json!({ "x": 0, "width": 10, "height": 10 })), Err(WebAdapterError::Serialization(_))));

    assert!(matches!(
        WebBridgeCommand::from_json_value(&json!({
            "kind": "render_rgba_frame",
            "width_px": 4_294_967_297_u64,
            "height_px": 10,
            "rgba_base64": ""
        })),
        Err(WebAdapterError::Serialization(_))
    ));
    assert!(matches!(
        WebBridgeCommand::from_json_value(&json!({
            "kind": "render_rgba_frame",
            "width_px": 16_384,
            "height_px": 16_384,
            "rgba_base64": ""
        })),
        Err(WebAdapterError::Validation(_))
    ));
    assert!(matches!(
        WebBridgeCommand::from_json_value(&json!({
            "kind": "apply_directory_sync",
            "root_name": "a.curvebook.d",
            "writes": [],
            "removals": [],
            "unchanged_count": -5
        })),
        Err(WebAdapterError::Serialization(_))
    ));
    assert!(matches!(WebClipboardPayload::from_json_value(&json!({ "width_px": 4_294_967_297_u64 })), Err(WebAdapterError::Serialization(_))));
    assert!(matches!(
        WebRgbaLayer::from_json_value(&json!({
            "z_layer": ZLayer::Workspace.as_str(),
            "rect_px": { "x": 0, "y": 0, "width": 16_384, "height": 16_384 },
            "width_px": 16_384,
            "height_px": 16_384,
            "rgba_base64": ""
        })),
        Err(WebAdapterError::Validation(_))
    ));

    let html = include_str!("../index.html");
    assert!(html.contains("const maxPixels = 8000000;"), "HiDPI pixel clamp must reserve modal+popup headroom under MAX_FRAME_RGBA_BYTES");
    let picker_pos = html.find("async function openDirectoryPickerTree()").expect("openDirectoryPickerTree present");
    let picker_end = html[picker_pos..].find("async function collectDroppedEntry").expect("collectDroppedEntry present");
    let picker_slice = &html[picker_pos..picker_pos + picker_end];
    assert!(
        picker_slice.contains("set_document_lock") && picker_slice.contains("activeDirectoryHandle = dirHandle;"),
        "openDirectoryPickerTree must bind activeDirectoryHandle only after import_directory_entries succeeds"
    );
}
