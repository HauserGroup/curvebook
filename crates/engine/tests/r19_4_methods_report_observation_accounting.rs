//! `R19.4`: the methods report must describe the observations a paired analysis actually used.
//!
//! Before this fix the report said "all finite active observations were included" whenever no cell
//! was masked as excluded, even when complete-case pairing had dropped rows. These tests check that
//! the report states the counts the analysis recorded, and that those counts agree with the
//! results-view checklist built from the same extraction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, PairedObservationAccounting, ProjectDocument, ResultSheet, ResultStatus, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

const OVERCLAIM: &str = "all finite active observations were included";

/// Paired table with 4 complete pairs and one omitted row per reason:
/// row 3 lacks a partner, row 4 has an excluded cell, row 5 has a text cell, and row 6 is empty on
/// both sides (outside the data, so not counted).
fn paired_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Paired accounting").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 8).unwrap();
    col.data_sets[0].title = String::from("Before");
    col.data_sets[1].title = String::from("After");
    let rows: [(Cell, Cell); 8] = [
        (Cell::number(10.0), Cell::number(12.0)),
        (Cell::number(11.0), Cell::number(13.5)),
        (Cell::number(9.0), Cell::number(11.0)),
        (Cell::number(12.0), Cell::empty()),
        (Cell::number(10.5), Cell::excluded_number(20.0)),
        (Cell::text("n/a"), Cell::number(14.0)),
        (Cell::empty(), Cell::empty()),
        (Cell::number(13.0), Cell::number(15.2)),
    ];
    for (r, (a, b)) in rows.into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = a;
        col.data_sets[1].cells[r][0] = b;
    }
    doc.add_data_sheet(DataSheet::new("paired", "Paired data", TableArchetype::Column(col)).unwrap()).unwrap();
    doc
}

const EXPECTED: PairedObservationAccounting =
    PairedObservationAccounting { pairs_analyzed: 4, omitted_excluded: 1, omitted_non_numeric: 1, omitted_missing_partner: 1 };

fn run(kind: &str) -> (Session, SheetId) {
    let mut session = Session::from_document(paired_doc());
    let id = format!("res-{kind}");
    session
        .execute("stats.run", json!({ "source_sheet_id": "paired", "sheet_id": id, "kind": kind }))
        .unwrap_or_else(|e| panic!("stats.run {kind} failed: {e}"));
    (session, SheetId::new(id))
}

fn result<'a>(session: &'a Session, id: &SheetId) -> &'a ResultSheet {
    session.document().result_sheet(id).unwrap()
}

#[test]
fn paired_analyses_report_pairs_analyzed_and_omitted_by_reason() {
    for kind in ["paired_t", "ratio_paired_t", "wilcoxon", "paired_sign_test", "bland_altman"] {
        let (session, id) = run(kind);
        let res = result(&session, &id);
        assert_eq!(res.status, ResultStatus::UpToDate, "{kind}: {:?}", res.status);
        assert_eq!(res.paired_observation_accounting(), Some(EXPECTED), "{kind}: recorded accounting");

        // The results view shows the same counts through the complete-case checklist row.
        let checklist =
            res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap_or_else(|| panic!("{kind}: no checklist row"));
        assert_eq!(checklist.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 4 retained"), "{kind}: checklist detail");

        for format in [ReportFormat::Markdown, ReportFormat::PlainText, ReportFormat::Latex] {
            let report = generate_methods_report(session.document(), &id, format).unwrap();
            assert!(!report.contains(OVERCLAIM), "{kind} {format:?}: report still claims every observation was included:\n{report}");
            assert!(
                report.contains(
                    "4 complete pair(s) analyzed; 3 row(s) omitted by complete-case pairing (1 missing a partner value, 1 containing an excluded cell, 1 containing a non-numeric cell)."
                ),
                "{kind} {format:?}: report must state the pairing counts:\n{report}"
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
    session.execute("stats.run", json!({ "source_sheet_id": "paired", "sheet_id": "res", "kind": "paired_t" })).unwrap();
    let report = generate_methods_report(session.document(), &SheetId::new("res"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("4 complete pair(s) analyzed; no rows omitted."), "{report}");
    assert!(report.contains("0 cells masked as excluded prior to analysis."), "{report}");
    assert!(!report.contains(OVERCLAIM), "{report}");
}

#[test]
fn stale_result_does_not_report_pairing_counts() {
    let (session, id) = run("paired_t");
    let mut doc = session.document().clone();
    doc.result_sheet_mut(&id).unwrap().status = ResultStatus::Dirty;
    let report = generate_methods_report(&doc, &id, ReportFormat::PlainText).unwrap();
    assert!(!report.contains("Paired observations"), "pending result must not report cached pairing counts:\n{report}");
}

#[test]
fn unpaired_analysis_makes_no_inclusion_claim() {
    let mut doc = ProjectDocument::new("Unpaired").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(14.0 + r as f64);
    }
    col.data_sets[1].cells[3][0] = Cell::text("lost");
    doc.add_data_sheet(DataSheet::new("unpaired", "Unpaired data", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);
    session.execute("stats.run", json!({ "source_sheet_id": "unpaired", "sheet_id": "res", "kind": "unpaired_t" })).unwrap();
    let res = result(&session, &SheetId::new("res"));
    assert_eq!(res.paired_observation_accounting(), None);
    let report = generate_methods_report(session.document(), &SheetId::new("res"), ReportFormat::PlainText).unwrap();
    assert!(!report.contains(OVERCLAIM), "a skipped text cell must not be reported as an included observation:\n{report}");
    assert!(report.contains("0 cells masked as excluded prior to analysis."), "{report}");
}
