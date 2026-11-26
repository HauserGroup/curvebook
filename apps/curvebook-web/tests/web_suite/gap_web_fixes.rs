//! Regression tests for Gap Audit Section 5 (`GAP-05-01`, `GAP-05-03`, `GAP-05-04`).

use curvebook_ui_egui::{
    KeyModifiers, UiInputEvent,
    serde_json::{self, json},
};
use curvebook_web::{AUTOSAVE_KEY_PREFIX, TabAccessMode, WebAdapterError, WebBridgeCommand, WebBridgeEvent, WebBridgeSession, WebStorageBackend};

fn seed_active_session_with_autosave() -> Result<(WebBridgeSession, String, String, Vec<u8>), String> {
    let mut session = WebBridgeSession::with_ids("study-active", "tab-gap-05");
    let _ = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    let _ = session
        .handle_event(WebBridgeEvent::AcquireLock { doc_id: String::from("study-active"), tab_id: String::from("tab-gap-05"), now_ms: 1_000 })
        .map_err(|e| e.to_string())?;
    let _ = session.host.execute_command("sheet.createData", json!({ "kind": "xy", "title": "Valuable Study", "auto_family": true }))?;
    let data_id = session.host.app().document().data_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing data sheet")?;
    let _ = session.host.execute_command("table.setCell", json!({ "sheet_id": data_id, "row": 0, "col": 0, "subcol": 0, "value": 42.5 }))?;
    let _ = session.host.execute_command("table.setCell", json!({ "sheet_id": data_id, "row": 0, "col": 1, "subcol": 0, "value": 99.0 }))?;

    // Persist an autosave backup for the active document.
    let _ = session.handle_event(WebBridgeEvent::SaveAutosave { now_ms: 2_000 }).map_err(|e| e.to_string())?;
    let doc_id = session.doc_id().to_owned();
    let autosave_key = format!("{AUTOSAVE_KEY_PREFIX}{doc_id}");
    let backup = session.storage.backend().get_item(&autosave_key).ok_or("missing seeded autosave backup")?;
    let doc_snapshot = serde_json::to_vec(session.host.app().document()).map_err(|e| e.to_string())?;
    Ok((session, doc_id, backup, doc_snapshot))
}

#[test]
fn gap_05_01_failed_non_utf8_csv_or_tsv_open_preserves_session_lock_and_autosave() {
    let (mut session, doc_id, backup_before, doc_before) = seed_active_session_with_autosave().expect("seed session");
    let autosave_key = format!("{AUTOSAVE_KEY_PREFIX}{doc_id}");
    let invalid_utf8 = vec![0xff, 0xfe, 0x80, b',', b'1', b'\n'];

    // 1. OpenFileBytes with non-UTF-8 CSV fails atomically without replacing Session, doc_id, or lock.
    let err = session
        .handle_event(WebBridgeEvent::OpenFileBytes { file_name: String::from("broken.csv"), bytes: invalid_utf8.clone() })
        .expect_err("non-UTF-8 CSV must fail");
    assert!(matches!(err, WebAdapterError::Workspace(ref msg) if msg.contains("UTF-8")), "expected UTF-8 Workspace error, got {err:?}");
    assert_eq!(session.doc_id(), doc_id);
    assert_eq!(session.access_mode(), &TabAccessMode::ExclusiveReadWrite);
    assert_eq!(serde_json::to_vec(session.host.app().document()).expect("doc after failed open"), doc_before);
    assert_eq!(session.storage.backend().get_item(&autosave_key).as_deref(), Some(backup_before.as_str()));

    // 2. ImportDroppedFiles single non-UTF-8 TSV also fails atomically.
    let drop_err = session
        .handle_event(WebBridgeEvent::ImportDroppedFiles { files: vec![(String::from("broken.tsv"), invalid_utf8)] })
        .expect_err("non-UTF-8 dropped TSV must fail");
    assert!(matches!(drop_err, WebAdapterError::Workspace(ref msg) if msg.contains("UTF-8")), "got {drop_err:?}");
    assert_eq!(session.doc_id(), doc_id);
    assert_eq!(serde_json::to_vec(session.host.app().document()).expect("doc after failed drop"), doc_before);

    // 3. Subsequent PointerMove (`chrome.hover`) still renders the preserved session (never an empty project).
    let hover_cmds = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x: 24.0, y: 12.0, modifiers: KeyModifiers::default() } })
        .expect("pointer move succeeds");
    assert_eq!(session.host.app().document().data_sheets.len(), 1);
    if let Some(WebBridgeCommand::RenderRgbaFrame { chrome, .. }) = hover_cmds.first() {
        assert!(chrome.dirty, "preserved dirty state must stay intact");
    }

    // 4. Verify index.html gates scheduleAutosave on documentDirty and excludes passive chrome.hover.
    let html = super::common::INDEX_HTML;
    let (_, after_sched) = html.split_once("function scheduleAutosave(action)").expect("scheduleAutosave exists");
    let sched_slice: String = after_sched.chars().take(700).collect();
    assert!(sched_slice.contains("!documentDirty"), "scheduleAutosave must gate on documentDirty: {sched_slice}");
    assert!(sched_slice.contains("isPassiveNonMutatingAction(action)"), "scheduleAutosave must exclude passive actions: {sched_slice}");
    assert!(html.contains("action === \"chrome.hover\""), "chrome.hover must be explicitly excluded from autosave scheduling");
}

#[test]
fn gap_05_03_drop_handler_snapshots_all_handles_synchronously_before_first_await() {
    let html = super::common::INDEX_HTML;
    let (_, after_drop) = html.split_once("window.addEventListener(\"drop\", async (ev) => {").expect("drop listener present");
    let (drop_body, _) = after_drop.split_once("function releaseFallbackLockEntry").expect("end of drop listener");

    let (before_await, after_first_await) = drop_body.split_once("await ").expect("drop handler awaits async file reads");

    assert!(before_await.contains("dt.items"), "dt.items must be enumerated before first await");
    assert!(before_await.contains("webkitGetAsEntry"), "webkitGetAsEntry must run synchronously before first await");
    assert!(before_await.contains("getAsFile"), "getAsFile must run synchronously before first await");
    assert!(before_await.contains("dt.files"), "dt.files fallback must run synchronously before first await");
    assert!(!after_first_await.contains("dt.items"), "dt.items must never be read after first await");
    assert!(!after_first_await.contains("dt.files"), "dt.files must never be read after first await");
}

#[test]
fn gap_05_04_index_html_handles_wasm_trap_disables_autosave_and_shows_recovery_banner() {
    let html = super::common::INDEX_HTML;
    let css = include_str!("../../styles.css");

    assert!(html.contains("id=\"curvebook_recovery_banner\""), "recovery banner markup required");
    assert!(html.contains("id=\"curvebook_recovery_reload\""), "recovery reload button required");
    assert!(html.contains("WebAssembly.RuntimeError"), "must detect WebAssembly.RuntimeError");
    assert!(html.contains("wasm_session_poisoned"), "must detect poisoned RefCell dispatch notice");
    assert!(html.contains("enterWasmTrapRecovery"), "must enter trap recovery mode");
    assert!(html.contains("wasmTrapDetected"), "must latch wasmTrapDetected to disable autosave");
    assert!(!html.contains("<style>") && !html.contains(" style=\""), "recovery banner must preserve strict CSP with zero inline styles");
    assert!(css.contains("#curvebook_recovery_banner"), "recovery banner styling must live in styles.css");
}
