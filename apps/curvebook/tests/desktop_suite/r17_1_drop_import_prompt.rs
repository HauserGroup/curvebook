#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

//! `R17.1` follow-up: dropping a `.prism`, `.pzfx`, or `.xlsx` file onto a desktop window behaves
//! like File › Import for those formats. A window whose project has sheets or unsaved changes gets
//! the same replace-or-merge prompt (`ActiveDialog::ImportProjectMode`) as the menu, and the
//! prompt's answer reaches the host through the same `{ "path", "merge" }` command path.
//!
//! These are desktop-host and `egui` frame harness tests. They do not count as human-path GUI
//! evidence (`docs/pitfalls.md` §6).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_doc::{CellCoord, DataSheet, ProjectDocument};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_format::zip::ZipWriter;
use curvebook_testkit::wf_datasets::{ColumnFixture, column_three_group};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

use crate::common::TempDirGuard as TempDir;

const BASE_SHEET_ID: &str = "base-assay";
const INCOMING_TITLE: &str = "Incoming Assay";
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Interchange files built in-test from the clean-room `curvebook-testkit` column fixture.
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

/// One-way `.pzfx` XML table with one `YColumn` per fixture group.
fn pzfx_from_fixture(fx: &ColumnFixture) -> String {
    let mut cols = String::new();
    for (title, values) in fx.group_titles.iter().zip(&fx.columns) {
        let cells: String = values.iter().map(|v| format!("<d>{v}</d>")).collect();
        cols.push_str(&format!("      <YColumn><Title>{title}</Title><Subcolumn>{cells}</Subcolumn></YColumn>\n"));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile SchemaVersion="2">
  <TableSequence>
    <Table ID="Table0" TableType="OneWay">
      <Title>{}</Title>
{cols}    </Table>
  </TableSequence>
</PrismFile>"#,
        fx.title
    )
}

/// Single-worksheet `.xlsx` with a header row of group titles and one numeric row per replicate.
fn xlsx_from_fixture(fx: &ColumnFixture) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let col_letter = |c: usize| char::from(b'A' + u8::try_from(c).unwrap_or(0));
    let mut rows = String::from(r#"<row r="1">"#);
    for (c, title) in fx.group_titles.iter().enumerate() {
        rows.push_str(&format!(r#"<c r="{}1" t="inlineStr"><is><t>{title}</t></is></c>"#, col_letter(c)));
    }
    rows.push_str("</row>");
    let n_rows = fx.columns.iter().map(Vec::len).max().unwrap_or(0);
    for r in 0..n_rows {
        let excel_row = r + 2;
        rows.push_str(&format!(r#"<row r="{excel_row}">"#));
        for (c, col) in fx.columns.iter().enumerate() {
            if let Some(v) = col.get(r) {
                rows.push_str(&format!(r#"<c r="{}{excel_row}"><v>{v}</v></c>"#, col_letter(c)));
            }
        }
        rows.push_str("</row>");
    }
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
        fx.title
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let mut zw = ZipWriter::new();
    zw.add_file("xl/workbook.xml", workbook.into_bytes())?;
    zw.add_file("xl/_rels/workbook.xml.rels", rels.as_bytes().to_vec())?;
    zw.add_file("xl/worksheets/sheet1.xml", sheet.into_bytes())?;
    Ok(zw.finish()?)
}

/// `.prism` container holding a CurveBook `document.json` built from the fixture.
fn zip_container_from_fixture(fx: &ColumnFixture) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(fx.to_data_sheet("incoming-container")?)?;
    let mut zw = ZipWriter::new();
    zw.add_file("manifest.json", serde_json::to_vec(&json!({ "title": INCOMING_TITLE }))?)?;
    zw.add_file("document.json", serde_json::to_vec(&doc)?)?;
    Ok(zw.finish()?)
}

fn write_interchange_fixtures(dir: &Path) -> Result<InterchangeFixtures, Box<dyn std::error::Error>> {
    let fx = incoming_fixture();
    let out = InterchangeFixtures { container: dir.join("incoming.prism"), pzfx: dir.join("incoming.pzfx"), xlsx: dir.join("incoming.xlsx") };
    fs::write(&out.container, zip_container_from_fixture(&fx)?)?;
    fs::write(&out.pzfx, pzfx_from_fixture(&fx))?;
    fs::write(&out.xlsx, xlsx_from_fixture(&fx)?)?;
    Ok(out)
}

fn base_document() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(column_three_group().to_data_sheet(BASE_SHEET_ID)?)?;
    Ok(doc)
}

/// Saved, clean project window bound to `existing.curvebook` with one fixture sheet.
fn saved_project_runtime(dir: &Path) -> Result<(DesktopHostRuntime, DesktopWindowId, PathBuf), Box<dyn std::error::Error>> {
    let doc = base_document()?;
    let saved = dir.join("existing.curvebook");
    save_project_file(&saved, &doc)?;
    let mut session = Session::from_document(doc);
    session.set_file_path(Some(saved.clone()), true);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { new_instance: false, ..DesktopHostConfig::default() });
    let wid = rt.open_window_with_session(session, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    Ok((rt, wid, saved))
}

fn sheet_titles(rt: &DesktopHostRuntime, wid: DesktopWindowId) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    Ok(rt.window(wid)?.app().document().data_sheets.iter().map(|s| s.title.clone()).collect())
}

/// Assert that `sheet` holds the fixture's values exactly (import copies decimal literals verbatim,
/// so equality is exact: tolerance `0`).
fn assert_sheet_matches_fixture(sheet: &DataSheet, fx: &ColumnFixture) {
    for (c, col) in fx.columns.iter().enumerate() {
        for (r, &want) in col.iter().enumerate() {
            let got = sheet.cell(CellCoord::new(r, c, 0)).and_then(|cell| cell.active_number());
            assert_eq!(got, Some(want), "sheet `{}` row {r} col {c}", sheet.title);
        }
    }
}

/// Test-only OS file drop handle fed through `egui::RawInput::dropped_files`.
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

struct Frames {
    ctx: egui::Context,
}

impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));
        Self { ctx }
    }

    fn step(&self, native: &mut CurveBookNativeApp, raw: egui::RawInput) {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), ..raw };
        let mut out = self.ctx.run_ui(raw, |ui| native.step_ui(ui));
        out.textures_delta.clear();
    }

    fn settle(&self, native: &mut CurveBookNativeApp) {
        self.step(native, egui::RawInput::default());
        self.step(native, egui::RawInput::default());
    }

    fn drop_file(&self, native: &mut CurveBookNativeApp, path: &Path) {
        let handle: egui::DroppedFileHandle = Arc::new(TestDroppedFile(path.to_path_buf()));
        self.step(native, egui::RawInput { dropped_files: vec![handle], ..Default::default() });
        self.settle(native);
    }

    fn click_widget(&self, native: &mut CurveBookNativeApp, widget_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let pt = native.inspect_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?.clipped_rect.center();
        self.step(
            native,
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pt),
                    egui::Event::PointerButton { pos: pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                    egui::Event::PointerButton { pos: pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
                ],
                ..Default::default()
            },
        );
        self.settle(native);
        Ok(())
    }
}

#[test]
fn r17_1_drop_interchange_onto_non_empty_project_opens_import_mode_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("r17-1-drop-prompt")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let base_title = column_three_group().title;

    for (path, command_id) in files.all() {
        let (mut rt, wid, saved) = saved_project_runtime(tmp.path())?;
        let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: path.to_path_buf() })?;
        assert!(out.handled);
        assert_eq!(out.action, format!("file.dropped_confirm_import_mode:{command_id}"));
        assert_eq!(out.command_id.as_deref(), Some(command_id));
        assert!(out.opened_window_id.is_none(), "{command_id}: prompt must not open a new window");
        assert_eq!(rt.window_count(), 1);

        let win = rt.window(wid)?;
        assert!(
            matches!(
                &win.app().canvas_dialogs.active_dialog,
                ActiveDialog::ImportProjectMode { command_id: cmd, path: p, has_unsaved_changes: false }
                    if cmd == command_id && *p == path.to_string_lossy()
            ),
            "{command_id}: expected the File › Import replace-or-merge prompt, got {:?}",
            win.app().canvas_dialogs.active_dialog
        );
        assert_eq!(sheet_titles(&rt, wid)?, vec![base_title.to_owned()], "{command_id}: drop must not touch the open project before the answer");
        assert_eq!(win.app().session().file_path(), Some(saved.as_path()));
        assert!(!win.has_unsaved_changes());
    }
    Ok(())
}

#[test]
fn r17_1_drop_pzfx_then_cancel_preserves_project_and_undo_history() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("r17-1-drop-cancel")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let base_title = column_three_group().title;
    let (mut rt, wid, saved) = saved_project_runtime(tmp.path())?;

    // Edit before the drop so the project has unsaved changes and an undo entry.
    rt.window_mut(wid)?.execute_and_journal("sheet.rename", json!({ "sheet_id": BASE_SHEET_ID, "title": "Edited Before Drop" }))?;
    let undo_depth_before = rt.window(wid)?.app().session().undo_depth();
    assert!(undo_depth_before > 0);

    let mut native = CurveBookNativeApp::new(rt, wid, None);
    let frames = Frames::new();
    frames.settle(&mut native);
    frames.drop_file(&mut native, &files.pzfx);
    assert!(matches!(
        &native.runtime().window(wid)?.app().canvas_dialogs.active_dialog,
        ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes: true, .. } if command_id == "table.importPzfx"
    ));

    frames.click_widget(&mut native, "modal.cancel")?;
    {
        let win = native.runtime().window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None), "cancel must close the prompt");
        assert_eq!(sheet_titles(native.runtime(), wid)?, vec![String::from("Edited Before Drop")]);
        assert_eq!(win.app().session().undo_depth(), undo_depth_before, "cancel must keep the undo history");
        assert_eq!(win.app().session().file_path(), Some(saved.as_path()), "cancel must keep the project file binding");
        assert!(win.has_unsaved_changes());
        assert!(win.app().import_warnings.warnings.is_empty());
    }

    // Undo still reverts the pre-drop edit.
    native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("edit.undo") })?;
    assert_eq!(sheet_titles(native.runtime(), wid)?, vec![base_title.to_owned()]);
    Ok(())
}

#[test]
fn r17_1_drop_pzfx_then_merge_adds_imported_sheets_to_existing_project() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("r17-1-drop-merge")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let base_title = column_three_group().title;
    let (rt, wid, saved) = saved_project_runtime(tmp.path())?;

    let mut native = CurveBookNativeApp::new(rt, wid, None);
    let frames = Frames::new();
    frames.settle(&mut native);
    frames.drop_file(&mut native, &files.pzfx);
    assert!(matches!(&native.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::ImportProjectMode { .. }));

    frames.click_widget(&mut native, "import_project_mode.merge")?;
    let win = native.runtime().window(wid)?;
    assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
    let doc = win.app().document();
    let titles: Vec<&str> = doc.data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles.len(), 2, "merge keeps the existing sheet and adds the imported one: {titles:?}");
    assert!(titles.contains(&base_title));
    let base = doc.data_sheet(&BASE_SHEET_ID.into()).ok_or("existing sheet lost on merge")?;
    assert_sheet_matches_fixture(base, &column_three_group());
    let imported = doc.data_sheets.iter().find(|s| s.title == INCOMING_TITLE).ok_or("imported sheet missing after merge")?;
    assert_sheet_matches_fixture(imported, &incoming_fixture());
    assert_eq!(win.app().session().file_path(), Some(saved.as_path()), "merge keeps the project file binding");
    assert!(win.has_unsaved_changes(), "merge is an unsaved edit to the open project");
    assert_eq!(native.runtime().window_count(), 1);
    Ok(())
}

#[test]
fn r17_1_drop_interchange_onto_empty_window_imports_without_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("r17-1-drop-empty")?;
    let files = write_interchange_fixtures(tmp.path())?;
    let fx = incoming_fixture();

    for (path, command_id) in files.all() {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { new_instance: false, ..DesktopHostConfig::default() });
        let wid = rt.open_window(None, ViewportMetrics::new(1280.0, 800.0, 1.0))?;
        let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: path.to_path_buf() })?;
        assert_eq!(out.action, "file.dropped_loaded_in_window", "{command_id}");
        assert_eq!(rt.window_count(), 1);
        let win = rt.window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None), "{command_id}: empty window must not prompt");
        let sheets = &win.app().document().data_sheets;
        assert_eq!(sheets.len(), 1, "{command_id}");
        let sheet = sheets.first().ok_or("missing imported sheet")?;
        assert_eq!(sheet.title, INCOMING_TITLE, "{command_id}");
        // The `.xlsx` importer infers an XY table from a leading numeric column, so only the
        // `.prism` / `.pzfx` one-way tables map cell-for-cell onto the column fixture.
        if command_id != "table.importXlsx" {
            assert_sheet_matches_fixture(sheet, &fx);
        }
        assert!(win.app().session().file_path().is_none(), "{command_id}: interchange import stays detached from any archive");
    }
    Ok(())
}

#[test]
fn r17_1_drop_curvebook_onto_non_empty_project_is_unchanged() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("r17-1-drop-curvebook")?;
    let mut other = ProjectDocument::default();
    other.add_data_sheet(incoming_fixture().to_data_sheet("other-assay")?)?;
    let other_path = tmp.path().join("other.curvebook");
    save_project_file(&other_path, &other)?;

    // Clean, non-empty window: `.curvebook` still loads in place with no prompt.
    let (mut rt, wid, _saved) = saved_project_runtime(tmp.path())?;
    let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: other_path.clone() })?;
    assert_eq!(out.action, "file.dropped_loaded_in_window");
    {
        let win = rt.window(wid)?;
        assert!(matches!(win.app().canvas_dialogs.active_dialog, ActiveDialog::None));
        assert_eq!(sheet_titles(&rt, wid)?, vec![INCOMING_TITLE.to_owned()]);
        assert_eq!(win.app().session().file_path(), Some(other_path.as_path()));
    }

    // Window with unsaved changes: `.curvebook` still opens a new window and leaves the edits alone.
    let (mut rt, wid, _saved) = saved_project_runtime(tmp.path())?;
    rt.window_mut(wid)?.execute_and_journal("sheet.rename", json!({ "sheet_id": BASE_SHEET_ID, "title": "Dirty Base" }))?;
    let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: other_path })?;
    assert_eq!(out.action, "file.dropped_opened_new_window");
    let new_wid = out.opened_window_id.ok_or("dirty `.curvebook` drop must open a new window")?;
    assert!(matches!(rt.window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::None));
    assert_eq!(sheet_titles(&rt, wid)?, vec![String::from("Dirty Base")]);
    assert_eq!(sheet_titles(&rt, new_wid)?, vec![INCOMING_TITLE.to_owned()]);
    Ok(())
}
