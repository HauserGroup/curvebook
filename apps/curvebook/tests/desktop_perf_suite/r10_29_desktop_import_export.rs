#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, NativeFileDialogKind,
    StubNativeFileDialogAdapter,
};
use curvebook::gui_window::CurveBookNativeApp;
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
