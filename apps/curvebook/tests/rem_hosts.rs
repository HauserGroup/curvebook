//! Wave 2 remediation regression suite for `curvebook` desktop host (`fix-hosts`).
//!
//! Covers `AUD-06-01`, `AUD-06-03`, `AUD-06-04`, `AUD-06-06`, `AUD-06-07`, `AUD-06-10`,
//! `AUD-06-12`, `AUD-06-13`, `AUD-06-14`, `AUD-13-01`, `AUD-13-09`, `AUD-13-11`, `AUD-13-12`,
//! `AUD-13-21`, and `AUD-13-23`.

use std::fs;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, NativeFileDialogRequest,
    dir_has_unrecovered_crash, execute_native_import_at_path_with_params, open_dir_with_policy_safe_recovery, open_fresh_journal_for_seeded_session,
};
use curvebook::gui_window::{CurveBookNativeApp, LiveControlChannel, LiveControlRequest};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ColumnDesignHint, ColumnTable, DataSheet, LinearRegressionSpec, ProjectDocument, ResultCell, ResultSheet,
    ResultStatus, ResultTab, ResultTabKind, TableArchetype,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::recovery::{CrashRecoveryJournal, DirectoryJournalBackend};
use curvebook_format::save_project_file;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{ControlAuth, UiApp};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!("cb-rem-hosts-desktop-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug)]
struct FixedPickAdapter(PathBuf);

impl NativeFileDialogAdapter for FixedPickAdapter {
    fn pick(&self, _request: &NativeFileDialogRequest) -> Result<Option<PathBuf>, String> {
        Ok(Some(self.0.clone()))
    }
}

fn build_sample_xlsx_bytes() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut zw = curvebook_format::zip::ZipWriter::new();
    zw.add_file(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#
            .to_vec(),
    )?;
    zw.add_file(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/workbook.xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="First" sheetId="1" r:id="rId1"/>
    <sheet name="Second" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/_rels/workbook.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/worksheets/sheet1.xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>A</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c></row>
  </sheetData>
</worksheet>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/worksheets/sheet2.xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>B</t></is></c></row>
    <row r="2"><c r="A2"><v>42</v></c></row>
  </sheetData>
</worksheet>"#
            .to_vec(),
    )?;
    Ok(zw.finish()?)
}

#[test]
fn aud_06_01_human_path_only_isolates_os_clipboard() -> TestResult {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { human_path_only: true, recovery_root: None, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, ViewportMetrics::default())?;
    rt.window_mut(win_id)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "d1", "kind": "Column", "rows": 2, "cols": 1 }))?;
    rt.window_mut(win_id)?.execute_and_journal("table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 99.0 }))?;

    let copy = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: win_id })?;
    let os_clip = copy.command_result.as_ref().and_then(|v| v.get("os_clipboard")).ok_or("missing os_clipboard")?;
    assert_eq!(os_clip["available"], json!(false), "human_path_only must isolate OS clipboard writes: {os_clip}");
    assert!(rt.clipboard_text().is_some(), "in-process clipboard cache must still work");

    let paste = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: win_id, text_override: None })?;
    assert!(paste.handled, "in-process clipboard paste should still succeed via cached text");
    Ok(())
}

#[test]
fn aud_06_04_secondary_windows_hydrate_and_sync_preferences() -> TestResult {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let persisted = Arc::new(curvebook_platform::sync::Mutex::named("test.prefs", Vec::<(String, UiThemeMode)>::new()));
    let persisted_hook = Arc::clone(&persisted);

    rt.set_preferences_init_hook(|app: &mut UiApp| {
        let _ = app.set_locale("de");
        app.set_theme_mode(UiThemeMode::Dark);
    });
    rt.set_preferences_persist_hook(move |app: &UiApp| {
        if let Ok(mut g) = persisted_hook.lock() {
            g.push((app.locale.clone(), app.theme.mode));
        }
    });

    let win1 = rt.open_window(None, ViewportMetrics::default())?;
    assert_eq!(rt.window(win1)?.app().locale, "de");
    assert_eq!(rt.window(win1)?.app().theme.mode, UiThemeMode::Dark);

    // Open secondary window via MenuAction (`window.new`): must inherit hydrated prefs, never factory defaults!
    let opened = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win1, command_id: String::from("window.new") })?;
    let win2 = opened.opened_window_id.ok_or("expected secondary window")?;
    assert_eq!(rt.window(win2)?.app().locale, "de");
    assert_eq!(rt.window(win2)?.app().theme.mode, UiThemeMode::Dark);

    // Updating theme & locale via `DesktopCommandRoute::UiCommand` MenuAction on Window 1 must
    // sync to Window 2 immediately and persist updated prefs (never Window 2's stale prefs on close)!
    let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win1, command_id: String::from("ui.locale.set:fr") })?;
    let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win1, command_id: String::from("ui.theme.set") })?;
    let expected_mode = rt.window(win1)?.app().theme.mode;
    assert_ne!(expected_mode, UiThemeMode::Dark, "ui.theme.set must advance theme mode");
    assert_eq!(rt.window(win2)?.app().locale, "fr", "UiCommand locale change on Window 1 must sync to Window 2");
    assert_eq!(rt.window(win2)?.app().theme.mode, expected_mode, "UiCommand theme change on Window 1 must sync to Window 2");

    let _ = rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: win2, force: false })?;
    let log = persisted.lock().map_err(|e| format!("{e}"))?;
    let last = log.last().ok_or("expected persisted preferences")?;
    assert_eq!(last.0, "fr", "closing Window 2 must persist synced locale, never stale/factory defaults: {log:?}");
    assert_eq!(last.1, expected_mode, "closing Window 2 must persist synced theme, never stale/factory defaults: {log:?}");
    Ok(())
}

#[test]
fn aud_06_07_dirty_commands_cleared_on_clean_open_and_save() -> TestResult {
    let td = TempDir::new()?;
    let archive = td.path().join("clean.curvebook");
    let mut doc = ProjectDocument::new("Saved")?;
    doc.add_data_sheet(DataSheet::new("d1", "Table", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1)?))?)?;
    save_project_file(&archive, &doc)?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, ViewportMetrics::default())?;
    rt.window_mut(win_id)?.execute_and_journal("project.open", json!({ "path": archive.display().to_string() }))?;
    assert!(!rt.window(win_id)?.has_unsaved_changes(), "clean project.open must clear dirty_commands");

    // Mutate then save -> must become clean again.
    rt.window_mut(win_id)?.execute_and_journal("table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 7.0 }))?;
    assert!(rt.window(win_id)?.has_unsaved_changes());
    rt.window_mut(win_id)?.execute_and_journal("project.save", json!({ "path": archive.display().to_string() }))?;
    assert!(!rt.window(win_id)?.has_unsaved_changes(), "project.save must clear dirty_commands");
    Ok(())
}

#[test]
fn aud_06_10_native_import_forwards_xlsx_sheet_selection() -> TestResult {
    let td = TempDir::new()?;
    let xlsx = td.path().join("multi.xlsx");
    fs::write(&xlsx, build_sample_xlsx_bytes()?)?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, ViewportMetrics::default())?;
    let win = rt.window_mut(win_id)?;
    execute_native_import_at_path_with_params(win, "table.importXlsx", &xlsx, false, &json!({ "sheets": ["Second"] }))?;
    assert_eq!(win.app().document().data_sheets.len(), 1);
    assert_eq!(win.app().document().data_sheets[0].title, "Second");
    Ok(())
}

#[test]
fn aud_06_12_and_aud_13_01_recovery_blocks_fs_wal_and_preserves_stale_lease() -> TestResult {
    let td = TempDir::new()?;
    let crafted_dir = td.path().join("crafted");
    fs::create_dir_all(&crafted_dir)?;
    let evil_out = td.path().join("should_not_exist.curvebook");

    // Seed a stale crash journal containing a malicious `project.save` WAL entry.
    let backend = DirectoryJournalBackend::open_dir(&crafted_dir)?;
    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "dead-owner", 1_000, 10_000)?;
    let _ = journal.record_command(json!({
        "command_id": "project.save",
        "params": { "path": evil_out.display().to_string() }
    }))?;
    drop(journal.abandon());

    // 1. Direct policy-safe recovery MUST refuse `project.save` and release its lease without writing to disk.
    let rec_err = open_dir_with_policy_safe_recovery(&crafted_dir, "new-owner", 60_000, 50_000, &ExternalPathPolicy::Unconstrained);
    assert!(rec_err.is_err(), "crafted filesystem WAL entry must be rejected during crash recovery");
    assert!(!evil_out.exists(), "malicious WAL command must never write to disk");

    // 2. Opening a seeded window (`has_doc == true`) must NOT replay the crafted WAL, must restore the
    //    evicted stale lease so `dir_has_unrecovered_crash` stays true, and must isolate into a fresh dir!
    let mut seeded = Session::new();
    let _ = seeded.execute("sheet.createData", json!({ "sheet_id": "d1", "kind": "Column" }))?;
    let (final_dir, final_journal) =
        open_fresh_journal_for_seeded_session(td.path(), &crafted_dir, DesktopWindowId(1), "live-win-1", 60_000, 50_000, &seeded)?;
    assert_ne!(final_dir, crafted_dir, "raced stale crash dir must be preserved and isolated");
    assert!(!evil_out.exists(), "seeding fresh window must NEVER replay untrusted WAL");
    assert!(
        dir_has_unrecovered_crash(&crafted_dir, 50_000),
        "evicted stale crash dir must remain reclaimable instead of stranded under live PID lease"
    );
    let _ = seeded.close_journal_cleanly(final_journal)?;
    Ok(())
}

#[test]
fn aud_13_09_export_all_tables_and_file_drop_enforce_path_policy_before_fs_ops() -> TestResult {
    let td = TempDir::new()?;
    let wd = td.path().join("wd");
    fs::create_dir_all(&wd)?;
    let outside_dir = td.path().join("outside_export_dir");
    let outside_csv = td.path().join("outside.csv");
    fs::write(&outside_csv, "X,Y\n1,2\n")?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: None,
        path_policy: ExternalPathPolicy::workdir(&wd)?,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, ViewportMetrics::default())?;
    rt.window_mut(win_id)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "d1", "kind": "Column" }))?;

    // 1. `ui.export.allTablesCsv` pointing outside `--workdir` must fail BEFORE `create_dir_all`!
    rt.set_file_dialog_adapter(Arc::new(FixedPickAdapter(outside_dir.clone())));
    let exp_err = rt.dispatch_native_file_command(win_id, "ui.export.allTablesCsv");
    assert!(exp_err.is_err(), "exporting outside workdir must fail");
    assert!(!outside_dir.exists(), "outside export dir must NEVER be created before policy validation");

    // 2. `FileDropped`, `open_window(Some(outside))`, and Welcome `project.open` must all enforce path_policy!
    assert!(rt.dispatch(DesktopHostEvent::FileDropped { window_id: win_id, path: outside_csv.clone() }).is_err());
    assert!(rt.open_window(Some(outside_csv.clone()), ViewportMetrics::default()).is_err());
    assert!(rt.dispatch_native_file_command_with_params(win_id, "project.open", &json!({ "path": outside_csv.display().to_string() })).is_err());
    Ok(())
}

#[test]
fn aud_13_21_and_aud_13_23_per_request_token_lifts_64k_cap_and_save_preserves_canonical_path() -> TestResult {
    let td = TempDir::new()?;
    let wd = td.path().join("wd");
    let sub = wd.join("sub");
    fs::create_dir_all(&sub)?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: None,
        path_policy: ExternalPathPolicy::workdir(&wd)?,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, ViewportMetrics::default())?;

    let auth = ControlAuth::from_token("rem-hosts-token-1321")?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());

    // Spawn worker thread running `serve_live_control_io` over two back-to-back frames:
    // Frame 1 (< 64 KiB): `project.save` with per-request `"token"` (NO `"auth"` handshake) and dot-segment path.
    // Frame 2 (> 64 KiB): large `table.setCells` or `doc.inspect` payload — must succeed because Frame 1 promoted auth!
    let big_pad = "x".repeat(70 * 1024);
    let frames = format!(
        "{}\n{}\n",
        json!({
            "id": "f1",
            "token": "rem-hosts-token-1321",
            "method": "engine.execute",
            "params": { "command_id": "project.save", "params": { "path": "sub/saved.curvebook" } }
        }),
        json!({
            "id": "f2",
            "token": "rem-hosts-token-1321",
            "method": "engine.execute",
            "params": { "command_id": "sheet.createData", "params": { "sheet_id": "d2", "title": big_pad, "kind": "Column" } }
        }),
    );

    let io_handle = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut reader = BufReader::new(Cursor::new(frames.into_bytes()));
        let mut writer = Vec::new();
        serve_live_control_io(&mut reader, &mut writer, &bridge, |_| {})?;
        Ok(writer)
    });

    while let Ok(req) = channel.request_rx.recv() {
        let out = rt.window_mut(win_id)?.dispatch_live_control_frame_with_hook(&req.raw_bytes, &req.auth, req.conn_authenticated, |_, _| Ok(()));
        let _ = req.reply_tx.send(out);
    }

    let out_bytes = io_handle.join().map_err(|_| "io thread panicked")??;
    let out_text = String::from_utf8(out_bytes)?;
    let lines: Vec<&str> = out_text.lines().collect();
    assert_eq!(lines.len(), 2, "expected 2 replies, got: {out_text}");
    let r1: Value = serde_json::from_str(lines[0])?;
    let r2: Value = serde_json::from_str(lines[1])?;
    assert_eq!(r1["ok"], json!(true), "frame 1 project.save failed: {r1}");
    // Frame 2 exceeds 64 KiB pre-auth cap; it MUST NOT fail with "control line exceeds 65536 bytes"!
    assert!(!r2["error"].as_str().unwrap_or("").contains("control line exceeds"), "per-request token must lift 64 KiB cap: {r2}");

    // `AUD-13-23`: canonicalized `session.file_path()` from `run_project_save` must NOT be clobbered with raw `sub/saved.curvebook`.
    let canonical_expected = fs::canonicalize(sub.join("saved.curvebook"))?;
    assert_eq!(rt.window(win_id)?.app().session().file_path(), Some(canonical_expected.as_path()));
    Ok(())
}

#[test]
fn aud_06_03_06_13_11_12_live_gui_window_screenshot_tab_and_idle_repaint() -> TestResult {
    let mut doc = ProjectDocument::new("GUI Window Study")?;
    let mut rs = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?)?;
    rs.status = ResultStatus::UpToDate;
    let num = |v: f64| ResultCell::Number { value: v, prefix: None, units: None };
    rs.tabs.push(ResultTab {
        id: String::from("tab_a"),
        title: String::from("Tab A"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("V")],
        row_labels: vec![String::from("1")],
        rows: vec![vec![num(1.0)]],
        footnotes: Vec::new(),
    });
    rs.tabs.push(ResultTab {
        id: String::from("tab_b"),
        title: String::from("Tab B"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![String::from("V")],
        row_labels: vec![String::from("2")],
        rows: vec![vec![num(2.0)]],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(rs)?;
    let _ = doc.select_in_navigator(&curvebook_doc::SheetId::new("r1"));

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, in_window_menus: true, ..DesktopHostConfig::default() });
    let win_id = rt.open_window_with_session(Session::from_document(doc), ViewportMetrics::default(), false)?;

    let auth = ControlAuth::from_token("rem-hosts-token-gui")?;
    let (req_tx, req_rx) = std::sync::mpsc::channel::<LiveControlRequest>();
    let channel = LiveControlChannel {
        request_rx: req_rx,
        egui_ctx_cell: Arc::new(std::sync::OnceLock::new()),
        shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        post_hook: None,
    };

    let mut native_app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);
    let egui_ctx = eframe::egui::Context::default();
    let repaints = Arc::new(AtomicUsize::new(0));
    let repaints_cb = Arc::clone(&repaints);
    egui_ctx.set_request_repaint_callback(move |_| {
        repaints_cb.fetch_add(1, Ordering::Relaxed);
    });

    // Step 3 frames to settle geometry.
    for _ in 0..3 {
        let mut raw = eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(1024.0, 680.0))),
            ..Default::default()
        };
        native_app.drain_synthetic_events_into(&mut raw);
        let mut out = egui_ctx.run_ui(raw, |ui| native_app.step_ui(ui));
        out.textures_delta.clear();
    }

    // 1. Live `ui.screenshot` with per-request `"token"` and omitted dimensions (`AUD-06-03` + `AUD-13-12` + `AUD-13-21`).
    let (reply_tx1, reply_rx1) = std::sync::mpsc::channel();
    req_tx.send(LiveControlRequest {
        raw_bytes: format!(
            "{}\n",
            json!({ "id": "shot1", "token": "rem-hosts-token-gui", "method": "ui.screenshot", "params": { "target": "full_window" } })
        )
        .into_bytes(),
        auth: auth.clone(),
        conn_authenticated: false,
        reply_tx: reply_tx1,
    })?;

    // 2. Live `ui.switch_tab` with integer `tab: 1` (`AUD-13-11`).
    let (reply_tx2, reply_rx2) = std::sync::mpsc::channel();
    req_tx.send(LiveControlRequest {
        raw_bytes: format!("{}\n", json!({ "id": "tab1", "token": "rem-hosts-token-gui", "method": "ui.switch_tab", "params": { "tab": 1 } }))
            .into_bytes(),
        auth,
        conn_authenticated: false,
        reply_tx: reply_tx2,
    })?;

    for _ in 0..12 {
        let mut raw = eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(1024.0, 680.0))),
            ..Default::default()
        };
        native_app.drain_synthetic_events_into(&mut raw);
        let mut out = egui_ctx.run_ui(raw, |ui| native_app.step_ui(ui));
        out.textures_delta.clear();
    }

    let shot_out = reply_rx1.recv_timeout(std::time::Duration::from_secs(2))??;
    assert_eq!(shot_out.auth_succeeded, Some(true), "per-request token on deferred screenshot must promote socket auth");
    let shot_json: Value = serde_json::from_slice(&shot_out.response_bytes)?;
    assert_eq!(shot_json["ok"], json!(true), "per-request token screenshot with omitted dims must succeed: {shot_json}");
    assert_eq!(shot_json["result"]["region"], json!("full_window"));
    assert_eq!(shot_json["result"]["format"], json!("png"));
    assert_eq!(shot_json["result"]["mime_type"], json!("image/png"));
    assert!(shot_json["result"]["bytes_len"].as_u64().is_some_and(|n| n > 0));
    assert!(shot_json["result"]["path"].is_null());
    assert!(shot_json["result"].get("data_base64").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    assert!(shot_json["result"].get("png_base64").is_none(), "screenshot must NOT duplicate PNG into png_base64");

    // 1b. Parity checks (`AUD-13-12`): `workspace` target + `scale_factor` fallback, `dialog` Conflict when no modal,
    // `dialog` success when a modal is open, and symlink `path` rejection.
    let step_frames = |app: &mut CurveBookNativeApp, ctx: &eframe::egui::Context, count: usize| {
        for _ in 0..count {
            let mut raw = eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(1024.0, 680.0))),
                ..Default::default()
            };
            app.drain_synthetic_events_into(&mut raw);
            let mut out = ctx.run_ui(raw, |ui| app.step_ui(ui));
            out.textures_delta.clear();
        }
    };
    let send_shot =
        |id: &str,
         params: Value|
         -> Result<std::sync::mpsc::Receiver<Result<curvebook::desktop_host::LiveControlFrameOutput, String>>, Box<dyn std::error::Error>> {
            let (tx, rx) = std::sync::mpsc::channel();
            req_tx.send(LiveControlRequest {
                raw_bytes: format!("{}\n", json!({ "id": id, "method": "ui.screenshot", "params": params })).into_bytes(),
                auth: ControlAuth::from_token("rem-hosts-token-gui")?,
                conn_authenticated: true,
                reply_tx: tx,
            })?;
            Ok(rx)
        };

    let rx_ws = send_shot("shot-ws", json!({ "target": "workspace", "scale_factor": 2.0 }))?;
    let rx_nodlg = send_shot("shot-nodlg", json!({ "region": "dialog" }))?;
    step_frames(&mut native_app, &egui_ctx, 12);

    let ws_json: Value = serde_json::from_slice(&rx_ws.recv_timeout(std::time::Duration::from_secs(2))??.response_bytes)?;
    assert_eq!(ws_json["ok"], json!(true), "workspace target must succeed: {ws_json}");
    assert_eq!(ws_json["result"]["region"], json!("workspace"));
    assert_eq!(ws_json["result"]["dpi"], json!(192), "scale_factor=2.0 must map to 192 dpi");

    let nodlg_json: Value = serde_json::from_slice(&rx_nodlg.recv_timeout(std::time::Duration::from_secs(2))??.response_bytes)?;
    assert_eq!(nodlg_json["ok"], json!(false), "dialog target without modal must fail: {nodlg_json}");
    assert_eq!(nodlg_json["error_detail"]["code"], json!("conflict"), "missing modal must map to Conflict: {nodlg_json}");

    // Open About modal and verify `target: "dialog"` succeeds.
    native_app.runtime_mut().window_mut(win_id)?.app_mut().canvas_dialogs.active_dialog = curvebook_ui_egui::dialogs_canvas::ActiveDialog::About;
    step_frames(&mut native_app, &egui_ctx, 3);
    let rx_dlg = send_shot("shot-dlg", json!({ "target": "dialog" }))?;
    step_frames(&mut native_app, &egui_ctx, 8);
    let dlg_json: Value = serde_json::from_slice(&rx_dlg.recv_timeout(std::time::Duration::from_secs(2))??.response_bytes)?;
    assert_eq!(dlg_json["ok"], json!(true), "dialog target with open About modal must succeed: {dlg_json}");
    assert_eq!(dlg_json["result"]["region"], json!("dialog"));
    assert!(dlg_json["result"]["bytes_len"].as_u64().is_some_and(|n| n > 0));
    native_app.runtime_mut().window_mut(win_id)?.app_mut().canvas_dialogs.active_dialog = curvebook_ui_egui::dialogs_canvas::ActiveDialog::None;
    step_frames(&mut native_app, &egui_ctx, 3);

    #[cfg(unix)]
    {
        let td = TempDir::new()?;
        let real_file = td.path().join("real.png");
        fs::write(&real_file, b"untouched")?;
        let link_path = td.path().join("link.png");
        std::os::unix::fs::symlink(&real_file, &link_path)?;
        let rx_sym = send_shot("shot-sym", json!({ "target": "full_window", "path": link_path.display().to_string() }))?;
        step_frames(&mut native_app, &egui_ctx, 8);
        let sym_json: Value = serde_json::from_slice(&rx_sym.recv_timeout(std::time::Duration::from_secs(2))??.response_bytes)?;
        assert_eq!(sym_json["ok"], json!(false), "symlink screenshot path must be rejected: {sym_json}");
        assert_eq!(fs::read(&real_file)?, b"untouched", "symlink target must remain untouched");
    }

    let tab_out = reply_rx2.recv_timeout(std::time::Duration::from_secs(2))??;
    let tab_json: Value = serde_json::from_slice(&tab_out.response_bytes)?;
    assert_eq!(tab_json["ok"], json!(true), "integer tab index must switch ResultSheet tab: {tab_json}");
    assert_eq!(tab_json["result"]["tab_id"], json!("tab_b"));
    assert_eq!(native_app.runtime().window(win_id)?.app().canvas_dialogs.result_view.active_tab_index, 1);

    // 3. Idle frame must trigger ZERO repaints (`AUD-06-06` regression guard).
    repaints.store(0, Ordering::Relaxed);
    let raw_idle = eframe::egui::RawInput {
        screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(1024.0, 680.0))),
        ..Default::default()
    };
    let mut out_idle = egui_ctx.run_ui(raw_idle, |ui| native_app.step_ui(ui));
    out_idle.textures_delta.clear();
    assert_eq!(repaints.load(Ordering::Relaxed), 0, "idle desktop frame must request 0 repaints");
    Ok(())
}

#[derive(Debug)]
struct TestDroppedFile(PathBuf);

impl eframe::egui::DroppedFile for TestDroppedFile {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn aud_06_13_multi_file_dnd_targets_opened_window_and_collects_errors() -> TestResult {
    let td = TempDir::new()?;
    let proj_path = td.path().join("dropped_study.curvebook");
    let mut doc = ProjectDocument::new("Dropped Project")?;
    doc.add_data_sheet(DataSheet::new("orig", "Original", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1)?))?)?;
    save_project_file(&proj_path, &doc)?;

    let extra_csv = td.path().join("extra_table.csv");
    fs::write(&extra_csv, "A,B\n1,2\n3,4\n")?;

    // Start with a dirty primary window so dropping `dropped_study.curvebook` opens a NEW window,
    // and verify the sibling `extra_table.csv` dropped in the SAME batch lands in that NEW window!
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, in_window_menus: true, ..DesktopHostConfig::default() });
    let win1 = rt.open_window(None, ViewportMetrics::default())?;
    rt.window_mut(win1)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "dirty1", "kind": "Column" }))?;
    assert!(rt.window(win1)?.has_unsaved_changes());

    let mut native_app = CurveBookNativeApp::new(rt, win1, None);
    let egui_ctx = eframe::egui::Context::default();
    let dropped: Vec<eframe::egui::DroppedFileHandle> = vec![Arc::new(TestDroppedFile(proj_path)), Arc::new(TestDroppedFile(extra_csv))];
    let raw = eframe::egui::RawInput {
        screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(1024.0, 680.0))),
        dropped_files: dropped,
        ..Default::default()
    };
    let mut out1 = egui_ctx.run_ui(raw, |ui| native_app.step_ui(ui));
    out1.textures_delta.clear();

    assert_eq!(native_app.runtime().window_count(), 2);
    let win2 = native_app.runtime().focused_window_id().ok_or("missing focused window")?;
    assert_ne!(win2, win1);
    assert_eq!(native_app.runtime().window(win1)?.app().document().data_sheets.len(), 1, "original dirty window must not receive extra_table.csv");
    assert_eq!(
        native_app.runtime().window(win2)?.app().document().data_sheets.len(),
        2,
        "newly opened project window must receive extra_table.csv from the same multi-file drop"
    );

    // Dropping two bad/non-existent files must join BOTH errors in the status banner rather than losing the first.
    let bad_a = td.path().join("missing_alpha.unknownext");
    let bad_b = td.path().join("missing_beta.unknownext");
    let bad_drop: Vec<eframe::egui::DroppedFileHandle> = vec![Arc::new(TestDroppedFile(bad_a)), Arc::new(TestDroppedFile(bad_b))];
    let raw_bad = eframe::egui::RawInput {
        screen_rect: Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(0.0, 0.0), eframe::egui::vec2(1024.0, 680.0))),
        dropped_files: bad_drop,
        ..Default::default()
    };
    let mut out_bad = egui_ctx.run_ui(raw_bad, |ui| native_app.step_ui(ui));
    out_bad.textures_delta.clear();
    let banner = native_app.status_banner().ok_or("expected aggregated error banner")?;
    assert!(banner.contains("missing_alpha") && banner.contains("missing_beta"), "status banner must retain all drop errors: {banner}");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn aud_06_14_mac_menu_accelerator_parses_plus_and_slash() {
    use curvebook::mac_menu::parse_muda_accelerator;
    assert!(parse_muda_accelerator("Cmd++").is_some(), "Cmd++ must parse");
    assert!(parse_muda_accelerator("Cmd+Shift++").is_some(), "Cmd+Shift++ must parse");
    assert!(parse_muda_accelerator("+").is_some(), "bare + must parse");
    assert!(parse_muda_accelerator("Cmd+/").is_some(), "Cmd+/ must parse");
    assert!(parse_muda_accelerator("Cmd+Shift+/").is_some(), "Cmd+Shift+/ must parse");
    assert!(parse_muda_accelerator("/").is_some(), "bare / must parse");
    assert!(parse_muda_accelerator("Cmd+/ / Ctrl+/").is_some(), "Cmd+/ / Ctrl+/ must parse");
    assert!(parse_muda_accelerator("Cmd+K / Ctrl+K").is_some(), "Cmd+K / Ctrl+K must parse");
    assert!(parse_muda_accelerator("Cmd+").is_none(), "trailing single + must fail");
    assert!(parse_muda_accelerator("++").is_none(), "bare ++ must fail");
}
