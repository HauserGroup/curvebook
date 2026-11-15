#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression tests for engine-level [`ExternalPathPolicy`] confinement (`R1-S12-01` / `R1-S07-02`).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lumencraft_doc::{
    CellCoord, CellValue, ErrorCode, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, ProjectDocument,
    SheetCellRange, SheetId,
};
use lumencraft_engine::{ExternalPathPolicy, Session};
use lumencraft_transform::{PlateGeometry, PlateMapTemplate, PlateWellRole};
use serde_json::json;

static SEQ: AtomicU64 = AtomicU64::new(1);

struct TempWorkspace {
    root: PathBuf,
    workdir: PathBuf,
    outside: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("lc-engine-policy-{label}-{}-{id}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        fs::create_dir_all(&workdir).expect("workdir");
        fs::create_dir_all(&outside).expect("outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn build_doc_with_external_link(external_path: &str) -> ProjectDocument {
    let mut builder = Session::from_document(ProjectDocument::new("Stored Link Doc").expect("doc"));
    builder.execute("sheet.createData", json!({ "sheet_id": "col-1", "title": "Target", "kind": "Column", "rows": 4, "cols": 1 })).expect("sheet");
    builder.execute("table.setCells", json!({ "sheet_id": "col-1", "edits": [{ "row": 0, "col": 0, "value": 42.0 }] })).expect("seed cell");
    let mut doc = builder.document().clone();
    let ds = doc.data_sheet_mut(&SheetId::new("col-1")).expect("col-1");
    ds.linked_regions.push(LinkedDataRegion {
        id: String::from("ext-link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: external_path.to_owned(), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("stored external link"),
    });
    doc
}

#[test]
fn workdir_policy_blocks_stored_external_link_escapes_during_recalculate() {
    let ws = TempWorkspace::new("stored-link");
    let secret_csv = ws.outside.join("secret.csv");
    fs::write(&secret_csv, "999.0\n888.0\n").expect("write outside");

    let inside_csv = ws.workdir.join("nested").join("allowed.csv");
    fs::create_dir_all(inside_csv.parent().expect("parent")).expect("mkdir nested");
    fs::write(&inside_csv, "11.5\n22.5\n").expect("write inside");

    let policy = ExternalPathPolicy::workdir(&ws.workdir).expect("workdir policy");

    let mut escape_cases =
        vec![secret_csv.display().to_string(), String::from("../outside/secret.csv"), String::from("nested/../../outside/secret.csv")];

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let sym_escape = ws.workdir.join("sym_secret.csv");
        symlink(&secret_csv, &sym_escape).expect("symlink escape");
        escape_cases.push(String::from("sym_secret.csv"));

        let dangling_sym = ws.workdir.join("dangling_escape.csv");
        symlink(ws.outside.join("never_existed.csv"), &dangling_sym).expect("dangling symlink");
        escape_cases.push(String::from("dangling_escape.csv"));
    }

    for bad_path in escape_cases {
        let doc = build_doc_with_external_link(&bad_path);
        let mut session = Session::from_document(doc).with_path_policy(policy.clone());
        let err =
            session.execute("project.recalculate", json!({ "include_external_links": true })).expect_err("must reject outside stored external link");
        assert_eq!(err.code, ErrorCode::PathEscape, "expected PathEscape for `{bad_path}`, got {err:?}");

        let val =
            session.document().data_sheet(&SheetId::new("col-1")).expect("col-1").archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value);
        assert_eq!(val, Some(&CellValue::Number(42.0)), "secret must not enter data cell for `{bad_path}`");

        // Excluding external links stays safe even when an external link exists on the sheet.
        session
            .execute("project.recalculate", json!({ "include_external_links": false }))
            .expect("recalculate with include_external_links=false should succeed");
    }

    // Valid relative path inside workdir succeeds and updates cells.
    let ok_doc = build_doc_with_external_link("nested/allowed.csv");
    let mut ok_session = Session::from_document(ok_doc).with_path_policy(policy);
    ok_session.execute("project.recalculate", json!({ "include_external_links": true })).expect("allowed inside link");
    let updated =
        ok_session.document().data_sheet(&SheetId::new("col-1")).expect("col-1").archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value);
    assert_eq!(updated, Some(&CellValue::Number(11.5)));
}

#[test]
fn workdir_policy_guards_command_path_params_paste_link_spec_and_preserves_plate_map_template() {
    let ws = TempWorkspace::new("cmd-params");
    let secret_csv = ws.outside.join("secret.csv");
    fs::write(&secret_csv, "99.0\n").expect("write outside");
    let inside_csv = ws.workdir.join("data.csv");
    fs::write(&inside_csv, "7.0\n8.0\n").expect("write inside");

    let policy = ExternalPathPolicy::workdir(&ws.workdir).expect("workdir policy");
    let mut session = Session::from_document(ProjectDocument::new("Cmd Params").expect("doc")).with_path_policy(policy);
    session.execute("sheet.createData", json!({ "sheet_id": "col-1", "title": "Target", "kind": "Column", "rows": 4, "cols": 1 })).expect("sheet");

    // 1. table.pasteLink via `path` escape is rejected.
    let err = session.execute("table.pasteLink", json!({ "target_sheet_id": "col-1", "path": "../outside/secret.csv" })).expect_err("escape path");
    assert_eq!(err.code, ErrorCode::PathEscape);

    // 2. table.pasteLink via `link_spec.source.ExternalDelimitedFile.path` escape is rejected.
    let err_spec = session
        .execute(
            "table.pasteLink",
            json!({
                "target_sheet_id": "col-1",
                "link_spec": {
                    "source": {
                        "ExternalDelimitedFile": {
                            "path": secret_csv.display().to_string(),
                            "delimiter": ",",
                            "decimal_separator": "."
                        }
                    },
                    "filter": {
                        "row_window": null,
                        "col_window": null,
                        "decimate_every_k": null,
                        "unstack_group_column": null,
                        "parse_asterisk_exclusions": true
                    },
                    "placement": {
                        "mode": "Overwrite",
                        "anchor": { "row": 0, "col": 0, "subcol": 0 },
                        "propagate_title_header": false,
                        "skip_contiguous_blanks": false,
                        "transpose": false
                    }
                }
            }),
        )
        .expect_err("escape link_spec");
    assert_eq!(err_spec.code, ErrorCode::PathEscape);

    // 3. Relative path inside workdir succeeds for pasteLink, exportCsv, and project.save.
    session.execute("table.pasteLink", json!({ "target_sheet_id": "col-1", "path": "data.csv" })).expect("inside pasteLink");
    session.execute("table.exportCsv", json!({ "sheet_id": "col-1", "path": "exports/out.csv" })).expect("inside exportCsv");
    assert!(ws.workdir.join("exports/out.csv").exists());

    // 4. Write escapes fail even on export commands outside table/project modules.
    let save_err = session.execute("project.save", json!({ "path": "../outside/escape.lumencraft" })).expect_err("save escape");
    assert_eq!(save_err.code, ErrorCode::PathEscape);
    assert!(!ws.outside.join("escape.lumencraft").exists());

    let csv_err = session.execute("table.exportCsv", json!({ "sheet_id": "col-1", "path": "../outside/leak.csv" })).expect_err("exportCsv escape");
    assert_eq!(csv_err.code, ErrorCode::PathEscape);
    assert!(!ws.outside.join("leak.csv").exists());

    // 5. Non-path `template` parameter on `table.importPlateMap` remains accepted under Workdir policy.
    let mut roles = vec![PlateWellRole::Blank, PlateWellRole::PositiveControl, PlateWellRole::Excluded];
    roles.resize(96, PlateWellRole::Treatment { group_name: String::from("Drug"), dilution_x: 1.0, replicate: 1 });
    let tpl = PlateMapTemplate::from_wells("P96", PlateGeometry::Wells96, "Conc", roles).expect("tpl");
    session.execute("table.importPlateMap", json!({ "values": vec![1.0_f64; 96], "template": tpl })).expect("importPlateMap under Workdir");
}

#[test]
fn reject_external_policy_and_non_regular_file_guards() {
    let ws = TempWorkspace::new("reject-ext");
    let inside_csv = ws.workdir.join("inside.csv");
    fs::write(&inside_csv, "1.0\n2.0\n").expect("write inside");
    let subdir = ws.workdir.join("folder");
    fs::create_dir_all(&subdir).expect("mkdir folder");

    // 1. RejectExternal denies external file reads/writes and external link refreshes while allowing inline imports.
    let doc = build_doc_with_external_link(&inside_csv.display().to_string());
    let mut locked = Session::from_document(doc).with_path_policy(ExternalPathPolicy::RejectExternal);
    let recalc_err = locked
        .execute("project.recalculate", json!({ "include_external_links": true }))
        .expect_err("RejectExternal must refuse external link refresh");
    assert_eq!(recalc_err.code, ErrorCode::PathEscape);

    let import_err = locked
        .execute("table.importCsv", json!({ "path": inside_csv.display().to_string() }))
        .expect_err("RejectExternal must refuse external path read");
    assert_eq!(import_err.code, ErrorCode::PathEscape);

    locked.execute("table.importCsv", json!({ "csv": "X,Y\n1,2\n" })).expect("inline csv should still succeed");

    // 2. Non-regular files (directories, /dev/null, FIFOs) are rejected without opening/blocking.
    let mut wd_session = Session::from_document(ProjectDocument::new("NonRegular").expect("doc"))
        .with_path_policy(ExternalPathPolicy::workdir(&ws.workdir).expect("workdir"));
    let dir_err = wd_session.execute("table.importCsv", json!({ "path": "folder" })).expect_err("directory as CSV");
    assert_eq!(dir_err.code, ErrorCode::BadInput);

    #[cfg(unix)]
    {
        let mut unconstrained = Session::from_document(ProjectDocument::new("UnconstrainedNonReg").expect("doc"));
        let dev_null = std::path::Path::new("/dev/null");
        if dev_null.exists() {
            let dev_err = unconstrained.execute("table.importCsv", json!({ "path": "/dev/null" })).expect_err("/dev/null rejected");
            assert_eq!(dev_err.code, ErrorCode::BadInput);
        }

        let fifo = ws.workdir.join("stream.csv");
        if let Ok(status) = std::process::Command::new("mkfifo").arg(&fifo).status()
            && status.success()
        {
            let fifo_err = wd_session.execute("table.importCsv", json!({ "path": "stream.csv" })).expect_err("FIFO rejected without hanging");
            assert_eq!(fifo_err.code, ErrorCode::BadInput);
        }
    }
}
