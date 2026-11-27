use std::fs;

use curvebook_doc::{CellCoord, ErrorCode, SheetId, TableArchetype};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::{DirectoryJournalBackend, JOURNAL_FILE_NAME, MemoryJournalBackend};
use serde_json::json;

use super::common::TempDir;

fn col_values(session: &Session, sheet_id: &str, col_idx: usize, rows: usize) -> Result<Vec<Option<f64>>, Box<dyn std::error::Error>> {
    let ds = session.document().data_sheet(&SheetId::new(sheet_id)).ok_or_else(|| format!("missing data sheet `{sheet_id}`"))?;
    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let cell = ds.cell(CellCoord::new(row, col_idx, 0));
        out.push(cell.and_then(|c| c.active_number()));
    }
    Ok(out)
}

#[test]
fn r1_s07_06a_recovery_after_checkpoint_then_undo_and_redo_matches_live() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Set 10 -> Set 42 -> checkpoint -> edit.undo -> abandon -> recover must see 10 (undone state).
    let (mut live, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "s07-06a-live", 5_000, 1_000)?;
    live.execute_and_record(
        &mut journal,
        "sheet.createData",
        json!({ "sheet_id": "d1", "title": "UndoTable", "archetype": "column", "cols": 2, "rows": 4 }),
    )?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 10.0 }))?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 42.0 }))?;
    live.checkpoint_journal(&mut journal)?;
    live.execute_and_record(&mut journal, "edit.undo", json!({}))?;
    assert_eq!(col_values(&live, "d1", 0, 1)?, vec![Some(10.0)]);

    let crashed_backend = journal.abandon();
    let (mut recovered, mut rec_journal, report) = Session::open_with_recovery(crashed_backend, "s07-06a-rec", 5_000, 7_000)?;
    assert_eq!(col_values(&recovered, "d1", 0, 1)?, col_values(&live, "d1", 0, 1)?);
    assert!(report.recovered_commands.is_empty());

    // 2. Subsequent edit + undo WITHOUT prior checkpoint + redo + new edit must also match after second crash.
    recovered.execute_and_record(&mut rec_journal, "table.setCell", json!({ "sheet_id": "d1", "row": 1, "col": 0, "value": 55.0 }))?;
    recovered.execute_and_record(&mut rec_journal, "edit.undo", json!({}))?;
    recovered.execute_and_record(&mut rec_journal, "edit.redo", json!({}))?;
    recovered.execute_and_record(&mut rec_journal, "table.setCell", json!({ "sheet_id": "d1", "row": 2, "col": 0, "value": 77.5 }))?;
    let expected = col_values(&recovered, "d1", 0, 3)?;
    assert_eq!(expected, vec![Some(10.0), Some(55.0), Some(77.5)]);

    let crashed_again = rec_journal.abandon();
    let (recovered_two, _, _) = Session::open_with_recovery(crashed_again, "s07-06a-rec2", 5_000, 13_000)?;
    assert_eq!(col_values(&recovered_two, "d1", 0, 3)?, expected);
    Ok(())
}

#[test]
fn r1_s07_06b_recovery_never_rereads_mutated_or_deleted_external_files() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::try_new("s07-06b-csv")?;
    let csv_path = dir.path.join("plate.csv");
    fs::write(&csv_path, "1,10\n2,20\n3,30\n")?;

    // 1. table.importCsv followed by overwriting external plate.csv before recovery.
    let (mut live, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "csv-1", 5_000, 1_000)?;
    live.execute_and_record(
        &mut journal,
        "sheet.createData",
        json!({ "sheet_id": "plate", "title": "Plate", "archetype": "column", "cols": 2, "rows": 4 }),
    )?;
    live.execute_and_record(&mut journal, "table.importCsv", json!({ "sheet_id": "plate", "path": csv_path.to_string_lossy(), "link": true }))?;
    // Also exercise unlinked table.importCsv creating a new sheet directly from an external path.
    live.execute_and_record(
        &mut journal,
        "table.importCsv",
        json!({ "sheet_id": "plate-new", "archetype": "column", "has_header": false, "path": csv_path.to_string_lossy() }),
    )?;
    live.execute_and_record(
        &mut journal,
        "sheet.createData",
        json!({ "sheet_id": "notes", "title": "Followup", "archetype": "column", "cols": 1, "rows": 2 }),
    )?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "notes", "row": 0, "col": 0, "value": 88.0 }))?;

    let expected_plate = col_values(&live, "plate", 1, 3)?;
    assert_eq!(expected_plate, vec![Some(10.0), Some(20.0), Some(30.0)]);
    assert_eq!(col_values(&live, "plate-new", 1, 3)?, expected_plate);

    // Instrument mutates plate.csv on disk after live import, then process crashes.
    fs::write(&csv_path, "9,999\n9,999\n9,999\n")?;
    let crashed = journal.abandon();
    let (mut rec1, mut j1, _) = Session::open_with_recovery(crashed, "csv-2", 5_000, 7_000)?;
    assert_eq!(col_values(&rec1, "plate", 1, 3)?, expected_plate);
    assert_eq!(col_values(&rec1, "plate-new", 1, 3)?, expected_plate);
    assert_eq!(col_values(&rec1, "notes", 0, 1)?, vec![Some(88.0)]);

    // 2. project.recalculate with live external link, then deleting external plate.csv before crash recovery.
    fs::write(&csv_path, "1,111\n2,222\n3,333\n")?;
    rec1.execute_and_record(&mut j1, "project.recalculate", json!({ "include_external_links": true }))?;
    let expected_recalc = col_values(&rec1, "plate", 1, 3)?;
    assert_eq!(expected_recalc, vec![Some(111.0), Some(222.0), Some(333.0)]);
    fs::remove_file(&csv_path)?;

    let crashed_deleted = j1.abandon();
    let (rec2, _, _) = Session::open_with_recovery(crashed_deleted, "csv-3", 5_000, 13_000)?;
    assert_eq!(col_values(&rec2, "plate", 1, 3)?, expected_recalc);

    // 3. table.pasteLink from external file followed by deleting source file before crash.
    let linked_csv = dir.path.join("paste-link.csv");
    fs::write(&linked_csv, "4,44\n5,55\n")?;
    let (mut paste_sess, mut paste_journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "paste-1", 5_000, 1_000)?;
    paste_sess.execute_and_record(
        &mut paste_journal,
        "sheet.createData",
        json!({ "sheet_id": "target", "title": "Target", "archetype": "column", "cols": 2, "rows": 4 }),
    )?;
    paste_sess.execute_and_record(
        &mut paste_journal,
        "table.pasteLink",
        json!({ "target_sheet_id": "target", "source_path": linked_csv.to_string_lossy() }),
    )?;
    let expected_paste = col_values(&paste_sess, "target", 1, 2)?;
    assert_eq!(expected_paste, vec![Some(44.0), Some(55.0)]);
    fs::remove_file(&linked_csv)?;

    let (paste_rec, _, _) = Session::open_with_recovery(paste_journal.abandon(), "paste-2", 5_000, 7_000)?;
    assert_eq!(col_values(&paste_rec, "target", 1, 2)?, expected_paste);
    Ok(())
}

#[test]
fn engine_open_with_recovery_quarantines_interior_corrupt_wal_lines_and_reports_warnings() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::try_new("s06-03-engine")?;
    let (mut live, mut journal, _) = Session::open_dir_with_recovery(&dir.path, "eng-1", 2_000, 1_000)?;
    live.execute_and_record(
        &mut journal,
        "sheet.createData",
        json!({ "sheet_id": "d-int", "title": "Intact", "archetype": "column", "cols": 1, "rows": 2 }),
    )?;
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(dir.path.join(JOURNAL_FILE_NAME))?;
        f.write_all(b"{\"format_version\":1,\"seq\":99,\"corrupt_mid")?;
        f.sync_data()?;
    }
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d-int", "row": 0, "col": 0, "value": 314.0 }))?;
    drop(journal.abandon());

    let (mut recovered, rec_journal, report) = Session::open_with_recovery(DirectoryJournalBackend::open_dir(&dir.path)?, "eng-2", 2_000, 4_000)?;
    assert_eq!(report.discarded_corrupt_interior_lines, 1);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.recovered_commands.len(), 2);
    let ds = recovered.document().data_sheet(&SheetId::new("d-int")).ok_or("missing d-int")?;
    let TableArchetype::Column(ref col) = ds.archetype else {
        return Err("expected Column archetype".into());
    };
    assert_eq!(col.data_sets[0].cells[0][0].active_number(), Some(314.0));
    let _ = recovered.close_journal_cleanly(rec_journal)?;
    Ok(())
}

#[test]
fn aud_02_19_and_13_01_wal_checkpoint_and_replay_filesystem_policy() -> Result<(), Box<dyn std::error::Error>> {
    // AUD-02-19: table.excludeCellsWithReason forces WAL checkpoint and preserves minted timestamp across crash recovery.
    assert!(Session::command_requires_wal_checkpoint("table.excludeCellsWithReason"));
    let tmp = TempDir::try_new("aud-wal")?;
    let wal_dir = tmp.child("wal_dir");
    let (mut s1, mut j1, _) = Session::open_dir_with_recovery(&wal_dir, "owner-1", 60_000, 1_000)?;
    s1.execute_and_record(&mut j1, "sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 1, "rows": 3 }))?;
    let out =
        s1.execute_and_record(&mut j1, "table.excludeCellsWithReason", json!({ "sheet_id": "col-1", "reason_tag": "artifact", "row": 0, "col": 0 }))?;
    let minted_ts = out.payload["timestamp_utc"].as_str().ok_or("missing timestamp_utc")?.to_owned();
    assert!(!minted_ts.is_empty());
    drop((j1, s1));
    let (reopened, _j2, rep2) = Session::open_dir_with_recovery(&wal_dir, "owner-2", 60_000, 200_000)?;
    assert!(rep2.recovered_commands.is_empty());
    let note = reopened
        .document()
        .data_sheet(&SheetId::new("col-1"))
        .ok_or("missing col-1")?
        .exclusion_note(CellCoord::new(0, 0, 0))
        .ok_or("missing note")?;
    assert_eq!(note.timestamp_utc, minted_ts);

    // AUD-13-01: crafted checksummed WAL filesystem command rejected under RejectExternal, skipped with warning under best-effort, and skipped by lease-only.
    let fs_dir = tmp.child("wal_fs");
    let secret_csv = tmp.child("host_secret.csv");
    fs::write(&secret_csv, "X,Y\n1,99\n2,199\n")?;
    let (mut seed_s, mut seed_j, _) = Session::open_dir_with_recovery(&fs_dir, "seed", 60_000, 1_000)?;
    seed_s.execute_and_record(&mut seed_j, "sheet.createData", json!({ "sheet_id": "xy-1", "kind": "xy", "cols": 1, "rows": 2 }))?;
    seed_s.checkpoint_journal(&mut seed_j)?;
    seed_s.record_to_journal(&mut seed_j, "table.importCsv", &json!({ "sheet_id": "stolen", "path": secret_csv.to_string_lossy() }))?;
    drop((seed_j, seed_s));

    let err = Session::open_dir_with_recovery_and_policy(&fs_dir, "owner-strict", 60_000, 200_000, ExternalPathPolicy::RejectExternal, false)
        .err()
        .ok_or("expected PathEscape")?;
    assert_eq!(err.code, ErrorCode::PathEscape);
    let (be, j_be, rep_be) = Session::open_dir_with_best_effort_recovery(&fs_dir, "owner-be", 60_000, 400_000)?;
    assert!(be.document().data_sheet(&SheetId::new("stolen")).is_none());
    assert!(rep_be.warnings.iter().any(|w| w.kind == "skipped_unreplayable_command" && w.message.contains("table.importCsv")));
    drop((j_be, be));
    let (_j_lease, rep_lease) = Session::open_dir_journal_lease_only(&fs_dir, "owner-lease", 60_000, 600_000)?;
    assert!(rep_lease.recovered_commands.is_empty());
    Ok(())
}

#[test]
fn s10_4_project_merge_and_merge_imports_wal_checkpoint_survive_mutated_or_deleted_sources() -> Result<(), Box<dyn std::error::Error>> {
    // Verify WAL checkpoint policy marks project.merge and path-based imports as requiring a snapshot checkpoint.
    for cmd_id in ["project.merge", "table.importPzfx", "table.importPrism", "table.importXlsx"] {
        assert!(Session::command_requires_wal_checkpoint(cmd_id), "{cmd_id} must force a WAL snapshot checkpoint");
    }

    let dir = TempDir::try_new("s10-4-merge-wal")?;
    let mut source_doc = curvebook_doc::ProjectDocument::new("Merge Source").unwrap();
    let mut col = curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    col.data_sets[0].cells = vec![vec![curvebook_doc::Cell::number(11.0)], vec![curvebook_doc::Cell::number(22.0)]];
    source_doc.add_data_sheet(curvebook_doc::DataSheet::new("d1", "Merged Study", TableArchetype::Column(col)).unwrap()).unwrap();
    let merge_proj_path = dir.path.join("incoming.curvebook");
    curvebook_format::save_project_file(&merge_proj_path, &source_doc)?;

    let pzfx_path = dir.path.join("incoming.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile><Table ID="T1" TableType="OneWay"><Title>Merged XML</Title><YColumn><Title>Y</Title><Subcolumn><d>33</d><d>44</d></Subcolumn></YColumn></Table></PrismFile>"#,
    )?;

    let (mut live, mut journal, _) = Session::open_with_recovery(MemoryJournalBackend::new(), "merge-wal-1", 5_000, 1_000)?;
    live.execute_and_record(
        &mut journal,
        "sheet.createData",
        json!({ "sheet_id": "d1", "title": "Base", "archetype": "column", "cols": 1, "rows": 2 }),
    )?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 5.0 }))?;
    live.execute_and_record(&mut journal, "project.merge", json!({ "path": merge_proj_path.to_string_lossy(), "merged_at": "unix:1700001000" }))?;
    live.execute_and_record(
        &mut journal,
        "table.importPzfx",
        json!({ "path": pzfx_path.to_string_lossy(), "merge": true, "merged_at": "unix:1700001001" }),
    )?;
    live.execute_and_record(&mut journal, "table.setCell", json!({ "sheet_id": "d1", "row": 1, "col": 0, "value": 9.5 }))?;

    let expected_doc = live.document().clone();
    assert_eq!(expected_doc.data_sheets.len(), 3);
    assert_eq!(expected_doc.data_sheets[1].origin.as_ref().map(|o| o.source_label.as_str()), Some("incoming.curvebook"));
    assert_eq!(expected_doc.data_sheets[2].origin.as_ref().map(|o| o.source_label.as_str()), Some("incoming.pzfx"));

    // Mutate the `.curvebook` source file with corrupt bytes and delete the `.pzfx` file before crash recovery.
    fs::write(&merge_proj_path, b"corrupted-after-merge")?;
    fs::remove_file(&pzfx_path)?;

    let (recovered, _, _) = Session::open_with_recovery(journal.abandon(), "merge-wal-2", 5_000, 7_000)?;
    assert_eq!(recovered.document(), &expected_doc);
    Ok(())
}
