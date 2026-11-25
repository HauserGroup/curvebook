#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use curvebook_doc::{BaselineOperation, CellCoord, DataSheet, ProjectDocument};
use curvebook_engine::Session;
use serde_json::{Value, json};

use super::common::{TempDir, col_sheet, ds, ok_cmd, rs, run_cmds, session_with};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn assert_row_state(sheet: &DataSheet, row: usize, expected_vals: &[f64], expected_excluded: bool) {
    for (col, &exp) in expected_vals.iter().enumerate() {
        let cell = sheet.cell(CellCoord::new(row, col, 0)).unwrap();
        assert_eq!(cell.excluded, expected_excluded, "row {row} col {col}");
        assert!((cell.value.finite_f64().unwrap() - exp).abs() < 1e-10, "row {row} col {col}: {cell:?}");
    }
}

fn assert_rows(sheet: &DataSheet, expected: &[(&[f64], bool)]) {
    for (r, (vals, excl)) in expected.iter().enumerate() {
        assert_row_state(sheet, r, vals, *excl);
    }
}

#[test]
fn fa_x2_01_internal_paste_link_preserves_baseline_and_exclude_row_predicate_across_edit_recalc_and_roundtrip() -> TestResult {
    let mut s = session_with(
        "FA-X2-01 Internal Live Link",
        vec![
            col_sheet("src", "Source", &[("A", &[10.0, 10.0, 20.0, 5.0]), ("B", &[25.0, 35.0, 30.0, 22.0]), ("C", &[40.0, 50.0, 45.0, 30.0])]),
            col_sheet("dst", "Normalized Target", &[("A", &[0.0; 4]), ("B", &[0.0; 4]), ("C", &[0.0; 4])]),
        ],
    );
    run_cmds(
        &mut s,
        &[
            ("analysis.add", json!({ "source_sheet_id": "dst", "sheet_id": "res-desc", "kind": "descriptive" })),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "dst", "source_sheet_id": "src", "baseline": { "col": 0, "op": "difference" }, "exclude_row_predicate": "B > 20" }),
            ),
        ],
    );
    let dst0 = ds(&s, "dst");
    assert_eq!(
        (dst0.linked_regions.len(), dst0.linked_regions[0].spec.filter.baseline, dst0.linked_regions[0].spec.filter.exclude_row_predicate.as_deref()),
        (1, Some((0, BaselineOperation::Difference)), Some("B > 20"))
    );
    assert_rows(dst0, &[(&[0.0, 15.0, 30.0], false), (&[0.0, 25.0, 40.0], true), (&[0.0, 10.0, 25.0], false), (&[0.0, 17.0, 25.0], false)]);
    assert!((rs(&s, "res-desc").lookup_scalar("Mean", Some(1)).unwrap() - 14.0).abs() < 1e-10);

    run_cmds(
        &mut s,
        &[
            ("table.setCell", json!({ "sheet_id": "src", "row": 1, "col": 0, "value": 20.0 })),
            ("table.setCell", json!({ "sheet_id": "src", "row": 2, "col": 1, "value": 50.0 })),
        ],
    );
    let expected_after: &[(&[f64], bool)] =
        &[(&[0.0, 15.0, 30.0], false), (&[0.0, 15.0, 30.0], false), (&[0.0, 30.0, 25.0], true), (&[0.0, 17.0, 25.0], false)];
    let expected_mean = (15.0 + 15.0 + 17.0) / 3.0;
    assert_rows(ds(&s, "dst"), expected_after);
    assert!((rs(&s, "res-desc").lookup_scalar("Mean", Some(1)).unwrap() - expected_mean).abs() < 1e-10);

    assert_eq!(ok_cmd(&mut s, "project.recalculate", json!({})).payload["refreshed_links"], json!(1));
    assert_rows(ds(&s, "dst"), expected_after);

    let mut reopened = Session::from_document(serde_json::from_str::<ProjectDocument>(&serde_json::to_string(s.document())?)?);
    assert_eq!(ok_cmd(&mut reopened, "project.recalculate", json!({})).payload["refreshed_links"], json!(1));
    assert_rows(ds(&reopened, "dst"), expected_after);
    assert!((rs(&reopened, "res-desc").lookup_scalar("Mean", Some(1)).unwrap() - expected_mean).abs() < 1e-10);
    Ok(())
}

#[test]
fn fa_x2_01_external_csv_paste_link_preserves_percent_change_and_predicate_on_recalculate() -> TestResult {
    let dir = TempDir::try_new("csv-baseline-pred")?;
    let csv_path = dir.child("plate.csv");
    fs::write(&csv_path, "10,20\n10,30\n10,15\n")?;

    let mut s = session_with("FA-X2-01 External CSV", vec![col_sheet("dst", "CSV Target", &[("A", &[0.0; 3]), ("B", &[0.0; 3])])]);
    ok_cmd(
        &mut s,
        "table.pasteLink",
        json!({ "target_sheet_id": "dst", "source_path": csv_path.to_string_lossy(), "delimiter": ",", "baseline": { "col": 0, "op": "percent_change" }, "exclude_row_predicate": "B > 150" }),
    );
    assert_eq!(
        (ds(&s, "dst").linked_regions[0].spec.filter.baseline, ds(&s, "dst").linked_regions[0].spec.filter.exclude_row_predicate.as_deref()),
        (Some((0, BaselineOperation::PercentChange)), Some("B > 150"))
    );
    assert_rows(ds(&s, "dst"), &[(&[0.0, 100.0], false), (&[0.0, 200.0], true), (&[0.0, 50.0], false)]);

    fs::write(&csv_path, "10,20\n10,20\n10,40\n")?;
    assert_eq!(ok_cmd(&mut s, "project.recalculate", json!({})).payload["refreshed_links"], json!(1));
    assert_rows(ds(&s, "dst"), &[(&[0.0, 100.0], false), (&[0.0, 100.0], false), (&[0.0, 300.0], true)]);
    Ok(())
}

#[test]
fn fa_x2_01_link_spec_precedence_and_legacy_project_deserialization_compat() -> TestResult {
    let mut s = session_with(
        "FA-X2-01 LinkSpec & Legacy",
        vec![
            col_sheet("src", "Source", &[("A", &[2.0, 4.0]), ("B", &[10.0, 12.0])]),
            col_sheet("dst-keep", "Keep", &[("A", &[0.0; 2]), ("B", &[0.0; 2])]),
            col_sheet("dst-override", "Override", &[("A", &[0.0; 2]), ("B", &[0.0; 2])]),
        ],
    );
    let base_link_spec = json!({
        "source": { "InternalSheet": { "sheet_id": "src", "kind": "DataSheet", "tab_id": null, "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1, "col": 1, "subcol": 0 } } } },
        "filter": { "row_window": null, "col_window": null, "decimate_every_k": null, "unstack_group_column": null, "parse_asterisk_exclusions": true, "baseline": [0, "Ratio"], "exclude_row_predicate": "B < 4" },
        "placement": { "mode": "Overwrite", "anchor": { "row": 0, "col": 0, "subcol": 0 }, "propagate_title_header": false, "skip_contiguous_blanks": false, "transpose": false }
    });
    run_cmds(
        &mut s,
        &[
            ("table.pasteLink", json!({ "target_sheet_id": "dst-keep", "link_spec": base_link_spec })),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "dst-override", "baseline": { "col": 0, "op": "difference" }, "exclude_row_predicate": "B > 8", "link_spec": base_link_spec }),
            ),
        ],
    );
    assert_rows(ds(&s, "dst-keep"), &[(&[1.0, 5.0], false), (&[1.0, 3.0], true)]);
    assert_eq!(
        (
            ds(&s, "dst-override").linked_regions[0].spec.filter.baseline,
            ds(&s, "dst-override").linked_regions[0].spec.filter.exclude_row_predicate.as_deref()
        ),
        (Some((0, BaselineOperation::Difference)), Some("B > 8"))
    );
    assert_rows(ds(&s, "dst-override"), &[(&[0.0, 8.0], false), (&[0.0, 8.0], false)]);

    ok_cmd(&mut s, "table.setCell", json!({ "sheet_id": "src", "row": 1, "col": 1, "value": 14.0 }));
    assert_rows(ds(&s, "dst-keep"), &[(&[1.0, 5.0], false), (&[1.0, 3.5], true)]);
    assert_rows(ds(&s, "dst-override"), &[(&[0.0, 8.0], false), (&[0.0, 10.0], true)]);

    let mut doc_json: Value = serde_json::to_value(s.document())?;
    for sheet in doc_json["data_sheets"].as_array_mut().unwrap() {
        for reg in sheet["linked_regions"].as_array_mut().unwrap() {
            if let Some(m) = reg["spec"]["filter"].as_object_mut() {
                m.remove("baseline");
                m.remove("exclude_row_predicate");
            }
        }
    }
    let mut legacy = Session::from_document(serde_json::from_value::<ProjectDocument>(doc_json)?);
    assert_eq!(
        (
            ds(&legacy, "dst-keep").linked_regions[0].spec.filter.baseline,
            ds(&legacy, "dst-keep").linked_regions[0].spec.filter.exclude_row_predicate.as_deref()
        ),
        (None, None)
    );
    ok_cmd(&mut legacy, "project.recalculate", json!({}));
    assert_rows(ds(&legacy, "dst-keep"), &[(&[2.0, 10.0], false), (&[4.0, 14.0], false)]);
    Ok(())
}
