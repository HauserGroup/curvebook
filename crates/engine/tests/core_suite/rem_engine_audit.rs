#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]

//! Comprehensive regression tests for Wave 2 `fix-engine` remediation:
//! `AUD-02-01`..`AUD-02-27`, `AUD-13-{01,02,04,07,14,15,19,22,23,25}`, and `EXT-01`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, Cell, CellCoord, ColumnDesignHint, ColumnTable, CurveSamplePoint, DataSheet, DemingErrorSpec,
    ErrorCode, FittedCurveOverlay, HookSource, InfoSheet, LayoutItem, LinearRegressionSpec, MultipleRowTestsSpec, MultiplicityMethod,
    MultiplicitySpec, MultivariateSpec, NormalizeAnchor, PointStyleOverride, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SheetCellRange, SheetId, SourceCellId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable,
};
use curvebook_engine::cmd::Caller;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::services::{DocumentExportFormat, ExportOptionsSpec, export_document_artifact};
use curvebook_engine::{ExternalPathPolicy, PATH_PARAM_KEYS, Session, dag};
use curvebook_format::zip::ZipWriter;
use curvebook_platform::WorkdirGuard;
use serde_json::{Value, json};

static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb_rem_engine_{tag}_{}_{seq}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn make_xy_session(xs: &[f64], ys: &[f64]) -> Session {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    xy.data_sets[0].title = String::from("Drug A");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut doc = ProjectDocument::new("Engine Audit Test").unwrap();
    doc.add_data_sheet(DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).unwrap()).unwrap();
    Session::from_document(doc)
}

fn make_column_session(cols: &[(&str, &[f64])]) -> Session {
    let max_rows = cols.iter().map(|(_, vals)| vals.len()).max().unwrap_or(1);
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), max_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let mut doc = ProjectDocument::new("Column Audit Test").unwrap();
    doc.add_data_sheet(DataSheet::new("col-1", "Groups", TableArchetype::Column(tbl)).unwrap()).unwrap();
    Session::from_document(doc)
}

fn minimal_xlsx_bytes() -> Vec<u8> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>X</t></is></c><c r="B1" t="inlineStr"><is><t>Y</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>10</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><v>20</v></c></row>
  </sheetData>
</worksheet>"#;
    let mut zw = ZipWriter::new();
    zw.add_file("xl/workbook.xml", workbook.as_bytes().to_vec()).unwrap();
    zw.add_file("xl/_rels/workbook.xml.rels", wb_rels.as_bytes().to_vec()).unwrap();
    zw.add_file("xl/worksheets/sheet1.xml", sheet1.as_bytes().to_vec()).unwrap();
    zw.finish().unwrap()
}

// ---------------------------------------------------------------------------
// AUD-02-01..27
// ---------------------------------------------------------------------------

#[test]
fn aud_02_01_exclude_cells_target_x_rejects_nonzero_col_and_counts_accurately() {
    let mut session = make_xy_session(&[1.0, 2.0, 3.0, 4.0], &[10.0, 20.0, 30.0, 40.0]);
    let err = session
        .execute(
            "table.excludeCells",
            json!({
                "sheet_id": "xy-1",
                "target": "x",
                "row_start": 0,
                "row_end": 2,
                "col_start": 0,
                "col_end": 1_000_000_000_usize,
            }),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadInput);

    let out = session
        .execute(
            "table.excludeCells",
            json!({
                "sheet_id": "xy-1",
                "target": "x",
                "row_start": 0,
                "row_end": 2,
                "col_start": 0,
                "col_end": 0,
            }),
        )
        .unwrap();
    assert_eq!(out.payload["updated_count"], 3);
    let ds = session.document().data_sheet(&SheetId::new("xy-1")).unwrap();
    assert!(ds.x_or_time_cell(0, 0).unwrap().excluded);
    assert!(ds.x_or_time_cell(2, 0).unwrap().excluded);
    assert!(!ds.x_or_time_cell(3, 0).unwrap().excluded);
}

#[test]
fn aud_02_02_paste_link_and_clear_cell_range_clamp_unbounded_ranges() {
    let mut session = make_column_session(&[("Ctrl", &[1.0, 2.0]), ("Drug", &[3.0, 4.0])]);
    session.execute("sheet.createData", json!({ "sheet_id": "col-dst", "kind": "column", "cols": 2, "rows": 2 })).unwrap();

    // Unclamped end row/col in pasteLink must clamp rather than hang.
    let out = session
        .execute(
            "table.pasteLink",
            json!({
                "target_sheet_id": "col-dst",
                "source_sheet_id": "col-1",
                "source_range": {
                    "start": { "row": 0, "col": 0, "subcol": 0 },
                    "end": { "row": 1_000_000_000_usize, "col": 1_000_000_000_usize, "subcol": 0 }
                },
                "propagate_title_header": true,
            }),
        )
        .unwrap();
    assert_eq!(out.payload["rows_placed"], 2);
    assert_eq!(out.payload["cols_placed"], 2);

    // Untrusted archive target_range with end.col = usize::MAX must not hang during clear_cell_range.
    let mut doc = session.document().clone();
    let dst = doc.data_sheet_mut(&SheetId::new("col-dst")).unwrap();
    dst.linked_regions[0].target_range = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(usize::MAX, usize::MAX, 0));
    let report = dag::recalculate_project_with_report(&mut doc).unwrap();
    assert!(!report.updated_data_sheets.is_empty());
}

#[test]
fn aud_02_03_randomize_groups_rejects_huge_subject_count_without_oom() {
    let mut session = Session::new();
    let err_huge =
        session.execute("table.randomizeGroups", json!({ "groups": ["Vehicle", "Drug"], "subject_count": 100_000_000_usize })).unwrap_err();
    assert_eq!(err_huge.code, ErrorCode::BadInput);

    let err_over_cap =
        session.execute("table.randomizeGroups", json!({ "groups": ["Vehicle", "Drug"], "subject_count": 1_000_001_usize })).unwrap_err();
    assert_eq!(err_over_cap.code, ErrorCode::BadInput);

    let ok = session.execute("table.randomizeGroups", json!({ "groups": ["Vehicle", "Drug"], "subject_count": 6, "seed": 7 })).unwrap();
    assert_eq!(ok.payload["subject_count"], 6);
}

#[test]
fn aud_02_04_linear_regression_preserves_standard_addition_extend_x_and_deming_error() {
    let mut session = make_xy_session(&[1.0, 2.0, 3.0, 4.0, 5.0], &[3.1, 4.9, 7.2, 8.8, 11.1]);

    session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "lin-std",
                "source_sheet_id": "xy-1",
                "kind": "linear",
                "standard_addition": true,
                "extend_line_to_x_intercept": true,
            }),
        )
        .unwrap();
    let res_lin = session.document().result_sheet(&SheetId::new("lin-std")).unwrap();
    let AnalysisKind::LinearRegression(ref lin) = res_lin.analysis.kind else {
        panic!("expected LinearRegression");
    };
    assert!(lin.standard_addition);
    assert!(lin.extend_line_to_x_intercept);

    session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "dem-eq",
                "source_sheet_id": "xy-1",
                "kind": "deming",
            }),
        )
        .unwrap();
    session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "dem-ratio",
                "source_sheet_id": "xy-1",
                "kind": "deming",
                "deming_error": { "VarianceRatio": { "lambda": 10.0 } },
                "extend_line_to_x_intercept": true,
            }),
        )
        .unwrap();
    let res_eq = session.document().result_sheet(&SheetId::new("dem-eq")).unwrap();
    let res_ratio = session.document().result_sheet(&SheetId::new("dem-ratio")).unwrap();
    let AnalysisKind::LinearRegression(ref dem) = res_ratio.analysis.kind else {
        panic!("expected Deming LinearRegression");
    };
    assert_eq!(dem.deming_error, DemingErrorSpec::VarianceRatio { lambda: 10.0 });
    assert!(dem.extend_line_to_x_intercept);

    let slope_eq = res_eq.scalars.iter().find(|s| s.key == "Slope").unwrap().value;
    let slope_ratio = res_ratio.scalars.iter().find(|s| s.key == "Slope").unwrap().value;
    assert!((slope_eq - slope_ratio).abs() > 1e-6, "Deming VarianceRatio lambda=10 must alter slope vs EqualError lambda=1");
}

#[test]
fn aud_02_06_convert_archetype_remaps_pcr_stratify_and_dunnett_columns() {
    let session = make_xy_session(&[1.0, 2.0, 3.0, 4.0, 5.0], &[10.0, 20.0, 30.0, 40.0, 50.0]);
    let mut doc = session.document().clone();
    // Add 3 analyses whose column indices reference Y column 0 on XY (which shifts to col 1 on XY -> MultipleVariables).
    let pca = AnalysisSpec::new(
        SheetId::new("res-pca"),
        SheetId::new("xy-1"),
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None, pcr_outcome_column: Some(0) }),
    )
    .unwrap();
    let sample = AnalysisSpec::new(
        SheetId::new("res-samp"),
        SheetId::new("xy-1"),
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 1,
            sample_size: 3,
            stratify_column: Some(0),
            proportional_allocation: true,
        }),
    )
    .unwrap();
    let mrow = AnalysisSpec::new(
        SheetId::new("res-mrow"),
        SheetId::new("xy-1"),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 0 }, ..MultiplicitySpec::default() },
            ..MultipleRowTestsSpec::default()
        }),
    )
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(pca).unwrap()).unwrap();
    doc.add_result_sheet(ResultSheet::new(sample).unwrap()).unwrap();
    doc.add_result_sheet(ResultSheet::new(mrow).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    session.execute("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "MultipleVariables", "allow_lossy": true })).unwrap();

    let pca_res = session.document().result_sheet(&SheetId::new("res-pca")).unwrap();
    assert!(matches!(pca_res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: Some(1), .. })));
    let samp_res = session.document().result_sheet(&SheetId::new("res-samp")).unwrap();
    assert!(matches!(samp_res.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { stratify_column: Some(1), .. })));
    let mrow_res = session.document().result_sheet(&SheetId::new("res-mrow")).unwrap();
    assert!(matches!(
        mrow_res.analysis.kind,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 1 }, .. },
            ..
        })
    ));
}

#[test]
fn aud_02_07_frozen_datasheet_protected_from_info_delete_and_upstream_convert() {
    // Case A: deleting InfoSheet paired to frozen DataSheet must fail with FrozenSheet.
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "cols": 1, "rows": 3, "auto_info": true })).unwrap();
    let info_id = session.document().data_sheet(&SheetId::new("d1")).unwrap().info_sheet_id.clone().unwrap();
    session.execute("sheet.freeze", json!({ "sheet_id": "d1" })).unwrap();

    let del_err = session.execute("sheet.delete", json!({ "sheet_id": info_id.as_str(), "cascade": true })).unwrap_err();
    assert_eq!(del_err.code, ErrorCode::FrozenSheet);
    assert_eq!(session.document().data_sheet(&SheetId::new("d1")).unwrap().info_sheet_id.as_ref(), Some(&info_id));

    // Case B: converting upstream XY sheet that shifts/breaks a frozen downstream linked DataSheet must fail with FrozenSheet.
    session.execute("sheet.createData", json!({ "sheet_id": "src-xy", "kind": "xy", "cols": 1, "rows": 3 })).unwrap();
    session.execute("table.setXCells", json!({ "sheet_id": "src-xy", "values": [1.0, 2.0, 3.0] })).unwrap();
    session.execute("table.setCells", json!({ "sheet_id": "src-xy", "rows": [[10.0], [20.0], [30.0]] })).unwrap();
    session.execute("sheet.createData", json!({ "sheet_id": "dst-frozen", "kind": "column", "cols": 1, "rows": 3 })).unwrap();
    session.execute("table.pasteLink", json!({ "target_sheet_id": "dst-frozen", "source_sheet_id": "src-xy" })).unwrap();
    session.execute("sheet.freeze", json!({ "sheet_id": "dst-frozen" })).unwrap();

    let conv_err = session
        .execute("table.convertArchetype", json!({ "sheet_id": "src-xy", "target_kind": "MultipleVariables", "allow_lossy": true }))
        .unwrap_err();
    assert_eq!(conv_err.code, ErrorCode::FrozenSheet);
    assert_eq!(session.document().data_sheet(&SheetId::new("dst-frozen")).unwrap().linked_regions.len(), 1);
}

#[test]
fn aud_02_08_exclude_cells_with_reason_batch_is_sorted_and_deduplicated() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "big", "kind": "column", "cols": 10, "rows": 200 })).unwrap();
    let out = session
        .execute(
            "table.excludeCellsWithReason",
            json!({
                "sheet_id": "big",
                "reason_tag": "qc_batch",
                "note": "plate rim artifact",
                "timestamp_utc": "2026-11-25T00:00:00Z",
                "range": { "start": { "row": 0, "col": 0 }, "end": { "row": 199, "col": 9 } }
            }),
        )
        .unwrap();
    assert_eq!(out.payload["count"], 2000);
    let ds = session.document().data_sheet(&SheetId::new("big")).unwrap();
    assert_eq!(ds.exclusion_notes.len(), 2000);
    assert!(ds.exclusion_notes.windows(2).all(|w| w[0].coord < w[1].coord));

    // Updating a subset overwrites notes without duplicating entries.
    session
        .execute(
            "table.excludeCellsWithReason",
            json!({
                "sheet_id": "big",
                "reason_tag": "updated_reason",
                "timestamp_utc": "2026-11-25T01:00:00Z",
                "coords": [[5, 3], [0, 0]]
            }),
        )
        .unwrap();
    let ds2 = session.document().data_sheet(&SheetId::new("big")).unwrap();
    assert_eq!(ds2.exclusion_notes.len(), 2000);
    assert_eq!(ds2.exclusion_note(CellCoord::new(5, 3, 0)).unwrap().reason_tag, "updated_reason");
}

#[test]
fn aud_02_09_layout_apply_canvas_preset_clamps_callout_font_and_keeps_editable() {
    let mut session = Session::new();
    session.execute("layout.create", json!({ "sheet_id": "lay-1", "width": 2000.0, "height": 2000.0, "unit": "pt" })).unwrap();
    session
        .execute(
            "layout.addPanel",
            json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Small note", "font_size_pt": 1.0, "bounds_pt": [10.0, 10.0, 100.0, 20.0] }),
        )
        .unwrap();

    session
        .execute("layout.applyCanvasPreset", json!({ "sheet_id": "lay-1", "preset": "SingleColumn88mm", "resize_mode": "proportional_scale" }))
        .unwrap();
    let lay = session.document().layout_sheet(&SheetId::new("lay-1")).unwrap();
    let LayoutItem::TextCallout { font_size_pt, .. } = &lay.spec.items[0] else {
        panic!("expected TextCallout");
    };
    assert!(*font_size_pt >= 0.5, "scaled callout font_size_pt ({font_size_pt}) must be clamped >= 0.5");

    // Subsequent edit must succeed without lockout.
    session
        .execute("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Follow-up", "bounds_pt": [5.0, 5.0, 40.0, 15.0] }))
        .unwrap();
}

#[test]
fn aud_02_10_graph_title_synced_across_create_initialize_and_set_spec() {
    let mut session = make_xy_session(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
    session.execute("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy-1", "title": "Created Title" })).unwrap();
    let g = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
    assert_eq!(g.title, "Created Title");
    assert_eq!(g.spec.title, "Created Title");

    session.execute("graph.initialize", json!({ "sheet_id": "g1", "kind": "xy", "title": "Initialized Title" })).unwrap();
    let g = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
    assert_eq!(g.title, "Initialized Title");
    assert_eq!(g.spec.title, "Initialized Title");

    session.execute("graph.setSpec", json!({ "sheet_id": "g1", "title": "SetSpec Title" })).unwrap();
    let g = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
    assert_eq!(g.title, "SetSpec Title");
    assert_eq!(g.spec.title, "SetSpec Title");

    session.execute("graph.setSpec", json!({ "sheet_id": "g1", "title": "Sheet Label", "spec": { "title": "Explicit Inner Spec Title" } })).unwrap();
    let g = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
    assert_eq!(g.title, "Sheet Label");
    assert_eq!(g.spec.title, "Explicit Inner Spec Title");
}

#[test]
fn aud_02_11_import_csv_link_true_mints_unique_ids_and_rejects_overlap() {
    let tmp = TempDir::new("aud0211");
    let csv_a = tmp.path().join("a.csv");
    let csv_b = tmp.path().join("b.csv");
    fs::write(&csv_a, "1,10\n2,20\n").unwrap();
    fs::write(&csv_b, "3,30\n4,40\n").unwrap();

    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 5, "rows": 4 })).unwrap();
    session
        .execute(
            "table.importCsv",
            json!({ "sheet_id": "col-1", "path": csv_a.to_str().unwrap(), "link": true, "has_header": false, "anchor": { "row": 0, "col": 0 } }),
        )
        .unwrap();

    // Overlapping live link import must be rejected.
    let overlap_err = session
        .execute(
            "table.importCsv",
            json!({ "sheet_id": "col-1", "path": csv_b.to_str().unwrap(), "link": true, "has_header": false, "anchor": { "row": 0, "col": 0 } }),
        )
        .unwrap_err();
    assert_eq!(overlap_err.code, ErrorCode::Conflict);

    // Disjoint live link import succeeds with a distinct link ID.
    session
        .execute(
            "table.importCsv",
            json!({ "sheet_id": "col-1", "path": csv_b.to_str().unwrap(), "link": true, "has_header": false, "anchor": { "row": 0, "col": 3 } }),
        )
        .unwrap();
    let ds = session.document().data_sheet(&SheetId::new("col-1")).unwrap();
    assert_eq!(ds.linked_regions.len(), 2);
    assert_ne!(ds.linked_regions[0].id, ds.linked_regions[1].id);
}

#[test]
fn aud_02_12_export_options_csv_with_tab_index_succeeds_when_data_sheet_active() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);
    session.execute("stats.run", json!({ "sheet_id": "res-1", "source_sheet_id": "col-1", "kind": "descriptive" })).unwrap();
    session.execute("sheet.select", json!({ "sheet_id": "col-1" })).unwrap();
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));

    let mut opts = ExportOptionsSpec::new(DocumentExportFormat::Csv);
    opts.tidy = false;
    opts.tab_index = Some(0);
    let req = opts.into_artifact_request(session.document(), true).unwrap();
    let artifact = export_document_artifact(session.document(), &req, None, "test.csv").unwrap();
    assert_eq!(artifact.target_id, "res-1");
    let text = artifact.into_utf8_string().unwrap();
    assert!(!text.trim().is_empty());
}

#[test]
fn aud_02_13_layout_panel_ids_unique_after_deletion_and_duplicate_rejected() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);
    session.execute("stats.run", json!({ "sheet_id": "res-1", "source_sheet_id": "col-1", "kind": "descriptive" })).unwrap();
    session.execute("layout.create", json!({ "sheet_id": "lay-1" })).unwrap();
    session.execute("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "One" })).unwrap();
    session.execute("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Two" })).unwrap();

    // Remove panel-1 via layout.setSpec so only panel-2 remains (len == 1).
    let remaining = session.document().layout_sheet(&SheetId::new("lay-1")).unwrap().spec.items[1].clone();
    session.execute("layout.setSpec", json!({ "sheet_id": "lay-1", "spec": { "items": [remaining] } })).unwrap();

    // Next auto-generated panel ID must not collide with panel-2.
    session.execute("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Three" })).unwrap();
    let items = &session.document().layout_sheet(&SheetId::new("lay-1")).unwrap().spec.items;
    assert_eq!(items.len(), 2);
    assert_ne!(items[0].id(), items[1].id());

    // edit.pasteResultTable onto layout must also avoid panel ID collisions.
    let paste_out = session.execute("edit.pasteResultTable", json!({ "source_sheet_id": "res-1", "target_sheet_id": "lay-1" })).unwrap();
    let pasted_id = paste_out.payload["panel_id"].as_str().unwrap();
    assert_ne!(pasted_id, "panel-2");

    // Explicitly passing a duplicate panel_id must be rejected.
    let dup_err =
        session.execute("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Dup", "panel_id": "panel-2" })).unwrap_err();
    assert_eq!(dup_err.code, ErrorCode::BadInput);
}

#[test]
fn aud_02_14_bracket_label_mode_preserves_case_and_rejects_blank() {
    let mut session = make_column_session(&[("Ctrl", &[1.0, 1.2, 0.9, 1.1]), ("Drug", &[5.0, 5.2, 4.9, 5.1])]);
    session.execute("stats.run", json!({ "sheet_id": "res-t", "source_sheet_id": "col-1", "kind": "unpaired_t" })).unwrap();
    session.execute("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "col-1" })).unwrap();

    let blank_err =
        session.execute("graph.addComparisonBrackets", json!({ "sheet_id": "g1", "result_sheet_id": "res-t", "label_mode": "   " })).unwrap_err();
    assert_eq!(blank_err.code, ErrorCode::BadInput);

    session
        .execute("graph.addComparisonBrackets", json!({ "sheet_id": "g1", "result_sheet_id": "res-t", "label_mode": "P < 0.001 (Bonferroni)" }))
        .unwrap();
    let g = session.document().graph_sheet(&SheetId::new("g1")).unwrap();
    assert!(!g.spec.comparisons.is_empty());
    assert_eq!(g.spec.comparisons[0].label_mode, BracketLabelMode::Custom(String::from("P < 0.001 (Bonferroni)")));
}

#[test]
fn aud_02_15_analysis_rejects_misplaced_create_graph_and_conflicting_spec_ids() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);

    let err_stats = session.execute("stats.run", json!({ "source_sheet_id": "col-1", "kind": "descriptive", "create_graph": true })).unwrap_err();
    assert_eq!(err_stats.code, ErrorCode::BadInput);

    let err_gid = session.execute("transform.run", json!({ "source_sheet_id": "col-1", "kind": "normalize", "graph_sheet_id": "g-99" })).unwrap_err();
    assert_eq!(err_gid.code, ErrorCode::BadInput);

    let err_recalc = session.execute("analysis.run", json!({ "create_graph": true })).unwrap_err();
    assert_eq!(err_recalc.code, ErrorCode::BadInput);

    let spec = AnalysisSpec::new(SheetId::new("res-spec"), SheetId::new("col-1"), AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let spec_json = serde_json::to_value(&spec).unwrap();
    let err_conflict_id = session.execute("analysis.run", json!({ "sheet_id": "other-id", "spec": spec_json })).unwrap_err();
    assert_eq!(err_conflict_id.code, ErrorCode::BadInput);

    let err_conflict_src =
        session.execute("analysis.run", json!({ "source_sheet_id": "other-src", "spec": serde_json::to_value(&spec).unwrap() })).unwrap_err();
    assert_eq!(err_conflict_src.code, ErrorCode::BadInput);
}

#[test]
fn aud_02_16_transform_simulate_rejects_zero_row_count_and_replicates() {
    let mut session = make_xy_session(&[1.0, 2.0], &[10.0, 20.0]);
    let err_rows =
        session.execute("transform.run", json!({ "source_sheet_id": "xy-1", "kind": "simulate", "row_count": 0, "replicates": 2 })).unwrap_err();
    assert_eq!(err_rows.code, ErrorCode::BadInput);

    let err_reps =
        session.execute("transform.run", json!({ "source_sheet_id": "xy-1", "kind": "simulate", "row_count": 5, "replicates": 0 })).unwrap_err();
    assert_eq!(err_reps.code, ErrorCode::BadInput);

    // row_count = 1 must NOT be silently clamped to 2.
    session
        .execute("transform.run", json!({ "sheet_id": "sim-one", "source_sheet_id": "xy-1", "kind": "simulate", "row_count": 1, "replicates": 1 }))
        .unwrap();
    let res = session.document().result_sheet(&SheetId::new("sim-one")).unwrap();
    assert_eq!(res.analyzable_data.as_ref().unwrap().archetype.row_count(), 1);
}

#[test]
fn aud_02_17_inspect_counts_normalize_hooked_anchors() {
    let mut session = make_column_session(&[("A", &[10.0, 50.0, 90.0])]);
    let mut doc = session.document().clone();
    let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Anchors").unwrap();
    info.set_constant("Zero", curvebook_doc::ConstantValue::Number(0.0), None).unwrap();
    info.set_constant("Hundred", curvebook_doc::ConstantValue::Number(100.0), None).unwrap();
    doc.add_info_sheet(info).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("res-norm"),
        SheetId::new("col-1"),
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("Zero"),
            }),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("Hundred"),
            }),
            output_percent: true,
            per_data_set: true,
        }),
    )
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();
    session = Session::from_document(doc);

    let inspect = session.execute("project.inspect", json!({})).unwrap();
    let hook_count = inspect.payload["result_sheets"][0]["hook_count"].as_u64().unwrap();
    assert_eq!(hook_count, 2);
}

#[test]
fn aud_02_18_recalculate_external_links_enforces_path_policy() {
    let tmp = TempDir::new("aud0218");
    let csv_path = tmp.path().join("ext.csv");
    fs::write(&csv_path, "1\n2\n").unwrap();

    let mut session = make_column_session(&[("A", &[1.0, 2.0])]);
    session
        .execute(
            "table.pasteLink",
            json!({ "target_sheet_id": "col-1", "source_path": csv_path.to_str().unwrap(), "mode": "overwrite", "anchor": { "row": 0, "col": 0 } }),
        )
        .unwrap();

    session.set_path_policy(ExternalPathPolicy::RejectExternal);
    let err = session.execute("project.recalculate", json!({ "include_external_links": true })).unwrap_err();
    assert_eq!(err.code, ErrorCode::PathEscape);
}

#[test]
fn aud_02_19_exclude_cells_with_reason_forces_wal_checkpoint() {
    assert!(Session::command_requires_wal_checkpoint("table.excludeCellsWithReason"));

    let tmp = TempDir::new("aud0219");
    let proj_dir = tmp.path().join("wal_dir");

    let (mut session, mut journal, _rep) = Session::open_dir_with_recovery(&proj_dir, "owner-1", 60_000, 1_000).unwrap();
    session.execute_and_record(&mut journal, "sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 1, "rows": 3 })).unwrap();
    let out = session
        .execute_and_record(
            &mut journal,
            "table.excludeCellsWithReason",
            json!({ "sheet_id": "col-1", "reason_tag": "artifact", "row": 0, "col": 0 }),
        )
        .unwrap();
    let minted_ts = out.payload["timestamp_utc"].as_str().unwrap().to_owned();
    assert!(!minted_ts.is_empty());

    // Simulate crash (drop without close_cleanly) and recover after lease TTL.
    drop(journal);
    drop(session);
    let (reopened, _journal2, rep2) = Session::open_dir_with_recovery(&proj_dir, "owner-2", 60_000, 200_000).unwrap();
    assert!(rep2.recovered_commands.is_empty(), "table.excludeCellsWithReason must checkpoint rather than remain as an uncheckpointed WAL entry");
    let recovered_note = reopened.document().data_sheet(&SheetId::new("col-1")).unwrap().exclusion_note(CellCoord::new(0, 0, 0)).unwrap();
    assert_eq!(recovered_note.timestamp_utc, minted_ts);
}

#[test]
fn aud_02_20_sheet_rename_rejects_whitespace_only_title() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0, 3.0])]);
    session.execute("stats.run", json!({ "sheet_id": "res-1", "source_sheet_id": "col-1", "kind": "descriptive" })).unwrap();

    let err_data = session.execute("sheet.rename", json!({ "sheet_id": "col-1", "title": "   \t  " })).unwrap_err();
    assert_eq!(err_data.code, ErrorCode::BadInput);
    assert_eq!(session.document().data_sheet(&SheetId::new("col-1")).unwrap().title, "Groups");

    let err_res = session.execute("sheet.rename", json!({ "sheet_id": "res-1", "title": "   " })).unwrap_err();
    assert_eq!(err_res.code, ErrorCode::BadInput);
    assert!(session.document().result_sheet(&SheetId::new("res-1")).unwrap().analysis.custom_title.is_none());
}

#[test]
fn aud_02_21_export_pdf_supports_grayscale_and_rejects_both_sheet_id_and_sheet_ids() {
    let mut session = make_xy_session(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
    session.execute("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy-1", "kind": "xy" })).unwrap();

    let gray = session.execute("export.pdf", json!({ "sheet_id": "g1", "color_space": "grayscale" })).unwrap();
    assert!(gray.payload["byte_len"].as_u64().unwrap() > 100);
    assert_eq!(gray.payload["color_space"], "Grayscale");

    let both_err = session.execute("export.pdf", json!({ "sheet_id": "g1", "sheet_ids": ["g1"] })).unwrap_err();
    assert_eq!(both_err.code, ErrorCode::BadInput);
}

#[test]
fn aud_02_22_strict_validation_across_annotation_paste_table_tidy_and_info_constant() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);
    session.execute("stats.run", json!({ "sheet_id": "res-1", "source_sheet_id": "col-1", "kind": "descriptive" })).unwrap();
    session.execute("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "col-1" })).unwrap();

    // 1. graph.addAnnotation
    let err_combo = session
        .execute(
            "graph.addAnnotation",
            json!({ "sheet_id": "g1", "annotation": { "TextBox": { "bounds_pt": { "x": 10.0, "y": 10.0, "width": 80.0, "height": 20.0 }, "markdown": "hi", "bordered": false } }, "kind": "text" }),
        )
        .unwrap_err();
    assert_eq!(err_combo.code, ErrorCode::BadInput);

    let err_unknown_key =
        session.execute("graph.addAnnotation", json!({ "sheet_id": "g1", "kind": "text", "text": "hi", "bordred": true })).unwrap_err();
    assert_eq!(err_unknown_key.code, ErrorCode::BadInput);

    let err_bad_num =
        session.execute("graph.addAnnotation", json!({ "sheet_id": "g1", "kind": "text", "text": "hi", "x_pt": "not_a_number" })).unwrap_err();
    assert_eq!(err_bad_num.code, ErrorCode::BadInput);

    let err_shape = session.execute("graph.addAnnotation", json!({ "sheet_id": "g1", "kind": "shape", "shape": "triangle" })).unwrap_err();
    assert_eq!(err_shape.code, ErrorCode::BadInput);

    // 2. edit.pasteResultTable unknown style key
    let err_style = session
        .execute("edit.pasteResultTable", json!({ "source_sheet_id": "res-1", "target_sheet_id": "g1", "style": { "bogus_key": true } }))
        .unwrap_err();
    assert_eq!(err_style.code, ErrorCode::BadInput);

    // 3. table.importTidyLong conflicting mapping + uppercase decimal_sep
    let err_tidy_combo = session
        .execute(
            "table.importTidyLong",
            json!({
                "headers": ["grp", "val"],
                "rows": [["A", "1,5"], ["B", "2,5"]],
                "mapping": { "group_col": "grp", "value_col": "val" },
                "group_col": "grp"
            }),
        )
        .unwrap_err();
    assert_eq!(err_tidy_combo.code, ErrorCode::BadInput);

    session
        .execute(
            "table.importTidyLong",
            json!({
                "sheet_id": "tidy-ok",
                "headers": ["grp", "val"],
                "rows": [["A", "1,5"], ["B", "2,5"]],
                "decimal_sep": "COMMA",
                "group_col": "grp",
                "value_col": "val"
            }),
        )
        .unwrap();

    // 4. info.setConstant contradictory value/kind pairs
    session.execute("sheet.createData", json!({ "sheet_id": "d-info", "kind": "column", "cols": 1, "rows": 2, "auto_info": true })).unwrap();
    let err_null_num =
        session.execute("info.setConstant", json!({ "sheet_id": "d-info", "name": "k1", "value": null, "kind": "number" })).unwrap_err();
    assert_eq!(err_null_num.code, ErrorCode::BadInput);

    let err_num_date = session.execute("info.setConstant", json!({ "sheet_id": "d-info", "name": "k2", "value": 42.0, "kind": "date" })).unwrap_err();
    assert_eq!(err_num_date.code, ErrorCode::BadInput);

    let err_str_empty =
        session.execute("info.setConstant", json!({ "sheet_id": "d-info", "name": "k3", "value": "abc", "kind": "empty" })).unwrap_err();
    assert_eq!(err_str_empty.code, ErrorCode::BadInput);
}

#[test]
fn aud_02_23_power_info_hooks_fail_on_oob_and_methods_report_resolves_n() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 2, "rows": 2, "auto_info": true })).unwrap();
    let info_id = session.document().data_sheet(&SheetId::new("col-1")).unwrap().info_sheet_id.clone().unwrap();
    session.execute("info.setConstant", json!({ "sheet_id": info_id.as_str(), "name": "Alpha", "value": 0.05 })).unwrap();
    session.execute("info.setConstant", json!({ "sheet_id": info_id.as_str(), "name": "N per group", "value": 25.0 })).unwrap();

    session
        .execute(
            "stats.run",
            json!({
                "sheet_id": "pwr-1",
                "source_sheet_id": "col-1",
                "kind": "power",
                "solve_target": "Power",
                "info_sheet_id": info_id.as_str(),
                "sample_size": 10,
                "effect_size": 0.8,
            }),
        )
        .unwrap();

    // Mark dirty and verify methods_report dirty fallback picks up `N per group = 25` (not stale 10).
    let mut dirty_doc = session.document().clone();
    dirty_doc.result_sheet_mut(&SheetId::new("pwr-1")).unwrap().status = ResultStatus::Dirty;
    let report = generate_methods_report(&dirty_doc, &SheetId::new("pwr-1"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("25"), "dirty methods report fallback must resolve `N per group` = 25; got: {report}");

    // Out-of-range Alpha = 1.5 must fail the result sheet rather than silently falling back.
    session.execute("info.setConstant", json!({ "sheet_id": info_id.as_str(), "name": "Alpha", "value": 1.5 })).unwrap();
    let res = session.document().result_sheet(&SheetId::new("pwr-1")).unwrap();
    assert!(matches!(res.status, ResultStatus::Failed { .. }), "expected Failed status on out-of-range Alpha=1.5, got {:?}", res.status);
}

#[test]
fn aud_02_24_format_selected_points_clear_removes_stale_oob_override() {
    let mut session = make_xy_session(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
    session.execute("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy-1", "kind": "xy" })).unwrap();

    // Inject a stale out-of-bounds point override (e.g. from shortened series or loaded archive).
    let mut doc = session.document().clone();
    doc.graph_sheet_mut(&SheetId::new("g1")).unwrap().spec.point_overrides.push(PointStyleOverride {
        series_index: 0,
        point_index: 99,
        subcol: None,
        hidden: true,
        ..PointStyleOverride::default()
    });
    session = Session::from_document(doc);

    let out = session
        .execute("graph.formatSelectedPoints", json!({ "sheet_id": "g1", "mode": "clear", "points": [{ "series_index": 0, "point_index": 99 }] }))
        .unwrap();
    assert_eq!(out.payload["override_count"], 0);
    assert!(session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec.point_overrides.is_empty());
}

#[test]
fn aud_02_25_conflicting_deprecated_alias_rejected_and_equal_accepted() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0])]);

    let conflict_err = session.execute("sheet.rename", json!({ "sheet_id": "col-1", "title": "Canonical", "new_title": "Conflicting" })).unwrap_err();
    assert_eq!(conflict_err.code, ErrorCode::BadInput);
    assert_eq!(conflict_err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/new_title"));

    let equal_ok = session.execute("sheet.rename", json!({ "sheet_id": "col-1", "title": "Same Title", "new_title": "Same Title" })).unwrap();
    assert_eq!(session.document().data_sheet(&SheetId::new("col-1")).unwrap().title, "Same Title");
    assert!(!equal_ok.warnings.is_empty());
}

#[test]
fn aud_02_26_error_messages_do_not_advertise_retired_aliases() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0])]);
    session.execute("sheet.createData", json!({ "sheet_id": "mv-1", "kind": "multiple_variables", "cols": 2, "rows": 2 })).unwrap();
    session.execute("layout.create", json!({ "sheet_id": "lay-1" })).unwrap();

    let e1 = session.execute("table.setCells", json!({ "sheet_id": "col-1" })).unwrap_err().to_string();
    assert!(!e1.contains("`cells`"), "unexpected retired alias in: {e1}");

    let e2 = session.execute("table.paste", json!({ "sheet_id": "col-1" })).unwrap_err().to_string();
    assert!(!e2.contains("`tsv`") && !e2.contains("`grid`"), "unexpected retired alias in: {e2}");

    let e3 = session.execute("table.pasteLink", json!({ "target_sheet_id": "col-1" })).unwrap_err().to_string();
    assert!(e3.contains("`source_path`") && !e3.contains("`path`"), "unexpected retired alias in: {e3}");

    let e4 = session.execute("table.importCsv", json!({})).unwrap_err().to_string();
    assert!(!e4.contains("/`text`") && !e4.contains("`csv`/`text`"), "unexpected retired alias in: {e4}");

    let e5 = session.execute("table.importPzfx", json!({})).unwrap_err().to_string();
    assert!(!e5.contains("`xml`/`text`"), "unexpected retired alias in: {e5}");

    let e6 = session.execute("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "text" })).unwrap_err().to_string();
    assert!(!e6.contains("`text`"), "unexpected retired alias in: {e6}");

    let e7 = session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "formula": "1 + 1" })).unwrap_err().to_string();
    assert!(!e7.contains("`column`"), "unexpected retired alias in: {e7}");
}

#[test]
fn aud_02_27_error_codes_not_found_and_cycle_detected() {
    let session = make_column_session(&[("A", &[1.0, 2.0])]);
    let nf_err = generate_methods_report(session.document(), &SheetId::new("missing-res"), ReportFormat::PlainText).unwrap_err();
    assert_eq!(nf_err.code, ErrorCode::NotFound);

    let mut doc = session.document().clone();
    let cyclic_spec =
        AnalysisSpec::new(SheetId::new("self-cyc"), SheetId::new("self-cyc"), AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    doc.result_sheets.push(ResultSheet::new(cyclic_spec).unwrap());
    let cyc_err = dag::topological_order(&doc).unwrap_err();
    assert_eq!(cyc_err.code, ErrorCode::CycleDetected);
}

// ---------------------------------------------------------------------------
// AUD-13 Engine Halves & EXT-01
// ---------------------------------------------------------------------------

#[test]
fn aud_13_01_wal_replay_blocks_filesystem_commands_and_supports_policy_and_lease_only() {
    let tmp = TempDir::new("aud1301");
    let proj_dir = tmp.path().join("wal_fs");
    let secret_csv = tmp.path().join("host_secret.csv");
    fs::write(&secret_csv, "X,Y\n1,99\n2,199\n").unwrap();

    // Seed a valid snapshot + a crafted checksummed WAL entry invoking a filesystem-reading command.
    let (mut seed_session, mut seed_journal, _) = Session::open_dir_with_recovery(&proj_dir, "seed", 60_000, 1_000).unwrap();
    seed_session
        .execute_and_record(&mut seed_journal, "sheet.createData", json!({ "sheet_id": "xy-1", "kind": "xy", "cols": 1, "rows": 2 }))
        .unwrap();
    seed_session.checkpoint_journal(&mut seed_journal).unwrap();
    seed_session
        .record_to_journal(&mut seed_journal, "table.importCsv", &json!({ "sheet_id": "stolen", "path": secret_csv.to_str().unwrap() }))
        .unwrap();
    drop(seed_journal);
    drop(seed_session);

    // Strict policy-aware open must reject filesystem commands during WAL replay.
    let err = Session::open_dir_with_recovery_and_policy(&proj_dir, "owner-strict", 60_000, 200_000, ExternalPathPolicy::RejectExternal, false)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PathEscape);

    // Best-effort open skips the unreplayable filesystem command with a structured warning and never imports it.
    let (best_effort, journal_be, rep_be) = Session::open_dir_with_best_effort_recovery(&proj_dir, "owner-be", 60_000, 400_000).unwrap();
    assert!(best_effort.document().data_sheet(&SheetId::new("stolen")).is_none());
    assert!(rep_be.warnings.iter().any(|w| w.kind == "skipped_unreplayable_command" && w.message.contains("table.importCsv")));
    drop(journal_be);
    drop(best_effort);

    // Lease-only open skips replay completely while claiming the lease.
    let (lease_journal, lease_rep) = Session::open_dir_journal_lease_only(&proj_dir, "owner-lease", 60_000, 600_000).unwrap();
    assert!(lease_rep.recovered_commands.is_empty());
    drop(lease_journal);
}

#[test]
fn aud_13_02_external_path_policy_enforced_for_ui_caller_on_csv_and_xlsx_import() {
    let tmp = TempDir::new("aud1302");
    let workdir = tmp.path().join("sandbox");
    fs::create_dir_all(&workdir).unwrap();
    let outside_csv = tmp.path().join("outside.csv");
    let outside_xlsx = tmp.path().join("outside.xlsx");
    fs::write(&outside_csv, "X,Y\n1,2\n").unwrap();
    fs::write(&outside_xlsx, minimal_xlsx_bytes()).unwrap();

    let guard = WorkdirGuard::new(&workdir).unwrap();
    let mut session = Session::new();
    session.set_path_policy(ExternalPathPolicy::Workdir(guard));
    let cancel = AtomicBool::new(false);

    let csv_err = session
        .execute_with_caller("table.importCsv", json!({ "path": outside_csv.to_str().unwrap(), "link": false }), Caller::Ui, &cancel)
        .unwrap_err();
    assert_eq!(csv_err.code, ErrorCode::PathEscape);

    let xlsx_err =
        session.execute_with_caller("table.importXlsx", json!({ "path": outside_xlsx.to_str().unwrap() }), Caller::Ui, &cancel).unwrap_err();
    assert_eq!(xlsx_err.code, ErrorCode::PathEscape);
}

#[test]
fn aud_13_04_dag_preserves_failed_status_from_ok_nonlinear_fit() {
    // Pathological non-monotonic / constant data with max_iters = 1 triggers FitStatus::HitMaxIters -> ResultStatus::Failed.
    let mut session = make_xy_session(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[100.0, 1.0, 99.0, 2.0, 98.0, 3.0]);
    let mut doc = session.document().clone();
    let mut nl = curvebook_doc::NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.max_iters = 1;
    let spec = AnalysisSpec::new(SheetId::new("fit-fail"), SheetId::new("xy-1"), AnalysisKind::NonlinearRegression(nl)).unwrap();
    doc.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();

    let report = dag::recalculate_project_with_report(&mut doc).unwrap();
    let res = doc.result_sheet(&SheetId::new("fit-fail")).unwrap();
    assert!(matches!(res.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }), "expected SolverFailed, got {:?}", res.status);
    assert!(res.scalars.is_empty(), "failed nonlinear fit must not leak scalars");
    assert!(res.fitted_curves.is_empty(), "failed nonlinear fit must not leak fitted curves");
    assert!(report.failed_result_sheets.iter().any(|f| f.as_str() == "fit-fail"));

    // Also verify command-level fit.run surfaces SolverFailed rather than reporting UpToDate.
    let cmd_err = session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-fail-cmd",
                "source_sheet_id": "xy-1",
                "equation_id": "log_agonist_vs_response_4pl",
                "max_iters": 1
            }),
        )
        .unwrap_err();
    assert_eq!(cmd_err.code, ErrorCode::SolverFailed);
}

#[test]
fn aud_13_07_graph_source_resolution_prefers_analyzable_data_and_skips_failed() {
    let mut session = make_xy_session(&[1.0, 2.0, 3.0, 4.0], &[10.0, 40.0, 70.0, 100.0]);
    // 1-hop transform: normalize -> 2-hop transform: prune_rows (2 rows) -> terminal descriptive stats (3rd hop).
    session.execute("transform.run", json!({ "sheet_id": "norm-1", "source_sheet_id": "xy-1", "kind": "normalize" })).unwrap();
    session.execute("transform.run", json!({ "sheet_id": "prune-2", "source_sheet_id": "norm-1", "kind": "prune_rows", "every_k_rows": 2 })).unwrap();
    session.execute("stats.run", json!({ "sheet_id": "desc-3", "source_sheet_id": "prune-2", "kind": "descriptive" })).unwrap();

    // 2-hop analyzable chain: graph on `prune-2` uses its 2-row analyzable_data (row 2 is OOB vs raw 4 rows).
    session.execute("graph.create", json!({ "sheet_id": "g-prune", "data_sheet_id": "prune-2" })).unwrap();
    let pruned_rows = session.document().result_sheet(&SheetId::new("prune-2")).unwrap().analyzable_data.as_ref().unwrap().archetype.row_count();
    assert_eq!(pruned_rows, 2);
    let oob_err = session
        .execute(
            "graph.formatSelectedPoints",
            json!({ "sheet_id": "g-prune", "overrides": [{ "series_index": 0, "point_index": 2, "hidden": true }] }),
        )
        .unwrap_err();
    assert_eq!(oob_err.code, ErrorCode::BadInput);

    // Diagnostic graph on terminal `desc-3` (tab_rows > 2-row upstream `prune-2`) preserves `result_row_upper` tab_rows bounds (not shadowed by upstream DataSheet).
    let g_out = session.execute("graph.create", json!({ "sheet_id": "g-chain", "data_sheet_id": "desc-3", "kind": "ResidualPlot" })).unwrap();
    assert_eq!(g_out.payload["data_sheet_id"], "desc-3");
    assert_eq!(g_out.payload["title"], "Descriptive statistics");
    let desc_tab_rows = session.document().result_sheet(&SheetId::new("desc-3")).unwrap().tabs.iter().map(|t| t.rows.len()).max().unwrap_or(0);
    assert!(desc_tab_rows > 4, "expected descriptive stats tab rows > raw 4 rows, got {desc_tab_rows}");
    session
        .execute(
            "graph.formatSelectedPoints",
            json!({ "sheet_id": "g-chain", "overrides": [{ "series_index": 0, "point_index": desc_tab_rows - 1, "hidden": true }] }),
        )
        .unwrap();

    // Methods report resolves 2-hop source cleanly without falling back to raw upstream data.
    let report = generate_methods_report(session.document(), &SheetId::new("desc-3"), ReportFormat::PlainText).unwrap();
    assert!(!report.is_empty());

    // If a ResultSheet is marked Failed (directly or upstream), `graph.create` AND `graph.formatSelectedPoints` must reject with Conflict rather than falling back to raw data.
    let mut doc = session.document().clone();
    doc.result_sheet_mut(&SheetId::new("norm-1")).unwrap().status =
        ResultStatus::Failed { code: ErrorCode::SolverFailed, message: String::from("boom") };
    let mut failed_session = Session::from_document(doc);
    let direct_fail = failed_session.execute("graph.create", json!({ "sheet_id": "g-fail-1", "data_sheet_id": "norm-1" })).unwrap_err();
    assert_eq!(direct_fail.code, ErrorCode::Conflict);

    // Upstream failure behind a terminal result sheet must also reject both `graph.create` and `graph.formatSelectedPoints` without falling back to raw `xy-1`.
    let mut doc2 = session.document().clone();
    doc2.result_sheet_mut(&SheetId::new("prune-2")).unwrap().status =
        ResultStatus::Failed { code: ErrorCode::SolverFailed, message: String::from("upstream failed") };
    let mut failed_chain = Session::from_document(doc2);
    let chain_fail = failed_chain.execute("graph.create", json!({ "sheet_id": "g-fail-2", "data_sheet_id": "desc-3" })).unwrap_err();
    assert_eq!(chain_fail.code, ErrorCode::Conflict);
    let bounds_fail = failed_chain
        .execute(
            "graph.formatSelectedPoints",
            json!({ "sheet_id": "g-chain", "overrides": [{ "series_index": 0, "point_index": 0, "hidden": true }] }),
        )
        .unwrap_err();
    assert_eq!(bounds_fail.code, ErrorCode::Conflict);
    let direct_bounds_fail = failed_chain
        .execute(
            "graph.formatSelectedPoints",
            json!({ "sheet_id": "g-prune", "overrides": [{ "series_index": 0, "point_index": 0, "hidden": true }] }),
        )
        .unwrap_err();
    assert_eq!(direct_bounds_fail.code, ErrorCode::Conflict);
}

#[test]
fn aud_13_14_import_xlsx_honors_explicit_title_override() {
    let mut session = Session::new();
    let b64 = curvebook_platform::base64::encode(&minimal_xlsx_bytes());
    let out = session.execute("table.importXlsx", json!({ "bytes_base64": b64, "title": "Custom Workbook Title" })).unwrap();
    assert_eq!(out.payload["title"], "Custom Workbook Title");
    assert_eq!(session.document().title, "Custom Workbook Title");

    let empty_err = session
        .execute("table.importXlsx", json!({ "bytes_base64": curvebook_platform::base64::encode(&minimal_xlsx_bytes()), "title": "   " }))
        .unwrap_err();
    assert_eq!(empty_err.code, ErrorCode::BadInput);
}

#[test]
fn aud_13_15_update_params_creates_summary_graph_when_enabled() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 7).unwrap();
    xy.data_sets[0].title = String::from("Compound 1");
    xy.data_sets[1].title = String::from("Compound 2");
    let xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0];
    let y1 = [2.0, 5.0, 20.0, 52.0, 82.0, 95.0, 99.0];
    let y2 = [1.0, 3.0, 10.0, 30.0, 68.0, 90.0, 98.0];
    for r in 0..xs.len() {
        xy.x_cells[r][0] = Cell::number(xs[r]);
        xy.data_sets[0].cells[r][0] = Cell::number(y1[r]);
        xy.data_sets[1].cells[r][0] = Cell::number(y2[r]);
    }
    let mut doc = ProjectDocument::new("Summary Graph Update").unwrap();
    doc.add_data_sheet(DataSheet::new("xy-1", "Assay", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    let fit_out = session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-1",
                "source_sheet_id": "xy-1",
                "equation_id": "log_agonist_vs_response_4pl",
                "create_summary_graph": false
            }),
        )
        .unwrap();
    assert!(fit_out.payload.get("summary_graph_sheet_id").is_none());
    assert_eq!(session.document().graph_sheets.len(), 0);

    let upd_out = session
        .execute(
            "analysis.updateParams",
            json!({
                "sheet_id": "fit-1",
                "summary_parameter": "LogEC50",
                "create_summary_graph": true
            }),
        )
        .unwrap();
    let sg_id = upd_out.payload["summary_graph_sheet_id"].as_str().expect("updateParams must create summary graph");
    assert_eq!(session.document().graph_sheets.len(), 1);
    assert_eq!(session.document().graph_sheet(&SheetId::new(sg_id)).unwrap().spec.primary_source_sheet_id.as_str(), "fit-1");

    // Re-updating without changing create_summary_graph must NOT duplicate the summary graph.
    let upd2 = session.execute("analysis.updateParams", json!({ "sheet_id": "fit-1", "confidence_percent": 90.0 })).unwrap();
    assert!(upd2.payload.get("summary_graph_sheet_id").is_none());
    assert_eq!(session.document().graph_sheets.len(), 1);
}

#[test]
fn aud_13_19_engine_exports_preserve_outliers_pred_bands_analyzable_and_footnotes() {
    let mut doc = ProjectDocument::new("Export Fidelity").unwrap();
    let ds = DataSheet::new("xy-1", "Raw", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).unwrap())).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    let spec =
        AnalysisSpec::new(SheetId::new("res-exp"), SheetId::new("xy-1"), AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    let mut res = ResultSheet::new(spec).unwrap();
    res.status = ResultStatus::UpToDate;
    res.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("r0")],
        rows: vec![vec![ResultCell::Text(String::from("Slope")), ResultCell::Number { value: 2.0, prefix: None, units: None }]],
        footnotes: vec![String::from("Footnote: 95% bands included.")],
    });
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint { x: 1.0, y: 2.0, ci_lower: Some(1.8), ci_upper: Some(2.2), pred_lower: Some(1.2), pred_upper: Some(2.8) }],
    });
    res.eliminated_outliers.push(SourceCellId { sheet_id: SheetId::new("xy-1"), row: 1, col: 0, subcol: 0 });
    res.analyzable_data = Some(ds);
    doc.add_result_sheet(res).unwrap();
    let mut session = Session::from_document(doc);

    let json_out = session.execute("analysis.exportJson", json!({ "sheet_id": "res-exp" })).unwrap();
    let json_str = json_out.payload["json"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["eliminated_outliers"].as_array().unwrap().len(), 1);
    assert!(parsed["analyzable_data"].is_object());
    assert_eq!(parsed["fitted_curves"][0]["points"][0]["pred_lower"], 1.2);
    assert_eq!(parsed["fitted_curves"][0]["points"][0]["pred_upper"], 2.8);
    assert_eq!(parsed["tabs"][0]["footnotes"][0], "Footnote: 95% bands included.");

    let csv_out = session.execute("table.exportCsv", json!({ "sheet_id": "res-exp" })).unwrap();
    let csv_str = csv_out.payload["csv"].as_str().unwrap();
    assert!(csv_str.contains("pred_lower") && csv_str.contains("pred_upper"));
    assert!(csv_str.contains("1.2") && csv_str.contains("2.8"));
    assert!(csv_str.contains("eliminated_outlier"));
    assert!(csv_str.contains("Footnote: 95% bands included."));
}

#[test]
fn aud_13_22_cooperative_cancellation_stops_recalculate_and_dag() {
    let mut session = make_column_session(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);
    let mut doc = session.document().clone();
    let spec = AnalysisSpec::new(SheetId::new("res-c"), SheetId::new("col-1"), AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    doc.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();

    let cancelled = AtomicBool::new(true);
    let dag_err = dag::recalculate_project_with_report_cancellable(&mut doc, Some(&cancelled)).unwrap_err();
    assert_eq!(dag_err.code, ErrorCode::Cancelled);
    assert_eq!(doc.result_sheet(&SheetId::new("res-c")).unwrap().status, ResultStatus::Dirty);

    session = Session::from_document(doc);
    let cmd_err = session.execute_with_caller("project.recalculate", json!({}), Caller::Cli, &cancelled).unwrap_err();
    assert_eq!(cmd_err.code, ErrorCode::Cancelled);

    let analyze_err = session.execute_with_caller("analyze.run", json!({}), Caller::Cli, &cancelled).unwrap_err();
    assert_eq!(analyze_err.code, ErrorCode::Cancelled);
    assert_eq!(session.document().result_sheet(&SheetId::new("res-c")).unwrap().status, ResultStatus::Dirty);
}

#[test]
fn aud_13_23_path_policy_keys_complete_and_save_preserves_canonical_path() {
    for required_key in ["dir", "directory", "destination_path", "unpacked_dir", "output_path"] {
        assert!(PATH_PARAM_KEYS.contains(&required_key), "PATH_PARAM_KEYS missing `{required_key}`");
    }

    let tmp = TempDir::new("aud1323");
    let sub = tmp.path().join("sub");
    fs::create_dir_all(&sub).unwrap();
    let guard = WorkdirGuard::new(tmp.path()).unwrap();

    let mut session = Session::new();
    session.set_path_policy(ExternalPathPolicy::Workdir(guard));
    let out = session.execute("project.save", json!({ "path": "sub/saved.curvebook" })).unwrap();
    let reported = out.payload["path"].as_str().unwrap();
    let expected_canon = fs::canonicalize(tmp.path().join("sub/saved.curvebook")).unwrap();
    assert_eq!(Path::new(reported), expected_canon.as_path(), "project.save must report canonicalized path");
    assert_eq!(session.file_path(), Some(expected_canon.as_path()), "session.file_path must store canonicalized path");

    // Calling set_file_path with an uncanonicalized relative path under Workdir policy also canonicalizes it.
    session.set_file_path(Some(PathBuf::from("sub/saved.curvebook")), false);
    assert_eq!(session.file_path(), Some(expected_canon.as_path()));
}

#[test]
fn aud_13_25_methods_report_deming_reports_x_intercept_and_plot_bounds() {
    let mut session = make_xy_session(&[2.0, 4.0, 6.0, 8.0, 10.0], &[5.1, 8.9, 13.2, 16.8, 21.1]);
    session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "dem-1",
                "source_sheet_id": "xy-1",
                "kind": "deming",
                "extend_line_to_x_intercept": true,
                "plot_x_min": 0.0,
                "plot_x_max": 25.0
            }),
        )
        .unwrap();
    let report = generate_methods_report(session.document(), &SheetId::new("dem-1"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("X-intercept"), "Deming methods report must mention X-intercept extension: {report}");
    assert!(report.contains("25"), "Deming methods report must mention custom plot X bounds: {report}");
}

#[test]
fn ext_01_fit_default_equation_uses_x_scale_and_surfaces_warnings() {
    // 1. Raw non-log10 concentrations -> default fit.run selects agonist_vs_response_4pl.
    let raw_xs = [0.001, 0.01, 0.1, 1.0, 10.0, 100.0, 1000.0];
    let raw_ys = [2.0, 5.0, 18.0, 50.0, 82.0, 95.0, 99.0];
    let mut raw_session = make_xy_session(&raw_xs, &raw_ys);
    let raw_out = raw_session.execute("fit.run", json!({ "sheet_id": "fit-raw", "source_sheet_id": "xy-1" })).unwrap();
    assert_eq!(raw_out.payload["equation_id"], "agonist_vs_response_4pl");
    let AnalysisKind::NonlinearRegression(ref nl_raw) = raw_session.document().result_sheet(&SheetId::new("fit-raw")).unwrap().analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    assert_eq!(nl_raw.equation_id, "agonist_vs_response_4pl");

    // 2. Log10 concentrations -> default fit.run selects log_agonist_vs_response_4pl.
    let log_xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0];
    let mut log_session = make_xy_session(&log_xs, &raw_ys);
    let log_out = log_session.execute("fit.run", json!({ "sheet_id": "fit-log", "source_sheet_id": "xy-1" })).unwrap();
    assert_eq!(log_out.payload["equation_id"], "log_agonist_vs_response_4pl");

    // 3. Forcing log_agonist_vs_response_4pl on raw large concentrations surfaces scale_warnings in payload & CommandOutcome.warnings.
    let mismatch_xs = [1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0];
    let mismatch_ys = [2.0, 12.0, 48.0, 85.0, 97.0, 99.0];
    let mut mismatch_session = make_xy_session(&mismatch_xs, &mismatch_ys);
    let mis_out = mismatch_session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-mis",
                "source_sheet_id": "xy-1",
                "equation_id": "log_agonist_vs_response_4pl"
            }),
        )
        .unwrap();
    let scale_warnings = mis_out.payload["scale_warnings"].as_array().expect("scale_warnings array");
    assert!(!scale_warnings.is_empty(), "expected non-empty scale_warnings on raw X with log10 equation; payload={}", mis_out.payload);
    assert!(!mis_out.warnings.is_empty(), "expected CommandOutcome.warnings to surface scale mismatch warning");
    assert!(mis_out.payload.get("ambiguous").and_then(Value::as_bool).is_some());
    assert!(mis_out.payload.get("fit_status").and_then(Value::as_str).is_some());
    assert!(mis_out.payload.get("identifiability_warnings").and_then(Value::as_array).is_some());

    // 4. Partial dose-response curve (no upper plateau) with identify_unstable_parameters surfaces identifiability diagnostics immediately.
    let mut amb_session = make_xy_session(&[-9.0, -8.5, -8.0, -7.5, -7.0, -6.5], &[1.0, 1.3, 2.2, 4.5, 9.8, 21.0]);
    let amb_out = amb_session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-amb",
                "source_sheet_id": "xy-1",
                "equation_id": "log_agonist_vs_response_4pl",
                "identify_unstable_parameters": true
            }),
        )
        .unwrap();
    let is_amb = amb_out.payload["ambiguous"].as_bool().unwrap();
    let id_warns = amb_out.payload["identifiability_warnings"].as_array().unwrap();
    assert!(is_amb || !id_warns.is_empty(), "expected ambiguity or identifiability warning on incomplete 4PL curve; payload={}", amb_out.payload);

    // 5. Multi-subcolumn RawReplicates X table where subcol 0 is empty and subcol 1 holds raw concentrations.
    let mut multi_x = XyTable::new_with_x_mode(
        curvebook_doc::XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(2) },
        SubcolumnMode::Single,
        1,
        raw_xs.len(),
    )
    .unwrap();
    multi_x.data_sets[0].title = String::from("Multi-X Drug");
    for (r, (&x, &y)) in raw_xs.iter().zip(raw_ys.iter()).enumerate() {
        multi_x.x_cells[r][0] = Cell::empty();
        multi_x.x_cells[r][1] = Cell::number(x);
        multi_x.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut multi_x_doc = ProjectDocument::new("Multi-X Scale").unwrap();
    multi_x_doc.add_data_sheet(DataSheet::new("xy-mx", "Multi-X Assay", TableArchetype::Xy(multi_x)).unwrap()).unwrap();
    let mut multi_x_session = Session::from_document(multi_x_doc);
    let mx_out = multi_x_session.execute("fit.run", json!({ "sheet_id": "fit-mx", "source_sheet_id": "xy-mx" })).unwrap();
    assert_eq!(mx_out.payload["equation_id"], "agonist_vs_response_4pl");
}
