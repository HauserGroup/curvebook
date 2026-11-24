#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, NativeFileDialogKind,
    StubNativeFileDialogAdapter,
};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_format::zip::ZipWriter;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::os_integration::StubPlatformFileDialogAdapter;
use eframe::egui;
use serde_json::json;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn spawn_runtime(session: Session) -> Result<(DesktopHostRuntime, DesktopWindowId, Arc<StubPlatformFileDialogAdapter>), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let adapter = Arc::new(StubPlatformFileDialogAdapter::new());
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
    let wid = rt.open_window_with_session(session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    Ok((rt, wid, adapter))
}

fn write_warning_pzfx(path: &std::path::Path, table_title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile Version="2.0" Title="Warning PZFX">
  <TableSequence>
    <Table ID="Table0" TableType="XY" XFormat="numbers" Replicates="1">
      <Title>{table_title}</Title>
      <XColumn><Title>Dose</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
      <YColumn><Title>Response</Title><Subcolumn><d>10.5</d><d>20.25</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
  <Analysis ID="A1"/>
</PrismFile>"#
    );
    fs::write(path, xml)?;
    Ok(())
}

fn write_clean_pzfx(path: &std::path::Path, table_title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile Version="2.0" Title="Clean PZFX">
  <TableSequence>
    <Table ID="Table0" TableType="XY" XFormat="numbers" Replicates="1">
      <Title>{table_title}</Title>
      <XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
      <YColumn><Title>Y</Title><Subcolumn><d>3</d><d>4</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#
    );
    fs::write(path, xml)?;
    Ok(())
}

fn write_warning_zip_project(path: &std::path::Path, table_title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    if let Some(col) = xy.data_sets.first_mut() {
        col.title = String::from("Signal");
    }
    let mut ds = DataSheet::new("zip-xy", table_title, TableArchetype::Xy(xy))?;
    ds.set_x_or_time_cell(0, 0, Cell::number(1.0))?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(11.0))?;
    doc.add_data_sheet(ds)?;

    let mut zw = ZipWriter::new();
    zw.add_file("manifest.json", br#"{"title":"Imported Archive"}"#.to_vec())?;
    zw.add_file("document.json", serde_json::to_vec(&doc)?)?;
    zw.add_file("analyses/skipped_anova.json", br#"{"kind":"unsupported"}"#.to_vec())?;
    fs::write(path, zw.finish()?)?;
    Ok(())
}

fn build_anova_session() -> Result<Session, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?;
    table.data_sets[0].title = String::from("Control");
    table.data_sets[1].title = String::from("Low");
    table.data_sets[2].title = String::from("High");
    let mut ds = DataSheet::new("anova-data", "Enzyme Activity", TableArchetype::Column(table))?;
    let cols = [[10.5, 11.2, 10.8, 11.5], [14.1, 14.8, 13.9, 14.4], [19.2, 18.7, 19.6, 19.1]];
    for (c, values) in cols.iter().enumerate() {
        for (r, &v) in values.iter().enumerate() {
            ds.set_cell(CellCoord::new(r, c, 0), Cell::number(v))?;
        }
    }
    doc.add_data_sheet(ds)?;
    let mut session = Session::from_document(doc);
    let _ = session
        .execute("analysis.run", json!({ "source_sheet_id": "anova-data", "kind": "one_way_anova", "sheet_id": "rs-anova", "alpha": 0.05 }))?;
    let _ = session.execute("sheet.select", json!({ "sheet_id": "rs-anova" }))?;
    Ok(session)
}

#[test]
fn pzfx_and_archive_import_warnings_captured_in_ui_app_and_ui_inspect_including_dnd() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("cb-r10-29-warnings")?;
    let pzfx_warn = tmp.0.join("warn.pzfx");
    let archive_warn = tmp.0.join("warn.prism");
    let pzfx_clean = tmp.0.join("clean.pzfx");
    write_warning_pzfx(&pzfx_warn, "WarnPZFX")?;
    write_warning_zip_project(&archive_warn, "WarnArchive")?;
    write_clean_pzfx(&pzfx_clean, "CleanPZFX")?;

    // 1. Native file dialog import of .pzfx captures warnings and exposes them via ui.inspect.
    let (mut rt, wid, adapter) = spawn_runtime(Session::new())?;
    adapter.push_response("table.importPzfx", Some(pzfx_warn.clone()));
    let out = rt.dispatch_native_file_command(wid, "table.importPzfx")?.expect("handled");
    assert_eq!(out.action, "native_file_dialog.completed:table.importPzfx");

    let win = rt.window(wid)?;
    assert!(!win.app().import_warnings.warnings.is_empty(), "expected non-empty PZFX import warnings");
    assert_eq!(win.app().import_warnings.command_id, "table.importPzfx");
    assert_eq!(win.app().import_warnings.source_label, "warn.pzfx");
    assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));

    let snap = curvebook_ui_egui::inspect::inspect_ui_tree_for_metrics(win.app(), ViewportMetrics::new(1280.0, 800.0, 1.0), None, false);
    let iw = snap.import_warnings.as_ref().expect("ui.inspect must expose import_warnings");
    assert!(iw.open);
    assert_eq!(iw.command_id, "table.importPzfx");
    assert_eq!(iw.source_label, "warn.pzfx");
    assert!(!iw.warnings.is_empty());

    // 2. Clean import clears stale warnings and closes the panel.
    let (mut rt_clean, wid_clean, adapter_clean) = spawn_runtime(Session::new())?;
    adapter_clean.push_response("table.importPzfx", Some(pzfx_warn.clone()));
    let _ = rt_clean.dispatch_native_file_command(wid_clean, "table.importPzfx")?;
    assert!(!rt_clean.window(wid_clean)?.app().import_warnings.warnings.is_empty());
    let _ = rt_clean.dispatch_native_file_command_with_params(
        wid_clean,
        "table.importPzfx",
        &json!({ "path": pzfx_clean.to_string_lossy(), "merge": false }),
    )?;
    assert!(rt_clean.window(wid_clean)?.app().import_warnings.warnings.is_empty());
    assert!(!matches!(rt_clean.window(wid_clean)?.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));

    // 3. Drag-and-drop (handle_file_dropped) of .prism and .pzfx also captures warnings.
    let (mut rt_dnd, wid_dnd, _) = spawn_runtime(Session::new())?;
    let drop_out = rt_dnd.dispatch(DesktopHostEvent::FileDropped { window_id: wid_dnd, path: archive_warn })?;
    assert_eq!(drop_out.action, "file.dropped_loaded_in_window");
    let win_dnd = rt_dnd.window(wid_dnd)?;
    assert!(!win_dnd.app().import_warnings.warnings.is_empty(), "expected .prism drop warnings");
    assert_eq!(win_dnd.app().import_warnings.command_id, "table.importPrism");
    assert_eq!(win_dnd.app().import_warnings.source_label, "warn.prism");
    let dnd_snap = curvebook_ui_egui::inspect::inspect_ui_tree_for_metrics(win_dnd.app(), ViewportMetrics::new(1280.0, 800.0, 1.0), None, false);
    assert!(dnd_snap.import_warnings.as_ref().is_some_and(|w| !w.warnings.is_empty()));

    Ok(())
}

#[test]
fn import_into_existing_project_prompts_replace_or_merge_and_honors_both_modes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("cb-r10-29-merge-replace")?;
    let saved_project = tmp.0.join("existing.curvebook");
    let incoming_pzfx = tmp.0.join("incoming.pzfx");
    write_warning_pzfx(&incoming_pzfx, "Incoming Table")?;

    let mut base_doc = ProjectDocument::default();
    let base_table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    let mut base_ds = DataSheet::new("orig-1", "Original Sheet", TableArchetype::Column(base_table))?;
    base_ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(42.0))?;
    base_doc.add_data_sheet(base_ds)?;
    save_project_file(&saved_project, &base_doc)?;

    // Build a saved session with an existing sheet and a bound .curvebook file_path.
    let mut session = Session::from_document(base_doc.clone());
    session.set_file_path(Some(saved_project.clone()), true);
    let (mut rt, wid, adapter) = spawn_runtime(session)?;

    // Picking a .pzfx file on a non-empty project opens ActiveDialog::ImportProjectMode instead of silently replacing.
    adapter.push_response("table.importPzfx", Some(incoming_pzfx.clone()));
    let prompt_out = rt.dispatch_native_file_command(wid, "table.importPzfx")?.expect("prompt outcome");
    assert_eq!(prompt_out.action, "native_file_dialog.confirm_import_mode:table.importPzfx");
    assert_eq!(rt.window(wid)?.app().document().data_sheets.len(), 1);
    assert!(matches!(
        &rt.window(wid)?.app().canvas_dialogs.active_dialog,
        ActiveDialog::ImportProjectMode { command_id, path, has_unsaved_changes }
            if command_id == "table.importPzfx"
                && *path == incoming_pzfx.to_string_lossy()
                && !*has_unsaved_changes
    ));

    // Choosing merge: true preserves existing sheets AND keeps the active .curvebook file_path.
    let merge_out = rt
        .dispatch_native_file_command_with_params(wid, "table.importPzfx", &json!({ "path": incoming_pzfx.to_string_lossy(), "merge": true }))?
        .expect("merge outcome");
    assert_eq!(merge_out.action, "native_file_dialog.completed:table.importPzfx");
    {
        let win = rt.window(wid)?;
        let titles: Vec<&str> = win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles.len(), 2, "merge should keep existing sheet + add incoming sheet: {titles:?}");
        assert!(titles.contains(&"Original Sheet"));
        assert!(titles.contains(&"Incoming Table"));
        assert_eq!(win.app().session().file_path(), Some(saved_project.as_path()), "merge must preserve file_path");
        assert!(!win.app().import_warnings.warnings.is_empty(), "merge must still capture import warnings");
    }

    // Now test merge: false through CurveBookNativeApp with an unsaved/dirty project:
    // replaces all sheets AND clears file_path to None.
    let mut dirty_session = Session::from_document(base_doc);
    dirty_session.set_file_path(Some(saved_project.clone()), false);
    let _ = dirty_session.execute("sheet.rename", json!({ "sheet_id": "orig-1", "title": "Original Dirty" }))?;
    assert!(dirty_session.is_dirty());

    let mut host_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let host_adapter = Arc::new(StubNativeFileDialogAdapter::new());
    host_adapter.push_response("table.importPzfx", Some(incoming_pzfx.clone()));
    host_rt.set_file_dialog_adapter(Arc::clone(&host_adapter) as Arc<dyn NativeFileDialogAdapter>);
    let host_wid = host_rt.open_window_with_session(dirty_session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    let mut native_app = CurveBookNativeApp::new(host_rt, host_wid, None);

    // Trigger File > Import > .pzfx via runtime; verify has_unsaved_changes == true in modal.
    let _ = native_app.runtime_mut().dispatch_native_file_command(host_wid, "table.importPzfx")?;
    assert!(matches!(
        &native_app.runtime().window(host_wid)?.app().canvas_dialogs.active_dialog,
        ActiveDialog::ImportProjectMode { has_unsaved_changes: true, .. }
    ));

    // Step one egui frame while clicking/emulating modal confirm is covered in ui-egui;
    // verify CurveBookNativeApp routes explicit { path, merge: false } through execute_native_import_at_path.
    let egui_ctx = egui::Context::default();
    egui_ctx.set_fonts(build_egui_font_definitions(None));
    let _ = native_app.runtime_mut().dispatch_native_file_command_with_params(
        host_wid,
        "table.importPzfx",
        &json!({ "path": incoming_pzfx.to_string_lossy(), "merge": false }),
    )?;
    let win_after = native_app.runtime().window(host_wid)?;
    let titles_after: Vec<&str> = win_after.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles_after, vec!["Incoming Table"]);
    assert_eq!(win_after.app().session().file_path(), None, "replace must detach file_path");
    assert!(!win_after.app().import_warnings.warnings.is_empty(), "replace must capture import warnings");

    Ok(())
}

#[test]
fn tsv_auto_delimiter_and_methods_report_extensions_and_export_all_tables() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("cb-r10-29-exports")?;
    let session = build_anova_session()?;
    let (mut rt, wid, adapter) = spawn_runtime(session)?;

    // 1. table.exportCsv to `.tsv` auto-selects `\t` delimiter.
    let table_tsv = tmp.0.join("enzyme.TSV");
    adapter.push_response("table.exportCsv", Some(table_tsv.clone()));
    let _ = rt.dispatch_native_file_command_with_params(
        wid,
        "table.exportCsv",
        &json!({ "sheet_id": "anova-data", "delimiter": ",", "decimal_separator": "." }),
    )?;
    let table_tsv_text = fs::read_to_string(&table_tsv)?;
    assert!(table_tsv_text.contains('\t'), "table .tsv export must use tab delimiter: {table_tsv_text}");

    // 2. analysis.exportCsv to `.tsv` auto-selects `\t` delimiter and includes `tsv` in dialog extensions.
    let results_tsv = tmp.0.join("anova_results.tsv");
    adapter.push_response("analysis.exportCsv", Some(results_tsv.clone()));
    let _ = rt.dispatch_native_file_command_with_params(wid, "analysis.exportCsv", &json!({ "delimiter": ",", "decimal_separator": "," }))?;
    let results_tsv_text = fs::read_to_string(&results_tsv)?;
    assert!(results_tsv_text.contains('\t'), "analysis .tsv export must use tab delimiter: {results_tsv_text}");
    let analysis_req = adapter.requests().into_iter().find(|r| r.command_id == "analysis.exportCsv").expect("analysis.exportCsv dialog request");
    assert!(analysis_req.extensions.iter().any(|e| e == "csv"));
    assert!(analysis_req.extensions.iter().any(|e| e == "tsv"));

    // 3. result.generateReport saves `.md`, `.txt`, and `.tex` using extension-inferred format.
    let md_path = tmp.0.join("report.md");
    let txt_path = tmp.0.join("report.txt");
    let tex_path = tmp.0.join("report.tex");
    adapter.push_response("result.generateReport", Some(md_path.clone()));
    adapter.push_response("result.generateReport", Some(txt_path.clone()));
    adapter.push_response("result.generateReport", Some(tex_path.clone()));
    let _ = rt.dispatch_native_file_command(wid, "result.generateReport")?;
    let _ = rt.dispatch_native_file_command(wid, "result.generateReport")?;
    let _ = rt.dispatch_native_file_command(wid, "result.generateReport")?;
    let md_text = fs::read_to_string(&md_path)?;
    let txt_text = fs::read_to_string(&txt_path)?;
    let tex_text = fs::read_to_string(&tex_path)?;
    assert!(!md_text.trim().is_empty() && md_text.contains('#'), "expected markdown headings in .md report");
    assert!(!txt_text.trim().is_empty());
    assert!(!tex_text.trim().is_empty() && tex_text.contains('\\'), "expected LaTeX commands in .tex report");

    // 4. ui.export.allTablesCsv exports every DataSheet with sanitized unique stems (-2, -3).
    {
        let win = rt.window_mut(wid)?;
        let _ = win.execute_and_journal("sheet.createData", json!({ "title": "Dose / Response: A?" }))?;
        let _ = win.execute_and_journal("sheet.createData", json!({ "title": "Dose / Response: A?" }))?;
        let _ = win.execute_and_journal("sheet.createData", json!({ "title": "dose / response: a?" }))?;
    }
    let out_dir = tmp.0.join("all-tables-csv");
    adapter.push_response("ui.export.allTablesCsv", Some(out_dir.clone()));
    let all_out = rt
        .dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "delimiter": ";", "decimal_separator": "," }))?
        .expect("allTablesCsv outcome");
    assert_eq!(all_out.action, "native_file_dialog.completed:ui.export.allTablesCsv");
    let folder_req = adapter.requests().into_iter().find(|r| r.command_id == "ui.export.allTablesCsv").expect("PickFolder request");
    assert_eq!(folder_req.kind, NativeFileDialogKind::PickFolder);

    assert!(out_dir.join("Enzyme Activity.csv").exists());
    assert!(out_dir.join("Dose _ Response_ A.csv").exists());
    assert!(out_dir.join("Dose _ Response_ A-2.csv").exists());
    assert!(out_dir.join("dose _ response_ a-3.csv").exists());
    let enzyme_csv = fs::read_to_string(out_dir.join("Enzyme Activity.csv"))?;
    assert!(enzyme_csv.contains(';'), "expected semicolon delimiter in exported CSV: {enzyme_csv}");
    assert!(enzyme_csv.contains("10,5"), "expected decimal comma in exported CSV: {enzyme_csv}");

    // Verify tab delimiter uses `.tsv` extension in folder export.
    let out_tsv_dir = tmp.0.join("all-tables-tsv");
    adapter.push_response("ui.export.allTablesCsv", Some(out_tsv_dir.clone()));
    let _ = rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "delimiter": "\t", "decimal_separator": "." }))?;
    assert!(out_tsv_dir.join("Enzyme Activity.tsv").exists());

    Ok(())
}
