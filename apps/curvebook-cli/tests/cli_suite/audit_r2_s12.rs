//! Permanent Round 2 (`R0.19`) Slice `S12` audit regression suite for `curvebook-cli`.
//!
//! Covers:
//! - `R2-S12-02`: `is_unpacked_project_path`, `convert`, and `<cmd> --out` never classify `.` or
//!   plain directories without `.curvebook.d` suffix as unpacked projects or delete `<dir>/sheets/`.
//! - `R2-S12-05`: `cmd_convert` fails closed on invalid `.cbtemplate` archive kind without falling back
//!   to `open_archive_bytes`.
//! - `R2-S12-07`: Hosted `UiApp` + `McpServerState::with_gui_host` shares a single `Session` & `resources/*`.
//! - `R2-S12-08`: Direct CLI `PropKind::Integer` parses full `u64` seeds `> i64::MAX`,
//!   `ensure_param_output_parents` ignores read-only commands, and `--out` export runs inside `run_guarded`.
use std::fs;
use std::process::{Command, Output};

use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::{EngineError, Session};
use curvebook_format::save_project_file;
use curvebook_mcp::{GuiSessionHost, McpServerState, call_tool, dispatch_json_value};
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn cli_bin() -> &'static str {
    CliHarness::bin()
}

struct SharedUiAppHost {
    app: UiApp,
}

impl SharedUiAppHost {
    fn new() -> Self {
        Self { app: UiApp::new() }
    }
}

impl GuiSessionHost for SharedUiAppHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        dispatch_gui_action(&mut self.app, method, params)
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

#[test]
fn r2_s12_02_unpacked_dir_check_never_classifies_dot_or_plain_dirs() -> TestResult {
    let dir = TempDir::new("unpacked-dir")?;
    let ws = dir.path().join("workspace");
    let sheets_dir = ws.join("sheets");
    fs::create_dir_all(&sheets_dir)?;
    let input_csv = dir.path().join("input.csv");
    fs::write(&input_csv, "X,Y\n1,10\n2,20\n")?;
    let assay_file = sheets_dir.join("raw_assay.csv");
    let notes_file = sheets_dir.join("notes.txt");
    fs::write(&assay_file, "A,B\n1,2\n")?;
    fs::write(&notes_file, "keep me\n")?;

    // `curvebook-cli convert input.csv ./workspace` must fail cleanly without touching `workspace/sheets/`.
    let convert_out: Output = Command::new(cli_bin()).args(["convert", &input_csv.display().to_string(), &ws.display().to_string()]).output()?;
    assert!(!convert_out.status.success(), "convert into non-.curvebook.d dir must fail");
    assert!(!ws.join("manifest.json").exists(), "manifest.json must not be created in plain dir");
    assert!(assay_file.is_file(), "sheets/raw_assay.csv must never be deleted");
    assert!(notes_file.is_file(), "sheets/notes.txt must never be deleted");

    // `curvebook-cli project.new --out .` inside `workspace` must fail cleanly without deleting `sheets/`.
    let direct_out: Output = Command::new(cli_bin()).current_dir(&ws).args(["project.new", "--out", "."]).output()?;
    assert!(!direct_out.status.success(), "project.new --out . must fail");
    assert!(!ws.join("manifest.json").exists(), "manifest.json must not be created in `.`");
    assert!(assay_file.is_file(), "sheets/raw_assay.csv must survive `project.new --out .`");

    // Explicit `.curvebook.d` suffix still succeeds cleanly.
    let valid_dir = dir.path().join("study.curvebook.d");
    let ok_out = Command::new(cli_bin()).args(["convert", &input_csv.display().to_string(), &valid_dir.display().to_string()]).output()?;
    assert!(ok_out.status.success(), "convert to .curvebook.d failed: {}", String::from_utf8_lossy(&ok_out.stderr));
    assert!(valid_dir.join("manifest.json").is_file());
    Ok(())
}

#[test]
fn r2_s12_05_cmd_convert_fails_closed_on_wrong_cbtemplate_archive_kind() -> TestResult {
    let dir = TempDir::new("cbtemplate-kind")?;
    let mut doc = ProjectDocument::new("Archive With Data").map_err(|e| e.to_string())?;
    let ds = DataSheet::new("d1", "Table", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1)?))?;
    doc.data_sheets.push(ds);

    // Save a Project archive, then copy its bytes to a `.cbtemplate` extension.
    let real_archive = dir.path().join("real.curvebook");
    save_project_file(&real_archive, &doc)?;
    let fake_tpl = dir.path().join("spoofed.cbtemplate");
    fs::copy(&real_archive, &fake_tpl)?;

    let out_csv = dir.path().join("out.csv");
    let res = Command::new(cli_bin()).args(["convert", &fake_tpl.display().to_string(), &out_csv.display().to_string()]).output()?;
    assert!(!res.status.success(), "converting a Project archive disguised as .cbtemplate must fail closed");
    assert!(!out_csv.exists(), "must not emit output CSV after archive-kind mismatch");
    Ok(())
}

#[test]
fn r2_s12_07_shared_ui_app_host_and_cli_resources_parity() -> TestResult {
    let dir = TempDir::new("uiapp-host")?;
    let mut state = McpServerState::with_gui_host(Box::new(SharedUiAppHost::new()));
    state.set_workdir(Some(&dir.dir))?;

    let created =
        call_tool(&mut state, "command_run", json!({ "command_id": "sheet.createData", "params": { "sheet_id": "gui-shared", "kind": "Xy" } }))
            .map_err(|e| format!("{e}"))?;
    assert!(!created.is_error);
    let insp = call_tool(&mut state, "ui_inspect", json!({ "scope": "navigator" })).map_err(|e| format!("{e}"))?;
    assert!(!insp.is_error);
    let insp_sc = insp.structured_content.ok_or("missing structuredContent")?;
    let nav = insp_sc["navigator_sheets"].as_array().ok_or("missing navigator_sheets")?;
    assert!(nav.iter().any(|s| s["sheet_id"] == json!("gui-shared")), "UiApp did not see shared mutation: {insp_sc}");
    assert!(state.session().document().data_sheet(&SheetId::new("gui-shared")).is_some());

    let res =
        dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": { "uri": "sheet://gui-shared" } }), &mut state)
            .ok_or("missing resources/read reply")?;
    assert!(res.get("error").is_none(), "resources/read failed on UiApp host: {res}");
    Ok(())
}

#[test]
fn r2_s12_08_cli_direct_u64_seed_readonly_workdir_parents_and_out_guard() -> TestResult {
    let dir = TempDir::new("direct-cmd")?;
    let wd = dir.path().join("wd");
    fs::create_dir_all(&wd)?;
    let wd_str = wd.display().to_string();

    // 1. Valid `u64` seed > `i64::MAX` (`18446744073709551615 == u64::MAX`) succeeds on direct CLI command.
    let seed_out = Command::new(cli_bin())
        .args(["table.randomizeGroups", "--groups", "[\"Ctrl\",\"Tx\"]", "--subject-count", "6", "--seed", "18446744073709551615", "--json"])
        .output()?;
    assert!(seed_out.status.success(), "table.randomizeGroups with u64::MAX seed failed: stderr={}", String::from_utf8_lossy(&seed_out.stderr));
    let seed_json: Value = serde_json::from_slice(&seed_out.stdout)?;
    assert_eq!(seed_json["result"]["seed"], json!(u64::MAX));

    // 2. Non-writing `--workdir` command must NOT create missing parent directories from path params.
    let read_only_err =
        Command::new(cli_bin()).args(["project.open", "--path", "missing_parent/study.curvebook", "--workdir", &wd_str, "--json"]).output()?;
    assert!(!read_only_err.status.success());
    assert!(!wd.join("missing_parent").exists(), "read-only project.open under --workdir must never create missing_parent/");

    // 3. Direct `--out` export runs cleanly and produces valid archive inside `--workdir`.
    let out_res = Command::new(cli_bin())
        .args(["sheet.createData", "--kind", "Column", "--workdir", &wd_str, "--out", "nested_out/direct.curvebook", "--json"])
        .output()?;
    assert!(out_res.status.success(), "direct --out failed: {}", String::from_utf8_lossy(&out_res.stderr));
    assert!(wd.join("nested_out/direct.curvebook").is_file());
    Ok(())
}

#[test]
fn r020_cli_rejects_legacy_lumencraft_batch_and_archive_inputs() -> TestResult {
    let dir = TempDir::new("legacy-reject")?;
    let wd = dir.path().join("wd");
    fs::create_dir_all(&wd)?;

    let legacy_manifest = wd.join("legacy_batch.json");
    fs::write(
        &legacy_manifest,
        serde_json::to_vec(&json!({
            "schema": "lumencraft.batch.v1",
            "exports": []
        }))?,
    )?;
    let batch_out =
        Command::new(cli_bin()).args(["batch", &legacy_manifest.display().to_string(), "--workdir", &wd.display().to_string(), "--json"]).output()?;
    assert_eq!(batch_out.status.code(), Some(2), "legacy lumencraft.batch.v1 must fail with exit 2");

    let csv_in = wd.join("in.csv");
    fs::write(&csv_in, "X,Y\n1,2\n")?;
    let out_cb = wd.join("out.curvebook");
    for fmt in ["lumencraft", "lumencraft.d", "lctemplate"] {
        let fmt_out =
            Command::new(cli_bin()).args(["convert", &csv_in.display().to_string(), &out_cb.display().to_string(), "--format", fmt]).output()?;
        assert_eq!(fmt_out.status.code(), Some(2), "legacy --format {fmt} must fail with exit 2");
    }

    let legacy_file = wd.join("old.lumencraft");
    fs::write(&legacy_file, b"legacy-payload")?;
    let info_out = Command::new(cli_bin()).args(["info", &legacy_file.display().to_string()]).output()?;
    assert!(!info_out.status.success(), "info on .lumencraft file must fail");

    let legacy_tpl = wd.join("old.lctemplate");
    fs::write(&legacy_tpl, b"legacy-tpl")?;
    let out_csv = wd.join("out.csv");
    let conv_out = Command::new(cli_bin()).args(["convert", &legacy_tpl.display().to_string(), &out_csv.display().to_string()]).output()?;
    assert!(!conv_out.status.success(), "convert on .lctemplate file must fail");
    Ok(())
}

#[test]
fn audit_70_cli_convert_export_and_batch_refuse_future_version_archive_targets() -> TestResult {
    let dir = TempDir::new("audit70-future-target")?;
    let wd = dir.path().join("wd");
    fs::create_dir_all(&wd)?;

    let future_ver = u64::from(curvebook_doc::CURRENT_SCHEMA_VERSION) + 1;
    let build_future_archive = |title: &str| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut w = curvebook_format::zip::ZipWriter::new();
        let manifest = format!(
            r#"{{"format":"curvebook-archive","schema_version":{future_ver},"kind":"project","title":"{title}","document_crc32":0,"sheets":[]}}"#
        );
        let doc = format!(r#"{{"schema_version":{future_ver},"title":"{title}"}}"#);
        w.add_file("manifest.json", manifest.into_bytes())?;
        w.add_file("document.json", doc.into_bytes())?;
        Ok(w.finish()?)
    };

    let csv_in = wd.join("input.csv");
    fs::write(&csv_in, "X,Y\n1,2\n3,4\n")?;

    let mut current_doc = ProjectDocument::new("Current Source")?;
    let mut table = curvebook_doc::XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 2)?;
    table.x_cells[0][0] = curvebook_doc::Cell::number(1.0);
    table.data_sets[0].cells[0][0] = curvebook_doc::Cell::number(10.0);
    table.x_cells[1][0] = curvebook_doc::Cell::number(2.0);
    table.data_sets[0].cells[1][0] = curvebook_doc::Cell::number(20.0);
    current_doc.add_data_sheet(DataSheet::new("d1", "Source Data", TableArchetype::Xy(table))?)?;
    let current_proj = wd.join("source.curvebook");
    save_project_file(&current_proj, &current_doc)?;

    let target_convert = wd.join("target_convert.curvebook");
    let convert_bytes = build_future_archive("Future Convert")?;
    fs::write(&target_convert, &convert_bytes)?;

    let target_export = wd.join("target_export.curvebook");
    let export_bytes = build_future_archive("Future Export")?;
    fs::write(&target_export, &export_bytes)?;

    let target_batch = wd.join("target_batch.curvebook");
    let batch_bytes = build_future_archive("Future Batch")?;
    fs::write(&target_batch, &batch_bytes)?;

    // 1. `convert` writing `.curvebook` refuses future-version existing target and keeps bytes intact.
    let conv_out =
        Command::new(cli_bin()).args(["convert", &csv_in.display().to_string(), &target_convert.display().to_string(), "--json"]).output()?;
    assert_eq!(conv_out.status.code(), Some(1), "convert against future archive must exit 1: {}", String::from_utf8_lossy(&conv_out.stdout));
    let conv_json: Value = serde_json::from_slice(&conv_out.stdout)?;
    assert_eq!(conv_json["error"]["code"], json!("unsupported_version"));
    assert_eq!(fs::read(&target_convert)?, convert_bytes, "convert must leave future-version archive byte-for-byte intact");

    // 2. `export` writing `.curvebook` refuses future-version existing target and keeps bytes intact.
    let exp_out = Command::new(cli_bin())
        .args(["export", &current_proj.display().to_string(), "--out", &target_export.display().to_string(), "--json"])
        .output()?;
    assert_eq!(exp_out.status.code(), Some(1), "export against future archive must exit 1: {}", String::from_utf8_lossy(&exp_out.stdout));
    let exp_json: Value = serde_json::from_slice(&exp_out.stdout)?;
    assert_eq!(exp_json["error"]["code"], json!("unsupported_version"));
    assert_eq!(fs::read(&target_export)?, export_bytes, "export must leave future-version archive byte-for-byte intact");

    // 3. `batch` writing `.curvebook` refuses future-version existing target and keeps bytes intact.
    let manifest_path = wd.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "project": "source.curvebook",
            "exports": [{ "format": "curvebook", "out": "target_batch.curvebook" }]
        }))?,
    )?;
    let batch_out =
        Command::new(cli_bin()).args(["batch", &manifest_path.display().to_string(), "--workdir", &wd.display().to_string(), "--json"]).output()?;
    assert_eq!(batch_out.status.code(), Some(1), "batch against future archive must exit 1: {}", String::from_utf8_lossy(&batch_out.stdout));
    let batch_json: Value = serde_json::from_slice(&batch_out.stdout)?;
    assert_eq!(batch_json["error"]["code"], json!("unsupported_version"));
    assert_eq!(fs::read(&target_batch)?, batch_bytes, "batch must leave future-version archive byte-for-byte intact");

    Ok(())
}
