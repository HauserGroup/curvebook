#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Event, Key, Modifiers};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
        Ok(Self(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ShellTestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl ShellTestState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        app.set_human_path_only(true);
        app.nav.welcome.open = false;
        app.egui_views.launcher_open = false;
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), last_intent_error: None }
    }
}

fn make_shell_driver(doc: ProjectDocument, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let app = UiApp::from_session(Session::from_document(doc));
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellTestState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, st| {
            st.views.canvas_view.synchronous_raster = true;
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_intent_error = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn commit_intents(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn set_textbox(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, wid: &str, text: &str) -> Result<(), String> {
    driver.click_widget_id(wid)?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text(text)?;
    Ok(())
}

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

#[test]
fn human_path_import_wizard_handles_messy_paste_quoted_newlines_thousands_and_multirow_headers() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2)?;
    doc.add_data_sheet(DataSheet::new("col-1", "Messy Target", TableArchetype::Column(col))?)?;
    doc.active_sheet_id = Some(SheetId::new("col-1"));

    let mut driver = make_shell_driver(doc, 1280.0, 800.0);
    driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::Clipboard, false, Some(SheetId::new("col-1")));
    commit_intents(&mut driver);

    let messy = concat!("\u{FEFF}Group A\tGroup A\tGroup B\n", "Rep 1\tRep 2\tCtrl\n", "\"1.234,56\"\t\"Line 1\nLine 2\"\t2,5*\n", "3,75\n",);
    driver.click_widget_id("import_paste.input.csv")?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.dispatch_events(vec![Event::Paste(messy.to_owned())]);
    driver.run_settled();

    driver.click_widget_id("import_paste.delim.tab")?;
    driver.click_widget_id("import_paste.decimal.comma")?;
    driver.click_widget_id("import_paste.header_rows.2")?;

    driver.click_widget_id("import_paste.tab.view")?;
    let summary = driver
        .frame_meta()
        .find_by_widget_id("import_paste.preview.summary")
        .and_then(|e| e.value.as_ref().and_then(|v| v.as_str()).map(str::to_owned))
        .ok_or("missing preview summary")?;
    assert!(summary.contains("3 rows × 3 cols"), "unexpected preview summary: {summary}");
    assert!(summary.contains("1 excluded"), "expected 1 excluded asterisk cell in summary: {summary}");

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(!driver.state().app.grid.import_wizard.open);

    let ds = driver.state().app.document().data_sheet(&SheetId::new("col-1")).unwrap();
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)), Some(&Cell::text("Group A - Rep 1")));
    assert_eq!(ds.cell(CellCoord::new(0, 1, 0)), Some(&Cell::text("Group A - Rep 2")));
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)), Some(&Cell::number(1234.56)));
    assert_eq!(ds.cell(CellCoord::new(1, 1, 0)), Some(&Cell::text("Line 1\nLine 2")));
    assert_eq!(ds.cell(CellCoord::new(1, 2, 0)), Some(&Cell { value: curvebook_doc::CellValue::Number(2.5), excluded: true }));
    assert_eq!(ds.cell(CellCoord::new(2, 0, 0)), Some(&Cell::number(3.75)));
    assert_eq!(ds.cell(CellCoord::new(2, 1, 0)), Some(&Cell::empty()));
    Ok(())
}

#[test]
fn human_path_import_wizard_loads_and_selects_multiple_xlsx_worksheets() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new("cb-r17-1-ui-xlsx")?;
    let xlsx_path = dir.0.join("multi.xlsx");

    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Alpha" sheetId="1" r:id="rId1"/>
    <sheet name="Beta" sheetId="2" r:id="rId2"/>
    <sheet name="Gamma" sheetId="3" r:id="rId3"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
</Relationships>"#;
    let mk_sheet = |val: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Signal</t></is></c></row>
  <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>{val}</v></c></row>
</sheetData></worksheet>"#
        )
    };
    let s1 = mk_sheet("10");
    let s2 = mk_sheet("20");
    let s3 = mk_sheet("30");
    fs::write(
        &xlsx_path,
        build_xlsx_zip(&[
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/worksheets/sheet1.xml", &s1),
            ("xl/worksheets/sheet2.xml", &s2),
            ("xl/worksheets/sheet3.xml", &s3),
        ]),
    )?;

    let mut driver = make_shell_driver(ProjectDocument::default(), 1280.0, 800.0);
    driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::Clipboard, false, None);
    commit_intents(&mut driver);

    driver.click_widget_id("import_paste.source.xlsx")?;
    set_textbox(&mut driver, "import_paste.input.path", &xlsx_path.to_string_lossy())?;
    driver.click_widget_id("import_wizard.xlsx.load_path")?;

    assert_eq!(driver.state().app.grid.import_wizard.xlsx_sheets.len(), 3);
    driver.click_widget_id("import_wizard.xlsx.select_none")?;
    driver.click_widget_id("import_wizard.xlsx_sheet.0")?;
    driver.click_widget_id("import_wizard.xlsx_sheet.2")?;

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(!driver.state().app.grid.import_wizard.open);

    let titles: Vec<&str> = driver.state().app.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Gamma"]);
    Ok(())
}
