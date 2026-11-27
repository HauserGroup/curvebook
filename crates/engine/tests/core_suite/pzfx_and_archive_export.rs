//! Integration tests for Wave 3 Track 3B (`§4.2`, `§6.1`, `§6.3`):
//! - `project.exportPzfx` atomic write, `ExternalPathPolicy`, warnings, round-trip, and session
//!   `file_path`/`dirty` invariance
//! - `project.exportPrism` fail-closed refusal without `experimental: true` and opt-in round-trip
//! - `table.importPrism` live-analysis recalculation in one undo step preserving `ResultStatus::Imported` snapshots
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ConstantValue, ErrorCode, ImportedSnapshotSpec, InfoSheet, ProjectDocument, ResultCell, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, SheetId,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{ZipWriter, compare_pzfx_data_layer, import_pzfx_bytes};
use serde_json::json;

use super::common::{TempDir, decode_base64, encode_base64, err_cmd, ok_cmd, xy_sheet};

fn seed_rich_project_session(dir: &TempDir) -> (Session, std::path::PathBuf) {
    let mut doc = ProjectDocument::new("Track 3B Export Study").unwrap();
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol").unwrap();
    info.set_constant("Experiment Date", ConstantValue::Date(String::from("2026-07-01")), None).unwrap();
    info.notes_markdown = String::from("Buffer: PBS");
    doc.add_info_sheet(info).unwrap();
    doc.add_data_sheet(xy_sheet("xy-1", "Dose Curve", "Dose", &[1.0, 10.0, 100.0], &[("Signal", &[12.5, 48.0, 91.5])])).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "xy-1", "kind": "descriptive" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "xy-1", "kind": "xy" }));
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }));
    ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "Baseline" }));

    let cb_path = dir.child("study.curvebook");
    ok_cmd(&mut session, "project.save", json!({ "path": cb_path.to_string_lossy() }));
    assert!(!session.is_dirty());
    assert_eq!(session.file_path(), Some(cb_path.as_path()));

    // Edit one cell so `dirty == true` while `file_path == Some(cb_path)`.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 15.0 }));
    assert!(session.is_dirty());
    assert_eq!(session.file_path(), Some(cb_path.as_path()));

    (session, cb_path)
}

#[test]
fn project_export_pzfx_atomic_write_preserves_session_state_reports_warnings_and_roundtrips() {
    let dir = TempDir::new("export-pzfx");
    let (mut session, cb_path) = seed_rich_project_session(&dir);
    let undo_before = session.undo_depth();

    let out_path = dir.child("nested/export_copy.pzfx");
    let outcome = ok_cmd(&mut session, "project.exportPzfx", json!({ "path": out_path.to_string_lossy(), "return_bytes": true }));

    assert!(!outcome.mutated, "project.exportPzfx must not mutate document or undo stack");
    assert_eq!(session.undo_depth(), undo_before);
    assert_eq!(session.file_path(), Some(cb_path.as_path()), "exporting .pzfx must never change session file_path");
    assert!(session.is_dirty(), "exporting .pzfx must never clear unsaved dirty flag");

    assert_eq!(outcome.payload["format"], json!("pzfx"));
    assert_eq!(outcome.payload["data_sheet_count"], json!(1));
    assert_eq!(outcome.payload["info_sheet_count"], json!(1));
    assert!(outcome.payload["total_dropped_count"].as_u64().unwrap() >= 4);
    assert!(!outcome.warnings.is_empty(), "expected dropped-content warnings on outcome.warnings");
    assert!(!outcome.payload["warnings"].as_array().unwrap().is_empty());
    assert!(
        outcome.payload["notice"].as_str().unwrap().contains("not tested in other applications"),
        "expected honest §6.3 notice, got {:?}",
        outcome.payload["notice"]
    );

    let disk_bytes = fs::read(&out_path).unwrap();
    let b64_bytes = decode_base64(outcome.payload["bytes_base64"].as_str().unwrap());
    assert_eq!(disk_bytes, b64_bytes);

    // Re-import exported `.pzfx` and verify exact data-layer fidelity.
    let reimported = import_pzfx_bytes(&disk_bytes).unwrap();
    let diff = compare_pzfx_data_layer(session.document(), &reimported.document);
    assert!(diff.is_identical(), "round-trip data-layer diff: {diff:?}");

    // Path policy enforcement (`RejectExternal`) blocks writing outside policy without creating target.
    let mut blocked_sess = session.clone().with_path_policy(ExternalPathPolicy::RejectExternal);
    let blocked_path = dir.child("blocked.pzfx");
    let err = err_cmd(&mut blocked_sess, "project.exportPzfx", json!({ "path": blocked_path.to_string_lossy() }));
    assert_eq!(err.code, ErrorCode::PathEscape);
    assert!(!blocked_path.exists());
}

#[test]
fn project_export_prism_requires_experimental_opt_in_and_preserves_session_state() {
    let dir = TempDir::new("export-archive");
    let (mut session, cb_path) = seed_rich_project_session(&dir);
    let out_path = dir.child("study.prism");

    // 1. Omitted or `experimental: false` fails closed with UnsupportedFormat pointing to `.pzfx`.
    for params in [json!({ "path": out_path.to_string_lossy() }), json!({ "path": out_path.to_string_lossy(), "experimental": false })] {
        let err = err_cmd(&mut session, "project.exportPrism", params);
        assert_eq!(err.code, ErrorCode::UnsupportedFormat);
        assert!(err.message().contains(".pzfx"), "refusal should guide users to .pzfx: {}", err.message());
        assert!(!out_path.exists());
        assert_eq!(session.file_path(), Some(cb_path.as_path()));
        assert!(session.is_dirty());
    }

    // 2. Explicit `experimental: true` writes atomically, returns warnings, keeps session state intact, and re-imports.
    let outcome =
        ok_cmd(&mut session, "project.exportPrism", json!({ "path": out_path.to_string_lossy(), "experimental": true, "return_bytes": true }));
    assert!(!outcome.mutated);
    assert_eq!(session.file_path(), Some(cb_path.as_path()));
    assert!(session.is_dirty());
    assert_eq!(outcome.payload["format"], json!(DESC_PRISM.id));
    assert_eq!(outcome.payload["experimental"], json!(true));
    assert!(!outcome.warnings.is_empty());

    let disk_bytes = fs::read(&out_path).unwrap();
    let b64_bytes = decode_base64(outcome.payload["bytes_base64"].as_str().unwrap());
    assert_eq!(disk_bytes, b64_bytes);

    let mut target = Session::new();
    let imported = ok_cmd(&mut target, "table.importPrism", json!({ "bytes_base64": outcome.payload["bytes_base64"] }));
    assert!(imported.mutated);
    assert_eq!(target.document().data_sheets.len(), 1);
    assert_eq!(target.document().info_sheets.len(), 1);
}

#[test]
fn table_import_prism_recalculates_live_analyses_in_single_undo_step_preserving_imported_snapshots() {
    let mut doc = ProjectDocument::new("Mixed Live And Snapshot Archive").unwrap();
    doc.add_data_sheet(xy_sheet("xy-1", "Dose Response", "Dose", &[1.0, 2.0, 3.0], &[("Y", &[10.0, 20.0, 30.0])])).unwrap();

    // 1. Live-translated (non-ImportedSnapshot) analysis arriving Dirty.
    let live_spec = AnalysisSpec::new("res-live", "xy-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let mut live_res = ResultSheet::new(live_spec).unwrap();
    live_res.status = ResultStatus::Dirty;
    doc.add_result_sheet(live_res).unwrap();

    // 2. Sibling read-only ImportedSnapshot on the same data sheet.
    let snap_spec = AnalysisSpec::new(
        "res-snap",
        "xy-1",
        AnalysisKind::ImportedSnapshot(ImportedSnapshotSpec { original_kind_label: String::from("Cached Fit") }),
    )
    .unwrap();
    let mut snap_res = ResultSheet::new_imported(snap_spec, DESC_PRISM.id, "Cached Fit", "unverified mapping").unwrap();
    snap_res.tabs.push(ResultTab {
        id: String::from("cached-main"),
        title: String::from("Cached Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("Row 1")],
        rows: vec![vec![ResultCell::Text(String::from("IC50")), ResultCell::Number { value: 42.0, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    doc.add_result_sheet(snap_res).unwrap();

    // Package as a `.prism` ZIP containing `document.json` so `import_prism_bytes` yields both sheets intact.
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    let mut zip = ZipWriter::new();
    zip.add_file("document.json", doc_bytes).unwrap();
    let b64 = encode_base64(&zip.finish().unwrap());

    let mut session = Session::new();
    let before_doc = session.document().clone();
    let outcome = ok_cmd(&mut session, "table.importPrism", json!({ "bytes_base64": b64 }));
    assert!(outcome.mutated);
    assert!(
        outcome.payload["recalculated_sheets"].as_u64().unwrap_or(0) >= 1,
        "expected live-translated analysis recalculation count in payload: {:?}",
        outcome.payload
    );

    let live_after = session.document().result_sheet(&SheetId::new("res-live")).unwrap();
    assert_eq!(live_after.status, ResultStatus::UpToDate, "live-translated analysis must be recalculated during table.importPrism");
    assert!(!live_after.tabs.is_empty(), "recalculated live analysis must populate result tabs");

    let snap_after = session.document().result_sheet(&SheetId::new("res-snap")).unwrap();
    assert_eq!(snap_after.status, ResultStatus::Imported, "sibling ImportedSnapshot must remain untouched as ResultStatus::Imported");
    assert_eq!(snap_after.tabs[0].id, "cached-main");

    // Single undo step must roll back the entire import + recalculation cleanly.
    session.undo().unwrap();
    assert_eq!(session.document(), &before_doc);
}
