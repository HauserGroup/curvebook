#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R10.30` automated human-path import/export verification suite.
//!
//! Exercises `.pzfx`, `.prism`, and `.xlsx` imports (including `R10.29` structured warnings,
//! `ui.inspect` exposure, status-footer & menu reopen, replace-or-merge confirmation
//! on dirty projects via **File › Import**, **File › Open…**, and the Command Palette, and
//! post-import live edits) alongside one-way ANOVA table and results exports (`CSV Table`
//! `,`/`.` + `.tsv` + `de` locale default `;`/`,`, `CSV Results` all-tabs tidy / single-tab
//! `;`/`,` / `.tsv`, `JSON`, Markdown / Plain / LaTeX Methods Reports including via the
//! Command Palette) and **Export All Data Tables…**, comparing every imported table and every
//! exported file against a fresh headless `curvebook_engine::Session`.

use std::fs;
use std::path::Path;

use crate::human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use crate::human_wf_common::{TestRes, ensure};
use curvebook_doc::{AnalysisKind, CellCoord, CellValue, DataSheet, OneWayAnovaSpec};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, CsvExportTargetKind};
use curvebook_ui_egui::inspect_ui_tree;
use curvebook_ui_egui::os_integration::NativeFileDialogKind;
use serde_json::json;

use crate::common::temp_dir;

/// Build a self-authored `.pzfx` XML fixture with one Column table and an unsupported `<Graph>`
/// section so `ImportWarning`s are deterministically emitted without external-app files.
fn build_pzfx_fixture_bytes() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile PrismXMLVersion="5.00">
  <Table ID="Table0" XFormat="none" TableType="OneWay" EVFormat="AsteriskAfterNumber">
    <Title>Drug Assay</Title>
    <YColumn Width="81" Decimals="1" Subcolumns="1">
      <Title>Control</Title>
      <Subcolumn>
        <d>10.5</d>
        <d>12.0</d>
        <d>11.5</d>
      </Subcolumn>
    </YColumn>
    <YColumn Width="81" Decimals="1" Subcolumns="1">
      <Title>Treated</Title>
      <Subcolumn>
        <d>18.5</d>
        <d>20.0</d>
        <d>19.5</d>
      </Subcolumn>
    </YColumn>
  </Table>
  <Graph ID="Graph0">
    <Title>Legacy Unsupported Graph</Title>
  </Graph>
</PrismFile>
"#;
    xml.as_bytes().to_vec()
}

/// Build a self-authored `.prism` ZIP fixture with a CSV table, JSON table metadata, and an
/// unrecognized JSON chunk so `ImportWarning`s are deterministically emitted.
fn build_archive_fixture_bytes() -> Result<Vec<u8>, String> {
    let mut zw = ZipWriter::new();
    zw.add_file("data/assay.csv", b"GroupA,GroupB\n4.2,9.1\n5.3,10.4\n6.1,11.2\n".to_vec())
        .map_err(|e| format!("failed to add data/assay.csv: {e}"))?;
    zw.add_file("data/assay.json", br#"{"id":"archive-tbl-1","title":"Archive Assay","kind":"column"}"#.to_vec())
        .map_err(|e| format!("failed to add data/assay.json: {e}"))?;
    zw.add_file("extras/unsupported_chunk.json", br#"{"unsupported_vendor_extension":true,"payload":[1,2,3]}"#.to_vec())
        .map_err(|e| format!("failed to add extras/unsupported_chunk.json: {e}"))?;
    zw.finish().map_err(|e| format!("failed to build synthetic .prism zip: {e}"))
}

/// Build a self-authored `.xlsx` OOXML workbook fixture with one worksheet (`Xlsx Assay`) and one
/// non-worksheet chartsheet tab so `ImportWarning`s are deterministically emitted without external-app files.
fn build_xlsx_fixture_bytes() -> Result<Vec<u8>, String> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Xlsx Assay" sheetId="1" r:id="rId1"/>
    <sheet name="SkippedChart" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
</Relationships>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Signal</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>12.5</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><v>24.5</v></c></row>
    <row r="4"><c r="A4"><v>3</v></c><c r="B4"><v>36.5</v></c></row>
  </sheetData>
</worksheet>"#;
    let mut zw = ZipWriter::new();
    zw.add_file("xl/workbook.xml", workbook.as_bytes().to_vec()).map_err(|e| format!("failed to add xl/workbook.xml: {e}"))?;
    zw.add_file("xl/_rels/workbook.xml.rels", rels.as_bytes().to_vec()).map_err(|e| format!("failed to add xl/_rels/workbook.xml.rels: {e}"))?;
    zw.add_file("xl/worksheets/sheet1.xml", sheet.as_bytes().to_vec()).map_err(|e| format!("failed to add xl/worksheets/sheet1.xml: {e}"))?;
    zw.finish().map_err(|e| format!("failed to build synthetic .xlsx zip: {e}"))
}

fn assert_datasheets_match(label: &str, gui_sheets: &[DataSheet], ref_sheets: &[DataSheet]) -> TestRes {
    ensure(gui_sheets.len() == ref_sheets.len(), format!("{label}: DataSheet count mismatch: gui={}, ref={}", gui_sheets.len(), ref_sheets.len()))?;
    for (idx, (g, r)) in gui_sheets.iter().zip(ref_sheets.iter()).enumerate() {
        ensure(g.title == r.title, format!("{label}: sheet[{idx}] title mismatch: gui={:?}, ref={:?}", g.title, r.title))?;
        ensure(g.archetype == r.archetype, format!("{label}: sheet[{idx}] archetype/cells mismatch: gui={:?}, ref={:?}", g.archetype, r.archetype))?;
    }
    Ok(())
}

fn assert_file_bytes_eq(label: &str, gui_path: &Path, ref_path: &Path) -> TestRes {
    let gui_bytes = fs::read(gui_path).map_err(|e| format!("{label}: failed to read GUI output `{}`: {e}", gui_path.display()))?;
    let ref_bytes = fs::read(ref_path).map_err(|e| format!("{label}: failed to read oracle output `{}`: {e}", ref_path.display()))?;
    ensure(!gui_bytes.is_empty(), format!("{label}: GUI output `{}` is empty", gui_path.display()))?;
    if gui_bytes != ref_bytes {
        let gui_preview = String::from_utf8_lossy(&gui_bytes);
        let ref_preview = String::from_utf8_lossy(&ref_bytes);
        return Err(format!(
            "{label}: byte-for-byte mismatch between GUI `{}` ({} bytes) and headless oracle `{}` ({} bytes)\n--- GUI ---\n{gui_preview}\n--- ORACLE ---\n{ref_preview}",
            gui_path.display(),
            gui_bytes.len(),
            ref_path.display(),
            ref_bytes.len(),
        ));
    }
    Ok(())
}

/// Run File › Export › Export All Data Tables… (comma, period) into `dir` through the GUI.
fn reexport_all_tables_into(run: &mut WorkflowRun, dir: &Path) -> TestRes {
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.allTablesCsv.open")?;
    run.click("csv_export.delim.comma")?;
    run.click("csv_export.decimal.period")?;
    run.script_file_dialog_for("ui.export.allTablesCsv", Some(dir.to_path_buf()));
    run.commit_modal_ok()
}

#[test]
fn r10_30_human_pzfx_and_archive_import_warnings_and_live_edit() -> TestRes {
    let dir = temp_dir("import-warnings")?;
    let pzfx_path = dir.path().join("drug_assay.pzfx");
    let archive_path = dir.path().join("assay_bundle.prism");
    let xlsx_path = dir.path().join("xlsx_assay.xlsx");
    fs::write(&pzfx_path, build_pzfx_fixture_bytes()).map_err(|e| e.to_string())?;
    fs::write(&archive_path, build_archive_fixture_bytes()?).map_err(|e| e.to_string())?;
    fs::write(&xlsx_path, build_xlsx_fixture_bytes()?).map_err(|e| e.to_string())?;

    // --- 1A. .pzfx import from empty UiApp via File > Import ---
    {
        let mut run = WorkflowRun::start("R10.30-PZFX");
        run.script_file_dialog_for("table.importPzfx", Some(pzfx_path.clone()));
        run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPzfx")?;

        let reqs = run.recorded_file_dialog_requests();
        ensure(
            reqs.iter().any(|r| r.command_id == "table.importPzfx" && r.kind == NativeFileDialogKind::OpenFile),
            format!("R10.30-PZFX: expected OpenFile dialog request for table.importPzfx, got {reqs:?}"),
        )?;

        // Cross-check imported tables against a fresh headless Session import of the exact same file.
        let mut oracle = Session::new();
        oracle
            .execute("table.importPzfx", json!({ "path": pzfx_path.to_string_lossy() }))
            .map_err(|e| format!("R10.30-PZFX: headless oracle import failed: {e}"))?;
        assert_datasheets_match("R10.30-PZFX", &run.document().data_sheets, &oracle.document().data_sheets)?;

        // Verify R10.29 warnings dialog opened AND ui.inspect exposes `import_warnings`.
        ensure(
            matches!(run.active_dialog(), ActiveDialog::ImportWarnings),
            format!("R10.30-PZFX: expected ActiveDialog::ImportWarnings, got {:?}", run.active_dialog()),
        )?;
        ensure(
            !run.app().import_warnings.warnings.is_empty(),
            "R10.30-PZFX: expected non-empty ImportWarnings after importing fixture with unsupported <Graph>",
        )?;
        let snap_open = inspect_ui_tree(run.app(), None, false);
        let iw_open = snap_open.import_warnings.as_ref().ok_or("R10.30-PZFX: ui.inspect missing import_warnings")?;
        ensure(iw_open.open, "R10.30-PZFX: ui.inspect import_warnings.open should be true while modal is open")?;
        ensure(iw_open.command_id == "table.importPzfx", format!("R10.30-PZFX: expected command_id=table.importPzfx, got {:?}", iw_open.command_id))?;
        ensure(iw_open.warnings.len() == run.app().import_warnings.warnings.len(), "R10.30-PZFX: ui.inspect import_warnings count mismatch")?;

        // Close the panel via synthesized click on Close button (`modal.cancel`).
        run.click("modal.cancel")?;
        ensure(
            matches!(run.active_dialog(), ActiveDialog::None),
            format!("R10.30-PZFX: expected ActiveDialog::None after closing warnings, got {:?}", run.active_dialog()),
        )?;
        let snap_closed = inspect_ui_tree(run.app(), None, false);
        let iw_closed = snap_closed.import_warnings.as_ref().ok_or("R10.30-PZFX: warnings should remain persisted after close")?;
        ensure(!iw_closed.open, "R10.30-PZFX: ui.inspect import_warnings.open should be false after close")?;
        ensure(!iw_closed.warnings.is_empty(), "R10.30-PZFX: warnings list should remain non-empty after close")?;

        // Reopen from status footer pill (`status.import_warnings`).
        run.click("status.import_warnings")?;
        ensure(
            matches!(run.active_dialog(), ActiveDialog::ImportWarnings),
            format!("R10.30-PZFX: status pill did not reopen ImportWarnings: {:?}", run.active_dialog()),
        )?;
        run.click("modal.cancel")?;

        // Reopen from File > Import > View Import Warnings...
        run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.ui.import.warnings.open")?;
        ensure(
            matches!(run.active_dialog(), ActiveDialog::ImportWarnings),
            format!("R10.30-PZFX: menu item did not reopen ImportWarnings: {:?}", run.active_dialog()),
        )?;
        run.click("modal.cancel")?;

        // Live-edit an imported table cell via synthesized GUI input.
        let sheet_id = run.document().data_sheets.first().ok_or("R10.30-PZFX: missing imported sheet")?.id.clone();
        run.select_sheet(&sheet_id)?;
        run.enter_grid_cell(0, 0, 0, "42.5")?;
        let edited_cell = run.document().data_sheet(&sheet_id).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).map(|c| &c.value);
        ensure(
            matches!(edited_cell, Some(CellValue::Number(v)) if (v - 42.5).abs() < 1e-9),
            format!("R10.30-PZFX: expected live-edited cell (0,0,0) to be 42.5, got {edited_cell:?}"),
        )?;

        run.assert_shell_health("R10.30-PZFX")?;
    }

    // --- 1B. .prism import from empty UiApp via File > Import ---
    {
        let mut run = WorkflowRun::start("R10.30-ARCHIVE");
        run.script_file_dialog_for("table.importPrism", Some(archive_path.clone()));
        run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPrism")?;

        let reqs = run.recorded_file_dialog_requests();
        ensure(
            reqs.iter().any(|r| r.command_id == "table.importPrism" && r.kind == NativeFileDialogKind::OpenFile),
            format!("R10.30-ARCHIVE: expected OpenFile dialog request for table.importPrism, got {reqs:?}"),
        )?;

        let mut oracle = Session::new();
        oracle
            .execute("table.importPrism", json!({ "path": archive_path.to_string_lossy() }))
            .map_err(|e| format!("R10.30-ARCHIVE: headless oracle import failed: {e}"))?;
        assert_datasheets_match("R10.30-ARCHIVE", &run.document().data_sheets, &oracle.document().data_sheets)?;

        ensure(
            matches!(run.active_dialog(), ActiveDialog::ImportWarnings),
            format!("R10.30-ARCHIVE: expected ActiveDialog::ImportWarnings, got {:?}", run.active_dialog()),
        )?;
        let snap_open = inspect_ui_tree(run.app(), None, false);
        let iw = snap_open.import_warnings.as_ref().ok_or("R10.30-ARCHIVE: ui.inspect missing import_warnings")?;
        ensure(iw.open && !iw.warnings.is_empty(), "R10.30-ARCHIVE: expected open non-empty import_warnings in ui.inspect")?;
        ensure(iw.command_id == "table.importPrism", format!("R10.30-ARCHIVE: expected command_id=table.importPrism, got {:?}", iw.command_id))?;

        // Close, reopen via status footer pill, and close again.
        run.click("modal.cancel")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::None), "R10.30-ARCHIVE: expected modal closed")?;
        run.click("status.import_warnings")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::ImportWarnings), "R10.30-ARCHIVE: expected modal reopened via footer pill")?;
        run.click("modal.cancel")?;

        // Live-edit an imported table cell.
        let sheet_id = run.document().data_sheets.first().ok_or("R10.30-ARCHIVE: missing imported sheet")?.id.clone();
        run.select_sheet(&sheet_id)?;
        run.enter_grid_cell(0, 0, 0, "99.5")?;
        let edited_cell = run.document().data_sheet(&sheet_id).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).map(|c| &c.value);
        ensure(
            matches!(edited_cell, Some(CellValue::Number(v)) if (v - 99.5).abs() < 1e-9),
            format!("R10.30-ARCHIVE: expected live-edited cell (0,0,0) to be 99.5, got {edited_cell:?}"),
        )?;

        run.assert_shell_health("R10.30-ARCHIVE")?;
    }

    // --- 1C. .xlsx import from empty UiApp via File > Import ---
    {
        let mut run = WorkflowRun::start("R10.30-XLSX");
        run.script_file_dialog_for("table.importXlsx", Some(xlsx_path.clone()));
        run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importXlsx")?;

        let reqs = run.recorded_file_dialog_requests();
        ensure(
            reqs.iter().any(|r| r.command_id == "table.importXlsx" && r.kind == NativeFileDialogKind::OpenFile),
            format!("R10.30-XLSX: expected OpenFile dialog request for table.importXlsx, got {reqs:?}"),
        )?;

        let mut oracle = Session::new();
        oracle
            .execute("table.importXlsx", json!({ "path": xlsx_path.to_string_lossy() }))
            .map_err(|e| format!("R10.30-XLSX: headless oracle import failed: {e}"))?;
        assert_datasheets_match("R10.30-XLSX", &run.document().data_sheets, &oracle.document().data_sheets)?;

        ensure(
            matches!(run.active_dialog(), ActiveDialog::ImportWarnings),
            format!("R10.30-XLSX: expected ActiveDialog::ImportWarnings, got {:?}", run.active_dialog()),
        )?;
        let snap_open = inspect_ui_tree(run.app(), None, false);
        let iw = snap_open.import_warnings.as_ref().ok_or("R10.30-XLSX: ui.inspect missing import_warnings")?;
        ensure(iw.open && !iw.warnings.is_empty(), "R10.30-XLSX: expected open non-empty import_warnings in ui.inspect")?;
        ensure(iw.command_id == "table.importXlsx", format!("R10.30-XLSX: expected command_id=table.importXlsx, got {:?}", iw.command_id))?;

        run.click("modal.cancel")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::None), "R10.30-XLSX: expected modal closed")?;
        run.click("status.import_warnings")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::ImportWarnings), "R10.30-XLSX: expected modal reopened via footer pill")?;
        run.click("modal.cancel")?;

        // Live-edit the imported worksheet's first Y cell.
        let sheet_id = run.document().data_sheets.first().ok_or("R10.30-XLSX: missing imported sheet")?.id.clone();
        run.select_sheet(&sheet_id)?;
        run.enter_grid_cell(0, 0, 0, "88.5")?;
        let edited_cell = run.document().data_sheet(&sheet_id).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).map(|c| &c.value);
        ensure(
            matches!(edited_cell, Some(CellValue::Number(v)) if (v - 88.5).abs() < 1e-9),
            format!("R10.30-XLSX: expected live-edited cell (0,0,0) to be 88.5, got {edited_cell:?}"),
        )?;

        run.assert_shell_health("R10.30-XLSX")?;
    }

    Ok(())
}

#[test]
fn r10_30_human_import_replace_or_merge_on_dirty_project() -> TestRes {
    let dir = temp_dir("replace-merge")?;
    let pzfx_path = dir.path().join("merge_source.pzfx");
    let archive_path = dir.path().join("replace_source.prism");
    let xlsx_path = dir.path().join("open_source.xlsx");
    fs::write(&pzfx_path, build_pzfx_fixture_bytes()).map_err(|e| e.to_string())?;
    fs::write(&archive_path, build_archive_fixture_bytes()?).map_err(|e| e.to_string())?;
    fs::write(&xlsx_path, build_xlsx_fixture_bytes()?).map_err(|e| e.to_string())?;

    let mut run = WorkflowRun::start("R10.30-MERGE-REPLACE");
    let orig_id = run.create_table_via_welcome("welcome.arch.column", "Original Dirty Table", &[])?;
    run.select_sheet(&orig_id)?;
    run.enter_grid_cell(0, 0, 0, "7.5")?;
    ensure(run.app().session().is_dirty(), "R10.30-MERGE-REPLACE: expected dirty session after GUI edit")?;

    // 1. Import .pzfx on dirty project -> ImportProjectMode modal -> Add to current project (merge: true).
    run.script_file_dialog_for("table.importPzfx", Some(pzfx_path.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPzfx")?;
    match run.active_dialog() {
        ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes, .. } => {
            ensure(command_id == "table.importPzfx", format!("expected table.importPzfx, got {command_id}"))?;
            ensure(*has_unsaved_changes, "expected has_unsaved_changes=true on dirty project")?;
        }
        other => {
            return Err(format!("R10.30-MERGE-REPLACE: expected ActiveDialog::ImportProjectMode, got {other:?}"));
        }
    }

    run.click("import_project_mode.merge")?;
    if matches!(run.active_dialog(), ActiveDialog::ImportWarnings) {
        run.click("modal.cancel")?;
    }
    let merged_titles: Vec<&str> = run.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    ensure(
        merged_titles.contains(&"Original Dirty Table") && merged_titles.contains(&"Drug Assay"),
        format!("R10.30-MERGE-REPLACE: merge should keep both original and imported sheets, got {merged_titles:?}"),
    )?;

    // 2. Import .prism on dirty project -> ImportProjectMode modal -> Open as new project (merge: false).
    run.script_file_dialog_for("table.importPrism", Some(archive_path.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPrism")?;
    match run.active_dialog() {
        ActiveDialog::ImportProjectMode { command_id, has_unsaved_changes, .. } => {
            ensure(command_id == "table.importPrism", format!("expected table.importPrism, got {command_id}"))?;
            ensure(*has_unsaved_changes, "expected has_unsaved_changes=true before replace")?;
        }
        other => {
            return Err(format!("R10.30-MERGE-REPLACE: expected ActiveDialog::ImportProjectMode for .prism, got {other:?}"));
        }
    }

    run.click("import_project_mode.replace")?;
    if matches!(run.active_dialog(), ActiveDialog::ImportWarnings) {
        run.click("modal.cancel")?;
    }
    let replaced_titles: Vec<&str> = run.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    ensure(
        replaced_titles == vec!["Archive Assay"],
        format!("R10.30-MERGE-REPLACE: replace should retain only imported .prism sheet, got {replaced_titles:?}"),
    )?;

    // 3. File > Open... on .xlsx onto occupied project prompts replace-or-merge; Cancel keeps sheets, Merge adds.
    run.script_file_dialog_for("project.open", Some(xlsx_path.clone()));
    run.open_menu_and_click("menu.file", &[], "menu.project.open")?;
    match run.active_dialog() {
        ActiveDialog::ImportProjectMode { command_id, .. } => {
            ensure(command_id == "table.importXlsx", format!("File > Open (.xlsx) expected table.importXlsx prompt, got {command_id}"))?;
        }
        other => {
            return Err(format!("R10.30-MERGE-REPLACE: expected ImportProjectMode on File > Open (.xlsx), got {other:?}"));
        }
    }
    run.click("modal.cancel")?;
    let after_cancel: Vec<&str> = run.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    ensure(after_cancel == vec!["Archive Assay"], format!("Cancel on File > Open prompt must preserve sheets, got {after_cancel:?}"))?;

    run.script_file_dialog_for("project.open", Some(xlsx_path));
    run.open_menu_and_click("menu.file", &[], "menu.project.open")?;
    run.click("import_project_mode.merge")?;
    if matches!(run.active_dialog(), ActiveDialog::ImportWarnings) {
        run.click("modal.cancel")?;
    }
    let after_open_merge: Vec<&str> = run.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    ensure(
        after_open_merge.contains(&"Archive Assay") && after_open_merge.contains(&"Xlsx Assay"),
        format!("File > Open (.xlsx) + Merge should retain Archive Assay and add Xlsx Assay, got {after_open_merge:?}"),
    )?;

    // 4. Command Palette (`table.importPzfx`) routes through the native file picker + replace-or-merge prompt (not CSV Import Wizard).
    run.script_file_dialog_for("table.importPzfx", Some(pzfx_path));
    run.click("toolbar.command_palette")?;
    run.replace_textbox("palette.query", "table.importPzfx")?;
    run.click("palette.item.0")?;
    match run.active_dialog() {
        ActiveDialog::ImportProjectMode { command_id, .. } => {
            ensure(command_id == "table.importPzfx", format!("Command Palette expected table.importPzfx prompt, got {command_id}"))?;
        }
        other => {
            return Err(format!("Command Palette table.importPzfx must open ImportProjectMode (not CSV wizard), got {other:?}"));
        }
    }
    run.click("modal.cancel")?;

    run.assert_shell_health("R10.30-MERGE-REPLACE")
}

#[test]
fn r10_30_human_anova_exports_and_all_tables_csv() -> TestRes {
    let dir = temp_dir("anova-export")?;
    let mut run = WorkflowRun::start("R10.30-ANOVA-EXPORT");

    // Stage 1 & 2: Create primary ANOVA table + a second data table via Welcome Hub and populate via GUI.
    let anova_table_id = run.create_table_via_welcome("welcome.arch.column", "Dose Groups", &[])?;
    run.select_sheet(&anova_table_id)?;
    let ctrl = ["10.2", "11.4", "9.8", "10.6"];
    let low = ["14.5", "15.2", "13.9", "14.8"];
    let high = ["21.0", "22.4", "20.5", "21.7"];
    for r in 0..4 {
        run.enter_grid_cell(r, 0, 0, ctrl[r])?;
        run.enter_grid_cell(r, 1, 0, low[r])?;
        run.enter_grid_cell(r, 2, 0, high[r])?;
    }

    let extra_table_id = run.create_table_via_welcome("welcome.arch.column", "Secondary Cohort", &[])?;
    run.select_sheet(&extra_table_id)?;
    run.enter_grid_cell(0, 0, 0, "3.1")?;
    run.enter_grid_cell(1, 0, 0, "4.2")?;
    run.enter_grid_cell(0, 1, 0, "6.5")?;
    run.enter_grid_cell(1, 1, 0, "7.8")?;

    // Stage 3: Run One-way ANOVA on "Dose Groups" through the GUI.
    run.select_sheet(&anova_table_id)?;
    let res_id = run.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;

    // Stage 4: Verify ANOVA results against a fresh headless Session.
    let ref_sheet = run.document().data_sheet(&anova_table_id).cloned().ok_or("R10.30-ANOVA-EXPORT: missing Dose Groups sheet")?;
    let anova_kind = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let ref_source_id = anova_table_id.as_str().to_owned();
    run.verify_reference(
        ReferenceSpec::for_result(res_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_source_id, "kind": anova_kind })).map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .scalar("F", None, 1e-8, 1e-8)
        .scalar("P value", None, 1e-8, 1e-8),
    )?;

    // Snapshot the exact live document via archive round-trip into a headless Session oracle for byte-for-byte export comparisons.
    run.select_sheet(&res_id)?;
    let snap_path = dir.path().join("oracle_snapshot.curvebook");
    curvebook_format::archive::save_project_file(&snap_path, run.document()).map_err(|e| format!("failed to save oracle snapshot: {e}"))?;
    let mut oracle = Session::new();
    oracle.execute("project.open", json!({ "path": snap_path.to_string_lossy() })).map_err(|e| format!("failed to open oracle snapshot: {e}"))?;
    let tab0_id = run
        .document()
        .result_sheet(&res_id)
        .and_then(|r| r.tabs.first())
        .map(|t| t.id.clone())
        .ok_or("R10.30-ANOVA-EXPORT: ANOVA ResultSheet has no tabs")?;

    // --- Export 1: CSV Results... (all tabs tidy, comma/period) ---
    let gui_csv_all = dir.path().join("gui_anova_all.csv");
    let ref_csv_all = dir.path().join("ref_anova_all.csv");
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.resultsCsv.open")?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::CsvExportOptions(CsvExportTargetKind::ResultSheet { .. })),
        format!("expected CsvExportOptions(ResultSheet), got {:?}", run.active_dialog()),
    )?;
    run.click("csv_export.tab.all")?;
    run.click("csv_export.delim.comma")?;
    run.click("csv_export.decimal.period")?;
    run.script_file_dialog_for("analysis.exportCsv", Some(gui_csv_all.clone()));
    run.commit_modal_ok()?;
    oracle
        .execute(
            "analysis.exportCsv",
            json!({
                "sheet_id": res_id.as_str(),
                "path": ref_csv_all.to_string_lossy(),
                "delimiter": ",",
                "decimal_separator": ".",
            }),
        )
        .map_err(|e| format!("oracle analysis.exportCsv (all) failed: {e}"))?;
    assert_file_bytes_eq("CSV Results (all tabs tidy ,/.)", &gui_csv_all, &ref_csv_all)?;

    // --- Export 2: CSV Results... (single selected tab, semicolon & decimal comma) ---
    let gui_csv_eu = dir.path().join("gui_anova_tab0_eu.csv");
    let ref_csv_eu = dir.path().join("ref_anova_tab0_eu.csv");
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.resultsCsv.open")?;
    run.click("csv_export.tab.0")?;
    run.click("csv_export.delim.semicolon")?;
    run.click("csv_export.decimal.comma")?;
    run.script_file_dialog_for("analysis.exportCsv", Some(gui_csv_eu.clone()));
    run.commit_modal_ok()?;
    oracle
        .execute(
            "analysis.exportCsv",
            json!({
                "sheet_id": res_id.as_str(),
                "tab_id": tab0_id,
                "path": ref_csv_eu.to_string_lossy(),
                "delimiter": ";",
                "decimal_separator": ",",
            }),
        )
        .map_err(|e| format!("oracle analysis.exportCsv (tab0 ;/,) failed: {e}"))?;
    assert_file_bytes_eq("CSV Results (single tab ;/,)", &gui_csv_eu, &ref_csv_eu)?;

    // --- Export 3: CSV Results... (.tsv tab-delimited auto-delimiter override) ---
    let gui_tsv = dir.path().join("gui_anova_all.tsv");
    let ref_tsv = dir.path().join("ref_anova_all.tsv");
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.resultsCsv.open")?;
    run.click("csv_export.tab.all")?;
    run.click("csv_export.delim.comma")?;
    run.click("csv_export.decimal.period")?;
    run.script_file_dialog_for("analysis.exportCsv", Some(gui_tsv.clone()));
    run.commit_modal_ok()?;
    oracle
        .execute(
            "analysis.exportCsv",
            json!({
                "sheet_id": res_id.as_str(),
                "path": ref_tsv.to_string_lossy(),
                "delimiter": "\t",
                "decimal_separator": ".",
            }),
        )
        .map_err(|e| format!("oracle analysis.exportCsv (.tsv) failed: {e}"))?;
    assert_file_bytes_eq("CSV Results (.tsv auto-tab)", &gui_tsv, &ref_tsv)?;

    // --- Export 3b: File > Export > CSV Table... (`table.exportCsv` as ,/. and .tsv) ---
    run.select_sheet(&anova_table_id)?;
    let gui_tbl_csv = dir.path().join("gui_dose_groups.csv");
    let ref_tbl_csv = dir.path().join("ref_dose_groups.csv");
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.tableCsv.open")?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::CsvExportOptions(CsvExportTargetKind::DataTable { .. })),
        format!("expected CsvExportOptions(DataTable), got {:?}", run.active_dialog()),
    )?;
    ensure(
        run.app().canvas_dialogs.csv_export.delimiter == ',' && run.app().canvas_dialogs.csv_export.decimal_separator == '.',
        format!("en locale should default CSV Table options to ,/., got {:?}", run.app().canvas_dialogs.csv_export),
    )?;
    run.script_file_dialog_for("table.exportCsv", Some(gui_tbl_csv.clone()));
    run.commit_modal_ok()?;
    oracle
        .execute(
            "table.exportCsv",
            json!({
                "sheet_id": anova_table_id.as_str(),
                "path": ref_tbl_csv.to_string_lossy(),
                "delimiter": ",",
                "decimal_separator": ".",
            }),
        )
        .map_err(|e| format!("oracle table.exportCsv (,/. ) failed: {e}"))?;
    assert_file_bytes_eq("CSV Table (,/. )", &gui_tbl_csv, &ref_tbl_csv)?;

    let gui_tbl_tsv = dir.path().join("gui_dose_groups.tsv");
    let ref_tbl_tsv = dir.path().join("ref_dose_groups.tsv");
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.tableCsv.open")?;
    run.script_file_dialog_for("table.exportCsv", Some(gui_tbl_tsv.clone()));
    run.commit_modal_ok()?;
    oracle
        .execute(
            "table.exportCsv",
            json!({
                "sheet_id": anova_table_id.as_str(),
                "path": ref_tbl_tsv.to_string_lossy(),
                "delimiter": "\t",
                "decimal_separator": ".",
            }),
        )
        .map_err(|e| format!("oracle table.exportCsv (.tsv) failed: {e}"))?;
    assert_file_bytes_eq("CSV Table (.tsv auto-tab)", &gui_tbl_tsv, &ref_tbl_tsv)?;

    // --- Export 3c: Locale-default CsvExportOptions in `de` locale defaults to `;` + `,` ---
    let de_idx = curvebook_ui_egui::i18n::LOCALES.iter().position(|loc| *loc == "de").ok_or("missing `de` in LOCALES")?;
    let en_idx = curvebook_ui_egui::i18n::LOCALES.iter().position(|loc| *loc == "en").ok_or("missing `en` in LOCALES")?;
    run.open_menu_and_click("menu.view", &["menu.ui.locale.set"], &format!("popup.menu.ui.locale.set.opt.{de_idx}"))?;
    ensure(run.app().locale == "de", format!("expected `de` locale after menu click, got {}", run.app().locale))?;
    let gui_tbl_de = dir.path().join("gui_dose_groups_de.csv");
    let ref_tbl_de = dir.path().join("ref_dose_groups_de.csv");
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.tableCsv.open")?;
    ensure(
        run.app().canvas_dialogs.csv_export.delimiter == ';' && run.app().canvas_dialogs.csv_export.decimal_separator == ',',
        format!("de locale must default CSV export options to ;/,, got {:?}", run.app().canvas_dialogs.csv_export),
    )?;
    run.script_file_dialog_for("table.exportCsv", Some(gui_tbl_de.clone()));
    run.commit_modal_ok()?;
    oracle
        .execute(
            "table.exportCsv",
            json!({
                "sheet_id": anova_table_id.as_str(),
                "path": ref_tbl_de.to_string_lossy(),
                "delimiter": ";",
                "decimal_separator": ",",
            }),
        )
        .map_err(|e| format!("oracle table.exportCsv (de ;/,) failed: {e}"))?;
    assert_file_bytes_eq("CSV Table (de locale default ;/,)", &gui_tbl_de, &ref_tbl_de)?;
    run.open_menu_and_click("menu.view", &["menu.ui.locale.set"], &format!("popup.menu.ui.locale.set.opt.{en_idx}"))?;
    ensure(run.app().locale == "en", format!("expected `en` locale restored, got {}", run.app().locale))?;
    run.select_sheet(&res_id)?;

    // --- Export 4: JSON Results... (`analysis.exportJson`) ---
    let gui_json = dir.path().join("gui_anova.json");
    let ref_json = dir.path().join("ref_anova.json");
    run.script_file_dialog_for("analysis.exportJson", Some(gui_json.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.analysis.exportJson")?;
    oracle
        .execute(
            "analysis.exportJson",
            json!({
                "sheet_id": res_id.as_str(),
                "path": ref_json.to_string_lossy(),
            }),
        )
        .map_err(|e| format!("oracle analysis.exportJson failed: {e}"))?;
    assert_file_bytes_eq("JSON Results", &gui_json, &ref_json)?;

    // --- Export 5: Save Methods Report... (`result.generateReport` as .md and .tex) ---
    let gui_md = dir.path().join("methods_report.md");
    let ref_md = dir.path().join("ref_methods_report.md");
    run.script_file_dialog_for("result.generateReport", Some(gui_md.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.result.generateReport")?;
    oracle
        .execute(
            "result.generateReport",
            json!({
                "result_sheet_id": res_id.as_str(),
                "format": "markdown",
                "path": ref_md.to_string_lossy(),
            }),
        )
        .map_err(|e| format!("oracle result.generateReport (.md) failed: {e}"))?;
    assert_file_bytes_eq("Methods Report (.md)", &gui_md, &ref_md)?;

    let gui_tex = dir.path().join("methods_report.tex");
    let ref_tex = dir.path().join("ref_methods_report.tex");
    run.script_file_dialog_for("result.generateReport", Some(gui_tex.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.result.generateReport")?;
    oracle
        .execute(
            "result.generateReport",
            json!({
                "result_sheet_id": res_id.as_str(),
                "format": "latex",
                "path": ref_tex.to_string_lossy(),
            }),
        )
        .map_err(|e| format!("oracle result.generateReport (.tex) failed: {e}"))?;
    assert_file_bytes_eq("Methods Report (.tex)", &gui_tex, &ref_tex)?;

    let gui_txt = dir.path().join("methods_report.txt");
    let ref_txt = dir.path().join("ref_methods_report.txt");
    run.script_file_dialog_for("result.generateReport", Some(gui_txt.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.result.generateReport")?;
    oracle
        .execute(
            "result.generateReport",
            json!({
                "result_sheet_id": res_id.as_str(),
                "format": "plain",
                "path": ref_txt.to_string_lossy(),
            }),
        )
        .map_err(|e| format!("oracle result.generateReport (.txt) failed: {e}"))?;
    assert_file_bytes_eq("Methods Report (.txt)", &gui_txt, &ref_txt)?;

    // Command Palette `result.generateReport` must also infer `.tex` instead of forcing Markdown.
    let palette_tex = dir.path().join("palette_methods_report.tex");
    run.script_file_dialog_for("result.generateReport", Some(palette_tex.clone()));
    run.click("toolbar.command_palette")?;
    run.replace_textbox("palette.query", "result.generateReport")?;
    run.click("palette.item.0")?;
    assert_file_bytes_eq("Methods Report via Command Palette (.tex)", &palette_tex, &ref_tex)?;

    // --- Export 6: Export All Data Tables... (`ui.export.allTablesCsv` to a folder) ---
    let gui_all_tables_dir = dir.path().join("gui_all_tables");
    let ref_all_tables_dir = dir.path().join("ref_all_tables");
    fs::create_dir_all(&ref_all_tables_dir).map_err(|e| e.to_string())?;
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.allTablesCsv.open")?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::CsvExportOptions(CsvExportTargetKind::AllDataTables)),
        format!("expected CsvExportOptions(AllDataTables), got {:?}", run.active_dialog()),
    )?;
    run.click("csv_export.delim.comma")?;
    run.click("csv_export.decimal.period")?;
    run.script_file_dialog_for("ui.export.allTablesCsv", Some(gui_all_tables_dir.clone()));
    run.commit_modal_ok()?;

    let reqs = run.recorded_file_dialog_requests();
    ensure(
        reqs.iter().any(|r| r.command_id == "ui.export.allTablesCsv" && r.kind == NativeFileDialogKind::PickFolder),
        format!("expected PickFolder request for ui.export.allTablesCsv, got {reqs:?}"),
    )?;

    for (sheet_id, expected_file) in [(&anova_table_id, "Dose Groups.csv"), (&extra_table_id, "Secondary Cohort.csv")] {
        let gui_tbl_path = gui_all_tables_dir.join(expected_file);
        let ref_tbl_path = ref_all_tables_dir.join(expected_file);
        oracle
            .execute(
                "table.exportCsv",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "path": ref_tbl_path.to_string_lossy(),
                    "delimiter": ",",
                    "decimal_separator": ".",
                }),
            )
            .map_err(|e| format!("oracle table.exportCsv for `{expected_file}` failed: {e}"))?;
        assert_file_bytes_eq(&format!("Export All Data Tables (`{expected_file}`)"), &gui_tbl_path, &ref_tbl_path)?;
    }

    // --- Export 6b: exporting again into the same folder never replaces files without asking ---
    // An earlier file the user changed by hand stands in for "work the user wants to keep".
    let gui_dose = gui_all_tables_dir.join("Dose Groups.csv");
    let gui_dose_copy = gui_all_tables_dir.join("Dose Groups-2.csv");
    let gui_cohort_copy = gui_all_tables_dir.join("Secondary Cohort-2.csv");
    let kept_bytes = b"earlier export the user edited by hand\n".to_vec();
    fs::write(&gui_dose, &kept_bytes).map_err(|e| e.to_string())?;
    let assert_kept = |stage: &str| -> TestRes {
        let now = fs::read(&gui_dose).map_err(|e| format!("{stage}: failed to read `{}`: {e}", gui_dose.display()))?;
        ensure(now == kept_bytes, format!("{stage}: existing `Dose Groups.csv` changed without the user choosing Replace"))
    };

    // The prompt opens before anything is written and lists every file that would be replaced.
    reexport_all_tables_into(&mut run, &gui_all_tables_dir)?;
    match run.active_dialog() {
        ActiveDialog::ExportAllTablesExistingFiles { existing_files, .. } => {
            let mut listed = existing_files.clone();
            listed.sort();
            ensure(
                listed == vec![String::from("Dose Groups.csv"), String::from("Secondary Cohort.csv")],
                format!("existing-files prompt must list both earlier exports, got {listed:?}"),
            )?;
        }
        other => return Err(format!("expected ExportAllTablesExistingFiles prompt, got {other:?}")),
    }
    assert_kept("prompt open")?;
    ensure(!gui_dose_copy.exists(), "no file may be written while the prompt is open")?;

    // Cancel writes nothing.
    run.click("modal.cancel")?;
    ensure(matches!(run.active_dialog(), ActiveDialog::None), format!("Cancel must close the prompt, got {:?}", run.active_dialog()))?;
    assert_kept("after Cancel")?;
    ensure(!gui_dose_copy.exists() && !gui_cohort_copy.exists(), "Cancel must not write numbered copies")?;

    // Keep Both leaves existing files byte-for-byte and writes numbered copies.
    reexport_all_tables_into(&mut run, &gui_all_tables_dir)?;
    run.click("export_existing_files.keep_both")?;
    assert_kept("after Keep Both")?;
    assert_file_bytes_eq("Keep Both (`Dose Groups-2.csv`)", &gui_dose_copy, &ref_all_tables_dir.join("Dose Groups.csv"))?;
    assert_file_bytes_eq("Keep Both (`Secondary Cohort-2.csv`)", &gui_cohort_copy, &ref_all_tables_dir.join("Secondary Cohort.csv"))?;

    // Replace overwrites the listed files only after the user chooses it.
    reexport_all_tables_into(&mut run, &gui_all_tables_dir)?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::ExportAllTablesExistingFiles { .. }),
        format!("expected the prompt again before Replace, got {:?}", run.active_dialog()),
    )?;
    run.click("export_existing_files.replace")?;
    assert_file_bytes_eq("Replace (`Dose Groups.csv`)", &gui_dose, &ref_all_tables_dir.join("Dose Groups.csv"))?;
    assert_file_bytes_eq("Replace keeps earlier copy (`Dose Groups-2.csv`)", &gui_dose_copy, &ref_all_tables_dir.join("Dose Groups.csv"))?;

    // Stage 5: Live-edit upstream ANOVA table cell and finish with full 14-locale + parity checks.
    run.live_edit(LiveEditTarget::sheet_grid_cell(anova_table_id, 0, 2, 0, "35.0"), LiveEditExpectation::scalar_changed(res_id, "F", None, 1.0))?;

    run.finish()
}
