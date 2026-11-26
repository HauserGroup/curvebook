#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

//! Regression tests for desktop-host & native window gap audit fixes:
//! - `GAP-06-02`: Desktop native Save-dialog figure export surfaces `CommandOutcome.warnings`.
//! - `GAP-07-03`: Opening/dropping `.prism`/`.pzfx`/`.xlsx` supports `"Open in new window"` and `--new-instance`.
//! - `GAP-07-04`: File drops blocked while any modal is open; mixed `[proj.prism, data.csv]` never imports behind prompt.
//! - `GAP-07-05`: Hardened Export All Data Tables (Unicode case-folding, 4096-byte titles, Windows reserved names, mid-batch failure).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, StubNativeFileDialogAdapter,
};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CurveSamplePoint, FittedCurveOverlay, GraphSheet, NonlinearFitSpec, ProjectDocument, RegressionOverlaySpec,
    ResultSheet, ResultStatus, SheetId,
};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_testkit::wf_datasets::{ColumnFixture, column_three_group};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

use crate::common::{NativeHarness, TempDirGuard, build_xy_doc, build_zip_entries};

#[derive(Debug)]
struct TestDroppedFile(PathBuf);
impl egui::DroppedFile for TestDroppedFile {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn drop_files(h: &mut NativeHarness, paths: &[&Path]) {
    let handles: Vec<egui::DroppedFileHandle> =
        paths.iter().map(|p| Arc::new(TestDroppedFile((*p).to_path_buf())) as egui::DroppedFileHandle).collect();
    let _ = h.step_raw(egui::RawInput { dropped_files: handles, ..Default::default() });
    h.settle(2);
}

fn write_pzfx_fixture(path: &Path, title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fx: ColumnFixture = column_three_group();
    let mut cols = String::new();
    for (col_title, values) in fx.group_titles.iter().zip(&fx.columns) {
        let cells: String = values.iter().map(|v| format!("<d>{v}</d>")).collect();
        cols.push_str(&format!("<YColumn><Title>{col_title}</Title><Subcolumn>{cells}</Subcolumn></YColumn>"));
    }
    fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile SchemaVersion="2"><TableSequence><Table ID="Table0" TableType="OneWay"><Title>{title}</Title>{cols}</Table></TableSequence></PrismFile>"#
        ),
    )?;
    Ok(())
}

fn write_container_fixture(path: &Path, title: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    let mut fx = column_three_group();
    fx.title = title;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(fx.to_data_sheet("incoming-container")?)?;
    let manifest = serde_json::to_string(&json!({ "title": title }))?;
    let doc_json = serde_json::to_string(&doc)?;
    fs::write(path, build_zip_entries(&[("manifest.json", &manifest), ("document.json", &doc_json)])?)?;
    Ok(())
}

fn saved_occupied_runtime(dir: &Path, new_instance: bool) -> Result<(DesktopHostRuntime, DesktopWindowId, PathBuf), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(column_three_group().to_data_sheet("base-assay")?)?;
    let saved = dir.join("existing.curvebook");
    save_project_file(&saved, &doc)?;
    let mut session = Session::from_document(doc);
    session.set_file_path(Some(saved.clone()), true);
    let cfg = DesktopHostConfig { new_instance, ..DesktopHostConfig::default() };
    let mut rt = DesktopHostRuntime::new(cfg);
    let wid = rt.open_window_with_session(session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    Ok((rt, wid, saved))
}

#[test]
fn gap_06_02_desktop_native_figure_export_surfaces_pending_regression_warnings() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("gap-06-02-export-warn")?;
    let mut doc = build_xy_doc("xy-1", "Dose Response", &[(0.1, 5.0), (1.0, 15.0), (10.0, 50.0), (100.0, 85.0), (1000.0, 95.0)])?;
    let ds = doc.data_sheet(&SheetId::new("xy-1")).expect("xy-1").clone();
    let spec = AnalysisSpec::new("rs-fit", "xy-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear")))?;
    let mut res = ResultSheet::new(spec)?;
    res.status = ResultStatus::Dirty;
    let pt = |x: f64, y: f64| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None };
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![pt(0.1, 5.0), pt(1000.0, 95.0)],
    });
    doc.add_result_sheet(res)?;
    let mut graph = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("rs-fit"),
        sample_segments: 32,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    doc.add_graph_sheet(graph)?;
    doc.active_sheet_id = Some(SheetId::new("g-1"));
    let dirty_session = Session::from_document(doc);

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    let out_svg = tmp.child("dirty_fit.svg");
    adapter.push_response("export.svg", Some(out_svg.clone()));
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
    let wid = rt.open_window_with_session(dirty_session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;

    // 1. Direct desktop_host dispatch returns warnings in command_result AND sets UiApp status_banner.
    let outcome = rt.dispatch_native_file_command(wid, "export.svg")?.expect("handled");
    assert_eq!(outcome.action, "native_file_dialog.completed:export.svg");
    assert!(out_svg.exists(), "SVG file should be written");
    let warnings = outcome
        .command_result
        .as_ref()
        .and_then(|v| v.get("warnings"))
        .and_then(serde_json::Value::as_array)
        .expect("warnings array must be preserved in DesktopEventOutcome");
    assert!(
        warnings.iter().any(|w| w.as_str().is_some_and(|s| s.contains("pending_regression_result"))),
        "expected pending_regression_result warning, got {warnings:?}"
    );
    let ui_banner = rt.window(wid)?.app().canvas_dialogs.status_banner.clone();
    assert!(
        ui_banner.as_deref().is_some_and(|s| s.contains("pending_regression_result")),
        "expected UiApp status_banner to surface export warning, got {ui_banner:?}"
    );

    // 2. Through CurveBookNativeApp menu action, native toast `status_banner()` surfaces the warning too.
    let out_svg_2 = tmp.child("dirty_fit_toast.svg");
    adapter.push_response("export.svg", Some(out_svg_2.clone()));
    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(2);
    let ctx = h.ctx.clone();
    h.app.trigger_menu_action(&ctx, "export.svg");
    h.settle(2);
    assert!(out_svg_2.exists());
    assert!(
        h.app.status_banner().is_some_and(|s| s.contains("pending_regression_result")),
        "expected desktop toast status_banner to surface export warning, got {:?}",
        h.app.status_banner()
    );
    Ok(())
}

#[test]
fn gap_07_03_import_mode_offers_open_in_new_window_and_respects_new_instance() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("gap-07-03-new-window")?;
    let pzfx_path = tmp.child("incoming.pzfx");
    let container_path = tmp.child("incoming.prism");
    write_pzfx_fixture(&pzfx_path, "Incoming PZFX")?;
    write_container_fixture(&container_path, "Incoming Archive")?;

    // 1. Dirty occupied window (default new_instance=false): drop prompts ImportProjectMode with
    //    `import_project_mode.new_window`, clicking it opens a second window preserving original work.
    {
        let (mut rt, wid, saved) = saved_occupied_runtime(tmp.path(), false)?;
        rt.window_mut(wid)?.execute_and_journal("sheet.rename", json!({ "sheet_id": "base-assay", "title": "Unsaved Work" }))?;
        let undo_before = rt.window(wid)?.app().session().undo_depth();

        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_files(&mut h, &[&pzfx_path]);
        assert!(matches!(
            &h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: true, .. } if command_id == "table.importPzfx"
        ));

        let _ = h.click_widget("import_project_mode.new_window")?;
        h.settle(2);

        assert_eq!(h.app.runtime().window_count(), 2, "Open in new window must spawn a second window");
        let orig = h.app.runtime().window(wid)?;
        assert!(matches!(orig.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        assert_eq!(orig.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["Unsaved Work"]);
        assert_eq!(orig.app().session().file_path(), Some(saved.as_path()));
        assert_eq!(orig.app().session().undo_depth(), undo_before);
        assert!(orig.has_unsaved_changes());

        let new_wid = h.app.runtime().window_ids().into_iter().find(|&id| id != wid).expect("second window");
        let new_win = h.app.runtime().window(new_wid)?;
        assert_eq!(new_win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["Incoming PZFX"]);
    }

    // 2. With `--new-instance` (`config.new_instance = true`), dropping or opening `.prism`/`.pzfx`
    //    directly spawns a new window without forcing in-place replacement.
    {
        let (mut rt, wid, _) = saved_occupied_runtime(tmp.path(), true)?;
        let out_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: container_path.clone() })?;
        assert_eq!(out_drop.action, "file.dropped_opened_new_window");
        let new_drop_id = out_drop.opened_window_id.expect("drop under --new-instance must open new window");
        assert_eq!(rt.window_count(), 2);
        assert!(matches!(rt.window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        assert_eq!(rt.window(new_drop_id)?.app().document().data_sheets[0].title, "Incoming Archive");

        let out_open =
            rt.dispatch_native_file_command_with_params(wid, "project.open", &json!({ "path": pzfx_path.to_string_lossy() }))?.expect("open");
        assert_eq!(out_open.action, "file.dropped_opened_new_window");
        assert!(out_open.opened_window_id.is_some());
        assert_eq!(rt.window_count(), 3);
    }
    Ok(())
}

#[test]
fn gap_07_04_file_drops_blocked_by_open_modals_and_mixed_multi_file_drop_safe() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("gap-07-04-modal-drop")?;
    let container_a = tmp.child("first.prism");
    let pzfx_b = tmp.child("second.pzfx");
    let extra_csv = tmp.child("extra.csv");
    write_container_fixture(&container_a, "First Archive")?;
    write_pzfx_fixture(&pzfx_b, "Second PZFX")?;
    fs::write(&extra_csv, "A,B\n1,2\n3,4\n")?;

    // 1. Dropping a file while EditAnalysisParameters is open must NOT overwrite active_dialog or draft.
    {
        let (mut rt, wid, _) = saved_occupied_runtime(tmp.path(), false)?;
        let rs_out = rt.window_mut(wid)?.execute_and_journal(
            "analysis.run",
            json!({ "source_sheet_id": "base-assay", "kind": "one_way_anova", "sheet_id": "rs-anova", "alpha": 0.05 }),
        )?;
        let rs_id = SheetId::new(rs_out.payload["sheet_id"].as_str().expect("rs id"));
        let doc_snap = rt.window(wid)?.app().document().clone();
        assert!(rt.window_mut(wid)?.app_mut().canvas_dialogs.open_edit_parameters(&doc_snap, &rs_id));

        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_files(&mut h, &[&container_a]);

        assert!(
            matches!(&h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { result_sheet_id } if *result_sheet_id == rs_id),
            "EditAnalysisParameters must remain open after blocked file drop"
        );
        assert!(
            h.app.runtime().window(wid)?.app().canvas_dialogs.edit_params.draft.is_some(),
            "EditAnalysisParameters draft must survive blocked drop"
        );
        assert!(h.app.status_banner().is_some_and(|s| s.to_ascii_lowercase().contains("dialog is open")));
    }

    // 2. Dropping a second interchange file while ImportProjectMode is open preserves the first prompt.
    {
        let (rt, wid, _) = saved_occupied_runtime(tmp.path(), false)?;
        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_files(&mut h, &[&container_a]);
        assert!(matches!(
            &h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, path, .. } if command_id == "table.importPrism" && *path == container_a.to_string_lossy()
        ));

        drop_files(&mut h, &[&pzfx_b]);
        assert!(
            matches!(
                &h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog,
                ActiveDialog::ImportProjectMode { command_id, path, .. } if command_id == "table.importPrism" && *path == container_a.to_string_lossy()
            ),
            "second drop must not overwrite first ImportProjectMode prompt"
        );
    }

    // 3. Mixed multi-file drop `[first.prism, extra.csv]` onto clean occupied window opens prompt
    //    for `first.prism` and MUST NOT import `extra.csv` behind the prompt.
    {
        let (rt, wid, _) = saved_occupied_runtime(tmp.path(), false)?;
        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_files(&mut h, &[&container_a, &extra_csv]);

        let win = h.app.runtime().window(wid)?;
        assert!(matches!(
            &win.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: false, .. } if command_id == "table.importPrism"
        ));
        assert_eq!(win.app().document().data_sheets.len(), 1, "extra.csv must not be imported behind ImportProjectMode");
        assert!(!win.has_unsaved_changes(), "window dirty state must remain false matching prompt");
        assert!(h.app.status_banner().is_some_and(|s| s.contains("extra.csv")), "status banner should explain skipped extra.csv");
    }
    Ok(())
}

#[test]
fn gap_07_05_all_tables_export_handles_unicode_long_titles_reserved_names_and_dir_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("gap-07-05-all-tables")?;
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
    let wid = rt.open_window(None, ViewportMetrics::new(1280.0, 800.0, 1.0))?;

    let long_title = "α".repeat(1500); // 3000 bytes (within MAX_TITLE_BYTES=4096, far above 255B FS limit)
    for (idx, title) in ["Überblick", "überblick", "CON", "nul.csv", long_title.as_str()].into_iter().enumerate() {
        let sid = format!("t-{idx}");
        rt.window_mut(wid)?.execute_and_journal("sheet.createData", json!({ "sheet_id": sid, "title": title, "auto_graph": false }))?;
        rt.window_mut(wid)?.execute_and_journal("table.setCell", json!({ "sheet_id": sid, "row": 0, "col": 0, "value": (idx + 1) as f64 }))?;
    }

    let out_dir = tmp.child("export-ok");
    adapter.push_response("ui.export.allTablesCsv", Some(out_dir.clone()));
    let out = rt
        .dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "delimiter": ",", "decimal_separator": "." }))?
        .expect("exported");
    assert_eq!(out.action, "native_file_dialog.completed:ui.export.allTablesCsv");
    let exported = out.command_result.as_ref().and_then(|v| v.get("exported_files")).and_then(serde_json::Value::as_array).expect("exported_files");
    assert_eq!(exported.len(), 5);
    let mut seen_folded = std::collections::HashSet::new();
    for entry in exported {
        let path = PathBuf::from(entry.as_str().expect("path str"));
        assert!(path.is_file(), "missing exported file {}", path.display());
        let fname = path.file_name().unwrap().to_str().unwrap();
        assert!(fname.len() < 255, "filename must be < 255 bytes: {}", fname.len());
        assert!(seen_folded.insert(fname.to_lowercase()), "folded filename collision: {fname}");
    }
    assert!(out_dir.join("Überblick.csv").is_file() && out_dir.join("überblick-2.csv").is_file());
    assert!(out_dir.join("table_CON.csv").is_file() && out_dir.join("table_nul.csv.csv").is_file());

    // Pre-existing directory blocking one planned destination fails BEFORE overwriting earlier files.
    let conflict_dir = tmp.child("export-dir-conflict");
    fs::create_dir_all(conflict_dir.join("überblick-2.csv"))?;
    let sentinel = conflict_dir.join("Überblick.csv");
    fs::write(&sentinel, b"DO_NOT_OVERWRITE")?;
    adapter.push_response("ui.export.allTablesCsv", Some(conflict_dir.clone()));
    let prompt = rt
        .dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "delimiter": ",", "decimal_separator": "." }))?
        .expect("prompt");
    assert_eq!(prompt.action, "native_file_dialog.confirm_existing_files:ui.export.allTablesCsv");
    let replace_err = rt
        .dispatch_native_file_command_with_params(wid, "ui.export.allTablesCsv", &json!({ "on_existing": "replace" }))
        .expect_err("directory collision must fail cleanly");
    assert!(replace_err.contains("directory"));
    assert_eq!(fs::read(&sentinel)?, b"DO_NOT_OVERWRITE", "earlier file must stay untouched when batch fails");
    assert!(!conflict_dir.join("table_CON.csv").exists(), "later file must not be partially created");
    Ok(())
}
