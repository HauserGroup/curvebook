#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! End-to-end `D1` oracle and edge-case tests for the standalone Levene / Brown–Forsythe
//! homogeneity-of-variance analysis (`R19.2`, `AnalysisKind::LeveneTest`), executed through
//! `Session::execute` (`stats.run`, `analysis.run`, `analysis.updateParams`, `table.setCell`).
//!
//! # D1 provenance
//! - SciPy 1.18.1 `scipy.stats.levene(*groups, center="median" | "mean")` (`scipy_statsmodels_oracles.py` §4)
//! - R 4.5.2 (2025-10-31) `anova(lm(abs(x - c) ~ g))` (`stats_extended_oracles.R` §§0, 3)
//! - Levene (1960); Brown & Forsythe (1974), *JASA* 69(346):364–367.
//!
//! # Tolerances
//! `F` and `P`: absolute and relative `1e-10`; DF and group sizes: `1e-12`; centres and MADs: `1e-12`.

use curvebook_doc::{
    AnalysisKind, DataSheet, ErrorCode, GroupedTable, LeveneCenter, LeveneTestSpec, ResultCell, ResultStatus, SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_testkit::assert_approx_eq;
use serde_json::json;

use super::common::{col_sheet, ok_cmd, rs, scalar_ds, session_with};

const HET: [&[f64]; 4] = [
    &[10.2, 10.8, 9.9, 11.1, 10.5, 10.3],
    &[12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1],
    &[18.5, 22.0, 16.4, 24.1, 19.8],
    &[25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9],
];
const TWO: [&[f64]; 2] = [&[23.1, 25.4, 21.8, 24.0, 26.2, 22.7], &[18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6]];

fn groups_sheet(id: &str, groups: &[&[f64]]) -> DataSheet {
    let names: Vec<String> = (1..=groups.len()).map(|i| format!("Group {i}")).collect();
    let cols: Vec<(&str, &[f64])> = names.iter().map(String::as_str).zip(groups.iter().copied()).collect();
    col_sheet(id, "Variance groups", &cols)
}

fn summary_text(s: &Session, id: &str, label: &str) -> String {
    let tab = rs(s, id).tabs.iter().find(|t| t.id == "levene-summary").unwrap();
    let idx = tab.row_labels.iter().position(|l| l == label).unwrap();
    match tab.rows[idx].first() {
        Some(ResultCell::Text(t)) => t.clone(),
        other => panic!("row `{label}` not text: {other:?}"),
    }
}

#[test]
fn d1_brown_forsythe_and_levene_match_scipy_and_r_through_session_execute() {
    let mut s = session_with("R19.2 Levene", vec![groups_sheet("het", &HET)]);
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-lev", "source_sheet_id": "het", "kind": "levene" }));
    assert_eq!(
        (rs(&s, "res-lev").status.clone(), &rs(&s, "res-lev").analysis.kind),
        (ResultStatus::UpToDate, &AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Median }))
    );
    for (k, exp, atol, rtol) in [
        ("F", 6.458_547_396_160_991, 1e-10, 1e-10),
        ("DFn", 3.0, 1e-12, 0.0),
        ("DFd", 22.0, 1e-12, 0.0),
        ("P value", 2.657_922_147_045_504e-3, 1e-10, 1e-10),
        ("Total N", 26.0, 1e-12, 0.0),
    ] {
        assert_approx_eq(scalar_ds(&s, "res-lev", k, None).unwrap(), exp, atol, rtol, k);
    }
    assert_eq!(summary_text(&s, "res-lev", "Method"), LeveneCenter::Median.method_label());
    for (g, (n, med, mad)) in
        [(6.0, 10.40, 0.333_333_333_333_333_3), (8.0, 13.75, 1.3875), (5.0, 19.80, 2.240_000_000_000_000_7), (7.0, 25.10, 0.271_428_571_428_571_2)]
            .into_iter()
            .enumerate()
    {
        assert_approx_eq(scalar_ds(&s, "res-lev", "N", Some(g)).unwrap(), n, 1e-12, 0.0, "n");
        assert_approx_eq(scalar_ds(&s, "res-lev", "Center", Some(g)).unwrap(), med, 1e-12, 0.0, "median");
        assert_approx_eq(scalar_ds(&s, "res-lev", "Mean abs deviation", Some(g)).unwrap(), mad, 1e-12, 0.0, "mad_median");
    }

    ok_cmd(&mut s, "analysis.updateParams", json!({ "sheet_id": "res-lev", "center": "mean" }));
    assert_eq!(
        (rs(&s, "res-lev").status.clone(), &rs(&s, "res-lev").analysis.kind),
        (ResultStatus::UpToDate, &AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Mean }))
    );
    assert_approx_eq(scalar_ds(&s, "res-lev", "F", None).unwrap(), 7.895_495_746_240_47, 1e-10, 1e-10, "Levene mean F");
    assert_approx_eq(scalar_ds(&s, "res-lev", "P value", None).unwrap(), 9.328_792_479_575_741e-4, 1e-10, 1e-10, "Levene mean P");
    assert_eq!(summary_text(&s, "res-lev", "Method"), LeveneCenter::Mean.method_label());
    for (g, (mean, mad)) in
        [(10.466_666_666_666_67, 0.333_333_333_333_333_3), (13.7875, 1.3875), (20.16, 2.312_000_000_000_000_3), (25.2, 0.285_714_285_714_285_2)]
            .into_iter()
            .enumerate()
    {
        assert_approx_eq(scalar_ds(&s, "res-lev", "Center", Some(g)).unwrap(), mean, 1e-12, 0.0, "mean");
        assert_approx_eq(scalar_ds(&s, "res-lev", "Mean abs deviation", Some(g)).unwrap(), mad, 1e-12, 0.0, "mad_mean");
    }
    assert!(generate_methods_report(s.document(), &SheetId::new("res-lev"), ReportFormat::Markdown).unwrap().contains("Levene's test"));
}

#[test]
fn d1_two_group_brown_forsythe_and_mean_shorthand_via_analysis_run() {
    let mut s = session_with("R19.2 Two", vec![groups_sheet("two", &TWO), groups_sheet("het", &HET)]);
    ok_cmd(&mut s, "analysis.run", json!({ "sheet_id": "res-two", "source_sheet_id": "two", "kind": "brown_forsythe_variance" }));
    assert_approx_eq(scalar_ds(&s, "res-two", "F", None).unwrap(), 0.676_290_810_583_538_6, 1e-10, 1e-10, "two-group BF F");
    assert_approx_eq(scalar_ds(&s, "res-two", "P value", None).unwrap(), 0.428_333_491_728_102_9, 1e-10, 1e-10, "two-group BF P");
    assert_approx_eq(scalar_ds(&s, "res-two", "DFd", None).unwrap(), 11.0, 1e-12, 0.0, "two-group DFd");
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-m", "source_sheet_id": "het", "kind": "levene_mean" }));
    assert_approx_eq(scalar_ds(&s, "res-m", "F", None).unwrap(), 7.895_495_746_240_47, 1e-10, 1e-10, "levene_mean F");
}

#[test]
fn selected_columns_live_edit_and_text_cells() {
    let mut s = session_with("R19.2 Sel", vec![groups_sheet("het", &HET), groups_sheet("pair", &[HET[1], HET[3]])]);
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-sel", "source_sheet_id": "het", "kind": "levene", "selected_data_sets": [1, 3] }));
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-ref", "source_sheet_id": "pair", "kind": "levene" }));
    let f_sel = scalar_ds(&s, "res-sel", "F", None).unwrap();
    assert_approx_eq(f_sel, scalar_ds(&s, "res-ref", "F", None).unwrap(), 1e-12, 1e-12, "selected vs pair");
    assert_approx_eq(scalar_ds(&s, "res-sel", "Groups", None).unwrap(), 2.0, 1e-12, 0.0, "group count");

    ok_cmd(&mut s, "table.setCell", json!({ "sheet_id": "het", "row": 0, "col": 1, "subcol": 0, "value": 30.0 }));
    assert!(rs(&s, "res-sel").status == ResultStatus::UpToDate && (scalar_ds(&s, "res-sel", "F", None).unwrap() - f_sel).abs() > 1e-6);

    ok_cmd(&mut s, "table.setCell", json!({ "sheet_id": "het", "row": 0, "col": 3, "subcol": 0, "value": "n/a" }));
    assert_eq!(rs(&s, "res-sel").status, ResultStatus::UpToDate);
    assert_approx_eq(scalar_ds(&s, "res-sel", "N", Some(3)).unwrap(), 6.0, 1e-12, 0.0, "text cell dropped");
    assert!(rs(&s, "res-sel").checklist.iter().any(|c| c.assumption == "Numeric input validation"));
}

#[test]
fn edge_cases_return_typed_errors_without_panicking() {
    let mut s = session_with(
        "R19.2 Edge",
        vec![
            groups_sheet("const", &[&[5.0, 5.0, 5.0], &[7.0, 7.0, 7.0, 7.0]]),
            groups_sheet("pairs", &[&[1.0, 3.0], &[2.0, 8.0], &[4.0, 4.5]]),
            groups_sheet("single", &[&[1.0, 2.0, 3.0], &[4.0]]),
            groups_sheet("onecol", &[&[1.0, 2.0, 3.0]]),
            DataSheet::new("grp", "Grouped", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap())).unwrap(),
            groups_sheet("het", &HET),
        ],
    );
    let run = |s: &mut Session, src: &str, extra: serde_json::Value| {
        let mut params = json!({ "source_sheet_id": src, "kind": "levene" });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        s.execute("stats.run", params).unwrap_err()
    };
    for (src, extra, needle) in [
        ("const", json!({}), "Levene test absolute deviations"),
        ("pairs", json!({}), "sample variance is zero"),
        ("single", json!({}), "Levene group: requires at least 2"),
        ("onecol", json!({}), "Levene test: requires at least 2"),
        ("het", json!({ "selected_data_sets": [2] }), "Levene test: requires at least 2"),
    ] {
        let msg = run(&mut s, src, extra).message().to_owned();
        assert!(msg.contains(needle), "expected `{needle}` in `{msg}`");
    }
    let _ = (run(&mut s, "het", json!({ "selected_data_sets": [0, 9] })), run(&mut s, "grp", json!({})));
    assert_eq!(run(&mut s, "het", json!({ "center": "mode" })).code, ErrorCode::BadInput);
    assert_eq!(run(&mut s, "het", json!({ "threshold": 0.01 })).code, ErrorCode::BadInput);
    assert!(s.document().result_sheets.is_empty());
}
