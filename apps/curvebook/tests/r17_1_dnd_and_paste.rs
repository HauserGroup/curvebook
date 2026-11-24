#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_doc::{Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use curvebook_format::{save_project_file, save_template_file};
use curvebook_ui_egui::StubPlatformClipboardAdapter;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use eframe::egui;

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

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

#[test]
fn desktop_drag_and_drop_supports_all_project_and_table_formats_and_hover_overlay() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new("cb-r17-1-dnd")?;
    let mut doc = ProjectDocument::default();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    if let Some(col) = xy.data_sets.first_mut() {
        col.title = String::from("Signal");
    }
    let mut ds = DataSheet::new("dnd-xy", "Project Seed", TableArchetype::Xy(xy))?;
    ds.set_x_or_time_cell(0, 0, Cell::number(1.0))?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0))?;
    doc.add_data_sheet(ds)?;

    let curvebook_path = tmp.0.join("sample.curvebook");
    save_project_file(&curvebook_path, &doc)?;

    let template_path = tmp.0.join("sample.cbtemplate");
    save_template_file(&template_path, &doc)?;

    let prism_path = tmp.0.join("legacy.prism");
    let doc_json = serde_json::to_vec(&doc)?;
    let mut pw = ZipWriter::new();
    pw.add_file("manifest.json", br#"{"title":"Legacy Imported Project"}"#.to_vec())?;
    pw.add_file("document.json", doc_json)?;
    fs::write(&prism_path, pw.finish()?)?;

    let pzfx_path = tmp.0.join("legacy.pzfx");
    let pzfx_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile Version="2.0" Title="Drop Check">
  <TableSequence>
    <Table ID="Table0" TableType="XY" XFormat="numbers" Replicates="1">
      <Title>PZFX Curve</Title>
      <XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
      <YColumn><Title>Y</Title><Subcolumn><d>5</d><d>9</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    fs::write(&pzfx_path, pzfx_xml)?;

    let xlsx_path = tmp.0.join("multi.xlsx");
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="WorkbookTabA" sheetId="1" r:id="rId1"/>
    <sheet name="WorkbookTabB" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    let s1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Resp</t></is></c></row>
  <row r="2"><c r="A2"><v>0.1</v></c><c r="B2"><v>12.5</v></c></row>
</sheetData></worksheet>"#;
    let s2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Ctrl</t></is></c></row>
  <row r="2"><c r="A2"><v>3.25</v></c></row>
</sheetData></worksheet>"#;
    fs::write(
        &xlsx_path,
        build_xlsx_zip(&[
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/worksheets/sheet1.xml", s1),
            ("xl/worksheets/sheet2.xml", s2),
        ]),
    )?;

    let csv_path = tmp.0.join("points.csv");
    fs::write(&csv_path, "Dose,Resp\n1,10\n")?;

    let tsv_path = tmp.0.join("eu_points.tsv");
    fs::write(&tsv_path, "Dose\tResp\n\"1.234,56\"\t\"2,50\"\n")?;

    // Drop every supported format onto a clean host window and verify load + detached path for templates/foreign formats.
    for (idx, path) in [&curvebook_path, &template_path, &prism_path, &pzfx_path, &xlsx_path].into_iter().enumerate() {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { new_instance: false, ..DesktopHostConfig::default() });
        let wid = rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        let out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: path.clone() })?;
        assert!(out.handled, "drop failed for {}", path.display());
        let win = rt.window(wid)?;
        assert!(!win.app().document().data_sheets.is_empty(), "empty doc for {}", path.display());
        if idx == 0 {
            assert_eq!(win.app().session().file_path().map(std::path::Path::to_path_buf), Some(curvebook_path.clone()));
        } else {
            assert!(win.app().session().file_path().is_none(), "foreign/template drop must stay detached: {}", path.display());
        }
        if path == &xlsx_path {
            let titles: Vec<&str> = win.app().document().data_sheets.iter().map(|s| s.title.as_str()).collect();
            assert_eq!(titles, vec!["WorkbookTabA", "WorkbookTabB"]);
        }
    }

    // Hover overlay & drop into CurveBookNativeApp (`egui`) step pass
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { new_instance: false, in_window_menus: true, ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
    let mut native = CurveBookNativeApp::new(rt, w1, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));

    // Simulate OS dragging an .xlsx file over the window (plus empty-path OS drag hover before drop)
    let mut raw_hover =
        egui::RawInput { hovered_files: vec![egui::HoveredFile { path: Some(xlsx_path.clone()), mime: String::new() }], ..Default::default() };
    let mut out1 = ctx.run_ui(raw_hover.take(), |ui| {
        native.step_ui(ui);
    });
    out1.textures_delta.clear();
    assert!(native.is_drop_hover_active());
    assert_eq!(native.runtime().window(w1)?.hovered_files(), std::slice::from_ref(&xlsx_path));

    let mut raw_os_hover = egui::RawInput {
        hovered_files: vec![egui::HoveredFile { path: None, mime: String::from("application/octet-stream") }],
        ..Default::default()
    };
    let mut out2 = ctx.run_ui(raw_os_hover.take(), |ui| {
        native.step_ui(ui);
    });
    out2.textures_delta.clear();
    assert!(native.is_drop_hover_active());

    let mut out3 = ctx.run_ui(egui::RawInput::default(), |ui| {
        native.step_ui(ui);
    });
    out3.textures_delta.clear();
    assert!(!native.is_drop_hover_active());

    // Drop .xlsx, then .csv and .tsv into window
    let drop_xlsx = native.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: xlsx_path })?;
    assert!(drop_xlsx.handled);
    let drop_csv = native.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: csv_path })?;
    assert_eq!(drop_csv.command_id.as_deref(), Some("table.importCsv"));
    let drop_tsv = native.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: tsv_path })?;
    assert_eq!(drop_tsv.command_id.as_deref(), Some("table.importCsv"));
    assert_eq!(native.runtime().window(w1)?.app().document().data_sheets.len(), 4);

    // Unsupported extension surfaces readable error without crashing
    let bad_path = tmp.0.join("notes.json");
    fs::write(&bad_path, "{}")?;
    let err = native.runtime_mut().dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: bad_path }).unwrap_err();
    assert!(err.contains(".xlsx") && err.contains(".tsv"), "unexpected unsupported message: {err}");
    Ok(())
}

#[test]
fn desktop_clipboard_paste_handles_locale_decimal_commas_and_thousands_grouping() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2)?;
    doc.add_data_sheet(DataSheet::new("col-1", "Paste Sheet", TableArchetype::Column(col))?)?;
    doc.active_sheet_id = Some("col-1".into());

    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    clip.seed_text("\u{FEFF}1,23\t\"1.234,56\"\n4,56\t\"Line 1\nLine 2\"\n");
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { clipboard_adapter: Some(clip), ..DesktopHostConfig::default() });
    let w1 = rt.open_window_with_session(Session::from_document(doc), ViewportMetrics::new(1024.0, 640.0, 1.0), false)?;

    let out = rt.dispatch(DesktopHostEvent::KeyboardInput {
        window_id: w1,
        key: String::from("v"),
        pressed: true,
        modifiers: KeyModifiers { meta: true, ..Default::default() },
        text: None,
    })?;
    assert_eq!(out.command_id.as_deref(), Some("table.paste"));

    let ds = rt.window(w1)?.app().document().data_sheet(&"col-1".into()).unwrap();
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)), Some(&Cell::number(1.23)));
    assert_eq!(ds.cell(CellCoord::new(0, 1, 0)), Some(&Cell::number(1234.56)));
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)), Some(&Cell::number(4.56)));
    assert_eq!(ds.cell(CellCoord::new(1, 1, 0)), Some(&Cell::text("Line 1\nLine 2")));
    let _ = CellValue::Empty;
    Ok(())
}
