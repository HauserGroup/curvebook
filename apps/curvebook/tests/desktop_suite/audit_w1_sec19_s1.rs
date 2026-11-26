#![forbid(unsafe_code)]

//! Audit regression tests for `FA-SEC19-02` & `FA-SEC19-03`:
//! - `ExportFigure` modal + in-window/Command Palette export, template, import, and image commands
//!   route through `NativeFileDialogAdapter` without dropping bytes or non-path options.
//! - Multi-window switcher preserves reachability of earlier/unsaved/CLI windows, and `app.quit` /
//!   `file.quit` walks unsaved confirmation across all dirty windows without aliasing `window.close`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use curvebook::desktop_host::{
    CloseDisposition, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, NativeFileDialogKind,
    StubNativeFileDialogAdapter, current_epoch_ms, default_recovery_ttl_ms,
};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_format::save_template_bytes;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui::{Key, Modifiers, Vec2, ViewportCommand};
use serde_json::json;

use crate::common::{NativeHarness, TempDirGuard, build_xy_doc, encode_base64, png_dimensions};

fn build_rich_test_project() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = build_xy_doc("ds-xy", "Dose Response", &[(1.0, 10.0), (2.0, 22.0), (4.0, 48.0)])?;
    let mut ds_col = DataSheet::new("ds-col", "Column Data", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?))?;
    ds_col.set_cell(CellCoord::new(0, 0, 0), Cell::number(5.0))?;
    ds_col.set_cell(CellCoord::new(0, 1, 0), Cell::number(15.0))?;
    doc.add_data_sheet(ds_col)?;

    let ds_ref = doc.data_sheet(&"ds-xy".into()).ok_or("missing ds-xy")?;
    let mut graph = GraphSheet::new_uninitialized_for_data("gs-1", ds_ref)?;
    graph.initialized = true;
    graph.title = String::from("Response Graph");
    graph.spec.kind = GraphKind::Xy;
    doc.add_graph_sheet(graph)?;
    doc.add_layout_sheet(LayoutSheet::new("ls-1", "Summary Layout", LayoutSpec::default())?)?;
    let _ = doc.select_in_navigator(&"gs-1".into());
    Ok(doc)
}

fn new_host_runtime(recovery_root: Option<PathBuf>) -> DesktopHostRuntime {
    DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        locale_override: Some(String::from("en")),
        recovery_root,
        initial_viewport: ViewportMetrics::new(1280.0, 820.0, 1.0),
        ..DesktopHostConfig::default()
    })
}

fn open_window_with_rich_project(rt: &mut DesktopHostRuntime) -> Result<DesktopWindowId, Box<dyn std::error::Error>> {
    let mut sess = Session::from_document(build_rich_test_project()?);
    sess.execute("analysis.run", json!({ "source_sheet_id": "ds-xy", "kind": "linear_regression" }))?;
    Ok(rt.open_window_with_session(sess, ViewportMetrics::new(1280.0, 820.0, 1.0), false)?)
}

#[test]
fn sec19_02_export_figure_modal_all_formats_write_files_and_preserve_options() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-sec19-export")?;
    let mut rt = new_host_runtime(None);
    let w1 = open_window_with_rich_project(&mut rt)?;

    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    let paths: Vec<(&str, PathBuf, &[&str])> = vec![
        ("graph.exportPng", dir.child("modal-graph.png"), &["export.fmt.png", "export.preset.174mm"]),
        ("export.tiff", dir.child("modal-graph.tiff"), &["export.fmt.tiff_lzw"]),
        ("graph.exportSvg", dir.child("modal-graph.svg"), &["export.fmt.svg"]),
        ("graph.exportPdf", dir.child("modal-graph.pdf"), &["export.fmt.pdf", "export.cs.cmyk"]),
        ("graph.exportEmf", dir.child("modal-graph.emf"), &["export.fmt.emf"]),
    ];
    for (cmd, p, _) in &paths {
        adapter.push_response(*cmd, Some(p.clone()));
    }
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, w1, None), Vec2::new(1280.0, 820.0));
    h.settle(4);

    let mut expected_png_w = 0u32;
    for (idx, (_, _, clicks)) in paths.iter().enumerate() {
        h.app.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
        h.settle(4);
        for wid in *clicks {
            let _ = h.click_widget(wid)?;
        }
        if idx == 0 {
            expected_png_w = h.app.runtime().window(w1)?.app().canvas_dialogs.export_figure.width_px;
        }
        let _ = h.click_widget("modal.ok")?;
        assert_eq!(h.app.status_banner(), None);
        assert_eq!(h.app.runtime().window(w1)?.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    }

    let (actual_w, actual_h) = png_dimensions(&fs::read(&paths[0].1)?).ok_or("invalid PNG")?;
    assert_eq!(actual_w, expected_png_w);
    assert!(actual_h > 0);
    let tiff_bytes = fs::read(&paths[1].1)?;
    assert!(tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*"));
    assert!(fs::read_to_string(&paths[2].1)?.contains("<svg"));
    assert!(fs::read(&paths[3].1)?.starts_with(b"%PDF-"));
    let emf_bytes = fs::read(&paths[4].1)?;
    assert!(emf_bytes.len() > 80 && emf_bytes.starts_with(&[1, 0, 0, 0]));

    // Direct host dispatch preserves exact non-path PNG dimensions & PDF color_space/compress_streams.
    let (custom_png, custom_rgb, custom_cmyk) = (dir.child("custom-777x444.png"), dir.child("custom-rgb.pdf"), dir.child("custom-cmyk.pdf"));
    adapter.push_response("graph.exportPng", Some(custom_png.clone()));
    adapter.push_response("graph.exportPdf", Some(custom_rgb.clone()));
    adapter.push_response("graph.exportPdf", Some(custom_cmyk.clone()));

    let _ = h.app.runtime_mut().dispatch_native_file_command_with_params(
        w1,
        "graph.exportPng",
        &json!({ "sheet_id": "gs-1", "width_px": 777, "height_px": 444, "dpi": 150 }),
    )?;
    assert_eq!(png_dimensions(&fs::read(&custom_png)?), Some((777, 444)));

    for cs in ["srgb", "cmyk"] {
        let _ = h.app.runtime_mut().dispatch_native_file_command_with_params(
            w1,
            "graph.exportPdf",
            &json!({ "sheet_id": "gs-1", "color_space": cs, "compress_streams": false }),
        )?;
    }
    let rgb_text = String::from_utf8_lossy(&fs::read(&custom_rgb)?).into_owned();
    let cmyk_bytes = fs::read(&custom_cmyk)?;
    let cmyk_text = String::from_utf8_lossy(&cmyk_bytes);
    assert!(cmyk_bytes.starts_with(b"%PDF-") && rgb_text.contains(" rg\n") && cmyk_text.contains(" k\n") && !cmyk_text.contains(" rg\n"));

    let reqs = adapter.requests();
    assert_eq!(reqs.len(), 8);
    assert!(reqs.iter().all(|r| r.kind == NativeFileDialogKind::SaveFile));
    Ok(())
}

#[test]
fn sec19_02_in_window_menus_and_intents_route_to_native_dialogs_respecting_inline_payloads() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-sec19-routing")?;
    let mut rt = new_host_runtime(None);
    let w1 = open_window_with_rich_project(&mut rt)?;
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());

    let (out_tiff, out_csv, out_json, out_res_csv, out_report, out_tpl, in_csv, seed_png, layout_emf) = (
        dir.child("menu-out.tiff"),
        dir.child("menu-table.csv"),
        dir.child("menu-res.json"),
        dir.child("menu-res.csv"),
        dir.child("cmd-report.md"),
        dir.child("menu-template.cbtpl"),
        dir.child("in-data.csv"),
        dir.child("seed-img.png"),
        dir.child("layout.emf"),
    );
    fs::write(&in_csv, "X,Y\n1.0,2.0\n3.0,6.0\n")?;

    for (cmd, resp) in [
        ("export.png", Some(seed_png.clone())),
        ("export.tiff", Some(out_tiff.clone())),
        ("project.saveTemplate", Some(out_tpl.clone())),
        ("table.importCsv", Some(in_csv)),
        ("graph.insertImage", Some(seed_png.clone())),
        ("table.exportCsv", Some(out_csv.clone())),
        ("analysis.exportJson", Some(out_json.clone())),
        ("analysis.exportCsv", Some(out_res_csv.clone())),
        ("result.generateReport", Some(out_report.clone())),
        ("layout.exportEmf", Some(layout_emf.clone())),
        ("layout.insertImage", Some(seed_png.clone())),
        ("project.instantiateTemplate", Some(out_tpl.clone())),
        ("table.importPrism", None),
        ("table.importPzfx", None),
        ("table.importXlsx", None),
    ] {
        adapter.push_response(cmd, resp);
    }

    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, w1, None), Vec2::new(1280.0, 820.0));
    h.settle(4);

    for (top, sub, item) in [
        ("menu.file", Some("menu.file.export"), "menu.export.png"),
        ("menu.file", Some("menu.file.export"), "menu.export.tiff"),
        ("menu.file", None, "menu.project.saveTemplate"),
        ("menu.file", Some("menu.file.import"), "menu.table.importCsv"),
        ("menu.insert", None, "menu.graph.insertImage"),
    ] {
        h.open_menu_item(top, sub, item)?;
    }
    assert!(seed_png.exists() && out_tiff.exists() && out_tpl.exists());
    assert!(h.app.runtime().window(w1)?.app().document().data_sheets.len() >= 3);
    assert_eq!(h.app.status_banner(), None);

    for cmd_id in [
        "table.exportCsv",
        "analysis.exportJson",
        "analysis.exportCsv",
        "result.generateReport",
        "layout.exportEmf",
        "layout.insertImage",
        "project.instantiateTemplate",
        "table.importPrism",
        "table.importPzfx",
        "table.importXlsx",
    ] {
        let fid = h.app.runtime().focused_window_id().ok_or("missing focused window")?;
        assert!(h.app.runtime_mut().dispatch_native_file_command_with_params(fid, cmd_id, &json!({}))?.is_some(), "`{cmd_id}` must be routed");
    }

    for expected in [&out_csv, &out_json, &out_res_csv, &out_report, &layout_emf] {
        assert!(expected.exists(), "expected `{}` written on disk", expected.display());
    }
    let reqs = adapter.requests();
    assert_eq!(reqs.len(), 15);
    for cmd in ["table.importPrism", "table.importPzfx", "table.importXlsx"] {
        assert!(reqs.iter().any(|r| r.command_id == cmd && r.kind == NativeFileDialogKind::OpenFile));
    }

    // Inline Welcome Hub `project.instantiateTemplate` carrying `bytes_base64` bypasses picker.
    let b64 = encode_base64(&save_template_bytes(&build_rich_test_project()?)?);
    let reqs_before = adapter.requests().len();
    let fid = h.app.runtime().focused_window_id().ok_or("missing focused window")?;
    assert!(h.app.runtime_mut().window_mut(fid)?.execute_and_journal("project.instantiateTemplate", json!({ "bytes_base64": b64 }))?.mutated);
    assert_eq!(adapter.requests().len(), reqs_before);
    Ok(())
}

#[test]
fn sec19_03_multi_window_switcher_and_app_quit_walks_all_dirty_windows() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-sec19-multiwin")?;
    let mut rt = new_host_runtime(None);
    let w1 = rt.open_window(None, ViewportMetrics::new(1280.0, 820.0, 1.0))?;
    rt.window_mut(w1)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-w1", "title": "Win1 Unsaved", "kind": "xy" }))?;

    let w2 =
        rt.dispatch(DesktopHostEvent::MenuAction { window_id: w1, command_id: String::from("window.new") })?.opened_window_id.ok_or("missing w2")?;
    rt.window_mut(w2)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-w2", "title": "Win2 Unsaved", "kind": "xy" }))?;
    assert_eq!((rt.window_count(), rt.focused_window_id()), (2, Some(w2)));

    let (save_w2, save_w1) = (dir.child("win2.curvebook"), dir.child("win1.curvebook"));
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    adapter.push_response("project.save", Some(save_w2.clone()));
    adapter.push_response("project.save", Some(save_w1.clone()));
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, w1, None), Vec2::new(1280.0, 820.0));
    h.settle(4);
    assert!(h.app.is_window_switcher_visible());
    let (w1_rect, _) = h.app.window_switcher_button_rects(w1).ok_or("missing switcher w1")?;
    let (w2_rect, _) = h.app.window_switcher_button_rects(w2).ok_or("missing switcher w2")?;
    assert!(!w1_rect.intersects(w2_rect));

    let _ = h.click_rect(w1_rect);
    assert_eq!(h.app.runtime().focused_window_id(), Some(w1));
    let _ = h.click_rect(w2_rect);
    assert_eq!(h.app.runtime().focused_window_id(), Some(w2));

    let w3 = h
        .app
        .runtime_mut()
        .dispatch(DesktopHostEvent::MenuAction { window_id: w2, command_id: String::from("window.new") })?
        .opened_window_id
        .ok_or("missing w3")?;
    h.settle(4);
    let close_w3 = h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: w3, command_id: String::from("window.close") })?;
    assert_eq!(close_w3.close_disposition, Some(CloseDisposition::Closed));
    assert_eq!(h.app.runtime().window_count(), 2);

    // `file.quit` -> Cancel keeps both windows open.
    let _ = h.app.runtime_mut().focus_window(w2)?;
    h.settle(4);
    h.app.trigger_menu_action(&h.ctx, "file.quit");
    h.settle(4);
    assert!(h.app.close_dialog_open() && !h.app.close_confirmed());
    let _ = h.click_widget("dialog.button.cancel")?;
    assert!(!h.app.close_dialog_open() && !h.app.close_confirmed() && h.app.runtime().window_count() == 2);

    // Cmd+Q (`app.quit`) -> Save w2 -> automatically prompts dirty w1 -> Save w1 -> closes app.
    let _ = h.key_down(Key::Q, Modifiers::COMMAND);
    assert!(h.app.close_dialog_open() && h.app.runtime().focused_window_id() == Some(w2));

    let _ = h.click_widget("dialog.button.save")?;
    assert!(save_w2.exists() && h.app.runtime().window_count() == 1 && h.app.runtime().focused_window_id() == Some(w1));
    assert!(h.app.close_dialog_open() && !h.app.close_confirmed());

    let cmds = h.click_widget("dialog.button.save")?;
    assert!(save_w1.exists() && h.app.runtime().window_count() == 0 && h.app.close_confirmed());
    assert!(cmds.iter().any(|c| matches!(c, ViewportCommand::Close)));
    Ok(())
}

#[test]
fn sec19_03_clean_multi_window_app_quit_and_startup_recovery_focus_preservation() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-sec19-cleanquit")?;
    let recovery_root = dir.child("recovery");
    {
        let mut crashed = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_root.clone()), ..DesktopHostConfig::default() });
        let cw = crashed.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        crashed
            .window_mut(cw)?
            .execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-crash", "title": "Recovered Table", "kind": "xy" }))?;
        std::mem::forget(crashed);
    }

    let mut rt = new_host_runtime(Some(recovery_root));
    let cli_win = rt.open_window_with_session(Session::from_document(build_rich_test_project()?), ViewportMetrics::new(1280.0, 820.0, 1.0), false)?;
    let recovered = rt.recover_stale_journals(
        ViewportMetrics::new(1280.0, 820.0, 1.0),
        current_epoch_ms().saturating_add(default_recovery_ttl_ms()).saturating_add(100),
    )?;
    assert!(!recovered.is_empty());
    let _ = rt.focus_window(cli_win)?;
    assert_eq!((rt.focused_window_id(), rt.window_count()), (Some(cli_win), 1 + recovered.len()));

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, cli_win, None), Vec2::new(1280.0, 820.0));
    h.settle(4);
    assert!(
        h.app.is_window_switcher_visible()
            && h.app.window_switcher_button_rects(cli_win).is_some()
            && h.app.window_switcher_button_rects(recovered[0]).is_some()
    );

    // Clean multi-window Cmd+Q closes all clean windows in one step.
    let mut clean_rt = new_host_runtime(None);
    let (c1, c2) = (
        clean_rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?,
        clean_rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?,
    );
    assert_ne!(c1, c2);
    let mut clean_h = NativeHarness::with_screen(CurveBookNativeApp::new(clean_rt, c1, None), Vec2::new(1280.0, 820.0));
    clean_h.settle(4);
    let quit_cmds = clean_h.key_down(Key::Q, Modifiers::COMMAND);
    assert!(
        clean_h.app.runtime().window_count() == 0 && clean_h.app.close_confirmed() && quit_cmds.iter().any(|c| matches!(c, ViewportCommand::Close))
    );
    Ok(())
}

#[test]
fn wave3_aud10_22_export_figure_stays_open_on_cancel_and_error_and_closes_on_ok() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook_engine::ExternalPathPolicy;

    let dir = TempDirGuard::new("cb-w3-export-seam")?;
    let wd = dir.child("workdir");
    fs::create_dir_all(&wd)?;
    let outside_emf = dir.child("outside.emf");
    let inside_emf = wd.join("inside.emf");

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        locale_override: Some(String::from("en")),
        recovery_root: None,
        path_policy: ExternalPathPolicy::workdir(&wd)?,
        initial_viewport: ViewportMetrics::new(1280.0, 820.0, 1.0),
        ..DesktopHostConfig::default()
    });
    let w1 = open_window_with_rich_project(&mut rt)?;

    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    adapter.push_response("graph.exportEmf", None);
    adapter.push_response("graph.exportEmf", Some(outside_emf.clone()));
    adapter.push_response("graph.exportEmf", Some(inside_emf.clone()));
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, w1, None), Vec2::new(1280.0, 820.0));
    h.settle(4);

    h.app.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
    h.settle(4);
    let _ = h.click_widget("export.fmt.emf")?;

    // 1. Cancelling the OS SaveFile dialog must keep ExportFigure open & preserve EMF selection.
    let _ = h.click_widget("modal.ok")?;
    assert_eq!(h.app.status_banner(), None);
    assert!(
        matches!(h.app.runtime().window(w1)?.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }),
        "AUD-10-22: cancelled OS SaveFile must keep ExportFigure modal open"
    );
    assert!(
        h.app.runtime().window(w1)?.app().egui_views.app_modals.export_emf_selected,
        "AUD-10-22: cancelled OS SaveFile must preserve export_emf_selected"
    );

    // 2. Policy error on export path must surface error banner, keep modal open, and preserve EMF selection.
    let _ = h.click_widget("modal.ok")?;
    assert!(h.app.status_banner().is_some(), "escaped path export must set status_banner");
    assert!(!outside_emf.exists(), "escaped path must not be written");
    assert!(
        matches!(h.app.runtime().window(w1)?.app().canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }),
        "AUD-10-22: failed host export must keep ExportFigure modal open"
    );
    assert!(
        h.app.runtime().window(w1)?.app().egui_views.app_modals.export_emf_selected,
        "AUD-10-22: failed host export must preserve export_emf_selected"
    );

    // 3. Valid in-workdir export succeeds, writes file, closes modal, and clears export_emf_selected.
    let _ = h.click_widget("modal.ok")?;
    assert_eq!(h.app.status_banner(), None);
    assert!(inside_emf.exists(), "valid in-workdir EMF export must write file");
    assert_eq!(h.app.runtime().window(w1)?.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(!h.app.runtime().window(w1)?.app().egui_views.app_modals.export_emf_selected);
    Ok(())
}
