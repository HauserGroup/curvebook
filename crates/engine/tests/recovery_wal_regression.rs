#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lumencraft_doc::{CellCoord, SheetId, TableArchetype};
use lumencraft_engine::Session;
use lumencraft_format::{DirectoryJournalBackend, JOURNAL_FILE_NAME, MemoryJournalBackend};
use serde_json::json;

static ENGINE_TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = ENGINE_TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("lumencraft-engine-wal-{label}-{}-{id}", std::process::id()));
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
    let dir = TempDirGuard::new("s07-06b-csv")?;
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
    let dir = TempDirGuard::new("s06-03-engine")?;
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
