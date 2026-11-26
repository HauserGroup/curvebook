#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Audit findings #10, #11, and #12 (`crates/fit/*`).

use curvebook_doc::{Cell, DataSheet, FitMethod, GroupedTable, NonlinearFitSpec, SubcolumnMode, TableArchetype};
use curvebook_fit::{FitError, FitStatus, HookContext, ParameterNote, SolverConfig, fit_nonlinear};
use curvebook_testkit::metrics::assert_approx_eq;

use crate::common::{
    cell_num, cfg_rules, dsl_compile, exec_nl, exec_nl_res, fit_with, rule_const, rule_local_bounds, series_xy, tab_row, xy_sheet_single,
};

#[test]
fn audit_10_grouped_error_limits_and_median_iqr_extract_only_central_values() {
    for mode in [SubcolumnMode::UpperLowerLimits, SubcolumnMode::MeanLowHigh, SubcolumnMode::MedianIqr] {
        let mut gt = GroupedTable::new(mode, 1, 4).unwrap();
        gt.data_sets[0].title = "Curve".to_owned();
        gt.row_titles = ["1", "2", "3", "4"].into_iter().map(str::to_owned).collect();
        gt.row_numeric_spacing = Some(vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        for r in 0..4 {
            let center = 10.0 + 2.0 * ((r + 1) as f64);
            gt.data_sets[0].cells[r] = match mode {
                SubcolumnMode::MeanLowHigh => vec![Cell::number(center), Cell::number(1.0), Cell::number(4.0)],
                SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                    vec![Cell::number(center), Cell::number(center - 1.0), Cell::number(center + 4.0)]
                }
                _ => panic!("unexpected mode {mode:?}"),
            };
        }
        let sheet = DataSheet::new(format!("gt-{mode:?}"), "GroupedLimits", TableArchetype::Grouped(gt)).unwrap();
        let res = exec_nl(&sheet, NonlinearFitSpec::new("straight_line"));

        assert_approx_eq(cell_num(&tab_row(&res, "nonlinear_gof", "Number of points analyzed")[0]), 4.0, 1e-12, 1e-12, "n_obs");
        assert_approx_eq(res.scalars.iter().find(|s| s.key == "YIntercept").unwrap().value, 10.0, 1e-10, 1e-10, "intercept");
        assert_approx_eq(res.scalars.iter().find(|s| s.key == "Slope").unwrap().value, 2.0, 1e-10, 1e-10, "slope");

        let err = exec_nl_res(&sheet, NonlinearFitSpec { method: FitMethod::RobustCauchy, ..NonlinearFitSpec::new("straight_line") }).unwrap_err();
        assert!(matches!(err, FitError::RequiresRawReplicates), "{mode:?}: {err:?}");
    }
}

#[test]
fn audit_11_poisson_irls_rejects_infeasible_non_positive_means() {
    let (xs, ys) = ([1.0, 2.0, 3.0, 4.0], [1.0, 3.0, 2.0, 4.0]);
    let series = [series_xy(0, "Counts", &xs, &ys)];
    let model = dsl_compile("Y = A + B*X");

    for rules in [
        vec![rule_const("A", -1.0), rule_const("B", 0.0)],
        vec![rule_local_bounds("A", Some(-2.0), Some(-10.0), Some(-0.5)), rule_local_bounds("B", Some(-0.5), Some(-5.0), Some(-0.1))],
    ] {
        let cfg = SolverConfig { method: FitMethod::PoissonDeviance, parameter_rules: rules, ..SolverConfig::default() };
        assert!(matches!(fit_nonlinear(&model, &series, &HookContext::default(), &cfg), Err(FitError::ImpossibleWeights { .. })));
    }

    let sheet = xy_sheet_single("xy-pois", "PoissonNeg", "Y", &xs, &ys);
    let nl = NonlinearFitSpec {
        method: FitMethod::PoissonDeviance,
        parameter_rules: vec![rule_const("YIntercept", -1.0), rule_const("Slope", 0.0)],
        ..NonlinearFitSpec::new("straight_line")
    };
    assert!(matches!(exec_nl_res(&sheet, nl), Err(FitError::ImpossibleWeights { .. })));
}

#[test]
fn audit_12_equal_inclusive_bounds_behave_as_fixed_constant_zero_free_params() {
    let (xs, ys) = ([1.0, 2.0, 3.0, 4.0, 5.0], [5.1, 6.9, 9.05, 10.95, 13.02]);
    let series = [series_xy(0, "Line", &xs, &ys)];
    let model = dsl_compile("Y = B0 + B1*X");

    let fit_const = fit_with(&model, &series, &cfg_rules(vec![rule_const("B1", 2.0)]));
    let fit_eq = fit_with(&model, &series, &cfg_rules(vec![rule_local_bounds("B1", Some(5.0), Some(2.0), Some(2.0))]));

    assert_eq!((fit_const.status, fit_eq.status), (FitStatus::Converged, FitStatus::Converged));
    assert_eq!((fit_eq.k_free, fit_eq.df, fit_eq.parameters.len()), (1, 4, 1));
    assert_eq!((fit_eq.k_free, fit_eq.df), (fit_const.k_free, fit_const.df));
    assert_eq!(fit_eq.fixed_constants, vec![("B1".to_owned(), 2.0, vec![ParameterNote::Constant])]);
    assert_eq!((fit_eq.covariance.covariance_matrix.len(), fit_eq.covariance.covariance_matrix[0].len()), (1, 1));
    assert_approx_eq(fit_eq.parameters[0].estimate, fit_const.parameters[0].estimate, 1e-14, 1e-14, "B0 estimate");
    assert_approx_eq(fit_eq.parameters[0].se, fit_const.parameters[0].se, 1e-14, 1e-14, "B0 SE");
    assert_approx_eq(fit_eq.aic_c.unwrap(), fit_const.aic_c.unwrap(), 1e-14, 1e-14, "AICc");

    let sheet = xy_sheet_single("xy-eqb", "EqBounds", "Y", &xs, &ys);
    let nl =
        NonlinearFitSpec { parameter_rules: vec![rule_local_bounds("Slope", None, Some(2.0), Some(2.0))], ..NonlinearFitSpec::new("straight_line") };
    let res_eq = exec_nl(&sheet, nl);
    assert_approx_eq(cell_num(&tab_row(&res_eq, "nonlinear_gof", "Degrees of Freedom")[0]), 4.0, 1e-12, 1e-12, "ResultSheet df");
}
