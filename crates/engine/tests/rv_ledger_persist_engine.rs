#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use curvebook_doc::{CellCoord, ErrorCode, InfoSheet, ProjectDocument, SheetId};
use curvebook_engine::cmd::{self, Caller, Effects};
use curvebook_engine::{GuiOnlyPolicy, Session};
use curvebook_format::recovery::JournalLimits;
use curvebook_format::{CrashRecoveryJournal, DirectoryJournalBackend, LOCKFILE_NAME, MemoryJournalBackend, SNAPSHOT_FILE_NAME};
use serde_json::{Value, json};

static PERSIST_ENGINE_TEST_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = PERSIST_ENGINE_TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-rv-persist-engine-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn pointer_of(err: &curvebook_engine::EngineError) -> Option<&str> {
    err.details.as_ref()?.get("pointer")?.as_str()
}

#[test]
fn rv04_execute_and_record_rolls_back_doc_and_undo_stack_on_journal_limits_and_lost_lease() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Simulate `max_records` journal failure: second recorded mutation must leave doc, undo, redo, dirty, and file_path untouched.
    let rec_limits = JournalLimits { max_records: 1, ..JournalLimits::default() };
    let (mut journal_rec, _) =
        CrashRecoveryJournal::open_or_recover_with_limits(MemoryJournalBackend::new(), "rv04-max-rec", 30_000, 1_000, rec_limits)?;
    let mut session_rec = Session::new();
    session_rec.execute_and_record(
        &mut journal_rec,
        "sheet.createData",
        json!({ "sheet_id": "d1", "title": "Cohort", "archetype": "column", "cols": 2, "rows": 3 }),
    )?;
    let before_doc = session_rec.document().clone();
    let before_undo = session_rec.undo_depth();
    let before_redo = session_rec.redo_depth();
    let before_dirty = session_rec.is_dirty();

    let err_max_rec = session_rec
        .execute_and_record(&mut journal_rec, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 99.5 }))
        .expect_err("exceeding max_records must fail execute_and_record");
    assert!(err_max_rec.message().contains("record count exceeds maximum limit"), "unexpected error: {err_max_rec}");
    assert_eq!(session_rec.document(), &before_doc, "document must roll back on max_records failure");
    assert_eq!(session_rec.undo_depth(), before_undo, "undo_stack must roll back on max_records failure");
    assert_eq!(session_rec.redo_depth(), before_redo, "redo_stack must remain unchanged on max_records failure");
    assert_eq!(session_rec.is_dirty(), before_dirty, "dirty flag must roll back on max_records failure");

    // 2. Simulate `max_bytes` journal failure.
    let byte_limits = JournalLimits { max_bytes: 240, ..JournalLimits::default() };
    let (mut journal_bytes, _) =
        CrashRecoveryJournal::open_or_recover_with_limits(MemoryJournalBackend::new(), "rv04-max-bytes", 30_000, 1_000, byte_limits)?;
    let mut session_bytes = Session::new();
    session_bytes.execute_and_record(&mut journal_bytes, "project.new", json!({ "title": "A" }))?;
    session_bytes.set_file_path(Some(PathBuf::from("clean-baseline.curvebook")), true);
    assert!(!session_bytes.is_dirty());
    let snap_doc = session_bytes.document().clone();
    let snap_undo = session_bytes.undo_depth();

    let err_max_bytes = session_bytes
        .execute_and_record(
            &mut journal_bytes,
            "sheet.createData",
            json!({ "sheet_id": "d-overflow", "title": "Overflowing Journal Budget", "archetype": "xy", "cols": 2, "rows": 4 }),
        )
        .expect_err("exceeding max_bytes must fail execute_and_record");
    assert!(err_max_bytes.message().contains("exceeds maximum replay limit"), "unexpected error: {err_max_bytes}");
    assert_eq!(session_bytes.document(), &snap_doc, "document must roll back on max_bytes failure");
    assert_eq!(session_bytes.undo_depth(), snap_undo, "undo_stack must roll back on max_bytes failure");
    assert!(!session_bytes.is_dirty(), "clean session must stay clean when journal append fails");
    assert_eq!(session_bytes.file_path(), Some(PathBuf::from("clean-baseline.curvebook").as_path()));

    // 3. Simulate lost lease across normal mutation, `project.new` (`file_path` clear), `edit.undo`, and `edit.redo`.
    let (mut session_lease, mut journal_lease, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "rv04-lease", 30_000, 1_000)?;
    session_lease.execute_and_record(
        &mut journal_lease,
        "sheet.createData",
        json!({ "sheet_id": "d1", "title": "Dose", "archetype": "column", "cols": 2, "rows": 2 }),
    )?;
    session_lease.execute_and_record(&mut journal_lease, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 12.0 }))?;
    session_lease.execute_and_record(&mut journal_lease, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 24.0 }))?;
    session_lease.execute_and_record(&mut journal_lease, "edit.undo", json!({}))?;
    session_lease.set_file_path(Some(PathBuf::from("study.curvebook")), true);
    assert!(!session_lease.is_dirty());

    // At this point cell (0,0) == 12.0, undo_depth == 2, redo_depth == 1, dirty == false, file_path == Some("study.curvebook").
    assert_eq!(
        session_lease.document().data_sheet(&SheetId::new("d1")).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).and_then(|c| c.active_number()),
        Some(12.0)
    );
    assert_eq!(session_lease.undo_depth(), 2);
    assert_eq!(session_lease.redo_depth(), 1);
    let frozen_session = session_lease.clone();

    // Drop the backend lease to force every subsequent journal append/checkpoint to fail.
    journal_lease.backend_mut().set_lock_lease(None);

    session_lease
        .execute_and_record(&mut journal_lease, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 99.0 }))
        .expect_err("normal mutation with lost lease must fail");
    assert_eq!(session_lease, frozen_session, "failed journal append must preserve doc, undo_stack, redo_stack, dirty, and file_path");

    session_lease
        .execute_and_record(&mut journal_lease, "project.new", json!({ "title": "Clobbered" }))
        .expect_err("project.new with lost lease must fail");
    assert_eq!(session_lease, frozen_session, "failed project.new journal append must roll back file_path and document");

    session_lease.execute_and_record(&mut journal_lease, "edit.undo", json!({})).expect_err("edit.undo checkpoint with lost lease must fail");
    assert_eq!(session_lease, frozen_session, "failed edit.undo checkpoint must restore popped undo entry and redo_stack");

    session_lease.execute_and_record(&mut journal_lease, "edit.redo", json!({})).expect_err("edit.redo checkpoint with lost lease must fail");
    assert_eq!(session_lease, frozen_session, "failed edit.redo checkpoint must restore popped redo entry and undo_stack");

    Ok(())
}

#[test]
fn rv05_open_with_recovery_releases_lock_on_replay_or_snapshot_failure_and_supports_best_effort() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("rv05-replay")?;

    // Seed a valid WAL command (seq 1) followed by a malformed entry missing `command_id` (seq 2), then crash after short TTL.
    {
        let backend = DirectoryJournalBackend::open_dir(&dir.path)?;
        let (mut writer, _) = CrashRecoveryJournal::open_or_recover(backend, "initial-writer", 100, 1_000)?;
        writer.record_command(json!({
            "command_id": "sheet.createData",
            "params": { "sheet_id": "d1", "title": "Salvaged Sheet", "archetype": "column", "cols": 2, "rows": 3 }
        }))?;
        writer.record_command(json!({ "params": { "orphan": true } }))?;
        let _ = writer.abandon();
    }

    // Strict `open_dir_with_recovery` with a 60s TTL must fail on seq 2 AND immediately release `curvebook.lock`.
    let missing_id_err = Session::open_dir_with_recovery(&dir.path, "strict-reader-1", 60_000, 5_000)
        .err()
        .ok_or("strict recovery must reject journal record missing command_id")?;
    assert!(missing_id_err.message().contains("missing `command_id`"), "unexpected error: {missing_id_err}");
    assert_eq!(missing_id_err.details.as_ref().and_then(|d| d.get("seq")).and_then(Value::as_u64), Some(2));
    assert_eq!(missing_id_err.details.as_ref().and_then(|d| d.get("replay_index")).and_then(Value::as_u64), Some(1));
    assert!(missing_id_err.remediation.as_deref().is_some_and(|r| r.contains("open_with_best_effort_recovery")));
    assert!(!dir.path.join(LOCKFILE_NAME).exists(), "curvebook.lock must be released when strict recovery fails");

    // Because `curvebook.lock` was released (not leaked with a 60s TTL!), another writer at t = 5_001ms can open immediately
    // and append a second failing record (an unknown command at seq 3) to test command-execution failure cleanup too.
    {
        let backend = DirectoryJournalBackend::open_dir(&dir.path)?;
        let (mut writer2, _) = CrashRecoveryJournal::open_or_recover(backend, "writer-after-fail", 100, 5_001)?;
        writer2.record_command(json!({
            "command_id": "nonexistent.command",
            "params": {}
        }))?;
        let _ = writer2.abandon();
    }

    // Strict recovery also releases `curvebook.lock` when command execution itself fails during replay.
    let dir_cmd_fail = TempDirGuard::new("rv05-cmd-fail")?;
    {
        let backend = DirectoryJournalBackend::open_dir(&dir_cmd_fail.path)?;
        let (mut writer, _) = CrashRecoveryJournal::open_or_recover(backend, "cmd-writer", 100, 1_000)?;
        writer.record_command(json!({ "command_id": "nonexistent.command", "params": {} }))?;
        let _ = writer.abandon();
    }
    let cmd_err = Session::open_dir_with_recovery(&dir_cmd_fail.path, "strict-reader-2", 60_000, 5_000)
        .err()
        .ok_or("strict recovery must fail on unknown command")?;
    assert_eq!(cmd_err.code, ErrorCode::NotFound);
    assert_eq!(cmd_err.details.as_ref().and_then(|d| d.get("seq")).and_then(Value::as_u64), Some(1));
    assert_eq!(cmd_err.details.as_ref().and_then(|d| d.get("command_id")).and_then(Value::as_str), Some("nonexistent.command"));
    assert!(!dir_cmd_fail.path.join(LOCKFILE_NAME).exists(), "curvebook.lock must be released when command replay fails");

    // Corrupt baseline snapshot also releases `curvebook.lock`.
    let dir_bad_snap = TempDirGuard::new("rv05-bad-snap")?;
    fs::write(dir_bad_snap.path.join(SNAPSHOT_FILE_NAME), b"not-a-valid-curvebook-archive")?;
    let _snap_err = Session::open_dir_with_recovery(&dir_bad_snap.path, "strict-snap-reader", 60_000, 5_000)
        .err()
        .ok_or("corrupt snapshot must fail recovery")?;
    assert!(!dir_bad_snap.path.join(LOCKFILE_NAME).exists(), "curvebook.lock must be released when snapshot open fails");

    // Opt-in `open_dir_with_best_effort_recovery` salvages `d1` (seq 1), skips seq 2 & seq 3 with warnings, and holds the lease cleanly.
    let (mut salvaged, salvaged_journal, report) = Session::open_dir_with_best_effort_recovery(&dir.path, "salvage-reader", 60_000, 6_000)?;
    assert!(salvaged.document().data_sheet(&SheetId::new("d1")).is_some(), "intact command seq 1 must be salvaged");
    let skip_warnings: Vec<_> = report.warnings.iter().filter(|w| w.kind == "skipped_unreplayable_command").collect();
    assert_eq!(skip_warnings.len(), 2, "expected warnings for both unreplayable records, got: {:?}", report.warnings);
    assert!(dir.path.join(LOCKFILE_NAME).exists(), "active salvage session must retain curvebook.lock");
    salvaged.close_journal_cleanly(salvaged_journal)?;
    assert!(!dir.path.join(LOCKFILE_NAME).exists(), "close_journal_cleanly releases curvebook.lock");

    Ok(())
}

#[test]
fn rv13_strict_gui_only_blocks_sheet_select_and_all_ui_state_mutations_for_non_ui_callers() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "First", "archetype": "column", "cols": 2, "rows": 2 }))?;
    session.execute("sheet.createData", json!({ "sheet_id": "d2", "title": "Second", "archetype": "column", "cols": 2, "rows": 2 }))?;
    session.execute("sheet.select", json!({ "sheet_id": "d1" }))?;
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

    session.set_gui_only(GuiOnlyPolicy::Strict);
    let cancel = AtomicBool::new(false);

    for caller in [Caller::Mcp, Caller::Control, Caller::Cli] {
        let err = session
            .execute_with_caller("sheet.select", json!({ "sheet_id": "d2" }), caller, &cancel)
            .expect_err("sheet.select must be blocked under GuiOnlyPolicy::Strict for non-UI callers");
        assert_eq!(err.code, ErrorCode::GuiOnlyViolation, "wrong error code for {caller:?}");
        assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));
    }

    // `Caller::Ui` and `Caller::Replay` remain permitted to drive `sheet.select`.
    session.execute_with_caller("sheet.select", json!({ "sheet_id": "d2" }), Caller::Ui, &cancel)?;
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("d2"));
    session.execute_with_caller("sheet.select", json!({ "sheet_id": "d1" }), Caller::Replay, &cancel)?;
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

    // Registry-wide invariant: every command with CHANGES_UI_STATE (or any other non-inspection effect)
    // is rejected with GuiOnlyViolation for Caller::Mcp and Caller::Control before parameter parsing.
    let mut checked_ui_state_cmds = 0usize;
    for spec in cmd::registry().specs() {
        if spec.effects.contains(Effects::CHANGES_UI_STATE) {
            checked_ui_state_cmds += 1;
            for caller in [Caller::Mcp, Caller::Control] {
                let err = session
                    .execute_with_caller(spec.id, json!({}), caller, &cancel)
                    .expect_err("CHANGES_UI_STATE command must be rejected under strict gui_only");
                assert_eq!(err.code, ErrorCode::GuiOnlyViolation, "command `{}` must return GuiOnlyViolation", spec.id);
            }
        }
    }
    assert!(checked_ui_state_cmds >= 1, "registry must contain at least `sheet.select` with CHANGES_UI_STATE");
    Ok(())
}

#[test]
fn rv16_closed_command_schemas_and_deserializer_reject_unknown_fields_with_rfc6901_pointers() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("RV-16 Check")?;
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Info")?)?;
    let mut session = Session::from_document(doc);
    session.execute("sheet.createData", json!({ "sheet_id": "xy-1", "title": "Dose", "archetype": "xy", "cols": 2, "rows": 4 }))?;
    session.execute("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" }))?;

    // Closed schemas in project, info, graph, sheet, and table reject misspelled/unknown fields with exact RFC-6901 pointers.
    let cases: [(&str, Value, &str); 8] = [
        ("project.new", json!({ "title": "Valid", "titel": "Typo" }), "/titel"),
        ("project.inspect", json!({ "sheet_id": "xy-1", "verbose": true }), "/verbose"),
        ("info.setNote", json!({ "sheet_id": "info-1", "text": "ok", "apend": true }), "/apend"),
        ("graph.setKind", json!({ "sheet_id": "g-1", "kind": "xy", "unknown_flag": 1 }), "/unknown_flag"),
        ("sheet.createData", json!({ "sheet_id": "xy-2", "archetype": "xy", "rowz": 10 }), "/rowz"),
        ("table.setTitles", json!({ "sheet_id": "xy-1", "x_title": "Time", "bogus_titles": ["Y"] }), "/bogus_titles"),
        (
            "table.randomizeGroups",
            json!({
                "n_subjectz": 6,
                "seed": 42,
                "groups": [{ "name": "Control", "weight": 1 }, { "name": "Treated", "weight": 1 }]
            }),
            "/n_subjectz",
        ),
        (
            "graph.setAdditionalTicks",
            json!({
                "sheet_id": "g-1",
                "axis": "x",
                "ticks": [{ "value": 10.0, "extra_nested": true }]
            }),
            "/ticks/0/extra_nested",
        ),
    ];

    for (command_id, bad_params, expected_ptr) in cases {
        let before = session.clone();
        let err = session.execute(command_id, bad_params).expect_err(&format!("`{command_id}` must reject unknown field at `{expected_ptr}`"));
        assert_eq!(err.code, ErrorCode::BadInput, "`{command_id}` wrong error code: {err}");
        assert_eq!(pointer_of(&err), Some(expected_ptr), "`{command_id}` wrong RFC-6901 pointer: {err:?}");
        assert_eq!(session, before, "`{command_id}` must leave session unchanged on BadInput");
    }

    // Declared aliases (`name`, `y_titles`) and intentional open patch commands (`graph.setAxes`, `analyze.run`) continue to succeed.
    session.execute("table.setTitles", json!({ "sheet_id": "xy-1", "x_title": "Dose", "y_titles": ["Resp A", "Resp B"] }))?;
    session.execute("graph.setAxes", json!({ "sheet_id": "g-1", "axis": "x", "min": 0.0, "max": 100.0 }))?;
    session.execute("analyze.run", json!({ "sheet_id": "xy-1" }))?;

    Ok(())
}
