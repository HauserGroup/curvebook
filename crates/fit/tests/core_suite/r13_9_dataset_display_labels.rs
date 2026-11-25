//! R13.9: per-dataset rows in multi-dataset fit results show column titles, not `DS n`.
//!
//! These tests cover display labels only. Fitted values come from an exact straight line plus a
//! fixed deterministic perturbation and are not compared against an oracle here. The numerical
//! behaviour of shared and local straight-line fits is verified in `d1_fit_uncertainty.rs`. The
//! checks below confirm that row labels follow the column titles, that blank and repeated titles
//! still give unique labels, and that published scalar keys (used by hooks and equation
//! annotations) are unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{DataSheet, FitMethod, ParameterRule, ParameterScope, ResultSheet, ResultStatus};

use crate::common::{cell_num, exec_nl, find_tab, nl_fast, rule_shared, tab_row, xy_sheet, xy_sheet_single};

const XS: [f64; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
const NOISE: [f64; 6] = [0.05, -0.03, 0.02, -0.04, 0.01, -0.02];

fn sheet_with_titles(titles: [&str; 3]) -> DataSheet {
    let ys: [Vec<f64>; 3] =
        std::array::from_fn(|k| XS.iter().enumerate().map(|(r, &x)| 2.0 * x + (k + 1) as f64 + NOISE[(r + k) % NOISE.len()]).collect());
    xy_sheet("r13-9-sheet", "R13.9 labels", &XS, &[(titles[0], &ys[0]), (titles[1], &ys[1]), (titles[2], &ys[2])])
}

fn fit(sheet: &DataSheet, intercept_scope: ParameterScope) -> ResultSheet {
    let mut nl = nl_fast("straight_line");
    nl.method = FitMethod::LeastSquares;
    nl.emit_covariance_tab = true;
    nl.parameter_rules = vec![
        rule_shared("Slope"),
        ParameterRule { name: "YIntercept".to_owned(), scope: intercept_scope, initial_override: None, lower_bound: None, upper_bound: None },
    ];
    let res = exec_nl(sheet, nl);
    assert_eq!(res.status, ResultStatus::UpToDate);
    res
}

fn assert_unique(labels: &[String]) {
    let mut sorted = labels.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), labels.len(), "row labels must be unique: {labels:?}");
}

#[test]
fn r13_9_parameter_and_covariance_rows_use_column_titles() {
    let res = fit(&sheet_with_titles(["VEHICLE", "RISPERIDONE", "CLOZAPINE"]), ParameterScope::Local);
    let expected = ["Slope", "YIntercept [VEHICLE]", "YIntercept [RISPERIDONE]", "YIntercept [CLOZAPINE]"];
    assert_eq!(find_tab(&res, "nonlinear_parameters").row_labels, expected);
    let cov = find_tab(&res, "nonlinear_covariance");
    assert_eq!(cov.row_labels, expected);
    assert_eq!(cov.column_headers, expected);

    // Scalar keys stay `name` + `data_set_index`, so hooks and annotations are unaffected.
    let shown = cell_num(&tab_row(&res, "nonlinear_parameters", "YIntercept [RISPERIDONE]")[0]);
    let scalar = res.lookup_scalar("YIntercept", Some(1)).expect("YIntercept scalar for column B");
    assert_eq!(shown.to_bits(), scalar.to_bits(), "row and scalar must report the same estimate");
    assert!(res.scalars.iter().all(|s| !s.key.contains("RISPERIDONE")), "scalar keys must not embed titles: {:?}", res.scalars);
}

#[test]
fn r13_9_blank_and_repeated_titles_stay_unique() {
    let res = fit(&sheet_with_titles(["VEHICLE", " vehicle ", "   "]), ParameterScope::Local);
    let labels = &find_tab(&res, "nonlinear_parameters").row_labels;
    assert_eq!(labels, &["Slope", "YIntercept [VEHICLE (DS1)]", "YIntercept [vehicle (DS2)]", "YIntercept [DS3]"]);
    assert_unique(labels);
}

#[test]
fn r13_9_title_matching_a_fallback_name_stays_unique() {
    // Column A is titled "DS2" and column B is blank, so both would otherwise read "DS2".
    let res = fit(&sheet_with_titles(["DS2", "", "C"]), ParameterScope::Local);
    let labels = &find_tab(&res, "nonlinear_parameters").row_labels;
    assert_eq!(labels, &["Slope", "YIntercept [DS2 (DS1)]", "YIntercept [DS2 (DS2)]", "YIntercept [C]"]);
    assert_unique(labels);
}

#[test]
fn r13_9_per_dataset_constants_show_titles_but_keep_stable_keys() {
    let pairs = vec![(0, 1.0), (1, 2.0), (2, 3.0)];
    let res = fit(&sheet_with_titles(["VEHICLE", "RISPERIDONE", "CLOZAPINE"]), ParameterScope::PerDatasetConstant(pairs));
    assert_eq!(
        find_tab(&res, "nonlinear_parameters").row_labels,
        ["Slope", "YIntercept [VEHICLE] (Constant)", "YIntercept [RISPERIDONE] (Constant)", "YIntercept [CLOZAPINE] (Constant)"]
    );
    // The stored constant keys keep the `[DS n]` form that links and equation annotations resolve.
    for (k, v) in [("YIntercept [DS1]", 1.0), ("YIntercept [DS2]", 2.0), ("YIntercept [DS3]", 3.0)] {
        assert_eq!(res.lookup_scalar(k, None), Some(v));
    }
}

#[test]
fn r13_9_single_dataset_rows_have_no_suffix() {
    let ys: Vec<f64> = XS.iter().enumerate().map(|(r, &x)| 2.0 * x + 1.0 + NOISE[r]).collect();
    let sheet = xy_sheet_single("r13-9-sheet", "R13.9 single", "VEHICLE", &XS, &ys);
    let res = exec_nl(&sheet, nl_fast("straight_line"));
    assert_eq!(find_tab(&res, "nonlinear_parameters").row_labels, ["Slope", "YIntercept"]);
}
