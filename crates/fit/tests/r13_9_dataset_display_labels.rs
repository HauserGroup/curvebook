//! R13.9: per-dataset rows in multi-dataset fit results show column titles, not `DS n`.
//!
//! These tests cover display labels only. Fitted values come from an exact straight line plus a
//! fixed deterministic perturbation and are not compared against an oracle here. The numerical
//! behaviour of shared and local straight-line fits is verified in `d1_fit_uncertainty.rs`. The
//! checks below confirm that row labels follow the column titles, that blank and repeated titles
//! still give unique labels, and that published scalar keys (used by hooks and equation
//! annotations) are unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, FitMethod, NonlinearFitSpec, ParameterRule, ParameterScope, ResultSheet, ResultStatus,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};

const SHEET_ID: &str = "r13-9-sheet";
const XS: [f64; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
const NOISE: [f64; 6] = [0.05, -0.03, 0.02, -0.04, 0.01, -0.02];

/// Three-column XY sheet with `Y = 2X + (k + 1)` plus a small fixed perturbation in column `k`.
fn sheet_with_titles(titles: [&str; 3]) -> DataSheet {
    let mut xy = XyTable::new(SubcolumnMode::Single, 3, XS.len()).unwrap();
    for (k, title) in titles.iter().enumerate() {
        xy.data_sets[k].title = (*title).to_owned();
    }
    for (r, &x) in XS.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        for k in 0..3 {
            let intercept = (k + 1) as f64;
            xy.data_sets[k].cells[r][0] = Cell::number(2.0 * x + intercept + NOISE[(r + k) % NOISE.len()]);
        }
    }
    DataSheet::new(SHEET_ID, "R13.9 labels", TableArchetype::Xy(xy)).unwrap()
}

fn rule(name: &str, scope: ParameterScope) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope, initial_override: None, lower_bound: None, upper_bound: None }
}

/// Straight-line fit with a shared slope and the given intercept rule, emitting the covariance tab.
fn fit(sheet: &DataSheet, intercept_scope: ParameterScope) -> ResultSheet {
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.method = FitMethod::LeastSquares;
    nl.parameter_rules = vec![rule("Slope", ParameterScope::SharedGlobal), rule("YIntercept", intercept_scope)];
    nl.profile_likelihood_ci = false;
    nl.emit_covariance_tab = true;
    let spec = AnalysisSpec::new("r13-9-fit", SHEET_ID, AnalysisKind::NonlinearRegression(nl)).unwrap();
    let res = execute_analysis(&spec, sheet, &HookContext::default()).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);
    res
}

fn row_labels<'a>(res: &'a ResultSheet, tab_id: &str) -> &'a [String] {
    &res.tabs.iter().find(|t| t.id == tab_id).unwrap_or_else(|| panic!("missing tab {tab_id}")).row_labels
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
    assert_eq!(row_labels(&res, "nonlinear_parameters"), expected);
    assert_eq!(row_labels(&res, "nonlinear_covariance"), expected);
    let cov = res.tabs.iter().find(|t| t.id == "nonlinear_covariance").unwrap();
    assert_eq!(cov.column_headers, expected);

    // Scalar keys stay `name` + `data_set_index`, so hooks and annotations are unaffected.
    let params = res.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
    let risperidone_row = params.row_labels.iter().position(|l| l == "YIntercept [RISPERIDONE]").unwrap();
    let curvebook_doc::ResultCell::Number { value: shown, .. } = params.rows[risperidone_row][0] else { panic!("numeric estimate") };
    let scalar = res.lookup_scalar("YIntercept", Some(1)).expect("YIntercept scalar for column B");
    assert_eq!(shown.to_bits(), scalar.to_bits(), "row and scalar must report the same estimate");
    assert!(res.scalars.iter().all(|s| !s.key.contains("RISPERIDONE")), "scalar keys must not embed titles: {:?}", res.scalars);
}

#[test]
fn r13_9_blank_and_repeated_titles_stay_unique() {
    let res = fit(&sheet_with_titles(["VEHICLE", " vehicle ", "   "]), ParameterScope::Local);
    let labels = row_labels(&res, "nonlinear_parameters");
    assert_eq!(labels, ["Slope", "YIntercept [VEHICLE (DS1)]", "YIntercept [vehicle (DS2)]", "YIntercept [DS3]"]);
    assert_unique(labels);
}

#[test]
fn r13_9_title_matching_a_fallback_name_stays_unique() {
    // Column A is titled "DS2" and column B is blank, so both would otherwise read "DS2".
    let res = fit(&sheet_with_titles(["DS2", "", "C"]), ParameterScope::Local);
    let labels = row_labels(&res, "nonlinear_parameters");
    assert_eq!(labels, ["Slope", "YIntercept [DS2 (DS1)]", "YIntercept [DS2 (DS2)]", "YIntercept [C]"]);
    assert_unique(labels);
}

#[test]
fn r13_9_per_dataset_constants_show_titles_but_keep_stable_keys() {
    let pairs = vec![(0, 1.0), (1, 2.0), (2, 3.0)];
    let res = fit(&sheet_with_titles(["VEHICLE", "RISPERIDONE", "CLOZAPINE"]), ParameterScope::PerDatasetConstant(pairs));
    assert_eq!(
        row_labels(&res, "nonlinear_parameters"),
        ["Slope", "YIntercept [VEHICLE] (Constant)", "YIntercept [RISPERIDONE] (Constant)", "YIntercept [CLOZAPINE] (Constant)"]
    );
    // The stored constant keys keep the `[DS n]` form that links and equation annotations resolve.
    assert_eq!(res.lookup_scalar("YIntercept [DS1]", None), Some(1.0));
    assert_eq!(res.lookup_scalar("YIntercept [DS2]", None), Some(2.0));
    assert_eq!(res.lookup_scalar("YIntercept [DS3]", None), Some(3.0));
}

#[test]
fn r13_9_single_dataset_rows_have_no_suffix() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, XS.len()).unwrap();
    xy.data_sets[0].title = "VEHICLE".to_owned();
    for (r, &x) in XS.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0 + NOISE[r]);
    }
    let sheet = DataSheet::new(SHEET_ID, "R13.9 single", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.profile_likelihood_ci = false;
    let spec = AnalysisSpec::new("r13-9-single", SHEET_ID, AnalysisKind::NonlinearRegression(nl)).unwrap();
    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(row_labels(&res, "nonlinear_parameters"), ["Slope", "YIntercept"]);
}
