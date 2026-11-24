#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, MAX_SHEETS_PER_PROJECT, ProjectDocument, ResultCell, ResultStatus, SheetId,
    TableArchetype, TableKind,
};
use curvebook_engine::Session;
use curvebook_format::is_unpacked_project_dir;
use curvebook_transform::{PlateGeometry, PlateMapTemplate, PlateWellRole};
use serde_json::{Value, json};

use super::common::{ok_cmd as exec_ok, unique_temp_path};

fn exec_err(session: &mut Session, command_id: &str, params: Value) -> String {
    match session.execute(command_id, params) {
        Ok(ok) => panic!("{command_id} unexpectedly succeeded: {}", ok.message),
        Err(err) => err.to_string(),
    }
}

fn find_result_metric(session: &Session, res_id: &str, metric_label: &str) -> Option<f64> {
    let res = session.document().result_sheet(&SheetId::new(res_id))?;
    let tab = res.tabs.first()?;
    let row_idx = tab.row_labels.iter().position(|label| label.eq_ignore_ascii_case(metric_label))?;
    match tab.rows.get(row_idx)?.first()? {
        ResultCell::Number { value, .. } => Some(*value),
        _ => None,
    }
}

#[test]
fn g2_paste_link_missing_or_deleted_external_file_fails_loudly() {
    let mut session = Session::from_document(ProjectDocument::new("G2 External Link").expect("doc"));
    exec_ok(&mut session, "sheet.createData", json!({ "sheet_id": "col-1", "title": "Target", "kind": "Column", "rows": 4, "cols": 1 }));
    let before_undo = session.undo_depth();

    let missing_path = unique_temp_path("missing-link", "csv");
    let err = exec_err(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "col-1",
            "source_path": missing_path.to_string_lossy()
        }),
    );
    assert!(err.contains("failed reading external link file"), "unexpected error: {err}");
    assert_eq!(session.undo_depth(), before_undo, "failed command must not push undo state");
    let col_id = SheetId::new("col-1");
    let target = session.document().data_sheet(&col_id).expect("col-1");
    assert!(target.linked_regions.is_empty(), "target sheet must not retain broken link on error");

    let live_path = unique_temp_path("live-link", "csv");
    std::fs::write(&live_path, "10.5\n20.5\n").expect("write temp csv");
    exec_ok(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "col-1",
            "source_path": live_path.to_string_lossy()
        }),
    );
    let target_after = session.document().data_sheet(&col_id).expect("col-1");
    assert_eq!(target_after.linked_regions.len(), 1);
    assert_eq!(target_after.archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(10.5)));

    let _ = std::fs::remove_file(&live_path);
    let refresh_err = exec_err(&mut session, "project.recalculate", json!({ "include_external_links": true }));
    assert!(refresh_err.contains("failed reading linked file"), "unexpected refresh error: {refresh_err}");
}

#[test]
fn qol1_convert_archetype_preview_commit_lossy_and_undo_redo() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-1 Convert").expect("doc"));
    exec_ok(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "grp-1", "title": "Assay", "kind": "Grouped", "rows": 2, "cols": 2, "replicates": 2 }),
    );
    exec_ok(&mut session, "table.setGroupTitles", json!({ "sheet_id": "grp-1", "titles": ["Ctrl", "Drug"] }));
    exec_ok(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "grp-1",
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 10.0 },
                { "row": 0, "col": 0, "subcol": 1, "value": 12.0 },
                { "row": 1, "col": 0, "subcol": 0, "value": 14.0 },
                { "row": 0, "col": 1, "subcol": 0, "value": 30.0 },
                { "row": 1, "col": 1, "subcol": 0, "value": 40.0 }
            ]
        }),
    );
    exec_ok(
        &mut session,
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": "grp-1",
            "coords": [{ "row": 0, "col": 0, "subcol": 1 }],
            "reason_tag": "Bubble",
            "note": "Air pocket",
            "timestamp_utc": "2026-11-14T08:00:00Z"
        }),
    );
    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-stats", "source_sheet_id": "grp-1", "kind": "descriptive_stats" }));

    let depth_before_preview = session.undo_depth();
    let preview = exec_ok(&mut session, "table.convertArchetype", json!({ "sheet_id": "grp-1", "target_kind": "Column", "preview_only": true }));
    assert!(!preview.mutated, "preview_only must be read-only");
    assert_eq!(session.undo_depth(), depth_before_preview);
    assert_eq!(preview.payload["preview_only"], json!(true));
    assert_eq!(preview.payload["preview"]["preserved_exclusions"], json!(1));
    let grp_id = SheetId::new("grp-1");
    assert_eq!(session.document().data_sheet(&grp_id).expect("grp-1").kind(), TableKind::Grouped);

    // Create a Column table with non-numeric text in col 0 to test lossy conversion to XY refusal and allow_lossy override.
    exec_ok(&mut session, "sheet.createData", json!({ "sheet_id": "col-lossy", "title": "Lossy Source", "kind": "Column", "rows": 2, "cols": 2 }));
    exec_ok(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "col-lossy",
            "edits": [
                { "row": 0, "col": 0, "value": "NotNumeric" },
                { "row": 1, "col": 0, "value": 2.0 },
                { "row": 0, "col": 1, "value": 5.0 },
                { "row": 1, "col": 1, "value": 6.0 }
            ]
        }),
    );

    let lossy_err = exec_err(&mut session, "table.convertArchetype", json!({ "sheet_id": "col-lossy", "target_kind": "XY" }));
    assert!(
        lossy_err.to_ascii_lowercase().contains("lossy") || lossy_err.to_ascii_lowercase().contains("allow_lossy"),
        "expected lossy conversion refusal, got: {lossy_err}"
    );
    let allow_lossy_out =
        exec_ok(&mut session, "table.convertArchetype", json!({ "sheet_id": "col-lossy", "target_kind": "XY", "allow_lossy": true }));
    assert_eq!(allow_lossy_out.payload["preview"]["lossy"], json!(true));
    assert_eq!(session.document().data_sheet(&SheetId::new("col-lossy")).expect("col-lossy").kind(), TableKind::Xy);

    // Non-lossy Grouped -> Column commit preserves SheetId, exclusion notes, and triggers DAG recalculation.
    let committed = exec_ok(&mut session, "table.convertArchetype", json!({ "sheet_id": "grp-1", "target_kind": "Column" }));
    assert!(committed.mutated);
    let converted_ds = session.document().data_sheet(&grp_id).expect("grp-1 after conversion");
    assert_eq!(converted_ds.kind(), TableKind::Column);
    assert_eq!(converted_ds.archetype.column_count(), 4);
    assert_eq!(converted_ds.exclusion_notes.len(), 1);
    let res = session.document().result_sheet(&SheetId::new("res-stats")).expect("res");
    assert_eq!(res.status, ResultStatus::UpToDate);

    // Undo restores Grouped archetype, Redo re-applies Column archetype.
    assert!(session.undo().expect("undo conversion"));
    assert_eq!(session.document().data_sheet(&grp_id).expect("grp-1").kind(), TableKind::Grouped);
    assert!(session.redo().expect("redo conversion"));
    assert_eq!(session.document().data_sheet(&grp_id).expect("grp-1").kind(), TableKind::Column);
}

#[test]
fn qol2_import_plate_map_96_and_384_wells_and_sheet_limit() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-2 Plates").expect("doc"));

    // 1. 96-well template with Blank subtraction & Excluded well.
    let mut tpl96 = PlateMapTemplate::new("IC50 Plate", PlateGeometry::Wells96, "Dose").expect("tpl");
    tpl96.wells[0] = PlateWellRole::Blank;
    tpl96.wells[1] = PlateWellRole::Blank;
    tpl96.wells[2] = PlateWellRole::Treatment { group_name: String::from("DrugA"), dilution_x: 1.0, replicate: 1 };
    tpl96.wells[3] = PlateWellRole::Treatment { group_name: String::from("DrugA"), dilution_x: 1.0, replicate: 2 };
    tpl96.wells[4] = PlateWellRole::PositiveControl;

    let mut values96: Vec<Option<f64>> = vec![None; 96];
    values96[0] = Some(8.0);
    values96[1] = Some(12.0); // blank mean = 10.0
    values96[2] = Some(25.0); // -> 15.0
    values96[3] = Some(35.0); // -> 25.0
    values96[4] = Some(100.0);

    let out96 = exec_ok(
        &mut session,
        "table.importPlateMap",
        json!({
            "title": "Plate 96",
            "template": tpl96,
            "values": values96,
            "target_kind": "Grouped",
            "subtract_blank_mean": true
        }),
    );
    let id96 = SheetId::new(out96.payload["sheet_id"].as_str().expect("sheet_id"));
    let ds96 = session.document().data_sheet(&id96).expect("ds96");
    assert_eq!(ds96.kind(), TableKind::Grouped);
    assert_eq!(ds96.archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(15.0)));
    assert_eq!(ds96.archetype.data_cell(CellCoord::new(0, 0, 1)).map(|c| &c.value), Some(&CellValue::Number(25.0)));

    // 2. 384-well explicit `roles` + 2D 16x24 `values` matrix, imported twice to test unique SheetId generation + undo/redo.
    let mut roles384: Vec<Value> = vec![json!("Excluded"); 384];
    roles384[0] = json!({ "role": "treatment", "group_name": "CompoundB", "dilution_x": 0.5, "replicate": 1 });
    roles384[1] = json!({ "role": "treatment", "group_name": "CompoundB", "dilution_x": 0.5, "replicate": 2 });
    roles384[24] = json!("Blank");
    let mut matrix384: Vec<Vec<Option<f64>>> = vec![vec![None; 24]; 16];
    matrix384[0][0] = Some(42.0);
    matrix384[0][1] = Some(44.0);
    matrix384[1][0] = Some(2.0);

    let out384_a = exec_ok(
        &mut session,
        "table.importPlateMap",
        json!({
            "title": "Plate 384",
            "geometry": "Wells384",
            "roles": roles384,
            "values": matrix384,
            "subtract_blank_mean": true
        }),
    );
    let out384_b = exec_ok(
        &mut session,
        "table.importPlateMap",
        json!({
            "title": "Plate 384",
            "geometry": 384,
            "roles": roles384,
            "values": matrix384
        }),
    );
    let id384_a = out384_a.payload["sheet_id"].as_str().expect("id a");
    let id384_b = out384_b.payload["sheet_id"].as_str().expect("id b");
    assert_ne!(id384_a, id384_b, "duplicate titles must receive distinct unique SheetIds");
    assert_eq!(session.document().data_sheets.len(), 3);

    assert!(session.undo().expect("undo second 384 import"));
    assert_eq!(session.document().data_sheets.len(), 2);
    assert!(session.redo().expect("redo second 384 import"));
    assert_eq!(session.document().data_sheets.len(), 3);

    // Verify MAX_SHEETS_PER_PROJECT check prevents exceeding cap.
    let mut full_doc = ProjectDocument::new("Full Project").expect("doc");
    for i in 0..MAX_SHEETS_PER_PROJECT {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).expect("col");
        let ds = DataSheet::new(format!("s-{i}"), format!("Sheet {i}"), TableArchetype::Column(col)).expect("ds");
        full_doc.add_data_sheet(ds).expect("seed up to cap");
    }
    let mut cap_roles: Vec<Value> = vec![json!("Blank"); 96];
    cap_roles[0] = json!({ "role": "treatment", "group_name": "G", "dilution_x": 1.0, "replicate": 1 });
    let mut full_session = Session::from_document(full_doc);
    let cap_err = exec_err(
        &mut full_session,
        "table.importPlateMap",
        json!({
            "title": "Overflow Plate",
            "geometry": "Wells96",
            "roles": cap_roles,
            "values": vec![1.0; 96]
        }),
    );
    assert!(cap_err.to_ascii_lowercase().contains("limit") || cap_err.to_ascii_lowercase().contains("max") || cap_err.contains("500"), "{cap_err}");
}

#[test]
fn qol2_import_tidy_long_rows_and_european_csv() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-2 Tidy").expect("doc"));

    // Variant A: headers + rows + mapping object
    let out_rows = exec_ok(
        &mut session,
        "table.importTidyLong",
        json!({
            "title": "Tidy Grouped",
            "headers": ["Dose", "Group", "Rep", "Signal", "Excl"],
            "rows": [
                ["1.0", "Veh", "1", "10.0", "false"],
                ["1.0", "Veh", "2", "12.0", "true"],
                ["1.0", "Treated", "1", "25.0", ""]
            ],
            "mapping": {
                "x_col": 0,
                "group_col": 1,
                "replicate_col": 2,
                "value_col": 3,
                "excluded_col": 4
            },
            "target_kind": "Grouped"
        }),
    );
    let sid_rows = SheetId::new(out_rows.payload["sheet_id"].as_str().expect("sid"));
    let ds_rows = session.document().data_sheet(&sid_rows).expect("ds");
    assert_eq!(ds_rows.kind(), TableKind::Grouped);
    assert_eq!(ds_rows.archetype.data_cell(CellCoord::new(0, 0, 1)).map(|c| c.excluded), Some(true));
    assert_eq!(ds_rows.archetype.data_cell(CellCoord::new(0, 1, 0)).map(|c| &c.value), Some(&CellValue::Number(25.0)));

    // Variant B: semicolon CSV + comma decimal + flat column name mapping
    let csv = "Dose;Group;Signal\n0,5;Alpha;12,5\n1,5;Alpha;18,25\n";
    let out_csv = exec_ok(
        &mut session,
        "table.importTidyLong",
        json!({
            "title": "Tidy EU XY",
            "csv": csv,
            "delimiter": ";",
            "decimal_sep": ",",
            "x_col": "Dose",
            "group_col": "Group",
            "value_col": "Signal",
            "target_kind": "XY"
        }),
    );
    let sid_csv = SheetId::new(out_csv.payload["sheet_id"].as_str().expect("sid csv"));
    let ds_csv = session.document().data_sheet(&sid_csv).expect("ds csv");
    assert_eq!(ds_csv.kind(), TableKind::Xy);
    assert_eq!(ds_csv.archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(12.5)));
    assert_eq!(ds_csv.archetype.data_cell(CellCoord::new(1, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(18.25)));

    assert!(session.undo().expect("undo tidy csv"));
    assert!(session.document().data_sheet(&sid_csv).is_none());
    assert!(session.redo().expect("redo tidy csv"));
    assert!(session.document().data_sheet(&sid_csv).is_some());
}

#[test]
fn qol3_exclude_cells_with_reason_recalculates_and_audit_report() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-3 Audit").expect("doc"));
    exec_ok(&mut session, "sheet.createData", json!({ "sheet_id": "s1", "title": "Sheet One", "kind": "Column", "rows": 3, "cols": 1 }));
    exec_ok(&mut session, "sheet.createData", json!({ "sheet_id": "s2", "title": "Sheet Two", "kind": "Column", "rows": 3, "cols": 1 }));
    exec_ok(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "s1",
            "edits": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 1, "col": 0, "value": 20.0 },
                { "row": 2, "col": 0, "value": 90.0 }
            ]
        }),
    );
    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-1", "source_sheet_id": "s1", "kind": "descriptive_stats" }));
    let mean_before = find_result_metric(&session, "res-1", "Mean");
    assert_eq!(mean_before, Some(40.0));

    // Single cell with omitted timestamp_utc (uses unix:<secs>) and triggers DAG recalc.
    let excl_out = exec_ok(
        &mut session,
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": "s1",
            "row": 2,
            "col": 0,
            "reason_tag": "Outlier",
            "note": "Pipette spike"
        }),
    );
    let auto_ts = excl_out.payload["timestamp_utc"].as_str().expect("timestamp");
    assert!(auto_ts.starts_with("unix:"), "expected unix timestamp default, got {auto_ts}");

    let mean_after = find_result_metric(&session, "res-1", "Mean");
    assert_eq!(mean_after, Some(15.0), "DAG recalc should drop excluded 90.0");

    // Batch coords on s2 + one unannotated exclusion on s2.
    exec_ok(
        &mut session,
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": "s2",
            "coords": [[0, 0], { "row": 1, "col": 0 }],
            "reason_tag": "Contaminated",
            "timestamp_utc": "2026-11-14T11:30:00Z"
        }),
    );
    exec_ok(&mut session, "table.excludeCells", json!({ "sheet_id": "s2", "cells": [{ "row": 2, "col": 0 }], "excluded": true }));

    let depth_before_audit = session.undo_depth();
    let s1_audit = exec_ok(&mut session, "table.exclusionAuditReport", json!({ "sheet_id": "s1" }));
    assert!(!s1_audit.mutated);
    assert_eq!(s1_audit.payload["total_excluded"], json!(1));
    assert_eq!(s1_audit.payload["annotated_count"], json!(1));
    assert_eq!(s1_audit.payload["exclusions"][0]["reason_tag"], json!("Outlier"));
    assert_eq!(s1_audit.payload["exclusions"][0]["note"], json!("Pipette spike"));
    assert_eq!(s1_audit.payload["exclusions"][0]["coord"], json!({ "row": 2, "col": 0, "subcol": 0 }));

    let all_audit = exec_ok(&mut session, "table.exclusionAuditReport", json!({}));
    assert!(!all_audit.mutated);
    assert_eq!(all_audit.payload["total_excluded"], json!(4));
    assert_eq!(all_audit.payload["annotated_count"], json!(3));
    assert_eq!(all_audit.payload["unannotated_count"], json!(1));
    assert_eq!(session.undo_depth(), depth_before_audit);

    let bad_tag_err = exec_err(&mut session, "table.excludeCellsWithReason", json!({ "sheet_id": "s1", "row": 0, "col": 0, "reason_tag": "   " }));
    assert!(bad_tag_err.contains("reason_tag"));
    let ds1 = session.document().data_sheet(&SheetId::new("s1")).expect("s1");
    assert_eq!(ds1.exclusion_note(CellCoord::new(2, 0, 0)).map(|n| n.reason_tag.as_str()), Some("Outlier"));
}

#[test]
fn qol11_checkpoints_and_unpacked_dir_open_save() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-11 Checkpoints").expect("doc"));
    exec_ok(&mut session, "sheet.createData", json!({ "sheet_id": "base-1", "title": "Base", "kind": "Column", "rows": 2, "cols": 1 }));
    exec_ok(&mut session, "table.setCell", json!({ "sheet_id": "base-1", "row": 0, "col": 0, "value": 11.0 }));

    let cp1 = exec_ok(
        &mut session,
        "project.createCheckpoint",
        json!({ "name": "Baseline", "note": "Initial values", "timestamp_utc": "2026-11-14T09:00:00Z" }),
    );
    let cp1_id = cp1.payload["id"].as_str().expect("cp1 id").to_owned();
    assert_eq!(session.document().checkpoints.len(), 1);
    let parsed_snap: ProjectDocument = serde_json::from_str(&session.document().checkpoints[0].snapshot_json).expect("valid snapshot json");
    assert!(parsed_snap.checkpoints.is_empty(), "checkpoint snapshot_json must strip nested checkpoints");

    // Mutate project further and create a second checkpoint.
    exec_ok(&mut session, "sheet.createData", json!({ "sheet_id": "extra-2", "title": "Extra", "kind": "Column", "rows": 2, "cols": 1 }));
    exec_ok(&mut session, "table.setCell", json!({ "sheet_id": "base-1", "row": 0, "col": 0, "value": 99.0 }));
    let cp2 = exec_ok(&mut session, "project.createCheckpoint", json!({ "name": "After Edit" }));
    let cp2_created = cp2.payload["created_utc"].as_str().expect("created_utc");
    assert!(cp2_created.starts_with("unix:"));

    // listCheckpoints is read-only and omits raw snapshot_json.
    let depth_before_list = session.undo_depth();
    let listed = exec_ok(&mut session, "project.listCheckpoints", json!({}));
    assert!(!listed.mutated);
    assert_eq!(session.undo_depth(), depth_before_list);
    assert_eq!(listed.payload["count"], json!(2));
    assert!(listed.payload["checkpoints"][0].get("snapshot_json").is_none());
    assert_eq!(listed.payload["checkpoints"][0]["name"], json!("Baseline"));

    // Save to `.curvebook.d` auto-detected by suffix, and to explicit `unpacked: true` folder without suffix.
    let unpacked_dir = unique_temp_path("project", "curvebook.d");
    let save_out = exec_ok(&mut session, "project.save", json!({ "path": unpacked_dir.to_string_lossy() }));
    assert!(!save_out.mutated, "project.save must remain read-only");
    assert_eq!(save_out.payload["unpacked"], json!(true));
    assert!(is_unpacked_project_dir(&unpacked_dir));

    let flagged_dir = unique_temp_path("project-flagged", "dir");
    let save_flagged = exec_ok(&mut session, "project.save", json!({ "path": flagged_dir.to_string_lossy(), "unpacked": true }));
    assert_eq!(save_flagged.payload["unpacked"], json!(true));
    assert!(is_unpacked_project_dir(&flagged_dir));

    let mut reopened = Session::new();
    let open_out = exec_ok(&mut reopened, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }));
    assert!(open_out.mutated);
    assert_eq!(open_out.payload["unpacked"], json!(true));
    assert_eq!(reopened.document().checkpoints.len(), 2);

    // Restore Baseline checkpoint on reopened session: sheets revert, checkpoints stay intact!
    exec_ok(&mut reopened, "project.restoreCheckpoint", json!({ "id": cp1_id }));
    assert!(reopened.document().data_sheet(&SheetId::new("extra-2")).is_none());
    let base = reopened.document().data_sheet(&SheetId::new("base-1")).expect("base");
    assert_eq!(base.archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(11.0)));
    assert_eq!(reopened.document().checkpoints.len(), 2, "restoreCheckpoint must keep doc.checkpoints intact");

    // Delete second checkpoint by name, then verify error on nonexistent delete.
    exec_ok(&mut reopened, "project.deleteCheckpoint", json!({ "name": "After Edit" }));
    assert_eq!(reopened.document().checkpoints.len(), 1);
    let del_err = exec_err(&mut reopened, "project.deleteCheckpoint", json!({ "name": "Missing" }));
    assert!(del_err.contains("not found"));

    // Clean up temporary directories.
    let _ = std::fs::remove_dir_all(&unpacked_dir);
    let _ = std::fs::remove_dir_all(&flagged_dir);
}
