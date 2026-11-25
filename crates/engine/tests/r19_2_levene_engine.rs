#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! End-to-end `D1` oracle and edge-case tests for the standalone Levene / Brown–Forsythe
//! homogeneity-of-variance analysis (`R19.2`, `AnalysisKind::LeveneTest`), executed through
//! `Session::execute` (`stats.run`, `analysis.run`, `analysis.updateParams`, `table.setCell`).
//!
//! # D1 provenance
//!
//! Reference values are black-box outputs of two independent implementations, already recorded for
//! the stats-level oracles in `crates/stats/tests/d1_extended_stats_oracles.rs` (section 3a) and
//! `crates/stats/tests/core_suite/d1_reference_oracles.rs`:
//!
//! - SciPy 1.18.1 `scipy.stats.levene(*groups, center="median" | "mean")`
//!   (`crates/stats/tests/reference/scipy_statsmodels_oracles.py`, section 4), and
//! - R 4.5.2 (2025-10-31) `anova(lm(abs(x - c) ~ g))` with `c = median(x)` or `mean(x)` per group
//!   (`crates/stats/tests/reference/stats_extended_oracles.R`, sections 0 and 3), re-run on
//!   2026-11-24 for this test, which also printed the per-group medians, means, and mean absolute
//!   deviations asserted below.
//!
//! Methods: Levene, H. (1960), in I. Olkin (Ed.), *Contributions to Probability and Statistics*,
//! Stanford University Press, pp. 278–292; Brown, M. B. & Forsythe, A. B. (1974), *JASA*
//! 69(346):364–367, doi:10.1080/01621459.1974.10482955.
//!
//! # Tolerances
//!
//! `F` and `P`: absolute **and** relative `1e-10` (the same bound as the stats-level oracle; the
//! routine is a closed-form ANOVA on deviations plus a regularized incomplete beta tail, so the only
//! error is floating-point rounding). Degrees of freedom and group sizes are exact integers (`1e-12`).
//! Group centres and mean absolute deviations: absolute `1e-12` (sums of at most eight values).

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, GroupedTable, LeveneCenter, LeveneTestSpec, ProjectDocument, ResultCell,
    ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

/// Four heteroscedastic groups used by the `scipy.stats.levene` / R oracles (`het` in the R script).
const HET: [&[f64]; 4] = [
    &[10.2, 10.8, 9.9, 11.1, 10.5, 10.3],
    &[12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1],
    &[18.5, 22.0, 16.4, 24.1, 19.8],
    &[25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9],
];

/// Two groups `g_a`, `g_b` from `d1_reference_oracles.rs` (R script section 0).
const TWO: [&[f64]; 2] = [&[23.1, 25.4, 21.8, 24.0, 26.2, 22.7], &[18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6]];

fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, what: &str) {
    let diff = (actual - expected).abs();
    let bound = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= bound, "{what}: actual {actual:.17e} vs expected {expected:.17e} (|diff| = {diff:.3e} > {bound:.3e})");
}

fn column_sheet(id: &str, groups: &[&[f64]]) -> DataSheet {
    let rows = groups.iter().map(|g| g.len()).max().unwrap_or(1).max(1);
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, groups.len(), rows).unwrap();
    for (c, (ds, values)) in t.data_sets.iter_mut().zip(groups.iter()).enumerate() {
        ds.title = format!("Group {}", c + 1);
        for (r, &v) in values.iter().enumerate() {
            ds.cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(id, "Variance groups", TableArchetype::Column(t)).unwrap()
}

fn session_with(sheets: Vec<DataSheet>) -> Session {
    let mut doc = ProjectDocument::new("R19.2 Levene").unwrap();
    doc.data_sheets.extend(sheets);
    Session::from_document(doc)
}

fn scalar(rs: &ResultSheet, key: &str, ds: Option<usize>) -> f64 {
    rs.scalars.iter().find(|s| s.key == key && s.data_set_index == ds).unwrap_or_else(|| panic!("missing scalar `{key}` ({ds:?})")).value
}

fn result<'a>(session: &'a Session, id: &str) -> &'a ResultSheet {
    session.document().result_sheet(&SheetId::try_new(id).unwrap()).expect("result sheet present")
}

fn summary_text(rs: &ResultSheet, label: &str) -> String {
    let tab = rs.tabs.iter().find(|t| t.id == "levene-summary").expect("levene-summary tab");
    let idx = tab.row_labels.iter().position(|l| l == label).unwrap_or_else(|| panic!("missing summary row `{label}`"));
    match tab.rows[idx].first() {
        Some(ResultCell::Text(t)) => t.clone(),
        other => panic!("row `{label}` is not text: {other:?}"),
    }
}

#[test]
fn d1_brown_forsythe_and_levene_match_scipy_and_r_through_session_execute() {
    let mut session = session_with(vec![column_sheet("het", &HET)]);

    // Default centre is the median (Brown–Forsythe).
    session.execute("stats.run", json!({ "sheet_id": "res-lev", "source_sheet_id": "het", "kind": "levene" })).expect("stats.run levene");
    let rs = result(&session, "res-lev");
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert_eq!(rs.analysis.kind, AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Median }));
    // SciPy `levene(center="median")` F = 6.458547396160991e0, p = 2.657922147045504e-3; R 4.5.2 agrees to 16 digits.
    assert_close(scalar(rs, "F", None), 6.458_547_396_160_991, 1e-10, 1e-10, "Brown-Forsythe F");
    assert_close(scalar(rs, "DFn", None), 3.0, 1e-12, 0.0, "DFn");
    assert_close(scalar(rs, "DFd", None), 22.0, 1e-12, 0.0, "DFd");
    assert_close(scalar(rs, "P value", None), 2.657_922_147_045_504e-3, 1e-10, 1e-10, "Brown-Forsythe P");
    assert_close(scalar(rs, "Total N", None), 26.0, 1e-12, 0.0, "Total N");
    assert_eq!(summary_text(rs, "Method"), LeveneCenter::Median.method_label());
    // R: sapply(het, median) and sapply(het, function(v) mean(abs(v - median(v)))).
    let medians = [10.40, 13.75, 19.80, 25.10];
    let mad_median = [0.333_333_333_333_333_3, 1.3875, 2.240_000_000_000_000_7, 0.271_428_571_428_571_2];
    let ns = [6.0, 8.0, 5.0, 7.0];
    for g in 0..4 {
        assert_close(scalar(rs, "N", Some(g)), ns[g], 1e-12, 0.0, "group n");
        assert_close(scalar(rs, "Center", Some(g)), medians[g], 1e-12, 0.0, "group median");
        assert_close(scalar(rs, "Mean abs deviation", Some(g)), mad_median[g], 1e-12, 0.0, "group mean |x - median|");
    }

    // Switch to the mean centre (Levene 1960) via analysis.updateParams.
    session.execute("analysis.updateParams", json!({ "sheet_id": "res-lev", "center": "mean" })).expect("updateParams center=mean");
    let rs = result(&session, "res-lev");
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert_eq!(rs.analysis.kind, AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Mean }));
    // SciPy `levene(center="mean")` F = 7.89549574624047e0, p = 9.328792479575741e-4.
    assert_close(scalar(rs, "F", None), 7.895_495_746_240_47, 1e-10, 1e-10, "Levene mean F");
    assert_close(scalar(rs, "P value", None), 9.328_792_479_575_741e-4, 1e-10, 1e-10, "Levene mean P");
    assert_eq!(summary_text(rs, "Method"), LeveneCenter::Mean.method_label());
    let means = [10.466_666_666_666_67, 13.7875, 20.16, 25.2];
    let mad_mean = [0.333_333_333_333_333_3, 1.3875, 2.312_000_000_000_000_3, 0.285_714_285_714_285_2];
    for g in 0..4 {
        assert_close(scalar(rs, "Center", Some(g)), means[g], 1e-12, 0.0, "group mean");
        assert_close(scalar(rs, "Mean abs deviation", Some(g)), mad_mean[g], 1e-12, 0.0, "group mean |x - mean|");
    }

    let report = generate_methods_report(session.document(), &SheetId::try_new("res-lev").unwrap(), ReportFormat::Markdown).expect("report");
    assert!(report.contains("Levene's test"), "methods report must name the mean-centered test");
}

#[test]
fn d1_two_group_brown_forsythe_and_mean_shorthand_via_analysis_run() {
    let mut session = session_with(vec![column_sheet("two", &TWO)]);
    session.execute("analysis.run", json!({ "sheet_id": "res-two", "source_sheet_id": "two", "kind": "brown_forsythe_variance" })).unwrap();
    let rs = result(&session, "res-two");
    // R 4.5.2 / scipy: F = 0.6762908105835386, p = 0.4283334917281029 (DFn = 1, DFd = 11).
    assert_close(scalar(rs, "F", None), 0.676_290_810_583_538_6, 1e-10, 1e-10, "two-group BF F");
    assert_close(scalar(rs, "P value", None), 0.428_333_491_728_102_9, 1e-10, 1e-10, "two-group BF P");
    assert_close(scalar(rs, "DFd", None), 11.0, 1e-12, 0.0, "two-group DFd");

    // `levene_mean` shorthand selects the mean centre; the four-group dataset reproduces the mean oracle.
    let mut session = session_with(vec![column_sheet("het", &HET)]);
    session.execute("stats.run", json!({ "sheet_id": "res-m", "source_sheet_id": "het", "kind": "levene_mean" })).unwrap();
    let rs = result(&session, "res-m");
    assert_close(scalar(rs, "F", None), 7.895_495_746_240_47, 1e-10, 1e-10, "levene_mean F");
}

#[test]
fn selected_columns_live_edit_and_text_cells() {
    let mut session = session_with(vec![column_sheet("het", &HET)]);
    // Selecting groups 1 and 3 must give the same answer as a two-group table of those columns.
    session.execute("stats.run", json!({ "sheet_id": "res-sel", "source_sheet_id": "het", "kind": "levene", "selected_data_sets": [1, 3] })).unwrap();
    let f_sel = scalar(result(&session, "res-sel"), "F", None);
    let mut reference = session_with(vec![column_sheet("pair", &[HET[1], HET[3]])]);
    reference.execute("stats.run", json!({ "sheet_id": "res-ref", "source_sheet_id": "pair", "kind": "levene" })).unwrap();
    assert_close(f_sel, scalar(result(&reference, "res-ref"), "F", None), 1e-12, 1e-12, "selected columns vs reference table");
    assert_close(scalar(result(&session, "res-sel"), "Groups", None), 2.0, 1e-12, 0.0, "group count");

    // Live edit of an upstream cell recomputes the result.
    session.execute("table.setCell", json!({ "sheet_id": "het", "row": 0, "col": 1, "subcol": 0, "value": 30.0 })).unwrap();
    let rs = result(&session, "res-sel");
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert!((scalar(rs, "F", None) - f_sel).abs() > 1e-6, "F must change after an upstream edit");

    // A text cell is not an observation: it is ignored and disclosed in the checklist.
    session.execute("table.setCell", json!({ "sheet_id": "het", "row": 0, "col": 3, "subcol": 0, "value": "n/a" })).unwrap();
    let rs = result(&session, "res-sel");
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert_close(scalar(rs, "N", Some(3)), 6.0, 1e-12, 0.0, "text cell dropped from group 4");
    assert!(rs.checklist.iter().any(|c| c.assumption == "Numeric input validation"), "ignored text cell must be disclosed");
}

#[test]
fn edge_cases_return_typed_errors_without_panicking() {
    let constant = column_sheet("const", &[&[5.0, 5.0, 5.0], &[7.0, 7.0, 7.0, 7.0]]);
    let pairs = column_sheet("pairs", &[&[1.0, 3.0], &[2.0, 8.0], &[4.0, 4.5]]);
    let singleton = column_sheet("single", &[&[1.0, 2.0, 3.0], &[4.0]]);
    let one_col = column_sheet("onecol", &[&[1.0, 2.0, 3.0]]);
    let grouped =
        DataSheet::new("grp", "Grouped", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap())).unwrap();
    let mut session = session_with(vec![constant, pairs, singleton, one_col, grouped, column_sheet("het", &HET)]);

    let run = |s: &mut Session, src: &str, extra: serde_json::Value| {
        let mut params = json!({ "source_sheet_id": src, "kind": "levene" });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        s.execute("stats.run", params)
    };

    // Analysis failures surface as an `Err` from `Session::execute` whose message carries the typed
    // `StatsError` text (the engine reports every failed result sheet with one shared error code).
    let expect_msg = |s: &mut Session, src: &str, extra: serde_json::Value, needle: &str| {
        let err = run(s, src, extra).expect_err(needle);
        assert!(err.message().contains(needle), "expected `{needle}` in `{}`", err.message());
    };
    // Zero spread of |x - centre| in every group: F undefined -> StatsError::ZeroVariance.
    expect_msg(&mut session, "const", json!({}), "Levene test absolute deviations");
    // Every group of size two has equal deviations, so F is undefined as well.
    expect_msg(&mut session, "pairs", json!({}), "sample variance is zero");
    // A group with n < 2 -> StatsError::InsufficientSampleSize("Levene group").
    expect_msg(&mut session, "single", json!({}), "Levene group: requires at least 2");
    // Fewer than two groups (one column, or one selected column) -> InsufficientSampleSize("Levene test").
    expect_msg(&mut session, "onecol", json!({}), "Levene test: requires at least 2");
    expect_msg(&mut session, "het", json!({ "selected_data_sets": [2] }), "Levene test: requires at least 2");
    // Out-of-range selection, incompatible table, bad centre value, inapplicable key.
    assert!(run(&mut session, "het", json!({ "selected_data_sets": [0, 9] })).is_err());
    assert!(run(&mut session, "grp", json!({})).is_err());
    assert_eq!(run(&mut session, "het", json!({ "center": "mode" })).expect_err("bad centre").code, ErrorCode::BadInput);
    assert_eq!(run(&mut session, "het", json!({ "threshold": 0.01 })).expect_err("inapplicable key").code, ErrorCode::BadInput);
    // No failed run left a result sheet behind.
    assert!(session.document().result_sheets.is_empty());
}
