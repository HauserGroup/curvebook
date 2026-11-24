use std::fs;

use curvebook_doc::{BaselineOperation, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use serde_json::{Value, json};

use super::common::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn require_ds<'a>(doc: &'a ProjectDocument, id: &str) -> Result<&'a DataSheet, Box<dyn std::error::Error>> {
    doc.data_sheet(&SheetId::new(id)).ok_or_else(|| format!("missing data sheet `{id}`").into())
}

fn cell_at(ds: &DataSheet, row: usize, col: usize) -> Result<&Cell, Box<dyn std::error::Error>> {
    ds.cell(CellCoord::new(row, col, 0)).ok_or_else(|| format!("missing cell ({row}, {col})").into())
}

fn assert_row_state(ds: &DataSheet, row: usize, expected_vals: &[f64], expected_excluded: bool) -> TestResult {
    for (col, &exp) in expected_vals.iter().enumerate() {
        let cell = cell_at(ds, row, col)?;
        assert_eq!(cell.excluded, expected_excluded, "row {row} col {col} excluded mismatch");
        let raw = cell.value.finite_f64().ok_or_else(|| format!("row {row} col {col} expected finite number, got {:?}", cell.value))?;
        assert!((raw - exp).abs() < 1e-10, "row {row} col {col} expected {exp}, got {raw}");
    }
    Ok(())
}

fn make_col_sheet(id: &str, title: &str, rows: &[&[f64]]) -> Result<DataSheet, Box<dyn std::error::Error>> {
    let cols = rows.first().map_or(1, |r| r.len());
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows.len())?;
    for (r, row_vals) in rows.iter().enumerate() {
        for (c, &val) in row_vals.iter().enumerate() {
            let cell = tbl
                .data_sets
                .get_mut(c)
                .and_then(|ds| ds.cells.get_mut(r))
                .and_then(|row| row.first_mut())
                .ok_or_else(|| format!("missing cell ({r}, {c}) in make_col_sheet"))?;
            *cell = Cell::number(val);
        }
    }
    Ok(DataSheet::new(id, title, TableArchetype::Column(tbl))?)
}

#[test]
fn fa_x2_01_internal_paste_link_preserves_baseline_and_exclude_row_predicate_across_edit_recalc_and_roundtrip() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-01 Internal Live Link")?;
    doc.add_data_sheet(make_col_sheet("src", "Source", &[&[10.0, 25.0, 40.0], &[10.0, 35.0, 50.0], &[20.0, 30.0, 45.0], &[5.0, 22.0, 30.0]])?)?;
    doc.add_data_sheet(make_col_sheet("dst", "Normalized Target", &[&[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0]])?)?;

    let mut s = Session::from_document(doc);
    s.execute("analysis.add", json!({ "source_sheet_id": "dst", "sheet_id": "res-desc", "kind": "descriptive" }))?;

    // Baseline subtraction against col 0 plus predicate excluding rows whose adjusted B exceeds 20.
    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst",
            "source_sheet_id": "src",
            "baseline": { "col": 0, "op": "difference" },
            "exclude_row_predicate": "B > 20",
        }),
    )?;

    let dst0 = require_ds(s.document(), "dst")?;
    assert_eq!(dst0.linked_regions.len(), 1);
    assert_eq!(dst0.linked_regions[0].spec.filter.baseline, Some((0, BaselineOperation::Difference)));
    assert_eq!(dst0.linked_regions[0].spec.filter.exclude_row_predicate.as_deref(), Some("B > 20"));

    assert_row_state(dst0, 0, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst0, 1, &[0.0, 25.0, 40.0], true)?;
    assert_row_state(dst0, 2, &[0.0, 10.0, 25.0], false)?;
    assert_row_state(dst0, 3, &[0.0, 17.0, 25.0], false)?;

    let mean_initial =
        s.document().result_sheet(&SheetId::new("res-desc")).ok_or("missing res-desc")?.lookup_scalar("Mean", Some(1)).ok_or("mean")?;
    assert!((mean_initial - 14.0).abs() < 1e-10, "expected initial col 1 mean 14.0, got {mean_initial}");

    // Upstream edits flip which rows satisfy `B > 20` after baseline subtraction:
    // - Row 1 baseline goes 10 -> 20, so adjusted B goes 25 -> 15 (no longer excluded).
    // - Row 2 Treat1 goes 30 -> 50, so adjusted B goes 10 -> 30 (newly excluded).
    s.execute("table.setCell", json!({ "sheet_id": "src", "row": 1, "col": 0, "value": 20.0 }))?;
    s.execute("table.setCell", json!({ "sheet_id": "src", "row": 2, "col": 1, "value": 50.0 }))?;

    let dst_after_edit = require_ds(s.document(), "dst")?;
    assert_row_state(dst_after_edit, 0, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst_after_edit, 1, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst_after_edit, 2, &[0.0, 30.0, 25.0], true)?;
    assert_row_state(dst_after_edit, 3, &[0.0, 17.0, 25.0], false)?;

    let mean_after_edit =
        s.document().result_sheet(&SheetId::new("res-desc")).ok_or("missing res-desc")?.lookup_scalar("Mean", Some(1)).ok_or("mean")?;
    let expected_mean_after_edit = (15.0 + 15.0 + 17.0) / 3.0;
    assert!((mean_after_edit - expected_mean_after_edit).abs() < 1e-10, "expected {expected_mean_after_edit}, got {mean_after_edit}");

    // Explicit `project.recalculate` must preserve normalized values and predicate exclusions.
    let recalc = s.execute("project.recalculate", json!({}))?;
    assert_eq!(recalc.payload["refreshed_links"], json!(1));
    let dst_after_recalc = require_ds(s.document(), "dst")?;
    assert_row_state(dst_after_recalc, 0, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst_after_recalc, 1, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst_after_recalc, 2, &[0.0, 30.0, 25.0], true)?;
    assert_row_state(dst_after_recalc, 3, &[0.0, 17.0, 25.0], false)?;

    // Save/reload JSON round-trip followed by `project.recalculate` must also preserve both settings.
    let saved_json = serde_json::to_string(s.document())?;
    let reopened: ProjectDocument = serde_json::from_str(&saved_json)?;
    let mut reopened_session = Session::from_document(reopened);
    let recalc_reopened = reopened_session.execute("project.recalculate", json!({}))?;
    assert_eq!(recalc_reopened.payload["refreshed_links"], json!(1));

    let dst_reopened = require_ds(reopened_session.document(), "dst")?;
    assert_row_state(dst_reopened, 0, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst_reopened, 1, &[0.0, 15.0, 30.0], false)?;
    assert_row_state(dst_reopened, 2, &[0.0, 30.0, 25.0], true)?;
    assert_row_state(dst_reopened, 3, &[0.0, 17.0, 25.0], false)?;
    let mean_reopened = reopened_session
        .document()
        .result_sheet(&SheetId::new("res-desc"))
        .ok_or("missing res-desc")?
        .lookup_scalar("Mean", Some(1))
        .ok_or("mean")?;
    assert!((mean_reopened - expected_mean_after_edit).abs() < 1e-10);
    Ok(())
}

#[test]
fn fa_x2_01_external_csv_paste_link_preserves_percent_change_and_predicate_on_recalculate() -> TestResult {
    let dir = TempDir::try_new("csv-baseline-pred")?;
    let csv_path = dir.path().join("plate.csv");
    fs::write(&csv_path, "10,20\n10,30\n10,15\n")?;

    let mut doc = ProjectDocument::new("FA-X2-01 External CSV Link")?;
    doc.add_data_sheet(make_col_sheet("dst", "CSV Target", &[&[0.0, 0.0], &[0.0, 0.0], &[0.0, 0.0]])?)?;
    let mut s = Session::from_document(doc);

    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst",
            "source_path": csv_path.to_string_lossy(),
            "delimiter": ",",
            "baseline": { "col": 0, "op": "percent_change" },
            "exclude_row_predicate": "B > 150",
        }),
    )?;

    let dst0 = require_ds(s.document(), "dst")?;
    assert_eq!(dst0.linked_regions[0].spec.filter.baseline, Some((0, BaselineOperation::PercentChange)));
    assert_eq!(dst0.linked_regions[0].spec.filter.exclude_row_predicate.as_deref(), Some("B > 150"));
    assert_row_state(dst0, 0, &[0.0, 100.0], false)?;
    assert_row_state(dst0, 1, &[0.0, 200.0], true)?;
    assert_row_state(dst0, 2, &[0.0, 50.0], false)?;

    // Update CSV on disk so row 1 drops below cutoff and row 2 crosses above cutoff.
    fs::write(&csv_path, "10,20\n10,20\n10,40\n")?;
    let recalc = s.execute("project.recalculate", json!({}))?;
    assert_eq!(recalc.payload["refreshed_links"], json!(1));

    let dst1 = require_ds(s.document(), "dst")?;
    assert_row_state(dst1, 0, &[0.0, 100.0], false)?;
    assert_row_state(dst1, 1, &[0.0, 100.0], false)?;
    assert_row_state(dst1, 2, &[0.0, 300.0], true)?;
    Ok(())
}

#[test]
fn fa_x2_01_link_spec_precedence_and_legacy_project_deserialization_compat() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-01 LinkSpec & Legacy")?;
    doc.add_data_sheet(make_col_sheet("src", "Source", &[&[2.0, 10.0], &[4.0, 12.0]])?)?;
    doc.add_data_sheet(make_col_sheet("dst-keep", "Keep LinkSpec", &[&[0.0, 0.0], &[0.0, 0.0]])?)?;
    doc.add_data_sheet(make_col_sheet("dst-override", "Override LinkSpec", &[&[0.0, 0.0], &[0.0, 0.0]])?)?;

    let mut s = Session::from_document(doc);

    // 1. Direct `link_spec` carrying `filter.baseline` and `filter.exclude_row_predicate` is retained without top-level params.
    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst-keep",
            "link_spec": {
                "source": {
                    "InternalSheet": {
                        "sheet_id": "src",
                        "kind": "DataSheet",
                        "tab_id": null,
                        "range": {
                            "start": { "row": 0, "col": 0, "subcol": 0 },
                            "end": { "row": 1, "col": 1, "subcol": 0 }
                        }
                    }
                },
                "filter": {
                    "row_window": null,
                    "col_window": null,
                    "decimate_every_k": null,
                    "unstack_group_column": null,
                    "parse_asterisk_exclusions": true,
                    "baseline": [0, "Ratio"],
                    "exclude_row_predicate": "B < 4"
                },
                "placement": {
                    "mode": "Overwrite",
                    "anchor": { "row": 0, "col": 0, "subcol": 0 },
                    "propagate_title_header": false,
                    "skip_contiguous_blanks": false,
                    "transpose": false
                }
            }
        }),
    )?;

    let keep0 = require_ds(s.document(), "dst-keep")?;
    assert_row_state(keep0, 0, &[1.0, 5.0], false)?;
    assert_row_state(keep0, 1, &[1.0, 3.0], true)?;

    // 2. Top-level `baseline` and `exclude_row_predicate` override `link_spec.filter` values.
    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst-override",
            "baseline": { "col": 0, "op": "difference" },
            "exclude_row_predicate": "B > 8",
            "link_spec": {
                "source": {
                    "InternalSheet": {
                        "sheet_id": "src",
                        "kind": "DataSheet",
                        "tab_id": null,
                        "range": {
                            "start": { "row": 0, "col": 0, "subcol": 0 },
                            "end": { "row": 1, "col": 1, "subcol": 0 }
                        }
                    }
                },
                "filter": {
                    "row_window": null,
                    "col_window": null,
                    "decimate_every_k": null,
                    "unstack_group_column": null,
                    "parse_asterisk_exclusions": true,
                    "baseline": [0, "Ratio"],
                    "exclude_row_predicate": "B < 4"
                },
                "placement": {
                    "mode": "Overwrite",
                    "anchor": { "row": 0, "col": 0, "subcol": 0 },
                    "propagate_title_header": false,
                    "skip_contiguous_blanks": false,
                    "transpose": false
                }
            }
        }),
    )?;

    let ov0 = require_ds(s.document(), "dst-override")?;
    assert_eq!(ov0.linked_regions[0].spec.filter.baseline, Some((0, BaselineOperation::Difference)));
    assert_eq!(ov0.linked_regions[0].spec.filter.exclude_row_predicate.as_deref(), Some("B > 8"));
    assert_row_state(ov0, 0, &[0.0, 8.0], false)?;
    assert_row_state(ov0, 1, &[0.0, 8.0], false)?;

    s.execute("table.setCell", json!({ "sheet_id": "src", "row": 1, "col": 1, "value": 14.0 }))?;
    let keep1 = require_ds(s.document(), "dst-keep")?;
    assert_row_state(keep1, 0, &[1.0, 5.0], false)?;
    assert_row_state(keep1, 1, &[1.0, 3.5], true)?;
    let ov1 = require_ds(s.document(), "dst-override")?;
    assert_row_state(ov1, 0, &[0.0, 8.0], false)?;
    assert_row_state(ov1, 1, &[0.0, 10.0], true)?;

    // 3. Deserializing legacy ProjectDocument JSON omitting `baseline`/`exclude_row_predicate` succeeds and recalculates cleanly.
    let mut doc_json: Value = serde_json::to_value(s.document())?;
    for ds in doc_json["data_sheets"].as_array_mut().ok_or("data_sheets array")? {
        for reg in ds["linked_regions"].as_array_mut().ok_or("linked_regions array")? {
            if let Some(filter_map) = reg["spec"]["filter"].as_object_mut() {
                filter_map.remove("baseline");
                filter_map.remove("exclude_row_predicate");
            }
        }
    }
    let legacy_doc: ProjectDocument = serde_json::from_value(doc_json)?;
    let legacy_keep = require_ds(&legacy_doc, "dst-keep")?;
    assert_eq!(legacy_keep.linked_regions[0].spec.filter.baseline, None);
    assert_eq!(legacy_keep.linked_regions[0].spec.filter.exclude_row_predicate, None);

    let mut legacy_session = Session::from_document(legacy_doc);
    legacy_session.execute("project.recalculate", json!({}))?;
    let legacy_after = require_ds(legacy_session.document(), "dst-keep")?;
    assert_row_state(legacy_after, 0, &[2.0, 10.0], false)?;
    assert_row_state(legacy_after, 1, &[4.0, 14.0], false)?;
    Ok(())
}
