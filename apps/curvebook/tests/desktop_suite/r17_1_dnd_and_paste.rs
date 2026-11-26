#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, NativeFileDialogAdapter, NativeFileDialogRequest,
    execute_native_import_at_path_with_params,
};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{Cell, CellCoord, CellValue};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::{save_project_file, save_template_file};
use curvebook_ui_egui::StubPlatformClipboardAdapter;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

use crate::common::{NativeHarness, TempDirGuard, build_col_doc, build_xy_doc, build_zip_entries};

#[test]
fn desktop_drag_and_drop_supports_all_project_and_table_formats_and_hover_overlay() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("cb-r17-1-dnd")?;
    let doc = build_xy_doc("dnd-xy", "Project Seed", &[(1.0, 10.0)])?;

    let curvebook_path = tmp.child("sample.curvebook");
    save_project_file(&curvebook_path, &doc)?;
    let template_path = tmp.child("sample.cbtemplate");
    save_template_file(&template_path, &doc)?;

    let prism_path = tmp.child("legacy.prism");
    let doc_json = serde_json::to_string(&doc)?;
    fs::write(&prism_path, build_zip_entries(&[("manifest.json", r#"{"title":"Legacy Imported Project"}"#), ("document.json", &doc_json)])?)?;

    let pzfx_path = tmp.child("legacy.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile Version="2.0" Title="Drop Check"><TableSequence><Table ID="Table0" TableType="XY" XFormat="numbers" Replicates="1"><Title>PZFX Curve</Title><XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn><YColumn><Title>Y</Title><Subcolumn><d>5</d><d>9</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#,
    )?;

    let xlsx_path = tmp.child("multi.xlsx");
    fs::write(
        &xlsx_path,
        build_zip_entries(&[
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="WorkbookTabA" sheetId="1" r:id="rId1"/><sheet name="WorkbookTabB" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Resp</t></is></c></row><row r="2"><c r="A2"><v>0.1</v></c><c r="B2"><v>12.5</v></c></row></sheetData></worksheet>"#,
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Ctrl</t></is></c></row><row r="2"><c r="A2"><v>3.25</v></c></row></sheetData></worksheet>"#,
            ),
        ])?,
    )?;

    let (csv_path, tsv_path) = (tmp.child("points.csv"), tmp.child("eu_points.tsv"));
    fs::write(&csv_path, "Dose,Resp\n1,10\n")?;
    fs::write(&tsv_path, "Dose\tResp\n\"1.234,56\"\t\"2,50\"\n")?;

    for (idx, path) in [&curvebook_path, &template_path, &prism_path, &pzfx_path, &xlsx_path].into_iter().enumerate() {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
        let wid = rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        assert!(rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: path.clone() })?.handled);
        let win = rt.window(wid)?;
        assert!(!win.app().document().data_sheets.is_empty());
        if idx == 0 {
            assert_eq!(win.app().session().file_path().map(std::path::Path::to_path_buf), Some(curvebook_path.clone()));
        } else {
            assert!(win.app().session().file_path().is_none());
        }
        if path == &xlsx_path {
            let titles: Vec<&str> = win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
            assert_eq!(titles, vec!["WorkbookTabA", "WorkbookTabB"]);
        }
    }

    // Hover overlay & drop into CurveBookNativeApp.
    let cfg = DesktopHostConfig { in_window_menus: true, initial_viewport: ViewportMetrics::new(1024.0, 640.0, 1.0), ..DesktopHostConfig::default() };
    let (mut h, w1) = NativeHarness::from_config(cfg, None)?;
    let _ = h.step_raw(egui::RawInput {
        hovered_files: vec![egui::HoveredFile { path: Some(xlsx_path.clone()), mime: String::new() }],
        ..Default::default()
    });
    assert!(h.app.is_drop_hover_active() && h.app.runtime().window(w1)?.hovered_files() == std::slice::from_ref(&xlsx_path));
    let _ = h.step_raw(egui::RawInput {
        hovered_files: vec![egui::HoveredFile { path: None, mime: String::from("application/octet-stream") }],
        ..Default::default()
    });
    assert!(h.app.is_drop_hover_active());
    let _ = h.step_events(Vec::new());
    assert!(!h.app.is_drop_hover_active());

    assert!(h.app.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: xlsx_path })?.handled);
    assert_eq!(
        h.app.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: csv_path })?.command_id.as_deref(),
        Some("table.importCsv")
    );
    assert_eq!(
        h.app.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: tsv_path })?.command_id.as_deref(),
        Some("table.importCsv")
    );
    assert_eq!(h.app.runtime().window(w1)?.app().document().data_sheets.len(), 4);

    let bad_path = tmp.child("notes.json");
    fs::write(&bad_path, "{}")?;
    let err = h.app.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: bad_path }).unwrap_err();
    assert!(err.contains(".xlsx") && err.contains(".tsv"));
    Ok(())
}

#[test]
fn desktop_clipboard_paste_handles_locale_decimal_commas_and_thousands_grouping() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = build_col_doc("col-1", "Paste Sheet", &[("A", &[]), ("B", &[]), ("C", &[])])?;
    doc.active_sheet_id = Some("col-1".into());
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    clip.seed_text("\u{FEFF}1,23\t\"1.234,56\"\n4,56\t\"Line 1\nLine 2\"\n");

    let cfg = DesktopHostConfig {
        clipboard_adapter: Some(clip),
        initial_viewport: ViewportMetrics::new(1024.0, 640.0, 1.0),
        ..DesktopHostConfig::default()
    };
    let (mut h, w1) = NativeHarness::from_config(cfg, Some(Session::from_document(doc)))?;
    h.settle(2);
    let _ = h.click_widget("grid.cell.0.0.0")?;
    let _ = h.key_down(egui::Key::V, egui::Modifiers::COMMAND);

    let ds = h.app.runtime().window(w1)?.app().document().data_sheet(&"col-1".into()).unwrap();
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)), Some(&Cell::number(1.23)));
    assert_eq!(ds.cell(CellCoord::new(0, 1, 0)), Some(&Cell::number(1234.56)));
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)), Some(&Cell::number(4.56)));
    assert_eq!(ds.cell(CellCoord::new(1, 1, 0)), Some(&Cell::text("Line 1\nLine 2")));
    let _ = CellValue::Empty;
    Ok(())
}

#[test]
fn host_clipboard_isolation_xlsx_selection_path_policy_and_multi_file_dnd() -> Result<(), Box<dyn std::error::Error>> {
    // 1. `human_path_only` isolates OS clipboard writes while preserving in-process copy/paste.
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { human_path_only: true, recovery_root: None, ..DesktopHostConfig::default() });
    let wid = rt.open_window(None, ViewportMetrics::default())?;
    rt.window_mut(wid)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "d1", "kind": "Column", "rows": 2, "cols": 1 }))?;
    rt.window_mut(wid)?.execute_and_journal("table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 99.0 }))?;
    let copy = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|v| v.get("available")), Some(&json!(false)));
    assert!(rt.clipboard_text().is_some());
    assert!(rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?.handled);

    // 2. Native import forwards XLSX sheet selection.
    let td = TempDirGuard::new("host-dnd-policy")?;
    let xlsx = td.child("multi.xlsx");
    fs::write(
        &xlsx,
        build_zip_entries(&[
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="First" sheetId="1" r:id="rId1"/><sheet name="Second" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>A</t></is></c></row><row r="2"><c r="A2"><v>1</v></c></row></sheetData></worksheet>"#,
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>B</t></is></c></row><row r="2"><c r="A2"><v>42</v></c></row></sheetData></worksheet>"#,
            ),
        ])?,
    )?;
    let mut rt2 = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let w2 = rt2.open_window(None, ViewportMetrics::default())?;
    let win2 = rt2.window_mut(w2)?;
    execute_native_import_at_path_with_params(win2, "table.importXlsx", &xlsx, false, &json!({ "sheets": ["Second"] }))?;
    assert_eq!((win2.app().document().data_sheets.len(), win2.app().document().data_sheets[0].title.as_str()), (1, "Second"));

    // 3. `ui.export.allTablesCsv`, `FileDropped`, `open_window(Some)`, and `project.open` enforce workdir policy BEFORE fs ops.
    #[derive(Debug)]
    struct FixedPick(PathBuf);
    impl NativeFileDialogAdapter for FixedPick {
        fn pick(&self, _: &NativeFileDialogRequest) -> Result<Option<PathBuf>, String> {
            Ok(Some(self.0.clone()))
        }
    }
    let wd = td.child("wd");
    fs::create_dir_all(&wd)?;
    let (outside_dir, outside_csv) = (td.child("outside_export"), td.child("outside.csv"));
    fs::write(&outside_csv, "X,Y\n1,2\n")?;
    let mut rt_pol = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: None,
        path_policy: ExternalPathPolicy::workdir(&wd)?,
        ..DesktopHostConfig::default()
    });
    let wp = rt_pol.open_window(None, ViewportMetrics::default())?;
    rt_pol.window_mut(wp)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "d1", "kind": "Column" }))?;
    rt_pol.set_file_dialog_adapter(Arc::new(FixedPick(outside_dir.clone())));
    assert!(rt_pol.dispatch_native_file_command(wp, "ui.export.allTablesCsv").is_err() && !outside_dir.exists());
    assert!(rt_pol.dispatch(DesktopHostEvent::FileDropped { window_id: wp, path: outside_csv.clone() }).is_err());
    assert!(rt_pol.open_window(Some(outside_csv.clone()), ViewportMetrics::default()).is_err());
    assert!(rt_pol.dispatch_native_file_command_with_params(wp, "project.open", &json!({ "path": outside_csv.display().to_string() })).is_err());

    // 4. Multi-file DnD targets newly opened project window & aggregates all invalid-file errors in status banner.
    #[derive(Debug)]
    struct TestDrop(PathBuf);
    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            fs::read(&self.0).map_err(|e| e.to_string())
        }
    }
    let proj_path = td.child("dropped_study.curvebook");
    save_project_file(&proj_path, &build_col_doc("orig", "Original", &[("A", &[1.0])])?)?;
    let extra_csv = td.child("extra_table.csv");
    fs::write(&extra_csv, "A,B\n1,2\n3,4\n")?;
    let mut rt_dnd = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, in_window_menus: true, ..DesktopHostConfig::default() });
    let d1 = rt_dnd.open_window(None, ViewportMetrics::default())?;
    rt_dnd.window_mut(d1)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "dirty1", "kind": "Column" }))?;
    assert!(rt_dnd.window(d1)?.has_unsaved_changes());
    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt_dnd, d1, None), egui::vec2(1024.0, 680.0));
    let _ = h.step_raw(egui::RawInput { dropped_files: vec![Arc::new(TestDrop(proj_path)), Arc::new(TestDrop(extra_csv))], ..Default::default() });
    assert_eq!(h.app.runtime().window_count(), 2);
    let d2 = h.app.runtime().focused_window_id().ok_or("missing focused window")?;
    assert!(
        d2 != d1
            && h.app.runtime().window(d1)?.app().document().data_sheets.len() == 1
            && h.app.runtime().window(d2)?.app().document().data_sheets.len() == 2
    );
    let _ = h.step_raw(egui::RawInput {
        dropped_files: vec![Arc::new(TestDrop(td.child("missing_alpha.unknownext"))), Arc::new(TestDrop(td.child("missing_beta.unknownext")))],
        ..Default::default()
    });
    let banner = h.app.status_banner().ok_or("expected aggregated error banner")?;
    assert!(banner.contains("missing_alpha") && banner.contains("missing_beta"), "{banner}");
    Ok(())
}
