#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

//! `R17.1` follow-up: dropping a `.prism`, `.pzfx`, or `.xlsx` file onto a desktop window behaves
//! like File › Import for those formats (replace-or-merge prompt on non-empty/dirty projects).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{CellCoord, DataSheet, ProjectDocument};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_testkit::wf_datasets::{ColumnFixture, column_three_group};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

use crate::common::{NativeHarness, TempDirGuard, build_zip_entries};

const BASE_SHEET_ID: &str = "base-assay";
const INCOMING_TITLE: &str = "Incoming Assay";

struct InterchangeFixtures {
    container: PathBuf,
    pzfx: PathBuf,
    xlsx: PathBuf,
}

impl InterchangeFixtures {
    fn all(&self) -> [(&Path, &'static str); 3] {
        [(&self.container, "table.importPrism"), (&self.pzfx, "table.importPzfx"), (&self.xlsx, "table.importXlsx")]
    }
}

fn incoming_fixture() -> ColumnFixture {
    let mut fx = column_three_group();
    fx.title = INCOMING_TITLE;
    fx
}

fn write_interchange_fixtures(dir: &Path) -> Result<InterchangeFixtures, Box<dyn std::error::Error>> {
    let fx = incoming_fixture();
    let out = InterchangeFixtures { container: dir.join("incoming.prism"), pzfx: dir.join("incoming.pzfx"), xlsx: dir.join("incoming.xlsx") };

    // .prism zip container
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(fx.to_data_sheet("incoming-container")?)?;
    let manifest = serde_json::to_string(&json!({ "title": INCOMING_TITLE }))?;
    let doc_json = serde_json::to_string(&doc)?;
    fs::write(&out.container, build_zip_entries(&[("manifest.json", &manifest), ("document.json", &doc_json)])?)?;

    // .pzfx XML
    let mut cols = String::new();
    for (title, values) in fx.group_titles.iter().zip(&fx.columns) {
        let cells: String = values.iter().map(|v| format!("<d>{v}</d>")).collect();
        cols.push_str(&format!("<YColumn><Title>{title}</Title><Subcolumn>{cells}</Subcolumn></YColumn>"));
    }
    fs::write(
        &out.pzfx,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile SchemaVersion="2"><TableSequence><Table ID="Table0" TableType="OneWay"><Title>{}</Title>{cols}</Table></TableSequence></PrismFile>"#,
            fx.title
        ),
    )?;

    // .xlsx zip
    let col_letter = |c: usize| char::from(b'A' + u8::try_from(c).unwrap_or(0));
    let mut rows = String::from(r#"<row r="1">"#);
    for (c, title) in fx.group_titles.iter().enumerate() {
        rows.push_str(&format!(r#"<c r="{}1" t="inlineStr"><is><t>{title}</t></is></c>"#, col_letter(c)));
    }
    rows.push_str("</row>");
    let n_rows = fx.columns.iter().map(Vec::len).max().unwrap_or(0);
    for r in 0..n_rows {
        let er = r + 2;
        rows.push_str(&format!(r#"<row r="{er}">"#));
        for (c, col) in fx.columns.iter().enumerate() {
            if let Some(v) = col.get(r) {
                rows.push_str(&format!(r#"<c r="{}{er}"><v>{v}</v></c>"#, col_letter(c)));
            }
        }
        rows.push_str("</row>");
    }
    let wb = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        fx.title
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    fs::write(
        &out.xlsx,
        build_zip_entries(&[("xl/workbook.xml", &wb), ("xl/_rels/workbook.xml.rels", rels), ("xl/worksheets/sheet1.xml", &sheet)])?,
    )?;
    Ok(out)
}

fn saved_project_runtime(dir: &Path) -> Result<(DesktopHostRuntime, DesktopWindowId, PathBuf), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(column_three_group().to_data_sheet(BASE_SHEET_ID)?)?;
    let saved = dir.join("existing.curvebook");
    save_project_file(&saved, &doc)?;
    let mut session = Session::from_document(doc);
    session.set_file_path(Some(saved.clone()), true);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window_with_session(session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    Ok((rt, wid, saved))
}

fn sheet_titles(rt: &DesktopHostRuntime, wid: DesktopWindowId) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    Ok(rt.window(wid)?.app().document().data_sheets.iter().map(|s| s.title.clone()).collect())
}

fn assert_sheet_matches_fixture(sheet: &DataSheet, fx: &ColumnFixture) {
    for (c, col) in fx.columns.iter().enumerate() {
        for (r, &want) in col.iter().enumerate() {
            assert_eq!(sheet.cell(CellCoord::new(r, c, 0)).and_then(|cell| cell.active_number()), Some(want), "sheet `{}` r={r} c={c}", sheet.title);
        }
    }
}

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

fn drop_file(h: &mut NativeHarness, path: &Path) {
    let handle: egui::DroppedFileHandle = Arc::new(TestDroppedFile(path.to_path_buf()));
    let _ = h.step_raw(egui::RawInput { dropped_files: vec![handle], ..Default::default() });
    h.settle(2);
}

#[test]
fn r17_1_drop_interchange_onto_non_empty_project_opens_import_mode_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r17-1-drop-prompt")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let base_title = column_three_group().title;

    for (path, command_id) in files.all() {
        let (mut rt, wid, saved) = saved_project_runtime(tmp.path())?;
        let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: path.to_path_buf() })?;
        assert!(out.handled && out.action == format!("file.dropped_confirm_import_mode:{command_id}"));
        assert!(out.command_id.as_deref() == Some(command_id) && out.opened_window_id.is_none() && rt.window_count() == 1);

        let win = rt.window(wid)?;
        assert!(matches!(
            &win.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id: cmd, path: p, has_unsaved_changes: false } if cmd == command_id && *p == path.to_string_lossy()
        ));
        assert_eq!(sheet_titles(&rt, wid)?, vec![base_title.to_owned()]);
        assert!(win.app().session().file_path() == Some(saved.as_path()) && !win.has_unsaved_changes());
    }
    Ok(())
}

#[test]
fn r17_1_drop_pzfx_then_cancel_preserves_project_and_undo_history() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r17-1-drop-cancel")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let base_title = column_three_group().title;
    let (mut rt, wid, saved) = saved_project_runtime(tmp.path())?;

    rt.window_mut(wid)?.execute_and_journal("sheet.rename", json!({ "sheet_id": BASE_SHEET_ID, "title": "Edited Before Drop" }))?;
    let undo_before = rt.window(wid)?.app().session().undo_depth();
    assert!(undo_before > 0);

    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(2);
    drop_file(&mut h, &files.pzfx);
    assert!(
        matches!(&h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: true, .. } if command_id == "table.importPzfx")
    );

    let _ = h.click_widget("modal.cancel")?;
    {
        let win = h.app.runtime().window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        assert_eq!(sheet_titles(h.app.runtime(), wid)?, vec![String::from("Edited Before Drop")]);
        assert_eq!(
            (win.app().session().undo_depth(), win.app().session().file_path(), win.has_unsaved_changes()),
            (undo_before, Some(saved.as_path()), true)
        );
        assert!(win.app().import_warnings.warnings.is_empty());
    }
    h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("edit.undo") })?;
    assert_eq!(sheet_titles(h.app.runtime(), wid)?, vec![base_title.to_owned()]);
    Ok(())
}

#[test]
fn r17_1_drop_pzfx_then_merge_adds_imported_sheets_to_existing_project() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r17-1-drop-merge")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let base_title = column_three_group().title;
    let (rt, wid, saved) = saved_project_runtime(tmp.path())?;

    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(2);
    drop_file(&mut h, &files.pzfx);
    assert!(matches!(&h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::ImportProjectMode { .. }));

    let _ = h.click_widget("import_project_mode.merge")?;
    let win = h.app.runtime().window(wid)?;
    assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
    let doc = win.app().document();
    let titles: Vec<&str> = doc.data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert!(titles.len() == 2 && titles.contains(&base_title));
    assert_sheet_matches_fixture(doc.data_sheet(&BASE_SHEET_ID.into()).ok_or("existing sheet lost")?, &column_three_group());
    assert_sheet_matches_fixture(doc.data_sheets.iter().find(|s| s.title == INCOMING_TITLE).ok_or("imported sheet missing")?, &incoming_fixture());
    assert_eq!((win.app().session().file_path(), win.has_unsaved_changes(), h.app.runtime().window_count()), (Some(saved.as_path()), true, 1));
    Ok(())
}

#[test]
fn r17_1_drop_interchange_onto_empty_window_imports_without_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("r17-1-drop-empty")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let fx = incoming_fixture();

    for (path, command_id) in files.all() {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
        let wid = rt.open_window(None, ViewportMetrics::new(1280.0, 800.0, 1.0))?;
        let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: path.to_path_buf() })?;
        assert!(out.action == "file.dropped_loaded_in_window" && rt.window_count() == 1);
        let win = rt.window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        let sheet = win.app().document().data_sheets.first().ok_or("missing imported sheet")?;
        assert_eq!((win.app().document().data_sheets.len(), sheet.title.as_str()), (1, INCOMING_TITLE));
        if command_id != "table.importXlsx" {
            assert_sheet_matches_fixture(sheet, &fx);
        }
        assert!(win.app().session().file_path().is_none());
    }
    Ok(())
}

#[test]
fn s10_1_drop_curvebook_onto_non_empty_project_prompts_replace_merge_or_new_window() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s10-1-drop-curvebook")?;
    let base_title = column_three_group().title;
    let mut other = ProjectDocument::default();
    other.add_data_sheet(incoming_fixture().to_data_sheet("other-assay")?)?;
    let other_path = tmp.child("other.curvebook");
    save_project_file(&other_path, &other)?;

    // 1. Clean non-empty project: dropping `.curvebook` opens ImportProjectMode -> Merge routes to `project.merge` with origin badge.
    {
        let (rt, wid, saved) = saved_project_runtime(tmp.path())?;
        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_file(&mut h, &other_path);
        assert!(matches!(
            &h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: false, .. } if command_id == "project.open"
        ));
        let _ = h.click_widget("import_project_mode.merge")?;
        let win = h.app.runtime().window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        let titles = sheet_titles(h.app.runtime(), wid)?;
        assert_eq!(titles, vec![base_title.to_owned(), INCOMING_TITLE.to_owned()]);
        assert_eq!(win.app().session().file_path(), Some(saved.as_path()));
        assert_eq!(win.app().document().data_sheets[0].origin, None);
        assert_eq!(
            win.app().document().data_sheets[1].origin.as_ref().map(|o| (o.source_label.as_str(), o.source_format.as_str())),
            Some(("other.curvebook", "curvebook"))
        );
    }

    // 2. Replace choice opens the dropped `.curvebook` directly as the active document.
    {
        let (rt, wid, _) = saved_project_runtime(tmp.path())?;
        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_file(&mut h, &other_path);
        let _ = h.click_widget("import_project_mode.replace")?;
        let win = h.app.runtime().window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        assert_eq!(sheet_titles(h.app.runtime(), wid)?, vec![INCOMING_TITLE.to_owned()]);
        assert!(win.app().document().data_sheets.iter().all(|s| s.origin.is_none()));
    }

    // 3. Dirty project: dropping `.curvebook` opens ImportProjectMode with unsaved warning -> Open in new window keeps dirty base intact.
    {
        let (mut rt, wid, _) = saved_project_runtime(tmp.path())?;
        rt.window_mut(wid)?.execute_and_journal("sheet.rename", json!({ "sheet_id": BASE_SHEET_ID, "title": "Dirty Base" }))?;
        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
        h.settle(2);
        drop_file(&mut h, &other_path);
        assert!(matches!(
            &h.app.runtime().window(wid)?.app().canvas_dialogs.active_dialog,
            ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: true, .. } if command_id == "project.open"
        ));
        let _ = h.click_widget("import_project_mode.new_window")?;
        assert_eq!(h.app.runtime().window_count(), 2);
        assert_eq!(sheet_titles(h.app.runtime(), wid)?, vec![String::from("Dirty Base")]);
    }
    Ok(())
}
