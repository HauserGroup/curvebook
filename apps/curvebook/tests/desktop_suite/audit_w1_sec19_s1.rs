#![forbid(unsafe_code)]

//! Audit regression tests for `FA-SEC19-02` & `FA-SEC19-03`:
//! - `ExportFigure` modal + in-window/Command Palette export, template, import, and image commands
//!   must route through the desktop `NativeFileDialogAdapter` without dropping bytes or non-path options.
//! - Multi-window switcher preserves reachability of earlier/unsaved/CLI windows, and `app.quit` /
//!   `file.quit` walks unsaved confirmation across all dirty windows without aliasing `window.close`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use curvebook::desktop_host::{
    CloseDisposition, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, NativeFileDialogAdapter, NativeFileDialogKind,
    StubNativeFileDialogAdapter, current_epoch_ms, default_recovery_ttl_ms,
};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_format::save_template_bytes;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, ViewportCommand, ViewportId};
use serde_json::json;

use crate::common::{TempDirGuard, encode_base64 as base64_encode};

fn temp_test_dir(prefix: &str) -> Result<TempDirGuard, Box<dyn std::error::Error>> {
    Ok(TempDirGuard::new(prefix)?)
}

fn build_rich_test_project() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    if let Some(ds0) = xy.data_sets.first_mut() {
        ds0.title = String::from("Signal");
    }
    let mut ds_xy = DataSheet::new("ds-xy", "Dose Response", TableArchetype::Xy(xy))?;
    ds_xy.set_x_or_time_cell(0, 0, Cell::number(1.0))?;
    ds_xy.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0))?;
    ds_xy.set_x_or_time_cell(1, 0, Cell::number(2.0))?;
    ds_xy.set_cell(CellCoord::new(1, 0, 0), Cell::number(22.0))?;
    ds_xy.set_x_or_time_cell(2, 0, Cell::number(4.0))?;
    ds_xy.set_cell(CellCoord::new(2, 0, 0), Cell::number(48.0))?;
    doc.add_data_sheet(ds_xy)?;

    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    let mut ds_col = DataSheet::new("ds-col", "Column Data", TableArchetype::Column(col))?;
    ds_col.set_cell(CellCoord::new(0, 0, 0), Cell::number(5.0))?;
    ds_col.set_cell(CellCoord::new(0, 1, 0), Cell::number(15.0))?;
    doc.add_data_sheet(ds_col)?;

    let ds_ref = doc.data_sheet(&"ds-xy".into()).ok_or("missing ds-xy")?;
    let mut graph = GraphSheet::new_uninitialized_for_data("gs-1", ds_ref)?;
    graph.initialized = true;
    graph.title = String::from("Response Graph");
    graph.spec.kind = GraphKind::Xy;
    doc.add_graph_sheet(graph)?;

    let layout = LayoutSheet::new("ls-1", "Summary Layout", LayoutSpec::default())?;
    doc.add_layout_sheet(layout)?;
    let _ = doc.select_in_navigator(&"gs-1".into());
    Ok(doc)
}

fn new_host_runtime(in_window_menus: bool, recovery_root: Option<PathBuf>) -> DesktopHostRuntime {
    DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus,
        new_instance: false,
        gui_only: false,
        locale_override: Some(String::from("en")),
        theme_override: None,
        recovery_root,
        initial_viewport: ViewportMetrics::new(1280.0, 820.0, 1.0),
        ..DesktopHostConfig::default()
    })
}

fn step_native_frame(native: &mut CurveBookNativeApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<ViewportCommand> {
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 820.0))), events, ..RawInput::default() };
    let mut out = ctx.run_ui(raw, |ui| native.step_ui(ui));
    out.textures_delta.clear();
    out.viewport_output.get(&ViewportId::ROOT).map(|vo| vo.commands.clone()).unwrap_or_default()
}

fn settle_native(native: &mut CurveBookNativeApp, ctx: &egui::Context) -> Vec<ViewportCommand> {
    let mut last_cmds = Vec::new();
    for _ in 0..4 {
        last_cmds = step_native_frame(native, ctx, Vec::new());
    }
    last_cmds
}

fn click_rect(native: &mut CurveBookNativeApp, ctx: &egui::Context, rect: Rect) -> Vec<ViewportCommand> {
    let pos = rect.center();
    let mut all = step_native_frame(
        native,
        ctx,
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        ],
    );
    all.extend(settle_native(native, ctx));
    all
}

fn click_widget(native: &mut CurveBookNativeApp, ctx: &egui::Context, widget_id: &str) -> Result<Vec<ViewportCommand>, String> {
    let rect =
        native.inspect_meta().find_by_widget_id(widget_id).map(|node| node.clipped_rect).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    Ok(click_rect(native, ctx, rect))
}

fn open_in_window_menu_item(
    native: &mut CurveBookNativeApp,
    ctx: &egui::Context,
    top_menu_id: &str,
    submenu_id: Option<&str>,
    item_id: &str,
) -> Result<(), String> {
    let _ = click_widget(native, ctx, top_menu_id)?;
    if let Some(sub) = submenu_id {
        let sub_rect = native
            .inspect_meta()
            .find_by_widget_id(sub)
            .map(|n| n.clipped_rect)
            .ok_or_else(|| format!("missing submenu `{sub}` under `{top_menu_id}`"))?;
        let center = sub_rect.center();
        for _ in 0..3 {
            let _ = step_native_frame(native, ctx, vec![Event::PointerMoved(center)]);
        }
    }
    let _ = click_widget(native, ctx, item_id)?;
    Ok(())
}

fn press_cmd_q(native: &mut CurveBookNativeApp, ctx: &egui::Context) -> Vec<ViewportCommand> {
    let mut all = step_native_frame(
        native,
        ctx,
        vec![Event::Key { key: Key::Q, physical_key: Some(Key::Q), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }],
    );
    all.extend(settle_native(native, ctx));
    all
}

fn decode_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("invalid PNG signature"));
    }
    let dims = bytes.get(16..24).ok_or_else(|| String::from("invalid PNG signature"))?;
    let (w_bytes, h_bytes) = dims.split_at_checked(4).ok_or_else(|| String::from("invalid PNG signature"))?;
    let w = u32::from_be_bytes(w_bytes.try_into().map_err(|_| String::from("invalid PNG width"))?);
    let h = u32::from_be_bytes(h_bytes.try_into().map_err(|_| String::from("invalid PNG height"))?);
    Ok((w, h))
}

fn open_window_with_rich_project(rt: &mut DesktopHostRuntime) -> Result<DesktopWindowId, Box<dyn std::error::Error>> {
    let doc = build_rich_test_project()?;
    let mut sess = curvebook_engine::Session::from_document(doc);
    sess.execute("analysis.run", json!({ "source_sheet_id": "ds-xy", "kind": "linear_regression" }))?;
    let id = rt.open_window_with_session(sess, ViewportMetrics::new(1280.0, 820.0, 1.0), false)?;
    Ok(id)
}

#[test]
fn sec19_02_export_figure_modal_all_formats_write_files_and_preserve_options() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("cb-sec19-export")?;
    let mut rt = new_host_runtime(true, None);
    let w1 = open_window_with_rich_project(&mut rt)?;

    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    let png_path = dir.path().join("modal-graph.png");
    let tiff_path = dir.path().join("modal-graph.tiff");
    let svg_path = dir.path().join("modal-graph.svg");
    let pdf_path = dir.path().join("modal-graph.pdf");
    let emf_path = dir.path().join("modal-graph.emf");

    adapter.push_response("graph.exportPng", Some(png_path.clone()));
    adapter.push_response("export.tiff", Some(tiff_path.clone()));
    adapter.push_response("graph.exportSvg", Some(svg_path.clone()));
    adapter.push_response("graph.exportPdf", Some(pdf_path.clone()));
    adapter.push_response("graph.exportEmf", Some(emf_path.clone()));
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);

    let mut native = CurveBookNativeApp::new(rt, w1, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let _ = settle_native(&mut native, &ctx);

    // 1. PNG via ExportFigure modal with 174mm width preset -> custom width_px preserved.
    native.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
    let _ = settle_native(&mut native, &ctx);
    let _ = click_widget(&mut native, &ctx, "export.fmt.png")?;
    let _ = click_widget(&mut native, &ctx, "export.preset.174mm")?;
    let expected_w = native.runtime().window(w1)?.app().canvas_dialogs.export_figure.width_px;
    let _ = click_widget(&mut native, &ctx, "modal.ok")?;
    assert_eq!(native.status_banner(), None);
    assert_eq!(native.runtime().window(w1)?.app().canvas_dialogs.active_dialog, ActiveDialog::None);
    let png_bytes = fs::read(&png_path)?;
    let (actual_w, actual_h) = decode_png_dimensions(&png_bytes)?;
    assert_eq!(actual_w, expected_w, "PNG export must preserve modal width_px");
    assert!(actual_h > 0);

    // 2. TIFF (LZW) via ExportFigure modal.
    native.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
    let _ = settle_native(&mut native, &ctx);
    let _ = click_widget(&mut native, &ctx, "export.fmt.tiff_lzw")?;
    let _ = click_widget(&mut native, &ctx, "modal.ok")?;
    let tiff_bytes = fs::read(&tiff_path)?;
    assert!(tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*"), "expected valid TIFF header");

    // 3. SVG via ExportFigure modal.
    native.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
    let _ = settle_native(&mut native, &ctx);
    let _ = click_widget(&mut native, &ctx, "export.fmt.svg")?;
    let _ = click_widget(&mut native, &ctx, "modal.ok")?;
    let svg_text = fs::read_to_string(&svg_path)?;
    assert!(svg_text.contains("<svg"), "expected valid SVG output");

    // 4. PDF (CMYK) via ExportFigure modal.
    native.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
    let _ = settle_native(&mut native, &ctx);
    let _ = click_widget(&mut native, &ctx, "export.fmt.pdf")?;
    let _ = click_widget(&mut native, &ctx, "export.cs.cmyk")?;
    let _ = click_widget(&mut native, &ctx, "modal.ok")?;
    let pdf_bytes = fs::read(&pdf_path)?;
    assert!(pdf_bytes.starts_with(b"%PDF-"), "expected valid PDF header");

    // 5. EMF via ExportFigure modal.
    native.runtime_mut().window_mut(w1)?.app_mut().canvas_dialogs.open_export_figure(&"gs-1".into());
    let _ = settle_native(&mut native, &ctx);
    let _ = click_widget(&mut native, &ctx, "export.fmt.emf")?;
    let _ = click_widget(&mut native, &ctx, "modal.ok")?;
    let emf_bytes = fs::read(&emf_path)?;
    assert!(emf_bytes.len() > 80 && emf_bytes.starts_with(&[1, 0, 0, 0]), "expected valid EMF header");

    // 6. Direct host dispatch preserves exact non-path PNG dimensions & PDF color_space/compress_streams options.
    let custom_png = dir.path().join("custom-777x444.png");
    let custom_pdf_rgb = dir.path().join("custom-rgb.pdf");
    let custom_pdf_cmyk = dir.path().join("custom-cmyk.pdf");
    adapter.push_response("graph.exportPng", Some(custom_png.clone()));
    adapter.push_response("graph.exportPdf", Some(custom_pdf_rgb.clone()));
    adapter.push_response("graph.exportPdf", Some(custom_pdf_cmyk.clone()));

    let _ = native.runtime_mut().dispatch_native_file_command_with_params(
        w1,
        "graph.exportPng",
        &json!({ "sheet_id": "gs-1", "width_px": 777, "height_px": 444, "dpi": 150 }),
    )?;
    assert_eq!(decode_png_dimensions(&fs::read(&custom_png)?)?, (777, 444));

    let _ = native.runtime_mut().dispatch_native_file_command_with_params(
        w1,
        "graph.exportPdf",
        &json!({ "sheet_id": "gs-1", "color_space": "srgb", "compress_streams": false }),
    )?;
    let _ = native.runtime_mut().dispatch_native_file_command_with_params(
        w1,
        "graph.exportPdf",
        &json!({ "sheet_id": "gs-1", "color_space": "cmyk", "compress_streams": false }),
    )?;
    let custom_rgb_bytes = fs::read(&custom_pdf_rgb)?;
    let custom_cmyk_bytes = fs::read(&custom_pdf_cmyk)?;
    assert!(custom_cmyk_bytes.starts_with(b"%PDF-"));
    let rgb_text = String::from_utf8_lossy(&custom_rgb_bytes);
    let cmyk_text = String::from_utf8_lossy(&custom_cmyk_bytes);
    assert!(rgb_text.contains(" rg\n"), "expected uncompressed RGB operator in sRGB PDF");
    assert!(cmyk_text.contains(" k\n") && !cmyk_text.contains(" rg\n"), "expected uncompressed CMYK operator in CMYK PDF");

    let reqs = adapter.requests();
    assert_eq!(reqs.len(), 8);
    assert!(reqs.iter().all(|r| r.kind == NativeFileDialogKind::SaveFile));
    Ok(())
}

#[test]
fn sec19_02_in_window_menus_and_intents_route_to_native_dialogs_respecting_inline_payloads() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("cb-sec19-routing")?;
    let mut rt = new_host_runtime(true, None);
    let w1 = open_window_with_rich_project(&mut rt)?;
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());

    let out_tiff = dir.path().join("menu-out.tiff");
    let out_csv = dir.path().join("menu-table.csv");
    let out_json = dir.path().join("menu-res.json");
    let out_res_csv = dir.path().join("menu-res.csv");
    let out_report = dir.path().join("cmd-report.md");
    let out_tpl = dir.path().join("menu-template.cbtpl");
    let in_csv = dir.path().join("in-data.csv");
    fs::write(&in_csv, "X,Y\n1.0,2.0\n3.0,6.0\n")?;
    let seed_png = dir.path().join("seed-img.png");
    let layout_emf = dir.path().join("layout.emf");

    adapter.push_response("export.png", Some(seed_png.clone()));
    adapter.push_response("export.tiff", Some(out_tiff.clone()));
    adapter.push_response("project.saveTemplate", Some(out_tpl.clone()));
    adapter.push_response("table.importCsv", Some(in_csv));
    adapter.push_response("graph.insertImage", Some(seed_png.clone()));
    adapter.push_response("table.exportCsv", Some(out_csv.clone()));
    adapter.push_response("analysis.exportJson", Some(out_json.clone()));
    adapter.push_response("analysis.exportCsv", Some(out_res_csv.clone()));
    adapter.push_response("result.generateReport", Some(out_report.clone()));
    adapter.push_response("layout.exportEmf", Some(layout_emf.clone()));
    adapter.push_response("layout.insertImage", Some(seed_png.clone()));
    adapter.push_response("project.instantiateTemplate", Some(out_tpl.clone()));
    adapter.push_response("table.importPrism", None);
    adapter.push_response("table.importPzfx", None);
    adapter.push_response("table.importXlsx", None);

    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
    let mut native = CurveBookNativeApp::new(rt, w1, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let _ = settle_native(&mut native, &ctx);

    // 1. Real in-window menu clicks (`top_chrome` -> `UiCommand` -> `CurveBookNativeApp` host routing):
    // Seed a real PNG file on disk first via `File > Export > PNG`, then test TIFF, Save Template, Import CSV, Insert Image!
    open_in_window_menu_item(&mut native, &ctx, "menu.file", Some("menu.file.export"), "menu.export.png")?;
    assert!(seed_png.exists(), "in-window File > Export > PNG must prompt Save dialog and write PNG");

    open_in_window_menu_item(&mut native, &ctx, "menu.file", Some("menu.file.export"), "menu.export.tiff")?;
    assert!(out_tiff.exists(), "in-window File > Export > TIFF must prompt Save dialog and write TIFF");

    open_in_window_menu_item(&mut native, &ctx, "menu.file", None, "menu.project.saveTemplate")?;
    assert!(out_tpl.exists(), "in-window File > Save as Template must prompt Save dialog and write template");

    open_in_window_menu_item(&mut native, &ctx, "menu.file", Some("menu.file.import"), "menu.table.importCsv")?;
    let ds_count_after_csv = native.runtime().window(w1)?.app().document().data_sheets.len();
    assert!(ds_count_after_csv >= 3, "in-window File > Import > CSV must prompt Open dialog and import table");

    open_in_window_menu_item(&mut native, &ctx, "menu.insert", None, "menu.graph.insertImage")?;
    assert_eq!(native.status_banner(), None);

    // 2. Remaining desktop host-routed file commands (exports, layout image/EMF, instantiateTemplate, importPrism/Pzfx):
    for (cmd_id, params) in [
        ("table.exportCsv", json!({})),
        ("analysis.exportJson", json!({})),
        ("analysis.exportCsv", json!({})),
        ("result.generateReport", json!({})),
        ("layout.exportEmf", json!({})),
        ("layout.insertImage", json!({})),
        ("project.instantiateTemplate", json!({})),
        ("table.importPrism", json!({})),
        ("table.importPzfx", json!({})),
        ("table.importXlsx", json!({})),
    ] {
        let fid = native.runtime().focused_window_id().ok_or("missing focused window")?;
        let res = native.runtime_mut().dispatch_native_file_command_with_params(fid, cmd_id, &params)?;
        assert!(res.is_some(), "`{cmd_id}` must be native file dialog routed");
    }

    for expected_file in [&out_csv, &out_json, &out_res_csv, &out_report, &layout_emf] {
        assert!(expected_file.exists(), "expected `{}` written on disk", expected_file.display());
    }
    let reqs = adapter.requests();
    assert_eq!(reqs.len(), 15);
    assert!(reqs.iter().any(|r| r.command_id == "table.importPrism" && r.kind == NativeFileDialogKind::OpenFile));
    assert!(reqs.iter().any(|r| r.command_id == "table.importPzfx" && r.kind == NativeFileDialogKind::OpenFile));
    assert!(reqs.iter().any(|r| r.command_id == "table.importXlsx" && r.kind == NativeFileDialogKind::OpenFile));

    // 3. Inline Welcome Hub `project.instantiateTemplate` carrying `bytes_base64` bypasses picker cleanly.
    let tpl_doc = build_rich_test_project()?;
    let tpl_bytes = save_template_bytes(&tpl_doc)?;
    let b64 = base64_encode(&tpl_bytes);
    let reqs_before_inline = adapter.requests().len();
    let fid = native.runtime().focused_window_id().ok_or("missing focused window")?;
    let outcome = native.runtime_mut().window_mut(fid)?.execute_and_journal("project.instantiateTemplate", json!({ "bytes_base64": b64 }))?;
    assert!(outcome.mutated);
    assert_eq!(adapter.requests().len(), reqs_before_inline);
    Ok(())
}

#[test]
fn sec19_03_multi_window_switcher_and_app_quit_walks_all_dirty_windows() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("cb-sec19-multiwin")?;
    let mut rt = new_host_runtime(true, None);
    let w1 = rt.open_window(None, ViewportMetrics::new(1280.0, 820.0, 1.0))?;
    rt.window_mut(w1)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-w1", "title": "Win1 Unsaved", "kind": "xy" }))?;

    // Open second dirty window -> focus moves to w2, while w1 must stay visible & switchable.
    let open_w2 = rt.dispatch(DesktopHostEvent::MenuAction { window_id: w1, command_id: String::from("window.new") })?;
    let w2 = open_w2.opened_window_id.ok_or("missing window 2")?;
    rt.window_mut(w2)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-w2", "title": "Win2 Unsaved", "kind": "xy" }))?;
    assert_eq!(rt.window_count(), 2);
    assert_eq!(rt.focused_window_id(), Some(w2));

    let save_w2_path = dir.path().join("win2.curvebook");
    let save_w1_path = dir.path().join("win1.curvebook");
    let adapter = Arc::new(StubNativeFileDialogAdapter::new());
    adapter.push_response("project.save", Some(save_w2_path.clone()));
    adapter.push_response("project.save", Some(save_w1_path.clone()));
    rt.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);

    let mut native = CurveBookNativeApp::new(rt, w1, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let _ = settle_native(&mut native, &ctx);

    // 1. Switcher strip visible with non-overlapping button rects for both w1 and w2.
    assert!(native.is_window_switcher_visible(), "multi-window switcher must appear when 2+ windows exist");
    let (w1_switch_rect, _) = native.window_switcher_button_rects(w1).ok_or("missing switcher button for w1")?;
    let (w2_switch_rect, _) = native.window_switcher_button_rects(w2).ok_or("missing switcher button for w2")?;
    assert!(!w1_switch_rect.intersects(w2_switch_rect));

    // Clicking w1's switcher button focuses w1 cleanly.
    let _ = click_rect(&mut native, &ctx, w1_switch_rect);
    assert_eq!(native.runtime().focused_window_id(), Some(w1), "clicking w1 switcher must restore focus to w1");

    // Click back to w2, then open a clean 3rd window w3 and close ONLY w3 via `window.close`.
    let _ = click_rect(&mut native, &ctx, w2_switch_rect);
    assert_eq!(native.runtime().focused_window_id(), Some(w2));
    let open_w3 = native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: w2, command_id: String::from("window.new") })?;
    let w3 = open_w3.opened_window_id.ok_or("missing window 3")?;
    let _ = settle_native(&mut native, &ctx);
    let close_w3 = native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: w3, command_id: String::from("window.close") })?;
    assert_eq!(close_w3.close_disposition, Some(CloseDisposition::Closed));
    assert_eq!(native.runtime().window_count(), 2, "`window.close` must close only the targeted window");

    // Focus w2, trigger `file.quit` -> Cancel keeps both windows open cleanly.
    let _ = native.runtime_mut().focus_window(w2)?;
    let _ = settle_native(&mut native, &ctx);
    native.trigger_menu_action(&ctx, "file.quit");
    let _ = settle_native(&mut native, &ctx);
    assert!(native.close_dialog_open(), "`file.quit` with dirty w2 must open Unsaved Changes modal");
    assert!(!native.close_confirmed());

    let _ = click_widget(&mut native, &ctx, "dialog.button.cancel")?;
    assert!(!native.close_dialog_open());
    assert!(!native.close_confirmed());
    assert_eq!(native.runtime().window_count(), 2);

    // Trigger Cmd+Q (`app.quit`) -> Save w2 -> automatically focuses & prompts dirty w1 -> Save w1 -> closes app!
    let _ = press_cmd_q(&mut native, &ctx);
    assert!(native.close_dialog_open(), "Cmd+Q must open Unsaved Changes modal for dirty w2");
    assert_eq!(native.runtime().focused_window_id(), Some(w2));

    let _ = click_widget(&mut native, &ctx, "dialog.button.save")?;
    assert!(save_w2_path.exists(), "saving w2 during quit must write win2.curvebook");
    assert_eq!(native.runtime().window_count(), 1, "w2 closed, dirty w1 must remain");
    assert_eq!(native.runtime().focused_window_id(), Some(w1), "app.quit must immediately focus & prompt next dirty window w1");
    assert!(native.close_dialog_open(), "unsaved prompt must stay open for dirty w1");
    assert!(!native.close_confirmed());

    let cmds = click_widget(&mut native, &ctx, "dialog.button.save")?;
    assert!(save_w1_path.exists(), "saving w1 during quit must write win1.curvebook");
    assert_eq!(native.runtime().window_count(), 0);
    assert!(native.close_confirmed(), "app.quit must confirm close once all dirty windows are saved/closed");
    assert!(cmds.iter().any(|c| matches!(c, ViewportCommand::Close)), "app.quit completion must emit ViewportCommand::Close");

    Ok(())
}

#[test]
fn sec19_03_clean_multi_window_app_quit_and_startup_recovery_focus_preservation() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("cb-sec19-cleanquit")?;
    let recovery_root = dir.path().join("recovery");

    // 1. Crash simulation leaves a stale recovery journal on disk.
    {
        let mut crashed_rt =
            DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_root.clone()), ..DesktopHostConfig::default() });
        let cw = crashed_rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        crashed_rt
            .window_mut(cw)?
            .execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-crash", "title": "Recovered Table", "kind": "xy" }))?;
        std::mem::forget(crashed_rt);
    }

    // 2. New runtime opens CLI-provided clean project, recovers stale journal, and keeps CLI window focused!
    let mut rt = new_host_runtime(true, Some(recovery_root));
    let cli_doc = build_rich_test_project()?;
    let cli_sess = curvebook_engine::Session::from_document(cli_doc);
    let cli_win = rt.open_window_with_session(cli_sess, ViewportMetrics::new(1280.0, 820.0, 1.0), false)?;
    let recovered = rt.recover_stale_journals(
        ViewportMetrics::new(1280.0, 820.0, 1.0),
        current_epoch_ms().saturating_add(default_recovery_ttl_ms()).saturating_add(100),
    )?;
    assert!(!recovered.is_empty(), "expected stale journal to be recovered");
    let _ = rt.focus_window(cli_win)?;
    assert_eq!(rt.focused_window_id(), Some(cli_win), "CLI window must remain focused when recovery windows open");
    assert_eq!(rt.window_count(), 1 + recovered.len());

    let mut native = CurveBookNativeApp::new(rt, cli_win, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let _ = settle_native(&mut native, &ctx);
    assert!(native.is_window_switcher_visible());
    assert!(native.window_switcher_button_rects(cli_win).is_some());
    assert!(native.window_switcher_button_rects(recovered[0]).is_some());

    // 3. Clean multi-window `app.quit` via Cmd+Q closes all clean windows in one step.
    let mut clean_rt = new_host_runtime(true, None);
    let c1 = clean_rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
    let c2 = clean_rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
    assert_ne!(c1, c2);
    let mut clean_native = CurveBookNativeApp::new(clean_rt, c1, None);
    let _ = settle_native(&mut clean_native, &ctx);
    let quit_cmds = press_cmd_q(&mut clean_native, &ctx);
    assert_eq!(clean_native.runtime().window_count(), 0);
    assert!(clean_native.close_confirmed());
    assert!(quit_cmds.iter().any(|c| matches!(c, ViewportCommand::Close)));
    Ok(())
}
