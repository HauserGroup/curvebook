//! R14.6: open a project saved by the `v0.1.0-alpha.1` build, edit it, and save it again.
//!
//! `fixtures/r14_6/alpha1-upgrade.curvebook` was written by the Alpha.1 `curvebook-cli` from
//! `fixtures/r14_6/make-alpha1-project.ndjson` (rebuild steps are in that script). Unlike the
//! synthetic v2 archive in `curvebook-format`'s suite, its `document.json` is exactly what Alpha.1
//! serialized, so a field the current model reads differently fails here.
//!
//! Numerical expectations:
//! - One-way ANOVA on the edited column table is checked against the textbook sums-of-squares
//!   formulas (Zar, *Biostatistical Analysis*, 5th ed., §10.1), computed by hand below, to `1e-9`.
//! - Every scalar of the upgraded project's recomputed analyses must match a project the current
//!   build creates from the same script and edits: `1e-9` relative for the closed-form ANOVA and
//!   `1e-6` relative for the iterative 4PL fit (solver convergence tolerance, not a model change).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::fs;

use common::TempDir;
use curvebook_doc::{AnalysisKind, CURRENT_SCHEMA_VERSION, ResultStatus, SheetId};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::ZipArchive;
use serde_json::{Value, json};

const ALPHA1_PROJECT: &[u8] = include_bytes!("fixtures/r14_6/alpha1-upgrade.curvebook");
const ALPHA1_SCRIPT: &str = include_str!("fixtures/r14_6/make-alpha1-project.ndjson");

// The upgrade test only means something while the current schema is newer than Alpha.1's v2.
const _: () = assert!(CURRENT_SCHEMA_VERSION > 2);

/// Edits applied to both the upgraded and the freshly built project.
fn apply_edits(session: &mut Session) {
    session.execute("table.setCell", json!({ "sheet_id": "col1", "row": 0, "col": 0, "value": 12.0 })).expect("edit column cell");
    session.execute("table.setCell", json!({ "sheet_id": "xy1", "row": 2, "col": 0, "value": 30.0 })).expect("edit xy cell");
}

/// Build the fixture project with the current build by replaying the Alpha.1 script (minus the save).
fn fresh_project_from_script() -> Session {
    let mut session = Session::new();
    for line in ALPHA1_SCRIPT.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let cmd: Value = serde_json::from_str(line).expect("script line is JSON");
        let id = cmd["cmd"].as_str().expect("script line has cmd");
        if id == "project.save" {
            continue;
        }
        session.execute(id, cmd["params"].clone()).unwrap_or_else(|e| panic!("fresh replay of `{id}` failed: {e:?}"));
    }
    session
}

fn session_in(dir: &TempDir) -> Session {
    let mut session = Session::new();
    session.set_path_policy(ExternalPathPolicy::workdir(dir.path()).expect("workdir policy"));
    session
}

fn scalars(session: &Session, sheet: &str) -> Vec<(String, f64)> {
    let res = session.document().result_sheet(&SheetId::new(sheet)).unwrap_or_else(|| panic!("result sheet `{sheet}`"));
    assert_eq!(res.status, ResultStatus::UpToDate, "`{sheet}` must be up to date");
    res.scalars.iter().map(|s| (s.key.clone(), s.value)).collect()
}

fn scalar(session: &Session, sheet: &str, key: &str) -> f64 {
    let res = session.document().result_sheet(&SheetId::new(sheet)).unwrap_or_else(|| panic!("result sheet `{sheet}`"));
    res.lookup_scalar(key, None).unwrap_or_else(|| panic!("`{sheet}` scalar `{key}`"))
}

fn assert_rel(actual: f64, expected: f64, rel_tol: f64, label: &str) {
    let scale = expected.abs().max(1e-12);
    assert!((actual - expected).abs() <= rel_tol * scale, "{label}: {actual} vs {expected} (rel tol {rel_tol})");
}

fn graph_svg(session: &mut Session, sheet: &str) -> String {
    let out = session.execute("graph.exportSvg", json!({ "sheet_id": sheet })).unwrap_or_else(|e| panic!("export `{sheet}`: {e:?}"));
    out.payload["svg"].as_str().unwrap_or_else(|| panic!("`{sheet}` SVG payload: {}", out.payload)).to_owned()
}

#[test]
fn fixture_is_an_alpha1_schema_v2_archive() {
    let zip = ZipArchive::from_bytes(ALPHA1_PROJECT).expect("fixture is a zip archive");
    let manifest: Value = serde_json::from_slice(zip.get("manifest.json").expect("manifest")).expect("manifest JSON");
    assert_eq!(manifest["schema_version"], 2, "fixture must stay the file Alpha.1 wrote");
    let document: Value = serde_json::from_slice(zip.get("document.json").expect("document")).expect("document JSON");
    assert_eq!(document["schema_version"], 2);
}

#[test]
fn alpha1_project_opens_edits_recomputes_and_round_trips() {
    let dir = TempDir::new("r14-6-alpha1");
    let original = dir.child("alpha1-upgrade.curvebook");
    fs::write(&original, ALPHA1_PROJECT).expect("write fixture copy");

    // 1. Open through the same command the desktop and MCP use.
    let mut session = session_in(&dir);
    let opened = session.execute("project.open", json!({ "path": original.to_string_lossy() })).expect("open Alpha.1 project");
    assert_eq!(opened.payload["schema_version"], CURRENT_SCHEMA_VERSION);
    assert_eq!(opened.payload["sheet_count"], 6);

    // 2. Sheets, analysis specs, and graph links survive the migration.
    let doc = session.document();
    assert_eq!(doc.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(doc.data_sheet(&SheetId::new("col1")).expect("col1").title, "Group response");
    assert_eq!(doc.data_sheet(&SheetId::new("xy1")).expect("xy1").title, "Dose response");
    let anova = doc.result_sheet(&SheetId::new("res-anova")).expect("res-anova");
    assert!(matches!(anova.analysis.kind, AnalysisKind::OneWayAnova(_)), "res-anova kind: {:?}", anova.analysis.kind);
    assert_eq!(anova.analysis.source_sheet_id.as_str(), "col1");
    let fit = doc.result_sheet(&SheetId::new("res-fit")).expect("res-fit");
    assert!(matches!(fit.analysis.kind, AnalysisKind::NonlinearRegression(_)), "res-fit kind: {:?}", fit.analysis.kind);
    assert_eq!(fit.analysis.source_sheet_id.as_str(), "xy1");
    for (graph, source) in [("g-col", "col1"), ("g-xy", "xy1")] {
        let g = doc.graph_sheet(&SheetId::new(graph)).unwrap_or_else(|| panic!("graph `{graph}`"));
        assert_eq!(g.spec.primary_source_sheet_id.as_str(), source, "`{graph}` source");
    }

    // 3. Alpha.1's cached results load as written (F = 224, MS residual = 2/3; Zar §10.1 by hand).
    assert_rel(scalar(&session, "res-anova", "F"), 224.0, 1e-9, "cached Alpha.1 F");
    assert_rel(scalar(&session, "res-anova", "MS residual"), 2.0 / 3.0, 1e-9, "cached Alpha.1 MS residual");

    // 4. Edit source cells: linked analyses and graphs recompute.
    let col_svg_before = graph_svg(&mut session, "g-col");
    let xy_svg_before = graph_svg(&mut session, "g-xy");
    let fit_logec50_before = scalar(&session, "res-fit", "LogEC50");
    apply_edits(&mut session);

    // Group A becomes [12, 10, 11, 10]: means 10.75, 14, 22; grand mean 187/12.
    // SS within = 2.75 + 2 + 2 = 6.75 (df 9); SS between = 4 * sum((mean - grand)^2) (df 2).
    let grand = 187.0 / 12.0;
    let ss_between = 4.0 * [10.75_f64, 14.0, 22.0].iter().map(|m| (m - grand).powi(2)).sum::<f64>();
    let expected_f = (ss_between / 2.0) / (6.75 / 9.0);
    assert_rel(scalar(&session, "res-anova", "F"), expected_f, 1e-9, "edited F");
    assert_rel(scalar(&session, "res-anova", "MS residual"), 0.75, 1e-9, "edited MS residual");
    assert!(
        (scalar(&session, "res-fit", "LogEC50") - fit_logec50_before).abs() > 1e-3,
        "editing xy1 must refit res-fit (LogEC50 stayed {fit_logec50_before})"
    );
    let col_svg_after = graph_svg(&mut session, "g-col");
    let xy_svg_after = graph_svg(&mut session, "g-xy");
    assert_ne!(col_svg_before, col_svg_after, "g-col must redraw after editing col1");
    assert_ne!(xy_svg_before, xy_svg_after, "g-xy must redraw after editing xy1");

    // 5. The upgraded analyses compute what a project made by the current build computes.
    let mut fresh = fresh_project_from_script();
    apply_edits(&mut fresh);
    for (sheet, tol) in [("res-anova", 1e-9), ("res-fit", 1e-6)] {
        let upgraded = scalars(&session, sheet);
        let reference = scalars(&fresh, sheet);
        for (key, expected) in &reference {
            let actual = upgraded.iter().find(|(k, _)| k == key).map(|(_, v)| *v);
            let actual = actual.unwrap_or_else(|| panic!("upgraded `{sheet}` lacks scalar `{key}` that a fresh project reports"));
            assert_rel(actual, *expected, tol, &format!("{sheet} {key}"));
        }
    }

    // 6. Save a copy, reopen it, and get the same document and drawings back.
    let copy = dir.child("alpha1-upgrade-copy.curvebook");
    session.execute("project.save", json!({ "path": copy.to_string_lossy() })).expect("save copy");
    assert_eq!(fs::read(&original).expect("read original"), ALPHA1_PROJECT, "saving a copy must not touch the Alpha.1 file");

    let mut reopened = session_in(&dir);
    reopened.execute("project.open", json!({ "path": copy.to_string_lossy() })).expect("reopen copy");
    assert_eq!(reopened.document().schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(reopened.document(), session.document(), "reopened copy must equal the saved document");
    assert_eq!(graph_svg(&mut reopened, "g-col"), col_svg_after);
    assert_eq!(graph_svg(&mut reopened, "g-xy"), xy_svg_after);
}
