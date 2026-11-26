#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;

use curvebook_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_format::zip::ZipWriter;
use curvebook_ui_egui::egui_shell::{Tokens, UiIntent};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Event, Key, Modifiers};

use crate::common::{ShellTestState, commit_shell_intents, make_shell_driver_with_state, set_textbox, temp_dir};

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

    let mut driver = make_shell_driver_with_state(ShellTestState::from_doc_human(doc), ViewportMetrics::new(1280.0, 800.0, 1.0), Tokens::default());
    driver.state_mut().intents.push(UiIntent::OpenImportWizard {
        origin: ImportOriginKind::Clipboard,
        is_live_link: false,
        target_sheet_id: Some(SheetId::new("col-1")),
    });
    commit_shell_intents(&mut driver)?;

    let messy = concat!("\u{FEFF}Group A\tGroup A\tGroup B\n", "Rep 1\tRep 2\tCtrl\n", "\"1.234,56\"\t\"Line 1\nLine 2\"\t2,5*\n", "3,75\n",);
    driver.click_widget_id("import_paste.input.csv")?;
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.dispatch_events(vec![Event::Paste(messy.to_owned())]);
    driver.run_settled();

    for wid in ["import_paste.delim.tab", "import_paste.decimal.comma", "import_paste.header_rows.2", "import_paste.tab.view"] {
        driver.click_widget_id(wid)?;
    }
    let summary = driver
        .frame_meta()
        .find_by_widget_id("import_paste.preview.summary")
        .and_then(|e| e.value.as_ref().and_then(|v| v.as_str()).map(str::to_owned))
        .ok_or("missing preview summary")?;
    assert!(summary.contains("3 rows × 3 cols") && summary.contains("1 excluded"), "unexpected preview summary: {summary}");

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(!driver.state().app.grid.import_wizard.open);

    let ds = driver.state().app.document().data_sheet(&SheetId::new("col-1")).unwrap();
    // With `header_rows == 2` (`propagate_title_header = true`), collapsed multi-row headers are
    // promoted to ColumnTable column titles (`AUD-04-43`) and data begins at row 0.
    let TableArchetype::Column(col_tbl) = &ds.archetype else {
        return Err("expected ColumnTable".into());
    };
    assert_eq!(col_tbl.data_sets.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), vec!["Group A - Rep 1", "Group A - Rep 2", "Group B - Ctrl"]);
    for (r, c, expected) in [
        (0, 0, Cell::number(1234.56)),
        (0, 1, Cell::text("Line 1\nLine 2")),
        (0, 2, Cell { value: curvebook_doc::CellValue::Number(2.5), excluded: true }),
        (1, 0, Cell::number(3.75)),
        (1, 1, Cell::empty()),
    ] {
        assert_eq!(ds.cell(CellCoord::new(r, c, 0)), Some(&expected), "mismatch at ({r},{c})");
    }
    Ok(())
}

#[test]
fn human_path_import_wizard_loads_and_selects_multiple_xlsx_worksheets() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_dir("cb-r17-1-ui-xlsx")?;
    let xlsx_path = dir.path().join("multi.xlsx");
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Alpha" sheetId="1" r:id="rId1"/><sheet name="Beta" sheetId="2" r:id="rId2"/><sheet name="Gamma" sheetId="3" r:id="rId3"/></sheets></workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/></Relationships>"#;
    let mk_sheet = |val: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Signal</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>{val}</v></c></row></sheetData></worksheet>"#
        )
    };
    let (s1, s2, s3) = (mk_sheet("10"), mk_sheet("20"), mk_sheet("30"));
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

    let mut driver = make_shell_driver_with_state(
        ShellTestState::from_doc_human(ProjectDocument::default()),
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        Tokens::default(),
    );
    driver.state_mut().intents.push(UiIntent::OpenImportWizard { origin: ImportOriginKind::Clipboard, is_live_link: false, target_sheet_id: None });
    commit_shell_intents(&mut driver)?;

    driver.click_widget_id("import_paste.source.xlsx")?;
    set_textbox(&mut driver, "import_paste.input.path", &xlsx_path.to_string_lossy())?;
    driver.click_widget_id("import_wizard.xlsx.load_path")?;

    assert_eq!(driver.state().app.grid.import_wizard.xlsx_sheets.len(), 3);
    for wid in ["import_wizard.xlsx.select_none", "import_wizard.xlsx_sheet.0", "import_wizard.xlsx_sheet.2", "modal.ok"] {
        driver.click_widget_id(wid)?;
    }
    assert_eq!(driver.state().last_intent_error, None);
    assert!(!driver.state().app.grid.import_wizard.open);
    let titles: Vec<&str> = driver.state().app.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Gamma"]);
    Ok(())
}
