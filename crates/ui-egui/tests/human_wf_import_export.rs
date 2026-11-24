#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R10.30` automated human-path import/export verification suite.
//!
//! Exercises `.pzfx` and `.prism` imports (including `R10.29` structured warnings,
//! `ui.inspect` exposure, status-footer & menu reopen, replace-or-merge confirmation
//! on dirty projects, and post-import live edits) alongside one-way ANOVA results
//! exports (`CSV` all-tabs tidy, single-tab `;`/`,` European format, `.tsv` auto-tab,
//! `JSON`, Markdown/LaTeX Methods Reports) and **Export All Data Tables…**, comparing
//! every imported table and every exported file against a fresh headless
//! `curvebook_engine::Session`.

mod human_wf_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{AnalysisKind, CellCoord, CellValue, DataSheet, OneWayAnovaSpec};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, CsvExportTargetKind};
use curvebook_ui_egui::os_integration::NativeFileDialogKind;
use curvebook_ui_egui::inspect_ui_tree;
use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use human_wf_common::{TestRes, ensure};
use serde_json::json;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-r10-30-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| format!("failed to create temp dir `{}`: {e}", dir.display()))?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build a self-authored `.pzfx` XML fixture with one Column table and an unsupported `<Graph>`
/// section so `ImportWarning`s are deterministically emitted without external-app files.
fn build_pzfx_fixture_bytes() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<GraphPadPrismFile PrismXMLVersion="5.00">
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
</GraphPadPrismFile>
"#;
    xml.as_bytes().to_vec()
}

/// Build a self-authored `.prism` ZIP fixture with a CSV table, JSON table metadata, and an
/// unrecognized JSON chunk so `ImportWarning`s are deterministically emitted.
fn build_prism_fixture_bytes() -> Result<Vec<u8>, String> {
    let mut zw = ZipWriter::new();
    zw.add_file("data/assay.csv", b"GroupA,GroupB\n4.2,9.1\n5.3,10.4\n6.1,11.2\n".to_vec())
        .map_err(|e| format!("failed to add data/assay.csv: {e}"))?;
    zw.add_file("data/assay.json", br#"{"id":"prism-tbl-1","title":"Prism Assay","kind":"column"}"#.to_vec())
        .map_err(|e| format!("failed to add data/assay.json: {e}"))?;
    zw.add_file(
        "extras/unsupported_chunk.json",
        br#"{"unsupported_vendor_extension":true,"payload":[1,2,3]}"#.to_vec(),
    )
    .map_err(|e| format!("failed to add extras/unsupported_chunk.json: {e}"))?;
    zw.finish().map_err(|e| format!("failed to build synthetic .prism zip: {e}"))
}

fn assert_datasheets_match(label: &str, gui_sheets: &[DataSheet], ref_sheets: &[DataSheet]) -> TestRes {
    ensure(
        gui_sheets.len() == ref_sheets.len(),
        format!("{label}: DataSheet count mismatch: gui={}, ref={}", gui_sheets.len(), ref_sheets.len()),
    )?;
    for (idx, (g, r)) in gui_sheets.iter().zip(ref_sheets.iter()).enumerate() {
        ensure(g.title == r.title, format!("{label}: sheet[{idx}] title mismatch: gui={:?}, ref={:?}", g.title, r.title))?;
        ensure(
            g.archetype == r.archetype,
            format!("{label}: sheet[{idx}] archetype/cells mismatch: gui={:?}, ref={:?}", g.archetype, r.archetype),
        )?;
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

#[test]
fn r10_30_human_pzfx_and_prism_import_warnings_and_live_edit() -> TestRes {
    let dir = TempDir::new("import-warnings")?;
    let pzfx_path = dir.path().join("drug_assay.pzfx");
    let prism_path = dir.path().join("prism_assay.prism");
    fs::write(&pzfx_path, build_pzfx_fixture_bytes()).map_err(|e| e.to_string())?;
    fs::write(&prism_path, build_prism_fixture_bytes()?).map_err(|e| e.to_string())?;

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
        ensure(
            iw_open.command_id == "table.importPzfx",
            format!("R10.30-PZFX: expected command_id=table.importPzfx, got {:?}", iw_open.command_id),
        )?;
        ensure(
            iw_open.warnings.len() == run.app().import_warnings.warnings.len(),
            "R10.30-PZFX: ui.inspect import_warnings count mismatch",
        )?;

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
        let edited_cell = run
            .document()
            .data_sheet(&sheet_id)
            .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
            .map(|c| &c.value);
        ensure(
            matches!(edited_cell, Some(CellValue::Number(v)) if (v - 42.5).abs() < 1e-9),
            format!("R10.30-PZFX: expected live-edited cell (0,0,0) to be 42.5, got {edited_cell:?}"),
        )?;

        run.assert_shell_health("R10.30-PZFX")?;
    }

    // --- 1B. .prism import from empty UiApp via File > Import ---
    {
        let mut run = WorkflowRun::start("R10.30-PRISM");
        run.script_file_dialog_for("table.importPrism", Some(prism_path.clone()));
        run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPrism")?;

        let reqs = run.recorded_file_dialog_requests();
        ensure(
            reqs.iter().any(|r| r.command_id == "table.importPrism" && r.kind == NativeFileDialogKind::OpenFile),
            format!("R10.30-PRISM: expected OpenFile dialog request for table.importPrism, got {reqs:?}"),
        )?;

        let mut oracle = Session::new();
        oracle
            .execute("table.importPrism", json!({ "path": prism_path.to_string_lossy() }))
            .map_err(|e| format!("R10.30-PRISM: headless oracle import failed: {e}"))?;
        assert_datasheets_match("R10.30-PRISM", &run.document().data_sheets, &oracle.document().data_sheets)?;

        ensure(
            matches!(run.active_dialog(), ActiveDialog::ImportWarnings),
            format!("R10.30-PRISM: expected ActiveDialog::ImportWarnings, got {:?}", run.active_dialog()),
        )?;
        let snap_open = inspect_ui_tree(run.app(), None, false);
        let iw = snap_open.import_warnings.as_ref().ok_or("R10.30-PRISM: ui.inspect missing import_warnings")?;
        ensure(iw.open && !iw.warnings.is_empty(), "R10.30-PRISM: expected open non-empty import_warnings in ui.inspect")?;
        ensure(
            iw.command_id == "table.importPrism",
            format!("R10.30-PRISM: expected command_id=table.importPrism, got {:?}", iw.command_id),
        )?;

        // Close, reopen via status footer pill, and close again.
        run.click("modal.cancel")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::None), "R10.30-PRISM: expected modal closed")?;
        run.click("status.import_warnings")?;
        ensure(matches!(run.active_dialog(), ActiveDialog::ImportWarnings), "R10.30-PRISM: expected modal reopened via footer pill")?;
        run.click("modal.cancel")?;

        // Live-edit an imported table cell.
        let sheet_id = run.document().data_sheets.first().ok_or("R10.30-PRISM: missing imported sheet")?.id.clone();
        run.select_sheet(&sheet_id)?;
        run.enter_grid_cell(0, 0, 0, "99.5")?;
        let edited_cell = run
            .document()
            .data_sheet(&sheet_id)
            .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
            .map(|c| &c.value);
        ensure(
            matches!(edited_cell, Some(CellValue::Number(v)) if (v - 99.5).abs() < 1e-9),
            format!("R10.30-PRISM: expected live-edited cell (0,0,0) to be 99.5, got {edited_cell:?}"),
        )?;

        run.assert_shell_health("R10.30-PRISM")?;
    }

    Ok(())
}

#[test]
fn r10_30_human_import_replace_or_merge_on_dirty_project() -> TestRes {
    let dir = TempDir::new("replace-merge")?;
    let pzfx_path = dir.path().join("merge_source.pzfx");
    let prism_path = dir.path().join("replace_source.prism");
    fs::write(&pzfx_path, build_pzfx_fixture_bytes()).map_err(|e| e.to_string())?;
    fs::write(&prism_path, build_prism_fixture_bytes()?).map_err(|e| e.to_string())?;

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
    run.script_file_dialog_for("table.importPrism", Some(prism_path.clone()));
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
        replaced_titles == vec!["Prism Assay"],
        format!("R10.30-MERGE-REPLACE: replace should retain only imported .prism sheet, got {replaced_titles:?}"),
    )?;

    run.assert_shell_health("R10.30-MERGE-REPLACE")
}

#[test]
fn r10_30_human_anova_exports_and_all_tables_csv() -> TestRes {
    let dir = TempDir::new("anova-export")?;
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
            sess.execute("analysis.run", json!({ "source_sheet_id": ref_source_id, "kind": anova_kind }))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .with_reference_sheet(ref_sheet)?
        .scalar("F", None, 1e-8, 1e-8)
        .scalar("P value", None, 1e-8, 1e-8),
    )?;

    // Snapshot the exact live document into a headless Session oracle for byte-for-byte export comparisons.
    run.select_sheet(&res_id)?;
    let mut oracle = Session::from_document(run.document().clone());
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

    for (sheet_id, expected_file) in [
        (&anova_table_id, "Dose Groups.csv"),
        (&extra_table_id, "Secondary Cohort.csv"),
    ] {
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

    // Stage 5: Live-edit upstream ANOVA table cell and finish with full 14-locale + parity checks.
    run.live_edit(
        LiveEditTarget::sheet_grid_cell(anova_table_id, 0, 2, 0, "35.0"),
        LiveEditExpectation::scalar_changed(res_id, "F", None, 1.0),
    )?;

    run.finish()
}
