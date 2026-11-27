#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, NativeFileDialogKind,
    StubNativeFileDialogAdapter,
};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{AnalysisKind, ResultProvenance, ResultStatus, SheetId};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::os_integration::StubPlatformFileDialogAdapter;
use serde_json::json;

use crate::common::{TempDirGuard, build_col_doc, build_xy_doc, build_zip_entries};

fn spawn_runtime(session: Session) -> Result<(DesktopHostRuntime, DesktopWindowId, Arc<StubPlatformFileDialogAdapter>), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let adapter = Arc::new(StubPlatformFileDialogAdapter::new());
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
    let wid = rt.open_window_with_session(session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    Ok((rt, wid, adapter))
}

fn write_pzfx(path: &Path, table_title: &str, with_warning_analysis: bool) -> Result<(), Box<dyn std::error::Error>> {
    let extra = if with_warning_analysis { r#"<Analysis ID="A1"/>"# } else { "" };
    fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile Version="2.0" Title="PZFX"><TableSequence><Table ID="Table0" TableType="XY" XFormat="numbers" Replicates="1"><Title>{table_title}</Title><XColumn><Title>Dose</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn><YColumn><Title>Response</Title><Subcolumn><d>10.5</d><d>20.25</d></Subcolumn></YColumn></Table></TableSequence>{extra}</PrismFile>"#
        ),
    )?;
    Ok(())
}

fn write_warning_zip_project(path: &Path, table_title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let doc_json = serde_json::to_string(&build_xy_doc("zip-xy", table_title, &[(1.0, 11.0)])?)?;
    fs::write(
        path,
        build_zip_entries(&[
            ("manifest.json", r#"{"title":"Imported Archive"}"#),
            ("document.json", &doc_json),
            ("analyses/skipped_anova.json", r#"{"kind":"unsupported"}"#),
        ])?,
    )?;
    Ok(())
}

fn build_anova_session() -> Result<Session, Box<dyn std::error::Error>> {
    let doc = build_col_doc(
        "anova-data",
        "Enzyme Activity",
        &[("Control", &[10.5, 11.2, 10.8, 11.5]), ("Low", &[14.1, 14.8, 13.9, 14.4]), ("High", &[19.2, 18.7, 19.6, 19.1])],
    )?;
    let mut session = Session::from_document(doc);
    let _ = session
        .execute("analysis.run", json!({ "source_sheet_id": "anova-data", "kind": "one_way_anova", "sheet_id": "rs-anova", "alpha": 0.05 }))?;
    let _ = session.execute("sheet.select", json!({ "sheet_id": "rs-anova" }))?;
    Ok(session)
}

#[test]
fn pzfx_and_archive_import_warnings_captured_in_ui_app_and_ui_inspect_including_dnd() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r10-29-warnings")?;
    let (pzfx_warn, archive_warn, pzfx_clean) = (tmp.child("warn.pzfx"), tmp.child("warn.prism"), tmp.child("clean.pzfx"));
    write_pzfx(&pzfx_warn, "WarnPZFX", true)?;
    write_warning_zip_project(&archive_warn, "WarnArchive")?;
    write_pzfx(&pzfx_clean, "CleanPZFX", false)?;

    let (mut rt, wid, adapter) = spawn_runtime(Session::new())?;
    adapter.push_response("table.importPzfx", Some(pzfx_warn.clone()));
    assert_eq!(rt.dispatch_native_file_command(wid, "table.importPzfx")?.expect("handled").action, "native_file_dialog.completed:table.importPzfx");

    let win = rt.window(wid)?;
    assert!(!win.app().import_warnings.warnings.is_empty());
    assert_eq!((win.app().import_warnings.command_id.as_str(), win.app().import_warnings.source_label.as_str()), ("table.importPzfx", "warn.pzfx"));
    assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));
    let snap = curvebook_ui_egui::inspect::inspect_ui_tree_for_metrics(win.app(), ViewportMetrics::new(1280.0, 800.0, 1.0), None, false);
    let iw = snap.import_warnings.as_ref().expect("ui.inspect import_warnings");
    assert!(iw.open && iw.command_id == "table.importPzfx" && iw.source_label == "warn.pzfx" && !iw.warnings.is_empty());

    // Clean import clears stale warnings.
    let (mut rt_clean, wid_clean, adapter_clean) = spawn_runtime(Session::new())?;
    adapter_clean.push_response("table.importPzfx", Some(pzfx_warn));
    let _ = rt_clean.dispatch_native_file_command(wid_clean, "table.importPzfx")?;
    assert!(!rt_clean.window(wid_clean)?.app().import_warnings.warnings.is_empty());
    let _ = rt_clean.dispatch_native_file_command_with_params(
        wid_clean,
        "table.importPzfx",
        &json!({ "path": pzfx_clean.to_string_lossy(), "merge": false }),
    )?;
    assert!(rt_clean.window(wid_clean)?.app().import_warnings.warnings.is_empty());
    assert!(!matches!(rt_clean.window(wid_clean)?.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));

    // Drag-and-drop of .prism captures warnings.
    let (mut rt_dnd, wid_dnd, _) = spawn_runtime(Session::new())?;
    assert_eq!(rt_dnd.dispatch(DesktopHostEvent::FileDropped { window_id: wid_dnd, path: archive_warn })?.action, "file.dropped_loaded_in_window");
    let win_dnd = rt_dnd.window(wid_dnd)?;
    assert!(
        !win_dnd.app().import_warnings.warnings.is_empty()
            && win_dnd.app().import_warnings.command_id == "table.importPrism"
            && win_dnd.app().import_warnings.source_label == "warn.prism"
    );
    let dnd_snap = curvebook_ui_egui::inspect::inspect_ui_tree_for_metrics(win_dnd.app(), ViewportMetrics::new(1280.0, 800.0, 1.0), None, false);
    assert!(dnd_snap.import_warnings.as_ref().is_some_and(|w| !w.warnings.is_empty()));
    Ok(())
}

#[test]
fn import_into_existing_project_prompts_replace_or_merge_and_honors_both_modes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r10-29-merge-replace")?;
    let (saved_project, incoming_pzfx) = (tmp.child("existing.curvebook"), tmp.child("incoming.pzfx"));
    write_pzfx(&incoming_pzfx, "Incoming Table", true)?;
    let base_doc = build_col_doc("orig-1", "Original Sheet", &[("A", &[42.0])])?;
    save_project_file(&saved_project, &base_doc)?;

    let mut session = Session::from_document(base_doc.clone());
    session.set_file_path(Some(saved_project.clone()), true);
    let (mut rt, wid, adapter) = spawn_runtime(session)?;

    adapter.push_response("table.importPzfx", Some(incoming_pzfx.clone()));
    assert_eq!(
        rt.dispatch_native_file_command(wid, "table.importPzfx")?.expect("prompt").action,
        "native_file_dialog.confirm_import_mode:table.importPzfx"
    );
    assert_eq!(rt.window(wid)?.app().document().data_sheets.len(), 1);
    assert!(
        matches!(&rt.window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::ImportProjectMode { command_id, path, has_unsaved_changes: false } if command_id == "table.importPzfx" && *path == incoming_pzfx.to_string_lossy())
    );

    // merge: true keeps existing sheet & file_path.
    assert_eq!(
        rt.dispatch_native_file_command_with_params(wid, "table.importPzfx", &json!({ "path": incoming_pzfx.to_string_lossy(), "merge": true }))?
            .expect("merge")
            .action,
        "native_file_dialog.completed:table.importPzfx"
    );
    let win = rt.window(wid)?;
    let titles: Vec<&str> = win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert!(titles.len() == 2 && titles.contains(&"Original Sheet") && titles.contains(&"Incoming Table"));
    assert!(win.app().session().file_path() == Some(saved_project.as_path()) && !win.app().import_warnings.warnings.is_empty());

    // merge: false on dirty project detaches file_path & replaces sheets.
    let mut dirty_session = Session::from_document(base_doc);
    dirty_session.set_file_path(Some(saved_project), false);
    let _ = dirty_session.execute("sheet.rename", json!({ "sheet_id": "orig-1", "title": "Original Dirty" }))?;
    let mut host_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let host_adapter = Arc::new(StubNativeFileDialogAdapter::new());
    host_adapter.push_response("table.importPzfx", Some(incoming_pzfx.clone()));
    host_rt.set_file_dialog_adapter(host_adapter as Arc<dyn NativeFileDialogAdapter>);
    let host_wid = host_rt.open_window_with_session(dirty_session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    let mut native_app = CurveBookNativeApp::new(host_rt, host_wid, None);

    let _ = native_app.runtime_mut().dispatch_native_file_command(host_wid, "table.importPzfx")?;
    assert!(matches!(
        &native_app.runtime().window(host_wid)?.app().canvas_dialogs.active_dialog,
        ActiveDialog::ImportProjectMode { has_unsaved_changes: true, .. }
    ));
    let _ = native_app.runtime_mut().dispatch_native_file_command_with_params(
        host_wid,
        "table.importPzfx",
        &json!({ "path": incoming_pzfx.to_string_lossy(), "merge": false }),
    )?;
    let win_after = native_app.runtime().window(host_wid)?;
    assert_eq!(win_after.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["Incoming Table"]);
    assert!(win_after.app().session().file_path().is_none() && !win_after.app().import_warnings.warnings.is_empty());
    Ok(())
}

#[test]
fn tsv_auto_delimiter_and_methods_report_extensions_and_export_all_tables() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r10-29-exports")?;
    let (mut rt, wid, adapter) = spawn_runtime(build_anova_session()?)?;

    // 1-2. .tsv exports auto-select tab delimiter.
    let (table_tsv, results_tsv) = (tmp.child("enzyme.TSV"), tmp.child("anova_results.tsv"));
    adapter.push_response("table.exportCsv", Some(table_tsv.clone()));
    let _ = rt.dispatch_native_file_command_with_params(
        wid,
        "table.exportCsv",
        &json!({ "sheet_id": "anova-data", "delimiter": ",", "decimal_separator": "." }),
    )?;
    assert!(fs::read_to_string(&table_tsv)?.contains('\t'));

    adapter.push_response("analysis.exportCsv", Some(results_tsv.clone()));
    let _ = rt.dispatch_native_file_command_with_params(wid, "analysis.exportCsv", &json!({ "delimiter": ",", "decimal_separator": "," }))?;
    assert!(fs::read_to_string(&results_tsv)?.contains('\t'));
    let analysis_req = adapter.requests().into_iter().find(|r| r.command_id == "analysis.exportCsv").expect("analysis request");
    assert!(analysis_req.extensions.iter().any(|e| e == "csv") && analysis_req.extensions.iter().any(|e| e == "tsv"));

    // 3. result.generateReport supports .md, .txt, .tex.
    let (md_path, txt_path, tex_path) = (tmp.child("report.md"), tmp.child("report.txt"), tmp.child("report.tex"));
    for p in [&md_path, &txt_path, &tex_path] {
        adapter.push_response("result.generateReport", Some(p.clone()));
        let _ = rt.dispatch_native_file_command(wid, "result.generateReport")?;
    }
    assert!(
        fs::read_to_string(&md_path)?.contains('#')
            && !fs::read_to_string(&txt_path)?.trim().is_empty()
            && fs::read_to_string(&tex_path)?.contains('\\')
    );

    // 4. ui.export.allTablesCsv sanitizes & deduplicates stems (-2, -3) and honors tab -> .tsv.
    for title in ["Dose / Response: A?", "Dose / Response: A?", "dose / response: a?"] {
        let _ = rt.window_mut(wid)?.execute_and_journal("sheet.createData", json!({ "title": title }))?;
    }
    let out_dir = tmp.child("all-tables-csv");
    adapter.push_response("ui.export.allTablesCsv", Some(out_dir.clone()));
    assert_eq!(
        rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "delimiter": ";", "decimal_separator": "," }))?
            .expect("out")
            .action,
        "native_file_dialog.completed:ui.export.allTablesCsv"
    );
    assert_eq!(
        adapter.requests().into_iter().find(|r| r.command_id == "ui.export.allTablesCsv").expect("folder req").kind,
        NativeFileDialogKind::PickFolder
    );
    for name in ["Enzyme Activity.csv", "Dose _ Response_ A.csv", "Dose _ Response_ A-2.csv", "dose _ response_ a-3.csv"] {
        assert!(out_dir.join(name).exists(), "missing {name}");
    }
    let enzyme_csv = fs::read_to_string(out_dir.join("Enzyme Activity.csv"))?;
    assert!(enzyme_csv.contains(';') && enzyme_csv.contains("10,5"));

    let out_tsv_dir = tmp.child("all-tables-tsv");
    adapter.push_response("ui.export.allTablesCsv", Some(out_tsv_dir.clone()));
    let _ = rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "delimiter": "\t", "decimal_separator": "." }))?;
    assert!(out_tsv_dir.join("Enzyme Activity.tsv").exists());
    Ok(())
}

#[test]
fn export_all_tables_existing_file_is_never_replaced_without_asking() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r10-29-export-existing")?;
    let (mut rt, wid, adapter) = spawn_runtime(build_anova_session()?)?;
    let _ = rt.window_mut(wid)?.execute_and_journal("sheet.createData", json!({ "title": "Second Table" }))?;
    let out_dir = tmp.child("exports");
    fs::create_dir_all(&out_dir)?;
    let kept = out_dir.join("Enzyme Activity.csv");
    let kept_bytes: &[u8] = b"earlier export the user edited by hand\n\xEF\xBB\xBFnot,regenerated\n";
    fs::write(&kept, kept_bytes)?;
    let second = out_dir.join("Second Table.csv");
    let export_params = json!({ "delimiter": ",", "decimal_separator": "." });

    // 1. Clash prompts without writing anything.
    adapter.push_response("ui.export.allTablesCsv", Some(out_dir.clone()));
    let prompt = rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &export_params)?.expect("prompt");
    assert_eq!(
        (prompt.action.as_str(), prompt.command_result.as_ref().and_then(|r| r.get("existing_files"))),
        ("native_file_dialog.confirm_existing_files:ui.export.allTablesCsv", Some(&json!(["Enzyme Activity.csv"])))
    );
    assert!(
        matches!(&rt.window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::ExportAllTablesExistingFiles { dir, existing_files } if *dir == out_dir.to_string_lossy() && existing_files == &vec![String::from("Enzyme Activity.csv")])
    );
    assert!(fs::read(&kept)? == kept_bytes && !second.exists());

    // 2. Cancel drops batch; stale resume fails.
    assert_eq!(
        rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "on_existing": "cancel" }))?.map(|o| o.action).as_deref(),
        Some("native_file_dialog.cancelled:ui.export.allTablesCsv")
    );
    assert!(rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "on_existing": "replace" })).is_err());
    assert!(fs::read(&kept)? == kept_bytes && !second.exists());

    // 3-4. Unknown choice rejected; Keep Both writes numbered copy.
    adapter.push_response("ui.export.allTablesCsv", Some(out_dir.clone()));
    let _ = rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &export_params)?;
    assert!(rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "on_existing": "overwrite_all" })).is_err());
    assert_eq!(
        rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "on_existing": "keep_both" }))?.expect("keep").action,
        "native_file_dialog.completed:ui.export.allTablesCsv"
    );
    let copy = out_dir.join("Enzyme Activity-2.csv");
    assert!(fs::read(&kept)? == kept_bytes && fs::read_to_string(&copy)?.contains("10.5") && second.exists());

    // 5. Replace overwrites only planned names.
    let copy_bytes = fs::read(&copy)?;
    adapter.push_response("ui.export.allTablesCsv", Some(out_dir.clone()));
    let prompt2 = rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &export_params)?.expect("prompt2");
    assert_eq!(prompt2.command_result.as_ref().and_then(|r| r.get("existing_files")), Some(&json!(["Enzyme Activity.csv", "Second Table.csv"])));
    assert_eq!(
        rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "on_existing": "replace" }))?.expect("replace").action,
        "native_file_dialog.completed:ui.export.allTablesCsv"
    );
    assert!(fs::read(&kept)? == copy_bytes && fs::read(&copy)? == copy_bytes && !out_dir.join("Enzyme Activity-3.csv").exists());

    // 6. Fresh dir exports without prompt.
    let fresh_dir = tmp.child("fresh");
    adapter.push_response("ui.export.allTablesCsv", Some(fresh_dir.clone()));
    assert_eq!(
        rt.dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &export_params)?.expect("fresh").action,
        "native_file_dialog.completed:ui.export.allTablesCsv"
    );
    assert!(fresh_dir.join("Enzyme Activity.csv").exists() && fresh_dir.join("Second Table.csv").exists());
    Ok(())
}

fn write_warning_xlsx(path: &Path, table_title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="{table_title}" sheetId="1" r:id="rId1"/>
    <sheet name="ChartTab" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
</Relationships>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Signal</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>12.5</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><v>24.5</v></c></row>
  </sheetData>
</worksheet>"#;
    fs::write(
        path,
        build_zip_entries(&[("xl/workbook.xml", &workbook), ("xl/_rels/workbook.xml.rels", rels), ("xl/worksheets/sheet1.xml", sheet)])?,
    )?;
    Ok(())
}

#[test]
fn file_open_interchange_on_occupied_project_prompts_replace_or_merge() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r10-29-file-open-prompt")?;
    let (saved_project, other_project) = (tmp.child("existing.curvebook"), tmp.child("other.curvebook"));
    let (incoming_pzfx, incoming_archive, incoming_xlsx) = (tmp.child("incoming.pzfx"), tmp.child("incoming.prism"), tmp.child("incoming.xlsx"));
    write_pzfx(&incoming_pzfx, "Opened PZFX", true)?;
    write_warning_zip_project(&incoming_archive, "Opened Archive")?;
    write_warning_xlsx(&incoming_xlsx, "Opened XLSX")?;

    let base_doc = build_col_doc("base-1", "Base Sheet", &[("A", &[7.0])])?;
    save_project_file(&saved_project, &base_doc)?;
    let other_doc = build_col_doc("other-1", "Other CurveBook", &[("A", &[1.0])])?;
    save_project_file(&other_project, &other_doc)?;

    // 1. Clean non-empty project: File > Open (.pzfx / .prism / .xlsx) prompts replace-or-merge.
    for (open_cmd, path, expected_import_cmd, expected_title) in [
        ("project.open", &incoming_pzfx, "table.importPzfx", "Opened PZFX"),
        ("file.open", &incoming_archive, "table.importPrism", "Opened Archive"),
        ("project.open", &incoming_xlsx, "table.importXlsx", "Opened XLSX"),
    ] {
        let mut session = Session::from_document(base_doc.clone());
        session.set_file_path(Some(saved_project.clone()), true);
        let (mut rt, wid, adapter) = spawn_runtime(session)?;
        adapter.push_response(open_cmd, Some(path.clone()));

        let out = rt.dispatch_native_file_command(wid, open_cmd)?.expect("handled");
        assert_eq!(out.action, format!("native_file_dialog.confirm_import_mode:{expected_import_cmd}"));
        assert!(out.opened_window_id.is_none());
        assert_eq!(rt.window_count(), 1);
        assert!(matches!(
            &rt.window(wid)?.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, path: p, has_unsaved_changes: false }
                if command_id == expected_import_cmd && *p == path.to_string_lossy()
        ));

        // Merge preserves existing sheet and file_path, adds incoming sheet, and captures warnings.
        let _ = rt.dispatch_native_file_command_with_params(wid, expected_import_cmd, &json!({ "path": path.to_string_lossy(), "merge": true }))?;
        let win = rt.window(wid)?;
        let titles: Vec<&str> = win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"Base Sheet") && titles.contains(&expected_title), "{expected_import_cmd}: {titles:?}");
        assert_eq!(win.app().session().file_path(), Some(saved_project.as_path()));
        assert!(!win.app().import_warnings.warnings.is_empty(), "{expected_import_cmd}: expected import warnings");
    }

    // 2. Dirty project + explicit path (Welcome Hub Open a File): prompts with has_unsaved_changes=true in-window.
    {
        let mut dirty = Session::from_document(base_doc.clone());
        dirty.set_file_path(Some(saved_project.clone()), false);
        let _ = dirty.execute("sheet.rename", json!({ "sheet_id": "base-1", "title": "Dirty Base" }))?;
        let (mut rt, wid, _) = spawn_runtime(dirty)?;
        let out =
            rt.dispatch_native_file_command_with_params(wid, "project.open", &json!({ "path": incoming_pzfx.to_string_lossy() }))?.expect("prompted");
        assert_eq!(out.action, "native_file_dialog.confirm_import_mode:table.importPzfx");
        assert_eq!(rt.window_count(), 1, "dirty interchange File > Open must prompt in-window rather than silently spawning a new window");
        assert!(matches!(
            &rt.window(wid)?.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: true, .. } if command_id == "table.importPzfx"
        ));

        let _ = rt.dispatch_native_file_command_with_params(
            wid,
            "table.importPzfx",
            &json!({ "path": incoming_pzfx.to_string_lossy(), "merge": false }),
        )?;
        let win = rt.window(wid)?;
        let titles: Vec<&str> = win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Opened PZFX"]);
        assert_eq!(win.app().session().file_path(), None);
    }

    // 3. Empty window opens interchange file without prompting and still records warnings.
    {
        let (mut rt, wid, adapter) = spawn_runtime(Session::new())?;
        adapter.push_response("project.open", Some(incoming_xlsx.clone()));
        let out = rt.dispatch_native_file_command(wid, "project.open")?.expect("loaded");
        assert_eq!(out.action, "file.dropped_loaded_in_window");
        let win = rt.window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));
        assert!(!win.app().import_warnings.warnings.is_empty());
        assert_eq!(win.app().session().file_path(), None);
    }

    // 4. Non-interchange `.curvebook` file on clean occupied window still loads directly without prompt.
    {
        let mut session = Session::from_document(base_doc);
        session.set_file_path(Some(saved_project), true);
        let (mut rt, wid, adapter) = spawn_runtime(session)?;
        adapter.push_response("project.open", Some(other_project.clone()));
        let out = rt.dispatch_native_file_command(wid, "project.open")?.expect("loaded");
        assert_eq!(out.action, "file.dropped_loaded_in_window");
        let win = rt.window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        assert_eq!(win.app().session().file_path(), Some(other_project.as_path()));
    }

    Ok(())
}

#[test]
fn csv_locale_default_and_command_palette_import_export_routing() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook::desktop_host::{DesktopCommandRoute, resolve_desktop_command_route};
    use curvebook_ui_egui::dialogs_canvas::CsvExportOptionsState;

    // 1. Comma decimal locales pair with semicolon delimiter; period locales pair with comma delimiter.
    let mut opts = CsvExportOptionsState::default();
    opts.reset_for_locale(',');
    assert_eq!((opts.delimiter, opts.decimal_separator), (';', ','));
    opts.reset_for_locale('.');
    assert_eq!((opts.delimiter, opts.decimal_separator), (',', '.'));

    // 2. Desktop route for `result.generateReport` omits hardcoded `format`, so `.tex` and `.txt` infer cleanly.
    let tmp = TempDirGuard::new("cb-r10-29-palette-route")?;
    let (mut rt, wid, adapter) = spawn_runtime(build_anova_session()?)?;
    let route = resolve_desktop_command_route(rt.window(wid)?.app(), None, "result.generateReport")?;
    let route_params = match route {
        DesktopCommandRoute::EngineCommand { engine_command_id, params } => {
            assert_eq!(engine_command_id, "result.generateReport");
            assert!(params.get("format").is_none(), "route must not hardcode markdown format: {params}");
            params
        }
        other => panic!("expected EngineCommand route for result.generateReport, got {other:?}"),
    };
    let (tex_path, txt_path) = (tmp.child("palette_report.tex"), tmp.child("palette_report.txt"));
    adapter.push_response("result.generateReport", Some(tex_path.clone()));
    adapter.push_response("result.generateReport", Some(txt_path.clone()));
    let _ = rt.dispatch_native_file_command_with_params(wid, "result.generateReport", &route_params)?;
    let _ = rt.dispatch_native_file_command_with_params(wid, "result.generateReport", &route_params)?;
    let tex_text = fs::read_to_string(&tex_path)?;
    let txt_text = fs::read_to_string(&txt_path)?;
    assert!(tex_text.contains('\\'), "expected LaTeX output when routed with default params: {tex_text}");
    assert!(!txt_text.trim().is_empty() && !txt_text.contains("\\section"), "expected plain text output: {txt_text}");

    Ok(())
}

#[test]
fn r10_34_native_file_picker_filters_use_extension_naming() -> Result<(), Box<dyn std::error::Error>> {
    let (mut rt, wid, adapter) = spawn_runtime(Session::new())?;

    for (locale, expected_pzfx, expected_archive) in
        [("en", ".pzfx files", ".prism files"), ("de", ".pzfx-Dateien", ".prism-Dateien"), ("ja", ".pzfx ファイル", ".prism ファイル")]
    {
        rt.window_mut(wid)?.app_mut().set_locale(locale);
        adapter.push_response("table.importPzfx", None);
        adapter.push_response("table.importPrism", None);
        let _ = rt.dispatch_native_file_command(wid, "table.importPzfx")?;
        let _ = rt.dispatch_native_file_command(wid, "table.importPrism")?;
        let reqs = adapter.requests();
        let pzfx_req = reqs.iter().rev().find(|r| r.command_id == "table.importPzfx").expect("pzfx dialog request");
        let archive_req = reqs.iter().rev().find(|r| r.command_id == "table.importPrism").expect("archive dialog request");
        assert_eq!(pzfx_req.filter_label, expected_pzfx, "locale {locale}");
        assert_eq!(pzfx_req.extensions, vec![String::from("pzfx")]);
        assert_eq!(archive_req.filter_label, expected_archive, "locale {locale}");
        assert_eq!(archive_req.extensions, vec![String::from("prism")]);
    }

    rt.window_mut(wid)?.app_mut().set_locale("en");
    adapter.push_response("project.open", None);
    let _ = rt.dispatch_native_file_command(wid, "project.open")?;
    let open_req = adapter.requests().into_iter().rev().find(|r| r.command_id == "project.open").expect("open dialog request");
    assert_eq!(open_req.filter_label, "CurveBook, .pzfx, .prism & Data files");
    assert!(open_req.extensions.iter().any(|e| e == "pzfx") && open_req.extensions.iter().any(|e| e.ends_with("ism")));

    Ok(())
}

fn write_promoted_archive_fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let doc_json = serde_json::to_string(&json!({
        "formatVersion": "1-6-0",
        "title": "Desktop Promoted Archive",
        "sheets": { "data": ["d-groups", "d-single"], "analyses": ["an-ttest", "an-stale", "an-fallback"] },
        "sheetAttributesMap": {
            "d-groups": { "title": "Groups" },
            "d-single": { "title": "Single" },
            "an-ttest": { "title": "Agreeing T-Test" },
            "an-stale": { "title": "Stale T-Test" },
            "an-fallback": { "title": "Degenerate T-Test" }
        }
    }))?;
    let ttest_params = serde_json::to_string(&json!({
        "content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}
    }))?;
    let d_groups = serde_json::to_string(&json!({
        "uid": "d-groups", "title": "Groups",
        "table": {"uid": "t-groups", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-a", "ds-b"]}
    }))?;
    let d_single = serde_json::to_string(&json!({
        "uid": "d-single", "title": "Single",
        "table": {"uid": "t-single", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-only"]}
    }))?;
    let ds_a = serde_json::to_string(&json!({"title": "Control", "replicates": [{"valueType": "real"}]}))?;
    let ds_b = serde_json::to_string(&json!({"title": "Treated", "replicates": [{"valueType": "real"}]}))?;
    let ds_only = serde_json::to_string(&json!({"title": "Only", "replicates": [{"valueType": "real"}]}))?;
    let an_ttest_sheet = serde_json::to_string(&json!({
        "uid": "an-ttest", "title": "Agreeing T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC", "inputSheets": [{"uid": "d-groups"}]
    }))?;
    let an_ttest_res = serde_json::to_string(&json!({
        "content": {"unpairedTTestModel": {"unpairedTTest": {"t": -6.123_724_356_957_945, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.003_602_232_609_1}}}}
    }))?;
    let an_stale_sheet = serde_json::to_string(&json!({
        "uid": "an-stale", "title": "Stale T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC", "inputSheets": [{"uid": "d-groups"}]
    }))?;
    let an_stale_res = serde_json::to_string(&json!({
        "content": {"unpairedTTestModel": {"unpairedTTest": {"t": -6.123_724_356_957_945, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.156}}}}
    }))?;
    let an_fb_sheet = serde_json::to_string(&json!({
        "uid": "an-fallback", "title": "Degenerate T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
        "inputSheets": [{"uid": "d-single"}], "preferredResultSheet": "rs-fb", "resultSheets": [{"uid": "rs-fb", "title": "Cached Fallback"}]
    }))?;
    let rs_fb = serde_json::to_string(&json!({"uid": "rs-fb", "name": "Cached Fallback", "dataSheet": "view-fb"}))?;
    let view_fb = serde_json::to_string(&json!({
        "uid": "view-fb", "flags": ["ANALYSIS_VIEW"],
        "table": {"uid": "t-view-fb", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-fb-rt", "dataSets": ["ds-fb-v"]}
    }))?;
    let ds_fb_rt = serde_json::to_string(&json!({"title": "Metric", "replicates": [{"valueType": "string"}]}))?;
    let ds_fb_v = serde_json::to_string(&json!({"title": "Value", "replicates": [{"valueType": "string"}]}))?;

    fs::write(
        path,
        build_zip_entries(&[
            ("document.json", &doc_json),
            ("data/sheets/d-groups/sheet.json", &d_groups),
            ("data/sets/ds-a.json", &ds_a),
            ("data/sets/ds-b.json", &ds_b),
            ("data/tables/t-groups/data.csv", "10,20\n12,22\n14,24\n"),
            ("data/sheets/d-single/sheet.json", &d_single),
            ("data/sets/ds-only.json", &ds_only),
            ("data/tables/t-single/data.csv", "1\n2\n3\n"),
            ("analyses/an-ttest/sheet.json", &an_ttest_sheet),
            ("analyses/an-ttest/parameters.json", &ttest_params),
            ("analyses/an-ttest/results.json", &an_ttest_res),
            ("analyses/an-stale/sheet.json", &an_stale_sheet),
            ("analyses/an-stale/parameters.json", &ttest_params),
            ("analyses/an-stale/results.json", &an_stale_res),
            ("analyses/an-fallback/sheet.json", &an_fb_sheet),
            ("analyses/an-fallback/parameters.json", &ttest_params),
            ("analyses/an-fallback/result_sheets/rs-fb.json", &rs_fb),
            ("data/sheets/view-fb/sheet.json", &view_fb),
            ("data/sets/ds-fb-rt.json", &ds_fb_rt),
            ("data/sets/ds-fb-v.json", &ds_fb_v),
            ("data/tables/t-view-fb/data.csv", "P value,0.042\n"),
        ])?,
    )?;
    Ok(())
}

#[test]
fn desktop_host_open_promoted_archive_recomputes_live_surfaces_warnings_and_tracks_edits() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r25-1-desktop-promoted")?;
    let archive_path = tmp.child("promoted.prism");
    write_promoted_archive_fixture(&archive_path)?;

    // 1. Open via `open_path_in_window` (`File > Open` / drag-and-drop on empty window).
    let (mut rt, wid, _) = spawn_runtime(Session::new())?;
    let drop_out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: archive_path.clone() })?;
    assert_eq!(drop_out.action, "file.dropped_loaded_in_window");

    {
        let win = rt.window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));
        let warnings = &win.app().import_warnings.warnings;
        assert!(
            warnings.iter().any(|w| w.kind == "RecomputedResultDisagreement" && w.message.contains("Stale T-Test") && w.message.contains("0.156")),
            "missing disagreement warning in app.import_warnings: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.kind == "AnalysisRecomputeFallback" && w.message.contains("Degenerate T-Test")),
            "missing fallback warning in app.import_warnings: {warnings:?}"
        );

        let snap = curvebook_ui_egui::inspect::inspect_ui_tree_for_metrics(win.app(), ViewportMetrics::new(1280.0, 800.0, 1.0), None, false);
        let iw = snap.import_warnings.as_ref().expect("ui.inspect import_warnings");
        assert!(iw.open && iw.command_id == "table.importPrism" && iw.source_label == "promoted.prism");
        assert!(iw.warnings.iter().any(|w| w.kind == "RecomputedResultDisagreement"));
        assert!(iw.warnings.iter().any(|w| w.kind == "AnalysisRecomputeFallback"));

        let doc = win.app().document();
        assert_eq!(doc.result_sheet(&SheetId::new("an-ttest")).expect("an-ttest").status, ResultStatus::UpToDate);
        assert_eq!(doc.result_sheet(&SheetId::new("an-stale")).expect("an-stale").status, ResultStatus::UpToDate);
        assert_eq!(doc.result_sheet(&SheetId::new("an-stale")).expect("an-stale").provenance, ResultProvenance::Computed);
        let fb = doc.result_sheet(&SheetId::new("an-fallback")).expect("an-fallback");
        assert_eq!(fb.status, ResultStatus::Imported);
        assert!(matches!(fb.analysis.kind, AnalysisKind::ImportedSnapshot(_)));
        assert_eq!(fb.tabs.len(), 1);
    }

    // 2. Live-editing a source cell immediately recomputes the promoted t-test.
    let p_before =
        rt.window(wid)?.app().document().result_sheet(&SheetId::new("an-ttest")).expect("an-ttest").lookup_scalar("PValue", None).expect("p");
    let _ = rt.window_mut(wid)?.execute_and_journal("table.setCell", json!({ "sheet_id": "d-groups", "row": 0, "col": 0, "value": 19.0 }))?;
    let p_after =
        rt.window(wid)?.app().document().result_sheet(&SheetId::new("an-ttest")).expect("an-ttest").lookup_scalar("PValue", None).expect("p");
    assert!((p_after - p_before).abs() > 1e-3, "live cell edit must recompute promoted t-test: before={p_before}, after={p_after}");

    // 3. Startup `open_window(Some(archive_path), ...)` and `open_document_from_files_with_warnings`
    //    + `record_startup_import_warnings` (used by `main.rs`) both finalize and record warnings.
    let mut startup_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let startup_wid = startup_rt.open_window(Some(archive_path.clone()), ViewportMetrics::new(1280.0, 800.0, 1.0))?;
    let startup_win = startup_rt.window(startup_wid)?;
    assert!(matches!(startup_win.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));
    assert!(startup_win.app().import_warnings.warnings.iter().any(|w| w.kind == "RecomputedResultDisagreement"));
    assert!(startup_win.app().import_warnings.warnings.iter().any(|w| w.kind == "AnalysisRecomputeFallback"));
    assert_eq!(startup_win.app().document().result_sheet(&SheetId::new("an-ttest")).expect("an-ttest").status, ResultStatus::UpToDate);
    assert_eq!(startup_win.app().document().result_sheet(&SheetId::new("an-fallback")).expect("an-fallback").status, ResultStatus::Imported);

    let (cli_session, cli_warnings) = curvebook::desktop_host::open_document_from_files_with_warnings(&[archive_path])?;
    let mut main_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let main_wid = main_rt.open_window_with_session(cli_session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    let sw = cli_warnings.expect("startup warnings must be returned for .prism file");
    main_rt.record_startup_import_warnings(main_wid, &sw);
    let main_win = main_rt.window(main_wid)?;
    assert!(matches!(main_win.app().canvas_dialogs.active_dialog, ActiveDialog::ImportWarnings));
    assert!(main_win.app().import_warnings.warnings.iter().any(|w| w.kind == "RecomputedResultDisagreement"));
    assert!(main_win.app().import_warnings.warnings.iter().any(|w| w.kind == "AnalysisRecomputeFallback"));

    Ok(())
}
