#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Five-stage `WorkflowRun` GUI integration tests for `R17.7`
//! (Composable Microplate & Kinetic Assay Support):
//! 1. `ImportWizard` multi-block `.xlsx` split (`skip_rows`, `fill_merged_cells`, `split_by_column`)
//!    + shared `InfoSheet` via `state.extract_info_metadata` (`import_paste.opt.extract_info_metadata`)
//! 2. Standalone `transform.auc` analysis modal (`Fixed`/`FirstPoint`, `x_min`/`x_max`,
//!    `independent_time_points`, live parameter update, and upstream live-edit recomputation)
//! 3. `ImportReshape` interactive `8x12` plate grid selection + dilution-series fill + `.cbplate`
//!    Save/Load round-trip + reading values directly from the `transform.auc` `ResultSheet`
//! 4. `Source well: <well>` origin tooltip & status-bar readout on mapped cells
//! 5. `ImportWizard` multi-block CSV split + comma-padded preamble extraction via
//!    `curvebook_format::extract_preamble_info`.

#[path = "common/mod.rs"]
mod common;
#[path = "human_wf_common/mod.rs"]
mod human_wf_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{AnalysisKind, AucBaseline, CellCoord, ConstantValue, DescriptiveSpec, PlateGeometry, PlateWellRole, ResultStatus, TableKind};
use curvebook_format::zip::ZipWriter;
use curvebook_format::{PLATE_MAP_EXTENSION, load_cbplate_file};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::grid_view::GridViewState;
use curvebook_ui_egui::interaction::KeyModifiers;
use curvebook_ui_egui::os_integration::{PlatformClipboardAdapter, StubPlatformClipboardAdapter};
use human_wf_common::workflow_run::{LiveEditExpectation, LiveEditTarget, ReferenceSpec, WorkflowRun};
use human_wf_common::{TestRes, ensure};
use serde_json::json;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Workspace-local temporary fixture directory cleaned up automatically on drop.
struct WorkspaceTempDir {
    path: PathBuf,
}

impl WorkspaceTempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target").join(format!("r17_7_gui_tmp_{label}_{}_{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| format!("create workspace tmp dir `{}`: {e}", dir.display()))?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).map_err(|e| format!("add zip entry `{name}`: {e}"))?;
    }
    zw.finish().map_err(|e| format!("finish zip: {e}"))
}

/// Build a synthetic multi-block kinetic microplate `.xlsx` workbook with:
/// - 2 leading preamble rows (`Assay: Kinetic Luminometry`, `Temperature: 37.0`)
/// - 1 header row (`Channel`, `Time (s)`, `A01..A04`, `B01..B04`)
/// - Two merged-label blocks (`Signal` in `A4:A6` and `Reference` in `A7:A9`)
fn build_kinetic_multiblock_xlsx() -> Result<Vec<u8>, String> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="KineticRead" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

    // Signal curves (t = 0, 10, 20):
    //   A01..A04 (Blank): constant 10 -> NetArea = 200
    //   B01 (Compound A, -9): [10, 20, 10] -> NetArea = 300 (blank-sub = 100)
    //   B02 (Compound A, -8): [10, 40, 10] -> NetArea = 500 (blank-sub = 300)
    //   B03 (Compound A, -7): [10, 70, 10] -> NetArea = 800 (blank-sub = 600)
    //   B04 (Compound A, -6): [10, 110, 10] -> NetArea = 1200 (blank-sub = 1000)
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Assay: Kinetic Luminometry</t></is></c></row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>Temperature: 37.0</t></is></c></row>
    <row r="3">
      <c r="A3" t="inlineStr"><is><t>Channel</t></is></c>
      <c r="B3" t="inlineStr"><is><t>Time (s)</t></is></c>
      <c r="C3" t="inlineStr"><is><t>A01</t></is></c>
      <c r="D3" t="inlineStr"><is><t>A02</t></is></c>
      <c r="E3" t="inlineStr"><is><t>A03</t></is></c>
      <c r="F3" t="inlineStr"><is><t>A04</t></is></c>
      <c r="G3" t="inlineStr"><is><t>B01</t></is></c>
      <c r="H3" t="inlineStr"><is><t>B02</t></is></c>
      <c r="I3" t="inlineStr"><is><t>B03</t></is></c>
      <c r="J3" t="inlineStr"><is><t>B04</t></is></c>
    </row>
    <row r="4">
      <c r="A4" t="inlineStr"><is><t>Signal</t></is></c>
      <c r="B4"><v>0</v></c>
      <c r="C4"><v>10</v></c><c r="D4"><v>10</v></c><c r="E4"><v>10</v></c><c r="F4"><v>10</v></c>
      <c r="G4"><v>10</v></c><c r="H4"><v>10</v></c><c r="I4"><v>10</v></c><c r="J4"><v>10</v></c>
    </row>
    <row r="5">
      <c r="B5"><v>10</v></c>
      <c r="C5"><v>10</v></c><c r="D5"><v>10</v></c><c r="E5"><v>10</v></c><c r="F5"><v>10</v></c>
      <c r="G5"><v>20</v></c><c r="H5"><v>40</v></c><c r="I5"><v>70</v></c><c r="J5"><v>110</v></c>
    </row>
    <row r="6">
      <c r="B6"><v>20</v></c>
      <c r="C6"><v>10</v></c><c r="D6"><v>10</v></c><c r="E6"><v>10</v></c><c r="F6"><v>10</v></c>
      <c r="G6"><v>10</v></c><c r="H6"><v>10</v></c><c r="I6"><v>10</v></c><c r="J6"><v>10</v></c>
    </row>
    <row r="7">
      <c r="A7" t="inlineStr"><is><t>Reference</t></is></c>
      <c r="B7"><v>0</v></c>
      <c r="C7"><v>5</v></c><c r="D7"><v>5</v></c><c r="E7"><v>5</v></c><c r="F7"><v>5</v></c>
      <c r="G7"><v>5</v></c><c r="H7"><v>5</v></c><c r="I7"><v>5</v></c><c r="J7"><v>5</v></c>
    </row>
    <row r="8">
      <c r="B8"><v>10</v></c>
      <c r="C8"><v>5</v></c><c r="D8"><v>5</v></c><c r="E8"><v>5</v></c><c r="F8"><v>5</v></c>
      <c r="G8"><v>6</v></c><c r="H8"><v>7</v></c><c r="I8"><v>8</v></c><c r="J8"><v>9</v></c>
    </row>
    <row r="9">
      <c r="B9"><v>20</v></c>
      <c r="C9"><v>5</v></c><c r="D9"><v>5</v></c><c r="E9"><v>5</v></c><c r="F9"><v>5</v></c>
      <c r="G9"><v>5</v></c><c r="H9"><v>5</v></c><c r="I9"><v>5</v></c><c r="J9"><v>5</v></c>
    </row>
  </sheetData>
  <mergeCells count="2">
    <mergeCell ref="A4:A6"/>
    <mergeCell ref="A7:A9"/>
  </mergeCells>
</worksheet>"#;

    build_xlsx_zip(&[("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", sheet1)])
}

#[test]
fn r17_7_workflow_xlsx_split_auc_plate_map_and_well_origin() -> TestRes {
    let dir = WorkspaceTempDir::new("xlsx-auc-plate")?;
    let xlsx_path = dir.path().join("kinetic_multiblock.xlsx");
    fs::write(&xlsx_path, build_kinetic_multiblock_xlsx()?).map_err(|e| format!("write xlsx: {e}"))?;
    let cbplate_path = dir.path().join(format!("assay_layout.{PLATE_MAP_EXTENSION}"));

    let mut run = WorkflowRun::start("R17.7-XLSX-AUC-PLATE");

    // -----------------------------------------------------------------------
    // Stage 1 & 2 (Create + Enter): ImportWizard multi-block .xlsx split + shared InfoSheet
    // -----------------------------------------------------------------------
    run.open_menu_and_click("menu.view", &[], "menu.ui.palette.open")?;
    run.replace_textbox("palette.query", "table.importCsv")?;
    run.press_key("Enter", KeyModifiers::default())?;
    run.click("import_paste.source.xlsx")?;
    run.replace_textbox("import_paste.input.path", &xlsx_path.to_string_lossy())?;
    run.click("import_wizard.xlsx.load_path")?;
    run.replace_textbox("import_wizard.xlsx.skip_rows", "2")?;
    run.click("import_wizard.xlsx.fill_merged_cells")?;

    // Stage 3 of ImportWizard: split by column 0 ("Channel")
    run.click("import_paste.tab.filter")?;
    run.click("import_paste.col_split.split")?;
    run.replace_textbox("import_paste.input.split_col", "0")?;

    // Stage 5 of ImportWizard: extract skipped preamble rows into shared InfoSheet via existing checkbox
    run.click("import_paste.tab.notes")?;
    run.click("import_paste.opt.extract_info_metadata")?;
    run.click("modal.ok")?;

    ensure(run.document().data_sheets.len() == 2, format!("expected 2 split DataSheets from .xlsx, got {}", run.document().data_sheets.len()))?;
    let signal_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| d.title.contains("Signal"))
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("missing Signal split DataSheet"))?;
    let ref_block_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| d.title.contains("Reference"))
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("missing Reference split DataSheet"))?;

    // Verify shared InfoSheet populated from skipped preamble rows and linked to both split sheets.
    let shared_info = run
        .document()
        .info_sheets
        .iter()
        .find(|info| {
            info.constants.iter().any(|c| c.name == "Assay" && c.value == ConstantValue::Text(String::from("Kinetic Luminometry")))
                && info.constants.iter().any(|c| c.name == "Temperature" && c.value == ConstantValue::Number(37.0))
        })
        .ok_or_else(|| format!("missing shared preamble InfoSheet; found {:?}", run.document().info_sheets))?;
    ensure(
        shared_info.linked_data_sheets.contains(&signal_id) && shared_info.linked_data_sheets.contains(&ref_block_id),
        format!("expected shared InfoSheet linked to both split sheets, got {:?}", shared_info.linked_data_sheets),
    )?;

    // -----------------------------------------------------------------------
    // Stage 3 (Configure): Standalone `transform.auc` modal (`FirstPoint`, bounds, Bailer SE, then `Fixed`)
    // -----------------------------------------------------------------------
    run.select_sheet(&signal_id)?;
    let auc_res_id = run.run_analyze_entry("Area Under", "analyze.entry.transform.auc")?;

    // Edit parameters via Cmd+T: switch to FirstPoint, set partial bounds [0, 20], toggle independent_time_points.
    run.open_cmd_t()?;
    run.click("edit.auc.baseline.first_point")?;
    run.replace_textbox("edit.auc.x_min", "0")?;
    run.replace_textbox("edit.auc.x_max", "20")?;
    run.click("edit.auc.independent_time_points")?;
    run.replace_textbox("edit.auc.min_peak_height", "0")?;
    run.replace_textbox("edit.auc.min_area_fraction", "0")?;
    run.commit_modal_ok()?;

    {
        let rs = run.document().result_sheet(&auc_res_id).ok_or_else(|| String::from("missing AUC ResultSheet"))?;
        ensure(rs.status == ResultStatus::UpToDate, format!("AUC ResultSheet not UpToDate: {:?}", rs.status))?;
        let AnalysisKind::AreaUnderCurve(spec) = &rs.analysis.kind else {
            return Err(format!("expected AreaUnderCurve spec, got {:?}", rs.analysis.kind));
        };
        ensure(spec.baseline == AucBaseline::FirstPoint, format!("expected FirstPoint baseline, got {:?}", spec.baseline))?;
        ensure(spec.x_min == Some(0.0) && spec.x_max == Some(20.0), format!("unexpected bounds: ({:?}, {:?})", spec.x_min, spec.x_max))?;
        ensure(spec.independent_time_points, "expected independent_time_points = true")?;
        // Under FirstPoint baseline (Y0 = 10), A01 is constant 10 -> NetArea = 0; B01 = [10,20,10] -> NetArea = 100.
        let a01_net = rs.lookup_scalar("NetArea", Some(0)).ok_or_else(|| String::from("missing NetArea for A01"))?;
        let b01_net = rs.lookup_scalar("NetArea", Some(4)).ok_or_else(|| String::from("missing NetArea for B01"))?;
        common::assert_close_res(a01_net, 0.0, 1e-9, "FirstPoint A01 NetArea")?;
        common::assert_close_res(b01_net, 100.0, 1e-9, "FirstPoint B01 NetArea")?;
    }

    // Switch back to Fixed Y = 0 and independent_time_points = false so Blank wells have non-zero baseline (200)
    // and Plate Map blank subtraction produces clean dose-response values [100, 300, 600, 1000].
    run.open_cmd_t()?;
    run.click("edit.auc.baseline.fixed")?;
    run.replace_textbox("edit.auc.baseline.value", "0")?;
    run.click("edit.auc.independent_time_points")?;
    run.commit_modal_ok()?;

    // -----------------------------------------------------------------------
    // Stage 3b: ImportReshape interactive 8x12 grid + dilution fill + .cbplate Save/Load + AUC source
    // -----------------------------------------------------------------------
    run.open_menu_and_click("menu.file", &["menu.file.import"], "menu.table.importPlateMap")?;
    run.click("import_paste.target.xy")?;
    run.replace_textbox("import_paste.input.title", "AUC Dose Response")?;
    run.replace_textbox("import_paste.input.x_title", "log10[Drug]")?;

    // Values Source sub-tab: read directly from the AUC ResultSheet's `Net Area` column.
    run.click("import_plate.values_src.sheet")?;
    run.replace_textbox("import_plate.input.source_value_col", "Net Area")?;

    // Plate Layout sub-tab: drag-select A01..A04 -> Blank; drag-select B01..B04 -> Fill Dilution (-9, +1).
    run.click("import_plate.subtab.layout")?;
    let a01_pos = run.widget_center("import_plate.well.A01")?;
    let a04_pos = run.widget_center("import_plate.well.A04")?;
    run.drag(a01_pos, a04_pos)?;
    run.click("import_plate.assign.blank")?;

    let b01_pos = run.widget_center("import_plate.well.B01")?;
    let b04_pos = run.widget_center("import_plate.well.B04")?;
    run.drag(b01_pos, b04_pos)?;
    run.click("import_plate.assign.treatment")?;
    run.replace_textbox("import_plate.input.group_name", "Compound A")?;
    run.replace_textbox("import_plate.input.dilution_start", "-9")?;
    run.replace_textbox("import_plate.input.dilution_step", "1")?;
    run.click("import_plate.btn.fill_dilution")?;

    // Grids & .cbplate sub-tab: Save .cbplate, validate disk file, then Load .cbplate back into grid.
    run.click("import_plate.subtab.grids_io")?;
    run.replace_textbox("import_plate.input.cbplate_path", &cbplate_path.to_string_lossy())?;
    run.click("import_plate.btn.save_cbplate")?;

    let saved_plate = load_cbplate_file(&cbplate_path).map_err(|e| format!("load saved .cbplate: {e}"))?;
    ensure(saved_plate.geometry == PlateGeometry::Wells96, format!("expected Wells96, got {:?}", saved_plate.geometry))?;
    ensure(saved_plate.wells.first() == Some(&PlateWellRole::Blank), format!("expected A01=Blank, got {:?}", saved_plate.wells.first()))?;
    ensure(
        matches!(
            saved_plate.wells.get(12),
            Some(PlateWellRole::Treatment { group_name, dilution_x, .. }) if group_name == "Compound A" && (*dilution_x - (-9.0)).abs() < 1e-9
        ),
        format!("expected B01=Compound A @ -9, got {:?}", saved_plate.wells.get(12)),
    )?;

    run.click("import_plate.btn.load_cbplate")?;

    // Blank-mean subtraction is enabled by default; commit the plate map into an XY dose-response sheet.
    run.commit_modal_ok()?;

    let plate_sheet_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| d.title == "AUC Dose Response")
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("missing mapped AUC Dose Response sheet"))?;

    // -----------------------------------------------------------------------
    // Well-origin tooltip & status-bar readout verification on mapped sheet
    // -----------------------------------------------------------------------
    run.select_sheet(&plate_sheet_id)?;
    run.click("grid.cell.0.0.0")?;
    run.hover("grid.cell.0.0.0")?;

    {
        let mapped_ds = run.document().data_sheet(&plate_sheet_id).ok_or_else(|| String::from("missing mapped DataSheet"))?;
        ensure(mapped_ds.kind() == TableKind::Xy, format!("expected Xy mapped sheet, got {:?}", mapped_ds.kind()))?;
        ensure(
            mapped_ds.plate_well_origin(CellCoord::new(0, 0, 0)) == Some("B01"),
            format!("expected origin B01 at (0,0,0), got {:?}", mapped_ds.plate_well_origin(CellCoord::new(0, 0, 0))),
        )?;
        let val0 = mapped_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()).ok_or_else(|| String::from("missing mapped B01 value"))?;
        common::assert_close_res(val0, 100.0, 1e-9, "blank-subtracted AUC B01")?;
        let val3 = mapped_ds.cell(CellCoord::new(3, 0, 0)).and_then(|c| c.active_number()).ok_or_else(|| String::from("missing mapped B04 value"))?;
        common::assert_close_res(val3, 1000.0, 1e-9, "blank-subtracted AUC B04")?;

        let tooltip = GridViewState::cell_hover_tooltip(mapped_ds, GridCellCoord::new(0, 0, 0))
            .ok_or_else(|| String::from("expected cell_hover_tooltip for mapped well cell"))?;
        ensure(tooltip.contains("Source well: B01"), format!("tooltip missing `Source well: B01`: {tooltip:?}"))?;
    }

    let cell_meta = run.frame_meta().find_by_widget_id("grid.cell.0.0.0").ok_or_else(|| String::from("missing inspect entry for grid.cell.0.0.0"))?;
    let source_well_val = cell_meta.value.as_ref().and_then(|v| v.get("source_well")).and_then(serde_json::Value::as_str);
    ensure(source_well_val == Some("B01"), format!("expected source_well == Some(\"B01\") in cell inspect value, got {:?}", cell_meta.value))?;

    let status_meta = run.frame_meta().find_by_widget_id("status.summary").ok_or_else(|| String::from("missing inspect entry for status.summary"))?;
    ensure(
        status_meta.label.contains("Source well: B01"),
        format!("expected `Source well: B01` in status bar summary, got {:?}", status_meta.label),
    )?;

    // -----------------------------------------------------------------------
    // Stage 4 (Verify): Compare GUI AUC ResultSheet against fresh headless Session oracle
    // -----------------------------------------------------------------------
    let xlsx_path_str = xlsx_path.to_string_lossy().into_owned();
    let ref_spec = ReferenceSpec::for_result(auc_res_id.clone(), move |session| {
        session
            .execute(
                "table.importXlsx",
                json!({
                    "path": xlsx_path_str,
                    "skip_rows": 2,
                    "fill_merged_cells": true,
                    "split_by_column": 0,
                    "extract_preamble_info": true,
                    "has_header": true,
                    "header_rows": 1,
                }),
            )
            .map_err(|e| e.to_string())?;
        let sig_id = session
            .document()
            .data_sheets
            .iter()
            .find(|d| d.title.contains("Signal"))
            .map(|d| d.id.as_str().to_owned())
            .ok_or_else(|| String::from("oracle missing Signal sheet"))?;
        session
            .execute(
                "analysis.run",
                json!({
                    "source_sheet_id": sig_id,
                    "kind": "transform.auc",
                    "baseline": 0.0,
                    "x_min": 0.0,
                    "x_max": 20.0,
                    "independent_time_points": false,
                    "min_peak_height": 0.0,
                    "min_area_fraction": 0.0,
                }),
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .scalar("NetArea", Some(0), 1e-9, 1e-9)
    .scalar("NetArea", Some(4), 1e-9, 1e-9)
    .scalar("NetArea", Some(7), 1e-9, 1e-9)
    .scalar("TotalArea", Some(4), 1e-9, 1e-9);
    run.verify_reference(ref_spec)?;

    // -----------------------------------------------------------------------
    // Stage 5 (LiveEdit): Edit upstream kinetic cell on Signal sheet -> AUC recomputes UpToDate
    // -----------------------------------------------------------------------
    run.live_edit(
        LiveEditTarget::sheet_grid_cell(signal_id, 1, 4, 0, "500.0"),
        LiveEditExpectation::scalar_changed(auc_res_id, "NetArea", Some(4), 100.0),
    )?;

    run.finish()
}

#[test]
fn r17_7_workflow_csv_split_and_preamble_info_wizard() -> TestRes {
    let csv_payload = concat!(
        "Operator: LabOne,,,\n",
        "Gain: 2.5,,,\n",
        "Block,Ctrl,Drug\n",
        "Plate1,10,20\n",
        "Plate1,12,24\n",
        "Plate1,14,28\n",
        "Plate2,30,60\n",
        "Plate2,32,64\n",
        "Plate2,34,68\n",
    );
    let clipboard = Arc::new(StubPlatformClipboardAdapter::new());
    clipboard.write_text(csv_payload).map_err(|e| e.to_string())?;
    let mut run = WorkflowRun::start_with_clipboard("R17.7-CSV-SPLIT", clipboard);

    // Stage 1 & 2 (Create + Enter): ImportWizard CSV multi-block split + comma-padded preamble extraction
    run.open_menu_and_click("menu.view", &[], "menu.ui.palette.open")?;
    run.replace_textbox("palette.query", "table.importCsv")?;
    run.press_key("Enter", KeyModifiers::default())?;
    run.click("import_paste.source.clipboard")?;
    run.click("import_paste.btn.paste_clipboard")?;
    run.click("import_paste.delim.comma")?;
    run.click("import_paste.tab.filter")?;
    run.click("import_paste.col_split.split")?;
    run.replace_textbox("import_paste.input.split_col", "0")?;
    run.click("import_paste.tab.notes")?;
    run.click("import_paste.opt.extract_info_metadata")?;
    run.click("modal.ok")?;

    ensure(run.document().data_sheets.len() == 2, format!("expected 2 split CSV DataSheets, got {}", run.document().data_sheets.len()))?;
    let plate1_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| d.title.contains("Plate1"))
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("missing Plate1 CSV block sheet"))?;
    let plate2_id = run
        .document()
        .data_sheets
        .iter()
        .find(|d| d.title.contains("Plate2"))
        .map(|d| d.id.clone())
        .ok_or_else(|| String::from("missing Plate2 CSV block sheet"))?;

    let shared_info = run
        .document()
        .info_sheets
        .iter()
        .find(|info| {
            info.constants.iter().any(|c| c.name == "Operator" && c.value == ConstantValue::Text(String::from("LabOne")))
                && info.constants.iter().any(|c| c.name == "Gain" && c.value == ConstantValue::Number(2.5))
        })
        .ok_or_else(|| format!("missing shared CSV preamble InfoSheet; found {:?}", run.document().info_sheets))?;
    ensure(
        shared_info.linked_data_sheets.contains(&plate1_id) && shared_info.linked_data_sheets.contains(&plate2_id),
        format!("expected shared CSV InfoSheet linked to Plate1 & Plate2, got {:?}", shared_info.linked_data_sheets),
    )?;

    // Stage 3 (Configure): Run Descriptive Statistics on Plate1
    run.select_sheet(&plate1_id)?;
    let desc_id = run.run_analyze_entry("Descriptive", "analyze.entry.stats.descriptive")?;

    // Stage 4 (Verify): Cross-check against fresh headless Session `table.importCsv` + `stats.descriptive`
    let ref_spec = ReferenceSpec::for_result(desc_id.clone(), move |session| {
        session
            .execute(
                "table.importCsv",
                json!({
                    "csv": csv_payload,
                    "title": "Imported Data",
                    "delimiter": ",",
                    "decimal_separator": ".",
                    "has_header": true,
                    "header_rows": 1,
                    "filter": { "row_window": [2, 999] },
                    "split_by_column": 0,
                    "extract_preamble_info": true,
                }),
            )
            .map_err(|e| e.to_string())?;
        let p1 = session
            .document()
            .data_sheets
            .iter()
            .find(|d| d.title.contains("Plate1"))
            .map(|d| d.id.as_str().to_owned())
            .ok_or_else(|| String::from("oracle missing Plate1 sheet"))?;
        let desc_kind = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
        session.execute("analysis.run", json!({ "source_sheet_id": p1, "kind": desc_kind })).map_err(|e| e.to_string())?;
        Ok(())
    })
    .scalar("Mean", Some(0), 1e-9, 1e-9)
    .scalar("SD", Some(0), 1e-9, 1e-9);
    run.verify_reference(ref_spec)?;

    // Stage 5 (LiveEdit): Edit Plate1 cell (0, 0, 0) -> Descriptive Mean updates UpToDate
    run.live_edit(LiveEditTarget::sheet_grid_cell(plate1_id, 0, 0, 0, "40.0"), LiveEditExpectation::scalar_changed(desc_id, "Mean", Some(0), 1.0))?;

    run.finish()
}
