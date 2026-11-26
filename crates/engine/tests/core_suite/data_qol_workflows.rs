#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{CellCoord, MAX_SHEETS_PER_PROJECT, ProjectDocument, ResultStatus, SheetId, TableKind};
use curvebook_engine::Session;
use curvebook_format::is_unpacked_project_dir;
use curvebook_transform::{PlateGeometry, PlateMapTemplate, PlateWellRole};
use serde_json::{Value, json};

use super::common::{TempDir, cell_num, cell_val, col_sheet, ds, err_cmd, ok_cmd, rs, run_cmds, tab_metric};

#[test]
fn g2_paste_link_missing_or_deleted_external_file_fails_loudly() {
    let dir = TempDir::new("g2-link");
    let mut session = Session::from_document(ProjectDocument::new("G2 External Link").expect("doc"));
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "col-1", "title": "Target", "kind": "Column", "rows": 4, "cols": 1 }));
    let before_undo = session.undo_depth();

    let missing_path = dir.child("missing-link.csv");
    let err =
        err_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "col-1", "source_path": missing_path.to_string_lossy() })).to_string();
    assert!(err.contains("failed reading external link file"), "unexpected error: {err}");
    assert_eq!(session.undo_depth(), before_undo, "failed command must not push undo state");
    assert!(ds(&session, "col-1").linked_regions.is_empty(), "target sheet must not retain broken link on error");

    let live_path = dir.child("live-link.csv");
    std::fs::write(&live_path, "10.5\n20.5\n").expect("write temp csv");
    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "col-1", "source_path": live_path.to_string_lossy() }));
    assert_eq!(ds(&session, "col-1").linked_regions.len(), 1);
    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), Some(10.5));

    std::fs::remove_file(&live_path).expect("remove live csv");
    let refresh_err = err_cmd(&mut session, "project.recalculate", json!({ "include_external_links": true })).to_string();
    assert!(refresh_err.contains("failed reading linked file"), "unexpected refresh error: {refresh_err}");
}

#[test]
fn qol1_convert_archetype_preview_commit_lossy_and_undo_redo() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-1 Convert").expect("doc"));
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "grp-1", "title": "Assay", "kind": "Grouped", "rows": 2, "cols": 2, "replicates": 2 })),
            ("table.setGroupTitles", json!({ "sheet_id": "grp-1", "titles": ["Ctrl", "Drug"] })),
            (
                "table.setCells",
                json!({ "sheet_id": "grp-1", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 10.0 }, { "row": 0, "col": 0, "subcol": 1, "value": 12.0 }, { "row": 1, "col": 0, "subcol": 0, "value": 14.0 }, { "row": 0, "col": 1, "subcol": 0, "value": 30.0 }, { "row": 1, "col": 1, "subcol": 0, "value": 40.0 }] }),
            ),
            (
                "table.excludeCellsWithReason",
                json!({ "sheet_id": "grp-1", "coords": [{ "row": 0, "col": 0, "subcol": 1 }], "reason_tag": "Bubble", "note": "Air pocket", "timestamp_utc": "2026-11-14T08:00:00Z" }),
            ),
            ("analysis.run", json!({ "sheet_id": "res-stats", "source_sheet_id": "grp-1", "kind": "descriptive_stats" })),
        ],
    );

    let depth_before = session.undo_depth();
    let preview = ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "grp-1", "target_kind": "Column", "preview_only": true }));
    assert!(!preview.mutated);
    assert_eq!(session.undo_depth(), depth_before);
    assert_eq!(preview.payload["preview_only"], true);
    assert_eq!(preview.payload["preview"]["preserved_exclusions"], 1);
    assert_eq!(ds(&session, "grp-1").kind(), TableKind::Grouped);

    // Lossy conversion refusal and allow_lossy override.
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "col-lossy", "title": "Lossy Source", "kind": "Column", "rows": 2, "cols": 2 })),
            (
                "table.setCells",
                json!({ "sheet_id": "col-lossy", "edits": [{ "row": 0, "col": 0, "value": "NotNumeric" }, { "row": 1, "col": 0, "value": 2.0 }, { "row": 0, "col": 1, "value": 5.0 }, { "row": 1, "col": 1, "value": 6.0 }] }),
            ),
        ],
    );
    let lossy_err =
        err_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "col-lossy", "target_kind": "XY" })).to_string().to_ascii_lowercase();
    assert!(lossy_err.contains("lossy") || lossy_err.contains("allow_lossy"), "got: {lossy_err}");

    let allow = ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "col-lossy", "target_kind": "XY", "allow_lossy": true }));
    assert_eq!(allow.payload["preview"]["lossy"], true);
    assert_eq!(ds(&session, "col-lossy").kind(), TableKind::Xy);

    // Non-lossy Grouped -> Column commit preserves SheetId, exclusion notes, DAG recalculation, and undo/redo.
    let committed = ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "grp-1", "target_kind": "Column" }));
    assert!(committed.mutated);
    let conv = ds(&session, "grp-1");
    assert_eq!(conv.kind(), TableKind::Column);
    assert_eq!(conv.archetype.column_count(), 4);
    assert_eq!(conv.exclusion_notes.len(), 1);
    assert_eq!(rs(&session, "res-stats").status, ResultStatus::UpToDate);

    assert!(session.undo().expect("undo conversion"));
    assert_eq!(ds(&session, "grp-1").kind(), TableKind::Grouped);
    assert!(session.redo().expect("redo conversion"));
    assert_eq!(ds(&session, "grp-1").kind(), TableKind::Column);
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
    for (i, v) in [(0, 8.0), (1, 12.0), (2, 25.0), (3, 35.0), (4, 100.0)] {
        values96[i] = Some(v);
    }
    let out96 = ok_cmd(
        &mut session,
        "table.importPlateMap",
        json!({ "title": "Plate 96", "template": tpl96, "values": values96, "target_kind": "Grouped", "subtract_blank_mean": true }),
    );
    let id96 = out96.payload["sheet_id"].as_str().expect("sheet_id");
    assert_eq!(ds(&session, id96).kind(), TableKind::Grouped);
    assert_eq!(cell_num(&session, id96, 0, 0, 0), Some(15.0));
    assert_eq!(cell_num(&session, id96, 0, 0, 1), Some(25.0));

    // 2. 384-well explicit roles + 2D 16x24 values matrix, imported twice for unique SheetIds + undo/redo.
    let mut roles384: Vec<Value> = vec![json!("Excluded"); 384];
    roles384[0] = json!({ "role": "treatment", "group_name": "CompoundB", "dilution_x": 0.5, "replicate": 1 });
    roles384[1] = json!({ "role": "treatment", "group_name": "CompoundB", "dilution_x": 0.5, "replicate": 2 });
    roles384[24] = json!("Blank");
    let mut matrix384: Vec<Vec<Option<f64>>> = vec![vec![None; 24]; 16];
    matrix384[0][0] = Some(42.0);
    matrix384[0][1] = Some(44.0);
    matrix384[1][0] = Some(2.0);

    let out_a = ok_cmd(
        &mut session,
        "table.importPlateMap",
        json!({ "title": "Plate 384", "geometry": "Wells384", "roles": roles384, "values": matrix384, "subtract_blank_mean": true }),
    );
    let out_b =
        ok_cmd(&mut session, "table.importPlateMap", json!({ "title": "Plate 384", "geometry": 384, "roles": roles384, "values": matrix384 }));
    assert_ne!(out_a.payload["sheet_id"], out_b.payload["sheet_id"]);
    assert_eq!(session.document().data_sheets.len(), 3);
    assert!(session.undo().expect("undo"));
    assert_eq!(session.document().data_sheets.len(), 2);
    assert!(session.redo().expect("redo"));
    assert_eq!(session.document().data_sheets.len(), 3);

    // MAX_SHEETS_PER_PROJECT check prevents exceeding cap.
    let mut full_doc = ProjectDocument::new("Full Project").expect("doc");
    for i in 0..MAX_SHEETS_PER_PROJECT {
        full_doc.add_data_sheet(col_sheet(&format!("s-{i}"), &format!("Sheet {i}"), &[("A", &[])])).expect("seed up to cap");
    }
    let mut cap_roles: Vec<Value> = vec![json!("Blank"); 96];
    cap_roles[0] = json!({ "role": "treatment", "group_name": "G", "dilution_x": 1.0, "replicate": 1 });
    let mut full_session = Session::from_document(full_doc);
    let cap_err = err_cmd(
        &mut full_session,
        "table.importPlateMap",
        json!({ "title": "Overflow Plate", "geometry": "Wells96", "roles": cap_roles, "values": vec![1.0; 96] }),
    )
    .to_string();
    assert!(cap_err.to_ascii_lowercase().contains("limit") || cap_err.to_ascii_lowercase().contains("max") || cap_err.contains("500"), "{cap_err}");
}

#[test]
fn qol2_import_tidy_long_rows_and_european_csv() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-2 Tidy").expect("doc"));

    let out_rows = ok_cmd(
        &mut session,
        "table.importTidyLong",
        json!({
            "title": "Tidy Grouped",
            "headers": ["Dose", "Group", "Rep", "Signal", "Excl"],
            "rows": [["1.0", "Veh", "1", "10.0", "false"], ["1.0", "Veh", "2", "12.0", "true"], ["1.0", "Treated", "1", "25.0", ""]],
            "mapping": { "x_col": 0, "group_col": 1, "replicate_col": 2, "value_col": 3, "excluded_col": 4 },
            "target_kind": "Grouped"
        }),
    );
    let sid_rows = out_rows.payload["sheet_id"].as_str().expect("sid");
    assert_eq!(ds(&session, sid_rows).kind(), TableKind::Grouped);
    assert_eq!(cell_val(&session, sid_rows, 0, 0, 1).map(|c| c.excluded), Some(true));
    assert_eq!(cell_num(&session, sid_rows, 0, 1, 0), Some(25.0));

    let out_csv = ok_cmd(
        &mut session,
        "table.importTidyLong",
        json!({ "title": "Tidy EU XY", "csv": "Dose;Group;Signal\n0,5;Alpha;12,5\n1,5;Alpha;18,25\n", "delimiter": ";", "decimal_sep": ",", "x_col": "Dose", "group_col": "Group", "value_col": "Signal", "target_kind": "XY" }),
    );
    let sid_csv = out_csv.payload["sheet_id"].as_str().expect("sid csv");
    assert_eq!(ds(&session, sid_csv).kind(), TableKind::Xy);
    assert_eq!(cell_num(&session, sid_csv, 0, 0, 0), Some(12.5));
    assert_eq!(cell_num(&session, sid_csv, 1, 0, 0), Some(18.25));

    assert!(session.undo().expect("undo tidy csv"));
    assert!(session.document().data_sheet(&SheetId::new(sid_csv)).is_none());
    assert!(session.redo().expect("redo tidy csv"));
    assert!(session.document().data_sheet(&SheetId::new(sid_csv)).is_some());
}

#[test]
fn qol3_exclude_cells_with_reason_recalculates_and_audit_report() {
    let mut session = Session::from_document(ProjectDocument::new("QOL-3 Audit").expect("doc"));
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "s1", "title": "Sheet One", "kind": "Column", "rows": 3, "cols": 1 })),
            ("sheet.createData", json!({ "sheet_id": "s2", "title": "Sheet Two", "kind": "Column", "rows": 3, "cols": 1 })),
            (
                "table.setCells",
                json!({ "sheet_id": "s1", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 20.0 }, { "row": 2, "col": 0, "value": 90.0 }] }),
            ),
            ("analysis.run", json!({ "sheet_id": "res-1", "source_sheet_id": "s1", "kind": "descriptive_stats" })),
        ],
    );
    assert_eq!(tab_metric(&session, "res-1", "Mean"), Some(40.0));

    let excl_out = ok_cmd(
        &mut session,
        "table.excludeCellsWithReason",
        json!({ "sheet_id": "s1", "row": 2, "col": 0, "reason_tag": "Outlier", "note": "Pipette spike" }),
    );
    assert!(excl_out.payload["timestamp_utc"].as_str().expect("ts").starts_with("unix:"));
    assert_eq!(tab_metric(&session, "res-1", "Mean"), Some(15.0));

    run_cmds(
        &mut session,
        &[
            (
                "table.excludeCellsWithReason",
                json!({ "sheet_id": "s2", "coords": [[0, 0], { "row": 1, "col": 0 }], "reason_tag": "Contaminated", "timestamp_utc": "2026-11-14T11:30:00Z" }),
            ),
            ("table.excludeCells", json!({ "sheet_id": "s2", "cells": [{ "row": 2, "col": 0 }], "excluded": true })),
        ],
    );

    let depth_before = session.undo_depth();
    let s1_audit = ok_cmd(&mut session, "table.exclusionAuditReport", json!({ "sheet_id": "s1" }));
    assert!(!s1_audit.mutated);
    assert_eq!(s1_audit.payload["total_excluded"], 1);
    assert_eq!(s1_audit.payload["annotated_count"], 1);
    assert_eq!(s1_audit.payload["exclusions"][0]["reason_tag"], "Outlier");
    assert_eq!(s1_audit.payload["exclusions"][0]["note"], "Pipette spike");
    assert_eq!(s1_audit.payload["exclusions"][0]["coord"], json!({ "row": 2, "col": 0, "subcol": 0 }));

    let all_audit = ok_cmd(&mut session, "table.exclusionAuditReport", json!({}));
    assert!(!all_audit.mutated);
    assert_eq!(all_audit.payload["total_excluded"], 4);
    assert_eq!(all_audit.payload["annotated_count"], 3);
    assert_eq!(all_audit.payload["unannotated_count"], 1);
    assert_eq!(session.undo_depth(), depth_before);

    let bad_tag_err =
        err_cmd(&mut session, "table.excludeCellsWithReason", json!({ "sheet_id": "s1", "row": 0, "col": 0, "reason_tag": "   " })).to_string();
    assert!(bad_tag_err.contains("reason_tag"));
    assert_eq!(ds(&session, "s1").exclusion_note(CellCoord::new(2, 0, 0)).map(|n| n.reason_tag.as_str()), Some("Outlier"));
}

#[test]
fn qol11_checkpoints_and_unpacked_dir_open_save() {
    let dir = TempDir::new("qol11-cp");
    let mut session = Session::from_document(ProjectDocument::new("QOL-11 Checkpoints").expect("doc"));
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "base-1", "title": "Base", "kind": "Column", "rows": 2, "cols": 1 })),
            ("table.setCell", json!({ "sheet_id": "base-1", "row": 0, "col": 0, "value": 11.0 })),
        ],
    );

    let cp1 = ok_cmd(
        &mut session,
        "project.createCheckpoint",
        json!({ "name": "Baseline", "note": "Initial values", "timestamp_utc": "2026-11-14T09:00:00Z" }),
    );
    let cp1_id = cp1.payload["id"].as_str().expect("cp1 id").to_owned();
    assert_eq!(session.document().checkpoints.len(), 1);
    let parsed_snap: ProjectDocument = serde_json::from_str(&session.document().checkpoints[0].snapshot_json).expect("valid snapshot json");
    assert!(parsed_snap.checkpoints.is_empty());

    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "extra-2", "title": "Extra", "kind": "Column", "rows": 2, "cols": 1 })),
            ("table.setCell", json!({ "sheet_id": "base-1", "row": 0, "col": 0, "value": 99.0 })),
        ],
    );
    let cp2 = ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "After Edit" }));
    assert!(cp2.payload["created_utc"].as_str().expect("created_utc").starts_with("unix:"));

    let depth_before = session.undo_depth();
    let listed = ok_cmd(&mut session, "project.listCheckpoints", json!({}));
    assert!(!listed.mutated && session.undo_depth() == depth_before);
    assert_eq!(listed.payload["count"], 2);
    assert!(listed.payload["checkpoints"][0].get("snapshot_json").is_none());
    assert_eq!(listed.payload["checkpoints"][0]["name"], "Baseline");

    let (unpacked_dir, flagged_dir) = (dir.child("project.curvebook.d"), dir.child("project-flagged.dir"));
    let save_out = ok_cmd(&mut session, "project.save", json!({ "path": unpacked_dir.to_string_lossy() }));
    assert!(!save_out.mutated && save_out.payload["unpacked"] == true && is_unpacked_project_dir(&unpacked_dir));
    let save_flagged = ok_cmd(&mut session, "project.save", json!({ "path": flagged_dir.to_string_lossy(), "unpacked": true }));
    assert!(save_flagged.payload["unpacked"] == true && is_unpacked_project_dir(&flagged_dir));

    let mut reopened = Session::new();
    let open_out = ok_cmd(&mut reopened, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }));
    assert!(open_out.mutated && open_out.payload["unpacked"] == true && reopened.document().checkpoints.len() == 2);

    ok_cmd(&mut reopened, "project.restoreCheckpoint", json!({ "id": cp1_id }));
    assert!(reopened.document().data_sheet(&SheetId::new("extra-2")).is_none());
    assert_eq!(cell_num(&reopened, "base-1", 0, 0, 0), Some(11.0));
    assert_eq!(reopened.document().checkpoints.len(), 2);

    ok_cmd(&mut reopened, "project.deleteCheckpoint", json!({ "name": "After Edit" }));
    assert_eq!(reopened.document().checkpoints.len(), 1);
    assert!(err_cmd(&mut reopened, "project.deleteCheckpoint", json!({ "name": "Missing" })).to_string().contains("not found"));
}
