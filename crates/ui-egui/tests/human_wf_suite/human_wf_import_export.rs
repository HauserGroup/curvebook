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

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use crate::human_wf_common::{TestRes, ensure};
use curvebook_doc::{AnalysisKind, CellCoord, CellValue, DataSheet, GraphKind, NonlinearFitSpec, OneWayAnovaSpec, ResultStatus, SheetId};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use curvebook_testkit::wf_datasets::dose_response_4pl;
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
            reqs.iter().any(|r| r.command_id == "table.importPzfx" && r.kind == NativeFileDialogKind::OpenFile && r.filter_label == ".pzfx files"),
            format!("R10.30-PZFX: expected OpenFile dialog request for table.importPzfx with `.pzfx files` filter, got {reqs:?}"),
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
            reqs.iter().any(|r| r.command_id == "table.importPrism" && r.kind == NativeFileDialogKind::OpenFile && r.filter_label == ".prism files"),
            format!("R10.30-ARCHIVE: expected OpenFile dialog request for table.importPrism with `.prism files` filter, got {reqs:?}"),
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

#[test]
fn r10_34_human_gui_file_extension_naming() -> TestRes {
    use curvebook_ui_egui::i18n::{LOCALES, tr};

    // 1. All 16 shipped locales use literal lowercase `.pzfx` / `.prism` and dot-free updated labels.
    for &loc in LOCALES {
        for (key, ext) in [
            ("cmd.table.importPzfx", ".pzfx"),
            ("cmd.table.importPrism", ".prism"),
            ("w6.view.shell.filter_xml_project", ".pzfx"),
            ("w6.view.shell.filter_archive", ".prism"),
        ] {
            let txt = tr(loc, key);
            ensure(txt.contains(ext), format!("R10.34: locale `{loc}` key `{key}` missing `{ext}`: {txt:?}"))?;
            ensure(
                !txt.contains("External Archive") && !txt.contains("XML Project"),
                format!("R10.34: locale `{loc}` key `{key}` kept legacy euphemism: {txt:?}"),
            )?;
            ensure(!txt.ends_with("...") && !txt.ends_with('…'), format!("R10.34: locale `{loc}` key `{key}` must be dot-free per R20.15: {txt:?}"))?;
        }
        let warn_lbl = tr(loc, "cmd.ui.import.warnings.open");
        ensure(
            !warn_lbl.ends_with("...") && !warn_lbl.ends_with('…'),
            format!("R10.34: locale `{loc}` `cmd.ui.import.warnings.open` must be dot-free: {warn_lbl:?}"),
        )?;
        for welcome_key in ["welcome.open.subtitle", "welcome.open.pipeline_label"] {
            let txt = tr(loc, welcome_key);
            ensure(
                txt.contains(".curvebook") && txt.contains(".pzfx") && txt.contains(".prism"),
                format!("R10.34: locale `{loc}` `{welcome_key}` must mention .curvebook, .pzfx, and .prism: {txt:?}"),
            )?;
        }
    }

    // 2. Live Welcome Hub Open File copy & File > Import menu labels.
    let dir = temp_dir("r10-34-naming")?;
    let pzfx_path = dir.path().join("plate.pzfx");
    let archive_path = dir.path().join("study.prism");
    fs::write(&pzfx_path, build_pzfx_fixture_bytes()).map_err(|e| e.to_string())?;
    fs::write(&archive_path, build_archive_fixture_bytes()?).map_err(|e| e.to_string())?;

    let mut run = WorkflowRun::start("R10.34-NAMING");
    run.click("welcome.action.open_existing")?;
    for wid in ["welcome.open.subtitle", "welcome.preview.pipeline"] {
        let lbl = &run.frame_meta().find_by_widget_id(wid).ok_or(wid)?.label;
        ensure(lbl.contains(".pzfx") && lbl.contains(".prism"), format!("R10.34: {wid} missing .pzfx/.prism: {lbl:?}"))?;
    }

    let _ = run.create_table_via_welcome("welcome.arch.column", "Base Sheet", &[])?;
    run.click("menu.file")?;
    run.hover("menu.file.import")?;
    let pzfx_menu = run.frame_meta().find_by_widget_id("menu.table.importPzfx").ok_or("missing menu.table.importPzfx")?.label.clone();
    let archive_menu = run.frame_meta().find_by_widget_id("menu.table.importPrism").ok_or("missing menu.table.importPrism")?.label.clone();
    let warn_menu = run.frame_meta().find_by_widget_id("menu.ui.import.warnings.open").ok_or("missing menu.ui.import.warnings.open")?.label.clone();
    ensure(pzfx_menu == "Import .pzfx files", format!("expected `Import .pzfx files`, got {pzfx_menu:?}"))?;
    ensure(archive_menu == "Import .prism files", format!("expected `Import .prism files`, got {archive_menu:?}"))?;
    ensure(warn_menu == "Import Warnings", format!("expected `Import Warnings`, got {warn_menu:?}"))?;

    // 3. Replace-or-merge modal title uses format-specific `.pzfx files` / `.prism files` label.
    run.script_file_dialog_for("table.importPzfx", Some(pzfx_path));
    run.click("menu.table.importPzfx")?;
    let dlg_pzfx = run.frame_meta().find_by_widget_id("dialog.import_project_mode").ok_or("missing dialog.import_project_mode")?.label.clone();
    ensure(dlg_pzfx == "Import .pzfx files", format!("expected ImportProjectMode title `Import .pzfx files`, got {dlg_pzfx:?}"))?;
    run.click("modal.cancel")?;

    run.script_file_dialog_for("table.importPrism", Some(archive_path));
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPrism")?;
    let dlg_archive = run.frame_meta().find_by_widget_id("dialog.import_project_mode").ok_or("missing dialog.import_project_mode")?.label.clone();
    ensure(dlg_archive == "Import .prism files", format!("expected ImportProjectMode title `Import .prism files`, got {dlg_archive:?}"))?;
    run.click("modal.cancel")?;

    // 4. Command Palette matches `.pzfx files` and `.prism files` by file extension.
    run.click("toolbar.command_palette")?;
    run.replace_textbox("palette.query", ".pzfx")?;
    let pal_pzfx: Vec<String> =
        run.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("palette.item.")).map(|e| e.label.clone()).collect();
    ensure(pal_pzfx.iter().any(|l| l.contains("Import .pzfx files")), format!("Command Palette `.pzfx` missing `Import .pzfx files`: {pal_pzfx:?}"))?;
    run.replace_textbox("palette.query", ".prism")?;
    let pal_archive: Vec<String> =
        run.frame_meta().entries.iter().filter(|e| e.widget_id.starts_with("palette.item.")).map(|e| e.label.clone()).collect();
    ensure(
        pal_archive.iter().any(|l| l.contains("Import .prism files")),
        format!("Command Palette `.prism` missing `Import .prism files`: {pal_archive:?}"),
    )?;
    run.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE)?;

    run.assert_shell_health("R10.34-NAMING")
}

/// Build a self-authored `.curvebook` project file containing one Column table with colliding ID `data-1`.
fn write_synthetic_curvebook_project(path: &Path, project_title: &str, sheet_title: &str, values: &[f64]) -> TestRes {
    let mut doc = curvebook_doc::ProjectDocument::new(project_title).map_err(|e| e.to_string())?;
    let mut col =
        curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 1, values.len().max(1)).map_err(|e| e.to_string())?;
    for (r, &val) in values.iter().enumerate() {
        col.data_sets[0].cells[r][0] = curvebook_doc::Cell::number(val);
    }
    let sheet = DataSheet::new("data-1", sheet_title, curvebook_doc::TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(sheet).map_err(|e| e.to_string())?;
    curvebook_format::save_project_file(path, &doc).map_err(|e| format!("failed to save `{}`: {e}", path.display()))
}

#[test]
fn s10_6_human_file_merge_project_oracle_badges_and_live_edit() -> TestRes {
    let dir = temp_dir("s10-6-merge-project")?;
    let proj_a_path = dir.path().join("project_a.curvebook");
    let proj_b_path = dir.path().join("project_b.curvebook");
    write_synthetic_curvebook_project(&proj_a_path, "Study Alpha", "Cohort Alpha", &[10.0, 12.0, 14.0])?;
    write_synthetic_curvebook_project(&proj_b_path, "Study Beta", "Cohort Beta", &[25.0, 27.0, 29.0])?;

    // Stage 1: Start from an empty UiApp and open the first project via File > Open...
    let mut run = WorkflowRun::start("S10.6-MERGE-PROJECT");
    run.script_file_dialog_for("project.open", Some(proj_a_path.clone()));
    run.open_menu_and_click("menu.file", &[], "menu.project.open")?;

    ensure(
        run.document().data_sheets.len() == 1 && run.document().data_sheets[0].title == "Cohort Alpha",
        format!("S10.6: expected Cohort Alpha after File > Open, got {:?}", run.document().data_sheets.iter().map(|s| &s.title).collect::<Vec<_>>()),
    )?;
    ensure(run.document().data_sheets[0].origin.is_none(), "S10.6: opened project must not carry an origin badge")?;

    // Stage 2: Merge the second project via File > Merge Project...
    run.script_file_dialog_for("project.merge", Some(proj_b_path.clone()));
    run.open_menu_and_click("menu.file", &[], "menu.project.merge")?;

    let reqs = run.recorded_file_dialog_requests();
    ensure(
        reqs.iter().any(|r| r.command_id == "project.merge" && r.kind == NativeFileDialogKind::OpenFile),
        format!("S10.6: expected OpenFile dialog request for project.merge, got {reqs:?}"),
    )?;

    // Stage 3 & 4: Verify merged sheets, colliding-ID renumbering, file path preservation, origin badge,
    // and exact parity against a fresh headless Session merge oracle.
    let mut oracle = Session::new();
    oracle
        .execute("project.open", json!({ "path": proj_a_path.to_string_lossy() }))
        .map_err(|e| format!("S10.6: headless oracle project.open failed: {e}"))?;
    oracle
        .execute("project.merge", json!({ "path": proj_b_path.to_string_lossy() }))
        .map_err(|e| format!("S10.6: headless oracle project.merge failed: {e}"))?;
    assert_datasheets_match("S10.6-MERGE-PROJECT", &run.document().data_sheets, &oracle.document().data_sheets)?;

    ensure(
        run.app().session().file_path() == Some(proj_a_path.as_path()),
        format!("S10.6: project.merge must preserve open file_path `{}`; got {:?}", proj_a_path.display(), run.app().session().file_path()),
    )?;
    ensure(run.document().data_sheets[0].origin.is_none(), "S10.6: original sheet must retain origin = None")?;

    let merged_sheet = run.document().data_sheets.get(1).ok_or("S10.6: missing merged Cohort Beta sheet")?;
    let merged_id = merged_sheet.id.clone();
    ensure(merged_id.as_str() != "data-1", format!("S10.6: colliding ID should be renumbered, got `{merged_id}`"))?;
    let origin = merged_sheet.origin.as_ref().ok_or("S10.6: merged sheet missing SheetOrigin")?;
    ensure(
        origin.source_label == "project_b.curvebook" && origin.source_format == "curvebook",
        format!("S10.6: unexpected SheetOrigin: {origin:?}"),
    )?;
    ensure(
        !origin.source_label.contains('/') && !origin.source_label.contains('\\'),
        format!("S10.6: origin source_label must be basename only, got {:?}", origin.source_label),
    )?;

    // Verify navigator entry and ui.inspect surface the origin badge without mutating sheet title.
    let snap = inspect_ui_tree(run.app(), None, false);
    let merged_nav_wid = format!("nav.sheet.{}", merged_id.as_str());
    let nav_node = snap
        .navigator_sheets
        .as_ref()
        .and_then(|sheets| sheets.iter().find(|w| w.widget_id == merged_nav_wid))
        .ok_or_else(|| format!("S10.6: missing navigator widget `{merged_nav_wid}` in ui.inspect"))?;
    ensure(nav_node.title == "Cohort Beta", format!("S10.6: navigator title must remain unchanged, got {:?}", nav_node.title))?;
    let nav_entry =
        run.app().nav.entries.iter().find(|e| e.sheet_id == merged_id).ok_or_else(|| format!("S10.6: missing NavigatorEntry for `{merged_id}`"))?;
    ensure(
        nav_entry.origin.as_ref().map(|o| (o.source_label.as_str(), o.source_format.as_str())) == Some(("project_b.curvebook", "curvebook")),
        format!("S10.6: NavigatorEntry missing expected origin badge: {:?}", nav_entry.origin),
    )?;

    // Stage 5: Live-edit a cell in the merged table via synthesized GUI grid input, then clear origin via Command Palette.
    run.select_sheet(&merged_id)?;
    run.enter_grid_cell(0, 0, 0, "77.25")?;
    let edited_val = run.document().data_sheet(&merged_id).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).map(|c| &c.value);
    ensure(
        matches!(edited_val, Some(CellValue::Number(v)) if (v - 77.25).abs() < 1e-9),
        format!("S10.6: expected live-edited cell (0,0,0) on merged table to be 77.25, got {edited_val:?}"),
    )?;
    let orig_val = run.document().data_sheets[0].cell(CellCoord::new(0, 0, 0)).map(|c| &c.value);
    ensure(
        matches!(orig_val, Some(CellValue::Number(v)) if (v - 10.0).abs() < 1e-9),
        format!("S10.6: editing merged table must not alter original sheet cell, got {orig_val:?}"),
    )?;

    run.click("toolbar.command_palette")?;
    run.replace_textbox("palette.query", "sheet.clearOrigin")?;
    run.click("palette.item.0")?;
    ensure(
        run.document().data_sheet(&merged_id).and_then(|ds| ds.origin.as_ref()).is_none(),
        "S10.6: sheet.clearOrigin via Command Palette must clear the active merged sheet's origin badge",
    )?;

    run.assert_shell_health("S10.6-MERGE-PROJECT")
}

/// Build a hand-authored external ID-indexed `.prism` archive with a 3-hop analysis snapshot
/// (`analyses/an-1/sheet.json` -> `result_sheets/rs-main.json` -> `data/sheets/view-1/sheet.json` -> `data.csv`).
fn build_external_3hop_archive_fixture_bytes() -> Result<Vec<u8>, String> {
    let entries: Vec<(&str, Vec<u8>)> = vec![
        (
            "document.json",
            serde_json::to_vec(&json!({
                "title": "Snapshot Study",
                "sheets": { "data": ["d-src"], "analyses": ["an-1"] },
                "sheetAttributesMap": {
                    "d-src": { "title": "Groups" },
                    "an-1": { "title": "Unpaired t test of Groups" }
                }
            }))
            .map_err(|e| e.to_string())?,
        ),
        (
            "data/sheets/d-src/sheet.json",
            serde_json::to_vec(&json!({
                "title": "Groups",
                "table": { "uid": "t-src", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-a", "ds-b"] }
            }))
            .map_err(|e| e.to_string())?,
        ),
        ("data/sets/ds-a.json", serde_json::to_vec(&json!({ "title": "Ctrl", "replicates": [{ "valueType": "real" }] })).map_err(|e| e.to_string())?),
        (
            "data/sets/ds-b.json",
            serde_json::to_vec(&json!({ "title": "Treated", "replicates": [{ "valueType": "real" }] })).map_err(|e| e.to_string())?,
        ),
        ("data/tables/t-src/data.csv", b"10,20\n12,22\n11,21\n".to_vec()),
        (
            "analyses/an-1/sheet.json",
            serde_json::to_vec(&json!({
                "uid": "an-1",
                "title": "Unpaired t test of Groups",
                "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
                "preferredResultSheet": "rs-main",
                "inputSheets": [{ "uid": "d-src", "title": "Groups" }],
                "resultSheets": [{ "uid": "rs-main", "title": "Tabular results" }]
            }))
            .map_err(|e| e.to_string())?,
        ),
        (
            "analyses/an-1/parameters.json",
            serde_json::to_vec(&json!({
                "content": { "experimentalDesign": "UNPAIRED_T_TEST", "options": { "pValueTailType": "P_VALUE_TWO_TAILED" } }
            }))
            .map_err(|e| e.to_string())?,
        ),
        (
            "analyses/an-1/results.json",
            serde_json::to_vec(&json!({
                "content": { "status": { "error": "", "warning": [] }, "tStatistic": 12.2474, "pValue": 0.00025 }
            }))
            .map_err(|e| e.to_string())?,
        ),
        (
            "analyses/an-1/result_sheets/rs-main.json",
            serde_json::to_vec(&json!({ "uid": "rs-main", "name": "Tabular results", "dataSheet": "view-1" })).map_err(|e| e.to_string())?,
        ),
        (
            "data/sheets/view-1/sheet.json",
            serde_json::to_vec(&json!({
                "uid": "view-1",
                "flags": ["ANALYSIS_VIEW"],
                "table": { "uid": "t-view-1", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-vrt", "dataSets": ["ds-vcol"] }
            }))
            .map_err(|e| e.to_string())?,
        ),
        (
            "data/sets/ds-vrt.json",
            serde_json::to_vec(&json!({ "title": "Metric", "replicates": [{ "valueType": "string" }] })).map_err(|e| e.to_string())?,
        ),
        (
            "data/sets/ds-vcol.json",
            serde_json::to_vec(&json!({ "title": "Value", "replicates": [{ "valueType": "string" }] })).map_err(|e| e.to_string())?,
        ),
        ("data/tables/t-view-1/data.csv", b"P value,0.00025\nt statistic,12.2474\n".to_vec()),
    ];
    let mut zw = ZipWriter::new();
    for (name, bytes) in entries {
        zw.add_file(name, bytes).map_err(|e| format!("failed to add `{name}`: {e}"))?;
    }
    zw.finish().map_err(|e| format!("failed to build synthetic 3-hop .prism zip: {e}"))
}

#[test]
fn s7_human_archive_imported_result_banner_stale_and_undo() -> TestRes {
    let dir = temp_dir("s7-imported-result")?;
    let archive_path = dir.path().join("snapshot_3hop.prism");
    fs::write(&archive_path, build_external_3hop_archive_fixture_bytes()?).map_err(|e| e.to_string())?;

    // Stage 1: Import 3-hop .prism snapshot from empty UiApp via visible File > Import > Import .prism File.
    let mut run = WorkflowRun::start("S7-IMPORTED-RESULT");
    run.script_file_dialog_for("table.importPrism", Some(archive_path.clone()));
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPrism")?;
    if matches!(run.active_dialog(), ActiveDialog::ImportWarnings) {
        run.click("modal.cancel")?;
    }

    // Verify R10.34 native picker filter label (".prism files") and oracle parity.
    let reqs = run.recorded_file_dialog_requests();
    ensure(
        reqs.iter().any(|r| r.command_id == "table.importPrism" && r.filter_label == ".prism files"),
        format!("S7-IMPORTED-RESULT: expected R10.34 `.prism files` filter label, got {reqs:?}"),
    )?;
    let mut oracle = Session::new();
    oracle
        .execute("table.importPrism", json!({ "path": archive_path.to_string_lossy() }))
        .map_err(|e| format!("S7-IMPORTED-RESULT: oracle import failed: {e}"))?;
    assert_datasheets_match("S7-IMPORTED-RESULT", &run.document().data_sheets, &oracle.document().data_sheets)?;

    // Stage 2 & 3: Select the imported ResultSheet and verify read-only banner, Edit Parameters refusal, and Re-run action.
    let data_id = run.document().data_sheets.first().ok_or("S7-IMPORTED-RESULT: missing imported data sheet")?.id.clone();
    let res_id = run.document().result_sheets.first().ok_or("S7-IMPORTED-RESULT: missing imported result sheet")?.id.clone();
    run.select_sheet(&res_id)?;

    let rs_initial = run.document().result_sheet(&res_id).ok_or("S7-IMPORTED-RESULT: missing result sheet")?;
    ensure(rs_initial.status == curvebook_doc::ResultStatus::Imported, format!("expected ResultStatus::Imported, got {:?}", rs_initial.status))?;
    ensure(rs_initial.is_imported(), "expected ResultSheet::is_imported() == true")?;

    let banner =
        run.frame_meta().find_by_widget_id("result.imported_banner").ok_or("S7-IMPORTED-RESULT: read-only imported result banner not rendered")?;
    ensure(
        banner.label.contains("Read-only imported result") && banner.label.contains("not computed by CurveBook"),
        format!("S7-IMPORTED-RESULT: unexpected imported banner copy: {:?}", banner.label),
    )?;

    let edit_btn = run.frame_meta().find_by_widget_id("result.edit_params").ok_or("S7-IMPORTED-RESULT: missing result.edit_params toolbar button")?;
    ensure(!edit_btn.enabled, "S7-IMPORTED-RESULT: Edit Parameters button must be disabled on imported result snapshots")?;
    let mut probe = run.app().clone();
    let edit_refusal = curvebook_ui_egui::dispatch_ui_command(&mut probe, "ui.editParams.open", &json!({ "sheet_id": res_id.as_str() }));
    ensure(
        matches!(&edit_refusal, Err(msg) if msg.contains("read-only imported snapshot")),
        format!("S7-IMPORTED-RESULT: expected ui.editParams.open refusal on imported snapshot, got {edit_refusal:?}"),
    )?;

    // Clicking "Re-run as a CurveBook analysis" opens AnalyzeSheet targeting the upstream data table.
    let rerun_btn = run
        .frame_meta()
        .find_by_widget_id("result.imported.rerun_analysis")
        .ok_or("S7-IMPORTED-RESULT: missing Re-run as a CurveBook analysis button")?;
    ensure(rerun_btn.label == "Re-run as a CurveBook analysis", format!("unexpected re-run label: {:?}", rerun_btn.label))?;
    run.click("result.imported.rerun_analysis")?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::AnalyzeSheet { source_sheet_id } if *source_sheet_id == data_id),
        format!("S7-IMPORTED-RESULT: expected AnalyzeSheet for `{data_id}`, got {:?}", run.active_dialog()),
    )?;
    run.click("modal.cancel")?;

    // Stage 4 & 5: Live-edit upstream data cell -> flips Imported to ImportedStale without wiping tabs -> Undo restores Imported.
    run.select_sheet(&data_id)?;
    run.enter_grid_cell(0, 0, 0, "42.5")?;
    run.select_sheet(&res_id)?;

    let rs_stale = run.document().result_sheet(&res_id).ok_or("S7-IMPORTED-RESULT: missing result sheet after edit")?;
    ensure(
        rs_stale.status == curvebook_doc::ResultStatus::ImportedStale,
        format!("S7-IMPORTED-RESULT: upstream edit must flip Imported -> ImportedStale, got {:?}", rs_stale.status),
    )?;
    ensure(!rs_stale.tabs.is_empty(), "S7-IMPORTED-RESULT: ImportedStale must preserve imported result tabs")?;
    let stale_banner = run.frame_meta().find_by_widget_id("result.imported_banner").ok_or("S7-IMPORTED-RESULT: missing stale imported banner")?;
    ensure(
        stale_banner.label.contains("stale: upstream data changed"),
        format!("S7-IMPORTED-RESULT: stale banner copy mismatch: {:?}", stale_banner.label),
    )?;

    run.open_menu_and_click("menu.edit", &[], "menu.edit.undo")?;
    run.select_sheet(&res_id)?;
    let rs_restored = run.document().result_sheet(&res_id).ok_or("S7-IMPORTED-RESULT: missing result sheet after undo")?;
    ensure(
        rs_restored.status == curvebook_doc::ResultStatus::Imported,
        format!("S7-IMPORTED-RESULT: Undo must restore ResultStatus::Imported, got {:?}", rs_restored.status),
    )?;
    let restored_banner =
        run.frame_meta().find_by_widget_id("result.imported_banner").ok_or("S7-IMPORTED-RESULT: missing restored imported banner")?;
    ensure(
        restored_banner.label.contains("not computed by CurveBook") && !restored_banner.label.contains("stale"),
        format!("S7-IMPORTED-RESULT: restored banner should not be stale: {:?}", restored_banner.label),
    )?;

    run.assert_shell_health("S7-IMPORTED-RESULT")
}

#[test]
fn s7_human_file_export_pzfx_preflight_modal_roundtrip_and_skip_prompt() -> TestRes {
    let dir = temp_dir("s7-export-pzfx")?;
    let rich_proj_path = dir.path().join("rich_study.curvebook");
    let data_only_proj_path = dir.path().join("data_only_study.curvebook");
    let exported_rich_path = dir.path().join("rich_export.pzfx");
    let exported_clean_path = dir.path().join("clean_export.pzfx");

    // Seed a saved .curvebook project containing a 3-group Column table + linked InfoSheet + GraphSheet.
    {
        let mut doc = curvebook_doc::ProjectDocument::new("Rich PZFX Export Study").map_err(|e| e.to_string())?;
        let mut col = curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::UnpairedReplicates, 3, 3).map_err(|e| e.to_string())?;
        col.data_sets[0].title = String::from("Control");
        col.data_sets[1].title = String::from("Low");
        col.data_sets[2].title = String::from("High");
        for (r, (a, b, c)) in [(10.0, 14.0, 21.0), (11.0, 15.0, 22.0), (12.0, 16.0, 23.0)].into_iter().enumerate() {
            col.data_sets[0].cells[r][0] = curvebook_doc::Cell::number(a);
            col.data_sets[1].cells[r][0] = curvebook_doc::Cell::number(b);
            col.data_sets[2].cells[r][0] = curvebook_doc::Cell::number(c);
        }
        let ds = DataSheet::new("data-rich", "Drug Assay", curvebook_doc::TableArchetype::Column(col)).map_err(|e| e.to_string())?;
        let mut info = curvebook_doc::InfoSheet::new_preseeded("info-rich", "Study Notes").map_err(|e| e.to_string())?;
        info.linked_data_sheets.push(ds.id.clone());
        let graph =
            curvebook_doc::GraphSheet::new_uninitialized_for_data(curvebook_doc::SheetId::new("graph-rich"), &ds).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.add_info_sheet(info).map_err(|e| e.to_string())?;
        doc.add_graph_sheet(graph).map_err(|e| e.to_string())?;
        curvebook_format::save_project_file(&rich_proj_path, &doc).map_err(|e| e.to_string())?;
    }
    write_synthetic_curvebook_project(&data_only_proj_path, "Data Only Study", "Plain Cohort", &[5.5, 6.5, 7.5])?;

    // --- Part A: Project with analyses + graphs triggers pre-export confirmation modal ---
    {
        let mut run = WorkflowRun::start("S7-PZFX-EXPORT-MODAL");
        run.script_file_dialog_for("project.open", Some(rich_proj_path.clone()));
        run.open_menu_and_click("menu.file", &[], "menu.project.open")?;

        // Verify File > Export contains .pzfx file… and NO .prism export entry (§6.3).
        let menus = curvebook_ui_egui::menus::menu_bar_tooltips(run.app());
        let file_cmds: Vec<&str> =
            menus.iter().find(|m| m.menu_key == "menu.file").map(|m| m.items.iter().map(|i| i.command_id).collect()).unwrap_or_default();
        ensure(file_cmds.contains(&"ui.export.pzfx.open"), format!("File menu missing ui.export.pzfx.open: {file_cmds:?}"))?;
        ensure(!file_cmds.contains(&"project.exportPrism"), format!("File menu must NOT expose experimental .prism export: {file_cmds:?}"))?;

        // Stage 2 & 3: Enter a live value and run One-way ANOVA through the GUI so the project has analyses, graphs, and dirty state.
        let data_id = run.document().data_sheets[0].id.clone();
        run.select_sheet(&data_id)?;
        run.enter_grid_cell(0, 0, 0, "10.5")?;
        let _res_id = run.run_analyze_entry("One-way", "analyze.entry.stats.one_way_anova")?;
        ensure(run.app().session().is_dirty(), "S7-PZFX-EXPORT-MODAL: expected dirty session after running ANOVA")?;
        ensure(
            run.app().session().file_path() == Some(rich_proj_path.as_path()),
            "S7-PZFX-EXPORT-MODAL: expected active file_path pointing to rich_study.curvebook",
        )?;

        let expected_preflight = curvebook_format::preflight_pzfx_export(run.document());
        ensure(
            expected_preflight.total_dropped_count() >= 2,
            format!("S7-PZFX-EXPORT-MODAL: expected >=2 dropped items (analyses + graphs), got {expected_preflight:?}"),
        )?;

        // Open visible File > Export > .pzfx file… -> confirmation modal must show exact preflight counts & sheet names.
        run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.pzfx.open")?;
        match run.active_dialog() {
            ActiveDialog::PzfxExportConfirm { preflight } => {
                ensure(
                    preflight == &expected_preflight,
                    format!("S7-PZFX-EXPORT-MODAL: modal preflight diverged from preflight_pzfx_export: {preflight:?} vs {expected_preflight:?}"),
                )?;
            }
            other => return Err(format!("S7-PZFX-EXPORT-MODAL: expected ActiveDialog::PzfxExportConfirm, got {other:?}")),
        }

        let notice_data_only = run
            .frame_meta()
            .find_by_widget_id("pzfx_export_confirm.data_only_notice")
            .ok_or("S7-PZFX-EXPORT-MODAL: missing data_only_notice widget")?;
        ensure(
            notice_data_only.label == curvebook_engine::services::PZFX_EXPORT_DATA_ONLY_NOTICE,
            format!("S7-PZFX-EXPORT-MODAL: data_only_notice mismatch: {:?}", notice_data_only.label),
        )?;
        let notice_structure = run
            .frame_meta()
            .find_by_widget_id("pzfx_export_confirm.structure_notice")
            .ok_or("S7-PZFX-EXPORT-MODAL: missing structure_notice widget")?;
        ensure(
            notice_structure.label == curvebook_engine::services::PZFX_EXPORT_STRUCTURE_NOTICE,
            format!("S7-PZFX-EXPORT-MODAL: structure_notice mismatch: {:?}", notice_structure.label),
        )?;
        for (idx, group) in expected_preflight.dropped.iter().enumerate() {
            let hdr = run
                .frame_meta()
                .find_by_widget_id(&format!("pzfx_export_confirm.group.{idx}"))
                .ok_or_else(|| format!("S7-PZFX-EXPORT-MODAL: missing dropped group {idx} header"))?;
            ensure(
                hdr.label.contains(&format!("({})", group.count)),
                format!("S7-PZFX-EXPORT-MODAL: group {idx} header missing count ({}): {:?}", group.count, hdr.label),
            )?;
            if !group.items.is_empty() {
                let items_w = run
                    .frame_meta()
                    .find_by_widget_id(&format!("pzfx_export_confirm.group.{idx}.items"))
                    .ok_or_else(|| format!("S7-PZFX-EXPORT-MODAL: missing dropped group {idx} items"))?;
                for item in &group.items {
                    ensure(
                        items_w.label.contains(item),
                        format!("S7-PZFX-EXPORT-MODAL: group {idx} items {:?} missing sheet name `{item}`", items_w.label),
                    )?;
                }
            }
        }

        // Cancel first -> no file written.
        run.click("modal.cancel")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::None), "S7-PZFX-EXPORT-MODAL: Cancel must close modal")?;
        ensure(!exported_rich_path.exists(), "S7-PZFX-EXPORT-MODAL: Cancel must not write .pzfx file")?;

        // Reopen and click "Export data only" -> runs project.exportPzfx via SaveFile picker.
        run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.pzfx.open")?;
        run.script_file_dialog_for("project.exportPzfx", Some(exported_rich_path.clone()));
        run.click("pzfx_export_confirm.export")?;

        ensure(matches!(run.active_dialog(), ActiveDialog::None), "S7-PZFX-EXPORT-MODAL: modal should close after Export data only")?;
        let reqs = run.recorded_file_dialog_requests();
        ensure(
            reqs.iter().any(|r| {
                r.command_id == "project.exportPzfx"
                    && r.kind == NativeFileDialogKind::SaveFile
                    && r.filter_label == ".pzfx files"
                    && r.extensions == vec![String::from("pzfx")]
            }),
            format!("S7-PZFX-EXPORT-MODAL: expected .pzfx SaveFile request, got {reqs:?}"),
        )?;

        // Session file_path & dirty stay unchanged, status repeats counts, and exported .pzfx re-imports cleanly.
        ensure(
            run.app().session().file_path() == Some(rich_proj_path.as_path()),
            "S7-PZFX-EXPORT-MODAL: project.exportPzfx must not change active file_path",
        )?;
        ensure(run.app().session().is_dirty(), "S7-PZFX-EXPORT-MODAL: project.exportPzfx must not clear dirty state")?;
        let status = run.app().canvas_dialogs.status_banner.as_deref().ok_or("S7-PZFX-EXPORT-MODAL: missing post-export status summary")?;
        ensure(
            status.contains("rich_export.pzfx")
                && status.contains(&format!("{} dropped", expected_preflight.total_dropped_count()))
                && status.contains("warning(s)"),
            format!("S7-PZFX-EXPORT-MODAL: status banner missing expected counts: {status:?}"),
        )?;

        let mut oracle = Session::new();
        oracle
            .execute("table.importPzfx", json!({ "path": exported_rich_path.to_string_lossy() }))
            .map_err(|e| format!("S7-PZFX-EXPORT-MODAL: re-importing exported .pzfx failed: {e}"))?;
        let snap_rich = dir.path().join("oracle_rich_snapshot.curvebook");
        curvebook_format::save_project_file(&snap_rich, run.document()).map_err(|e| e.to_string())?;
        let mut expected_reimport = Session::new();
        expected_reimport
            .execute("project.open", json!({ "path": snap_rich.to_string_lossy() }))
            .map_err(|e| format!("S7-PZFX-EXPORT-MODAL: oracle open failed: {e}"))?;
        let oracle_pzfx = dir.path().join("oracle_rich_export.pzfx");
        expected_reimport
            .execute("project.exportPzfx", json!({ "path": oracle_pzfx.to_string_lossy(), "return_bytes": false }))
            .map_err(|e| format!("S7-PZFX-EXPORT-MODAL: headless oracle exportPzfx failed: {e}"))?;
        assert_file_bytes_eq("S7-PZFX-EXPORT-MODAL (.pzfx bytes)", &exported_rich_path, &oracle_pzfx)?;
        ensure(
            oracle.document().data_sheets.len() == run.document().data_sheets.len(),
            "S7-PZFX-EXPORT-MODAL: data sheet count mismatch on re-import",
        )?;
        let re_ds = &oracle.document().data_sheets[0];
        ensure(re_ds.title == "Drug Assay", format!("unexpected re-imported title: {:?}", re_ds.title))?;
        ensure(
            re_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()) == Some(10.5)
                && re_ds.cell(CellCoord::new(2, 2, 0)).and_then(|c| c.active_number()) == Some(23.0),
            "S7-PZFX-EXPORT-MODAL: re-imported cell values mismatch",
        )?;
        ensure(
            oracle.document().info_sheets.len() == run.document().info_sheets.len(),
            format!(
                "S7-PZFX-EXPORT-MODAL: info sheet count mismatch on re-import: gui={}, reimport={}",
                run.document().info_sheets.len(),
                oracle.document().info_sheets.len()
            ),
        )?;

        run.assert_shell_health("S7-PZFX-EXPORT-MODAL")?;
    }

    // --- Part B: Data-only project (0 dropped items) skips confirmation prompt and exports directly ---
    {
        let mut run = WorkflowRun::start("S7-PZFX-EXPORT-SKIP-PROMPT");
        run.script_file_dialog_for("project.open", Some(data_only_proj_path.clone()));
        run.open_menu_and_click("menu.file", &[], "menu.project.open")?;

        let sheet_id = run.document().data_sheets[0].id.clone();
        run.select_sheet(&sheet_id)?;
        run.enter_grid_cell(0, 0, 0, "88.25")?;
        ensure(run.app().session().is_dirty(), "S7-PZFX-EXPORT-SKIP-PROMPT: expected dirty session after cell edit")?;
        ensure(
            curvebook_format::preflight_pzfx_export(run.document()).total_dropped_count() == 0,
            "S7-PZFX-EXPORT-SKIP-PROMPT: expected 0 dropped items on data-only project",
        )?;

        run.script_file_dialog_for("project.exportPzfx", Some(exported_clean_path.clone()));
        run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.pzfx.open")?;

        ensure(
            matches!(run.active_dialog(), ActiveDialog::None),
            format!("S7-PZFX-EXPORT-SKIP-PROMPT: data-only export must skip confirmation modal, got {:?}", run.active_dialog()),
        )?;
        ensure(exported_clean_path.is_file(), "S7-PZFX-EXPORT-SKIP-PROMPT: expected direct .pzfx export file on disk")?;
        ensure(
            run.app().session().file_path() == Some(data_only_proj_path.as_path()) && run.app().session().is_dirty(),
            "S7-PZFX-EXPORT-SKIP-PROMPT: file_path and dirty must remain unchanged after direct .pzfx export",
        )?;
        let status = run.app().canvas_dialogs.status_banner.as_deref().ok_or("S7-PZFX-EXPORT-SKIP-PROMPT: missing status summary")?;
        ensure(
            status.contains("clean_export.pzfx") && status.contains("0 dropped"),
            format!("S7-PZFX-EXPORT-SKIP-PROMPT: unexpected status summary: {status:?}"),
        )?;

        let mut oracle = Session::new();
        oracle
            .execute("table.importPzfx", json!({ "path": exported_clean_path.to_string_lossy() }))
            .map_err(|e| format!("S7-PZFX-EXPORT-SKIP-PROMPT: re-import failed: {e}"))?;
        let snap_clean = dir.path().join("oracle_clean_snapshot.curvebook");
        curvebook_format::save_project_file(&snap_clean, run.document()).map_err(|e| e.to_string())?;
        let mut expected_reimport = Session::new();
        expected_reimport
            .execute("project.open", json!({ "path": snap_clean.to_string_lossy() }))
            .map_err(|e| format!("S7-PZFX-EXPORT-SKIP-PROMPT: oracle open failed: {e}"))?;
        let oracle_pzfx = dir.path().join("oracle_clean_export.pzfx");
        expected_reimport
            .execute("project.exportPzfx", json!({ "path": oracle_pzfx.to_string_lossy(), "return_bytes": false }))
            .map_err(|e| format!("S7-PZFX-EXPORT-SKIP-PROMPT: headless oracle exportPzfx failed: {e}"))?;
        assert_file_bytes_eq("S7-PZFX-EXPORT-SKIP-PROMPT (.pzfx bytes)", &exported_clean_path, &oracle_pzfx)?;
        let re_ds = oracle.document().data_sheets.first().ok_or("S7-PZFX-EXPORT-SKIP-PROMPT: missing re-imported sheet")?;
        ensure(
            re_ds.title == "Plain Cohort" && re_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()) == Some(88.25),
            format!("S7-PZFX-EXPORT-SKIP-PROMPT: re-imported data mismatch: title={:?}", re_ds.title),
        )?;

        run.assert_shell_health("S7-PZFX-EXPORT-SKIP-PROMPT")?;
    }

    Ok(())
}

/// Five-stage `WorkflowRun` human-path verification (`R19.5`) that exporting a graph whose
/// regression overlay is pending recalculation via the interactive `ExportFigure` dialog:
/// 1. Omits the pending fitted curve from the exported SVG (matching a graph without an overlay),
/// 2. Surfaces the structured `pending_regression_result` warning in both `CommandOutcome` and
///    the rendered shell status footer (`status.figure_export_warning`, dismissible on click and
///    isolated from `status_banner`), and
/// 3. Clears the warning banner automatically and renders the fitted curve on a subsequent clean
///    export after **Analyze › Recalculate All**, followed by `D1` reference verification and
///    upstream live-edit propagation.
#[test]
fn human_export_figure_pending_fit_warning_omits_curve_and_clears_on_clean_export() -> TestRes {
    let dir = temp_dir("pending-fit-export-warning")?;
    let fx = dose_response_4pl();
    let mut run = WorkflowRun::start("EXPORT-PENDING-FIT-WARNING");

    // Stage 1 (Create): empty UiApp -> create XY dose-response table + linked GraphSheet via Welcome Hub.
    let graphs_before: BTreeSet<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
    let xy_id = run.create_table_via_welcome("welcome.arch.xy", "Dose Response", &["welcome.stepper.cols.dec", "welcome.subcol.raw"])?;
    let graph_id = run
        .document()
        .graph_sheets
        .iter()
        .find(|g| !graphs_before.contains(&g.id) && g.spec.primary_source_sheet_id == xy_id)
        .map(|g| g.id.clone())
        .ok_or("EXPORT-PENDING-FIT-WARNING: missing auto-created XY GraphSheet")?;

    // Stage 2 (Enter): populate dose-response XY table via synthesized GUI input + clipboard paste.
    run.enter_column_title(0, "Agonist_A")?;
    run.enter_column_title(1, "Agonist_B")?;
    let data_tsv: String = fx
        .rows
        .iter()
        .map(|row| {
            let mut cols = vec![format!("{}", row.x)];
            for series in &row.series_replicates {
                for &v in series {
                    cols.push(format!("{v}"));
                }
            }
            cols.join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    run.click("grid.xcell.0.0")?;
    run.paste_clipboard(&data_tsv)?;

    // Stage 3 (Configure): run 4PL nonlinear fit, attach regression overlay to graph, then clone the
    // pipeline with "Re-run analyses immediately" unchecked so the cloned fit is genuinely pending (`Dirty`).
    let fit_id = run.run_analyze_entry("Nonlinear", "analyze.entry.fit.nonlinear")?;
    run.select_sheet(&fit_id)?;
    run.open_cmd_t()?;
    run.click("fit.tab.model")?;
    run.click("fit.eq.item.log_agonist_vs_response_4pl")?;
    run.commit_modal_ok()?;
    ensure(
        run.document().result_sheet(&fit_id).is_some_and(|r| r.status == ResultStatus::UpToDate && !r.fitted_curves.is_empty()),
        "EXPORT-PENDING-FIT-WARNING: initial 4PL fit must be UpToDate with fitted curves",
    )?;

    run.select_sheet(&graph_id)?;
    run.click("toolbar.format_graph")?;
    run.click("format_graph.overlay.attach.0")?;
    run.commit_modal_ok()?;
    ensure(
        run.document()
            .graph_sheet(&graph_id)
            .and_then(|g| g.spec.series.first())
            .and_then(|s| s.regression_overlay.as_ref())
            .is_some_and(|ov| ov.result_sheet_id == fit_id),
        "EXPORT-PENDING-FIT-WARNING: expected regression overlay attached to original XY graph",
    )?;

    // Clone pipeline through Welcome Hub keeping copied data while deferring analysis recalculation.
    let data_before_clone: BTreeSet<SheetId> = run.document().data_sheets.iter().map(|d| d.id.clone()).collect();
    let results_before_clone: BTreeSet<SheetId> = run.document().result_sheets.iter().map(|r| r.id.clone()).collect();
    let graphs_before_clone: BTreeSet<SheetId> = run.document().graph_sheets.iter().map(|g| g.id.clone()).collect();

    run.click("toolbar.home")?;
    run.click("welcome.mode.clone")?;
    run.click(&format!("welcome.clone.source.{}", xy_id.as_str()))?;
    run.click("welcome.btn.clone_options")?;
    run.click("welcome.clone.copy_data")?;
    run.click("welcome.clone.rerun_analyses")?;
    run.click("welcome.clone_modal.apply")?;
    run.click("welcome.btn.create")?;

    let cloned_xy_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| !data_before_clone.contains(&d.id))
        .map(|d| d.id.clone())
        .ok_or("EXPORT-PENDING-FIT-WARNING: missing cloned XY DataSheet")?;
    let cloned_fit_id = run
        .document()
        .result_sheets
        .iter()
        .find(|r| !results_before_clone.contains(&r.id))
        .map(|r| r.id.clone())
        .ok_or("EXPORT-PENDING-FIT-WARNING: missing cloned ResultSheet")?;
    let cloned_graph_id = run
        .document()
        .graph_sheets
        .iter()
        .find(|g| !graphs_before_clone.contains(&g.id))
        .map(|g| g.id.clone())
        .ok_or("EXPORT-PENDING-FIT-WARNING: missing cloned GraphSheet")?;

    ensure(
        run.document().result_sheet(&cloned_fit_id).is_some_and(|r| r.status == ResultStatus::Dirty),
        format!(
            "EXPORT-PENDING-FIT-WARNING: cloned fit `{}` must start pending (Dirty), got {:?}",
            cloned_fit_id.as_str(),
            run.document().result_sheet(&cloned_fit_id).map(|r| &r.status)
        ),
    )?;
    ensure(
        run.document()
            .graph_sheet(&cloned_graph_id)
            .and_then(|g| g.spec.series.first())
            .and_then(|s| s.regression_overlay.as_ref())
            .is_some_and(|ov| ov.result_sheet_id == cloned_fit_id),
        "EXPORT-PENDING-FIT-WARNING: cloned graph must reference the pending cloned fit overlay",
    )?;

    // Open interactive desktop ExportFigure dialog on the cloned graph and export SVG while pending.
    run.select_sheet(&cloned_graph_id)?;
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.figure.open")?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::ExportFigure { sheet_id } if *sheet_id == cloned_graph_id),
        format!("EXPORT-PENDING-FIT-WARNING: expected ExportFigure dialog for `{}`, got {:?}", cloned_graph_id.as_str(), run.active_dialog()),
    )?;
    run.click("export.fmt.svg")?;
    run.commit_modal_ok()?;
    ensure(
        matches!(run.active_dialog(), ActiveDialog::None),
        format!("EXPORT-PENDING-FIT-WARNING: ExportFigure must close after Export, got {:?}", run.active_dialog()),
    )?;

    let pending_svg = {
        let out = run.last_engine_outcome("graph.exportSvg").ok_or("EXPORT-PENDING-FIT-WARNING: missing graph.exportSvg outcome")?;
        ensure(
            out.payload["sheet_id"].as_str() == Some(cloned_graph_id.as_str()),
            format!("EXPORT-PENDING-FIT-WARNING: unexpected exported sheet_id: {}", out.payload),
        )?;
        let payload_warnings = out.payload["warnings"].as_array().ok_or("EXPORT-PENDING-FIT-WARNING: missing payload.warnings array")?;
        ensure(
            payload_warnings.iter().any(|w| w["code"] == json!("pending_regression_result")),
            format!("EXPORT-PENDING-FIT-WARNING: payload warnings missing pending_regression_result: {payload_warnings:?}"),
        )?;
        ensure(
            out.warnings.iter().any(|w| w.contains("pending_regression_result") && w.contains(cloned_fit_id.as_str())),
            format!("EXPORT-PENDING-FIT-WARNING: CommandOutcome warnings missing pending_regression_result: {:?}", out.warnings),
        )?;
        let svg = out.payload["svg"].as_str().ok_or("EXPORT-PENDING-FIT-WARNING: missing exported SVG string")?.to_owned();
        curvebook_svg::validate_svg_structure(&svg).map_err(|e| format!("EXPORT-PENDING-FIT-WARNING: invalid pending SVG: {e}"))?;
        svg
    };

    // Warning must surface cleanly in the shell footer (`status.figure_export_warning`) and stay
    // isolated from `status_banner`.
    let notice = run.app().canvas_dialogs.figure_export_warning.clone();
    ensure(
        notice.as_deref().is_some_and(|w| w.contains("pending_regression_result")),
        format!("EXPORT-PENDING-FIT-WARNING: figure_export_warning missing pending_regression_result: {notice:?}"),
    )?;
    ensure(
        run.app().canvas_dialogs.status_banner.is_none(),
        format!("EXPORT-PENDING-FIT-WARNING: export warning must not leak into status_banner: {:?}", run.app().canvas_dialogs.status_banner),
    )?;
    let chip = run
        .frame_meta()
        .find_by_widget_id("status.figure_export_warning")
        .ok_or("EXPORT-PENDING-FIT-WARNING: status.figure_export_warning chip not rendered")?;
    let chip_text = chip.full_label.clone().unwrap_or_else(|| chip.label.clone());
    ensure(
        chip.enabled && chip_text.contains("pending_regression_result"),
        format!("EXPORT-PENDING-FIT-WARNING: footer warning chip text mismatch: enabled={}, text={chip_text:?}", chip.enabled),
    )?;

    // Clicking the footer warning chip dismisses it.
    run.click("status.figure_export_warning")?;
    ensure(run.app().canvas_dialogs.figure_export_warning.is_none(), "EXPORT-PENDING-FIT-WARNING: clicking footer warning chip must dismiss it")?;
    ensure(
        run.frame_meta().find_by_widget_id("status.figure_export_warning").is_none(),
        "EXPORT-PENDING-FIT-WARNING: dismissed warning chip must not render",
    )?;

    // Re-export via ExportFigure (PDF format) while still pending so the warning surfaces again
    // across formats, ready to test automatic clearing on clean export.
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.figure.open")?;
    run.click("export.fmt.pdf")?;
    run.commit_modal_ok()?;
    let pdf_out = run.last_engine_outcome("graph.exportPdf").ok_or("EXPORT-PENDING-FIT-WARNING: missing graph.exportPdf outcome")?;
    ensure(
        pdf_out.warnings.iter().any(|w| w.contains("pending_regression_result")),
        format!("EXPORT-PENDING-FIT-WARNING: PDF export missing pending_regression_result warning: {:?}", pdf_out.warnings),
    )?;
    ensure(
        run.app().canvas_dialogs.figure_export_warning.as_deref().is_some_and(|w| w.contains("pending_regression_result")),
        "EXPORT-PENDING-FIT-WARNING: PDF export with pending fit must re-surface figure_export_warning",
    )?;

    // Snapshot the pending document (stripped of overlay on a copy) via archive round-trip so a
    // fresh headless Session oracle verifies that the pending export omitted the fitted curve.
    let mut no_overlay_doc = run.document().clone();
    if let Some(gs) = no_overlay_doc.graph_sheet_mut(&cloned_graph_id) {
        for s in &mut gs.spec.series {
            s.regression_overlay = None;
        }
    }
    let no_overlay_snap = dir.path().join("no_overlay_oracle.curvebook");
    curvebook_format::archive::save_project_file(&no_overlay_snap, &no_overlay_doc)
        .map_err(|e| format!("failed to save no-overlay oracle snapshot: {e}"))?;
    let mut no_overlay_oracle = Session::new();
    no_overlay_oracle
        .execute("project.open", json!({ "path": no_overlay_snap.to_string_lossy() }))
        .map_err(|e| format!("failed to open no-overlay oracle: {e}"))?;
    let no_overlay_svg = no_overlay_oracle
        .execute("graph.exportSvg", json!({ "sheet_id": cloned_graph_id.as_str() }))
        .map_err(|e| format!("no-overlay oracle exportSvg failed: {e}"))?
        .payload["svg"]
        .as_str()
        .ok_or("no-overlay oracle missing svg")?
        .to_owned();
    ensure(
        pending_svg == no_overlay_svg,
        "EXPORT-PENDING-FIT-WARNING: pending-fit exported SVG must match a graph with no regression overlay (fitted curve omitted)",
    )?;

    // Recalculate all pending sheets through the GUI menu (`Analyze > Recalculate All`).
    run.open_menu_and_click("menu.analyze", &[], "menu.project.recalculate")?;
    ensure(
        run.document().result_sheet(&cloned_fit_id).is_some_and(|r| r.status == ResultStatus::UpToDate && !r.fitted_curves.is_empty()),
        format!("EXPORT-PENDING-FIT-WARNING: Analyze > Recalculate All must bring `{}` UpToDate with fitted curves", cloned_fit_id.as_str()),
    )?;

    // Re-export the cloned graph via interactive ExportFigure SVG dialog now that the fit is current:
    // warning banner must clear automatically and the exported SVG must include the fitted curve.
    run.select_sheet(&cloned_graph_id)?;
    run.open_menu_and_click("menu.file", &["menu.file.export"], "menu.ui.export.figure.open")?;
    run.click("export.fmt.svg")?;
    run.commit_modal_ok()?;

    let clean_svg = {
        let out = run.last_engine_outcome("graph.exportSvg").ok_or("EXPORT-PENDING-FIT-WARNING: missing clean graph.exportSvg outcome")?;
        ensure(out.warnings.is_empty(), format!("EXPORT-PENDING-FIT-WARNING: clean export emitted unexpected warnings: {:?}", out.warnings))?;
        ensure(
            out.payload["warnings"].as_array().is_some_and(Vec::is_empty),
            format!("EXPORT-PENDING-FIT-WARNING: clean export payload.warnings must be empty: {}", out.payload),
        )?;
        let svg = out.payload["svg"].as_str().ok_or("EXPORT-PENDING-FIT-WARNING: missing clean SVG string")?.to_owned();
        curvebook_svg::validate_svg_structure(&svg).map_err(|e| format!("EXPORT-PENDING-FIT-WARNING: invalid clean SVG: {e}"))?;
        svg
    };
    ensure(
        run.app().canvas_dialogs.figure_export_warning.is_none(),
        format!(
            "EXPORT-PENDING-FIT-WARNING: clean export must clear prior figure_export_warning, got {:?}",
            run.app().canvas_dialogs.figure_export_warning
        ),
    )?;
    ensure(
        run.frame_meta().find_by_widget_id("status.figure_export_warning").is_none(),
        "EXPORT-PENDING-FIT-WARNING: status.figure_export_warning chip must not render after clean export",
    )?;
    ensure(
        clean_svg != pending_svg,
        "EXPORT-PENDING-FIT-WARNING: clean export with UpToDate fit must differ from pending export (fitted curve now drawn)",
    )?;

    // Cross-check clean exported SVG byte-for-byte against a fresh headless Session oracle.
    let clean_snap = dir.path().join("clean_oracle.curvebook");
    curvebook_format::archive::save_project_file(&clean_snap, run.document()).map_err(|e| format!("failed to save clean oracle snapshot: {e}"))?;
    let mut clean_oracle = Session::new();
    clean_oracle.execute("project.open", json!({ "path": clean_snap.to_string_lossy() })).map_err(|e| format!("failed to open clean oracle: {e}"))?;
    let oracle_clean_svg = clean_oracle
        .execute("graph.exportSvg", json!({ "sheet_id": cloned_graph_id.as_str() }))
        .map_err(|e| format!("clean oracle exportSvg failed: {e}"))?
        .payload["svg"]
        .as_str()
        .ok_or("clean oracle missing svg")?
        .to_owned();
    ensure(clean_svg == oracle_clean_svg, "EXPORT-PENDING-FIT-WARNING: clean GUI ExportFigure SVG must match headless oracle byte-for-byte")?;

    // Stage 4 (Verify): D1 reference verification of the recalculated 4PL fit & graph overlay.
    let ref_sheet = fx.to_data_sheet("ref-r19-5-xy").map_err(|e| e.to_string())?;
    let nl_kind =
        serde_json::to_value(AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).map_err(|e| e.to_string())?;
    let expected_fit_id = cloned_fit_id.clone();
    run.verify_reference(
        ReferenceSpec::for_result(cloned_fit_id.clone(), move |sess| {
            sess.execute("analysis.run", json!({ "source_sheet_id": "ref-r19-5-xy", "sheet_id": "ref-r19-5-fit", "kind": nl_kind }))
                .map_err(|e| e.to_string())?;
            sess.execute("graph.create", json!({ "sheet_id": "ref-r19-5-g", "data_sheet_id": "ref-r19-5-xy", "kind": GraphKind::Xy }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .ref_result_id(SheetId::new("ref-r19-5-fit"))
        .with_graph(cloned_graph_id.clone())
        .ref_graph_id(SheetId::new("ref-r19-5-g"))
        .scalar("LogEC50", Some(0), 1e-4, 1e-4)
        .scalar("HillSlope", Some(0), 1e-4, 1e-4)
        .scalar("R2", None, 1e-6, 1e-6)
        .check_graph(move |gui_gs, _ref_gs| {
            ensure(
                gui_gs.spec.series.first().and_then(|s| s.regression_overlay.as_ref()).is_some_and(|ov| ov.result_sheet_id == expected_fit_id),
                "EXPORT-PENDING-FIT-WARNING: verified graph must keep regression overlay linked to cloned fit",
            )
        }),
    )?;

    // Stage 5 (LiveEdit): live-edit upstream XY cell on cloned sheet and verify fit + graph update.
    run.live_edit_all(
        LiveEditTarget::sheet_grid_cell(cloned_xy_id, 4, 0, 0, "45.0"),
        vec![
            LiveEditExpectation::scalar_changed(cloned_fit_id.clone(), "LogEC50", Some(0), 1e-4),
            LiveEditExpectation::scalar_changed(cloned_fit_id, "R2", None, 1e-5),
            LiveEditExpectation::graph_svg_changed(cloned_graph_id),
        ],
    )?;

    run.finish()
}
