//! `R19.4`: the methods report must describe the observations a paired analysis actually used.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, PairedObservationAccounting, ProjectDocument, ResultStatus, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

use super::common::{col_sheet, ok_cmd, rs, session_with};

const OVERCLAIM: &str = "all finite active observations were included";
const EXPECTED: PairedObservationAccounting =
    PairedObservationAccounting { pairs_analyzed: 4, omitted_excluded: 1, omitted_non_numeric: 1, omitted_missing_partner: 1 };

/// Paired table with 4 complete pairs and one omitted row per reason (missing partner, excluded, non-numeric, empty).
fn paired_doc() -> ProjectDocument {
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 8).unwrap();
    col.data_sets[0].title = String::from("Before");
    col.data_sets[1].title = String::from("After");
    for (r, (a, b)) in [
        (Cell::number(10.0), Cell::number(12.0)),
        (Cell::number(11.0), Cell::number(13.5)),
        (Cell::number(9.0), Cell::number(11.0)),
        (Cell::number(12.0), Cell::empty()),
        (Cell::number(10.5), Cell::excluded_number(20.0)),
        (Cell::text("n/a"), Cell::number(14.0)),
        (Cell::empty(), Cell::empty()),
        (Cell::number(13.0), Cell::number(15.2)),
    ]
    .into_iter()
    .enumerate()
    {
        col.data_sets[0].cells[r][0] = a;
        col.data_sets[1].cells[r][0] = b;
    }
    let mut doc = ProjectDocument::new("Paired accounting").unwrap();
    doc.add_data_sheet(DataSheet::new("paired", "Paired data", TableArchetype::Column(col)).unwrap()).unwrap();
    doc
}

fn run(kind: &str) -> (Session, String) {
    let mut session = Session::from_document(paired_doc());
    let id = format!("res-{kind}");
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "paired", "sheet_id": id, "kind": kind }));
    (session, id)
}

#[test]
fn paired_analyses_report_pairs_analyzed_and_omitted_by_reason() {
    for kind in ["paired_t", "ratio_paired_t", "wilcoxon", "paired_sign_test", "bland_altman"] {
        let (session, id) = run(kind);
        let res = rs(&session, &id);
        assert_eq!((&res.status, res.paired_observation_accounting()), (&ResultStatus::UpToDate, Some(EXPECTED)), "{kind}");
        let checklist =
            res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap_or_else(|| panic!("{kind}: no checklist row"));
        assert_eq!(checklist.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 4 retained"), "{kind}");

        let sid = SheetId::new(&id);
        for format in [ReportFormat::Markdown, ReportFormat::PlainText, ReportFormat::Latex] {
            let report = generate_methods_report(session.document(), &sid, format).unwrap();
            assert!(!report.contains(OVERCLAIM), "{kind} {format:?}:\n{report}");
            assert!(
                report.contains(
                    "4 complete pair(s) analyzed; 3 row(s) omitted by complete-case pairing (1 missing a partner value, 1 containing an excluded cell, 1 containing a non-numeric cell)."
                ),
                "{kind} {format:?}:\n{report}"
            );
        }
    }
}

#[test]
fn complete_pairs_report_no_omissions() {
    let mut doc = paired_doc();
    let Some(TableArchetype::Column(col)) = doc.data_sheet_mut(&SheetId::new("paired")).map(|ds| &mut ds.archetype) else {
        panic!("paired column table");
    };
    for r in 3..=6 {
        col.data_sets[0].cells[r][0] = Cell::empty();
        col.data_sets[1].cells[r][0] = Cell::empty();
    }
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "paired", "sheet_id": "res", "kind": "paired_t" }));
    let report = generate_methods_report(session.document(), &SheetId::new("res"), ReportFormat::PlainText).unwrap();
    assert!(
        report.contains("4 complete pair(s) analyzed; no rows omitted.")
            && report.contains("0 cells masked as excluded prior to analysis.")
            && !report.contains(OVERCLAIM),
        "{report}"
    );
}

#[test]
fn stale_result_does_not_report_pairing_counts() {
    let (session, id) = run("paired_t");
    let sid = SheetId::new(id);
    let mut doc = session.document().clone();
    doc.result_sheet_mut(&sid).unwrap().status = ResultStatus::Dirty;
    let report = generate_methods_report(&doc, &sid, ReportFormat::PlainText).unwrap();
    assert!(!report.contains("Paired observations"), "{report}");
}

#[test]
fn unpaired_analysis_makes_no_inclusion_claim() {
    let mut sheet = col_sheet("unpaired", "Unpaired data", &[("", &[10.0, 11.0, 12.0, 13.0]), ("", &[14.0, 15.0, 16.0, 17.0])]);
    if let TableArchetype::Column(col) = &mut sheet.archetype {
        col.data_sets[1].cells[3][0] = Cell::text("lost");
    }
    let mut session = session_with("Unpaired", vec![sheet]);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "unpaired", "sheet_id": "res", "kind": "unpaired_t" }));
    assert_eq!(rs(&session, "res").paired_observation_accounting(), None);
    let report = generate_methods_report(session.document(), &SheetId::new("res"), ReportFormat::PlainText).unwrap();
    assert!(!report.contains(OVERCLAIM) && report.contains("0 cells masked as excluded prior to analysis."), "{report}");
}
