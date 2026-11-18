#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Audit findings #10, #11, and #12 (`crates/fit/*`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, FitMethod, GroupedTable, NonlinearFitSpec, ParameterRule, ParameterScope, ResultCell, SubcolumnMode,
    TableArchetype, XColumnMode, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitError, FitStatus, HookContext, ParameterNote, SolverConfig, execute_analysis, fit_nonlinear,
};
use curvebook_testkit::metrics::assert_approx_eq;

// ============================================================================
// Audit #10: Pre-summarized Grouped subcolumn extraction (no replicate inflation)
// ============================================================================

#[test]
fn audit_10_grouped_error_limits_and_median_iqr_extract_only_central_values() {
    // Ground truth line: y = 10 + 2*x at x = 1, 2, 3, 4 (centers = [12, 14, 16, 18]).
    // Put asymmetric error/limit bounds on subcolumns 1 & 2 (`center - 1`, `center + 4`).
    // If subcolumn 1 & 2 were mistakenly flattened as replicates, fitted mean y would shift up by +1.0 (intercept -> 11.0).
    for mode in [SubcolumnMode::UpperLowerLimits, SubcolumnMode::MeanLowHigh, SubcolumnMode::MedianIqr] {
        let mut gt = GroupedTable::new(mode, 1, 4).unwrap();
        gt.data_sets[0].title = String::from("Curve");
        gt.row_titles = vec![String::from("1"), String::from("2"), String::from("3"), String::from("4")];
        gt.row_numeric_spacing = Some(vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        for r in 0..4 {
            let x = (r + 1) as f64;
            let center = 10.0 + 2.0 * x;
            gt.data_sets[0].cells[r][0] = Cell::number(center);
            match mode {
                SubcolumnMode::MeanLowHigh => {
                    gt.data_sets[0].cells[r][1] = Cell::number(1.0); // -Error magnitude
                    gt.data_sets[0].cells[r][2] = Cell::number(4.0); // +Error magnitude
                }
                SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                    gt.data_sets[0].cells[r][1] = Cell::number(center - 1.0);
                    gt.data_sets[0].cells[r][2] = Cell::number(center + 4.0);
                }
                _ => panic!("unexpected subcolumn mode {mode:?}"),
            }
        }
        let sheet = DataSheet::new(format!("gt-{mode:?}"), "GroupedLimits", TableArchetype::Grouped(gt)).unwrap();

        let nl = NonlinearFitSpec::new("straight_line");
        let spec = AnalysisSpec::new("res-ok", sheet.id.clone(), AnalysisKind::NonlinearRegression(nl)).unwrap();
        let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();

        let gof = res.tabs.iter().find(|t| t.id == "nonlinear_gof").expect("nonlinear_gof tab");
        let n_idx = gof.row_labels.iter().position(|l| l == "Number of points analyzed").unwrap();
        let ResultCell::Number { value: n_obs, .. } = gof.rows[n_idx][0] else {
            panic!("expected Number cell for n_obs");
        };
        let y_int = res.scalars.iter().find(|s| s.key == "YIntercept").unwrap().value;
        let slope = res.scalars.iter().find(|s| s.key == "Slope").unwrap().value;
        assert_approx_eq(n_obs, 4.0, 1e-12, 1e-12, "only 4 central observations extracted");
        assert_approx_eq(y_int, 10.0, 1e-10, 1e-10, "intercept unaffected by limit/quartile subcolumns");
        assert_approx_eq(slope, 2.0, 1e-10, 1e-10, "slope unaffected by limit/quartile subcolumns");

        // Pre-summarized flag must block raw-replicate-only methods (e.g. RobustCauchy).
        let mut nl_robust = NonlinearFitSpec::new("straight_line");
        nl_robust.method = FitMethod::RobustCauchy;
        let spec_robust = AnalysisSpec::new("res-rob", sheet.id.clone(), AnalysisKind::NonlinearRegression(nl_robust)).unwrap();
        let err = execute_analysis(&spec_robust, &sheet, &HookContext::default()).unwrap_err();
        assert!(matches!(err, FitError::RequiresRawReplicates), "{mode:?} expected RequiresRawReplicates, got {err:?}");
    }
}

// ============================================================================
// Audit #11: Poisson IRLS rejects infeasible non-positive fitted means
// ============================================================================

#[test]
fn audit_11_poisson_irls_rejects_infeasible_non_positive_means() {
    let xs = [1.0, 2.0, 3.0, 4.0];
    let ys = [1.0, 3.0, 2.0, 4.0];
    let series = vec![DatasetSeries {
        data_set_index: 0,
        label: String::from("Counts"),
        points: xs
            .iter()
            .zip(ys)
            .enumerate()
            .map(|(row, (&x, y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: 0, row })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }];

    // Case 1: All-constant infeasible model mu = -1 <= 0 everywhere -> ImpossibleWeights (never Converged!).
    let model = CompiledDslModel::compile("line", "Y = A + B*X").unwrap();
    let cfg_const = SolverConfig {
        method: FitMethod::PoissonDeviance,
        parameter_rules: vec![
            ParameterRule {
                name: String::from("A"),
                scope: ParameterScope::Constant(-1.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: String::from("B"),
                scope: ParameterScope::Constant(0.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        ..SolverConfig::default()
    };
    let err1 = fit_nonlinear(&model, &series, &HookContext::default(), &cfg_const).unwrap_err();
    assert!(matches!(err1, FitError::ImpossibleWeights { .. }), "got {err1:?}");

    // Case 2: Box bounds forcing negative/zero predicted means at every feasible parameter vector.
    let cfg_bounded = SolverConfig {
        method: FitMethod::PoissonDeviance,
        parameter_rules: vec![
            ParameterRule {
                name: String::from("A"),
                scope: ParameterScope::Local,
                initial_override: Some(-2.0),
                lower_bound: Some(-10.0),
                upper_bound: Some(-0.5),
            },
            ParameterRule {
                name: String::from("B"),
                scope: ParameterScope::Local,
                initial_override: Some(-0.5),
                lower_bound: Some(-5.0),
                upper_bound: Some(-0.1),
            },
        ],
        ..SolverConfig::default()
    };
    let err2 = fit_nonlinear(&model, &series, &HookContext::default(), &cfg_bounded).unwrap_err();
    assert!(matches!(err2, FitError::ImpossibleWeights { .. }), "got {err2:?}");

    // Case 3: execute_analysis with negative predicted constant intercept and zero slope on XY sheet.
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 4).unwrap();
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number(xs[r])).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(ys[r]);
    }
    let sheet = DataSheet::new("xy-pois", "PoissonNeg", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.method = FitMethod::PoissonDeviance;
    nl.parameter_rules = vec![
        ParameterRule {
            name: String::from("YIntercept"),
            scope: ParameterScope::Constant(-1.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("Slope"),
            scope: ParameterScope::Constant(0.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    let spec = AnalysisSpec::new("res-pois", "xy-pois", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let err3 = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap_err();
    assert!(matches!(err3, FitError::ImpossibleWeights { .. }), "got {err3:?}");
}

// ============================================================================
// Audit #12: Equal inclusive bounds (lower == upper) treated as fixed constant
// ============================================================================

#[test]
fn audit_12_equal_inclusive_bounds_behave_as_fixed_constant_zero_free_params() {
    // 5-point dataset: y = 3.0 + 2.0 * x + small noise.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [5.1, 6.9, 9.05, 10.95, 13.02];
    let series = vec![DatasetSeries {
        data_set_index: 0,
        label: String::from("Line"),
        points: xs
            .iter()
            .zip(ys)
            .enumerate()
            .map(|(row, (&x, y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: 0, row })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }];
    let model = CompiledDslModel::compile("line", "Y = B0 + B1*X").unwrap();

    let cfg_const = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: String::from("B1"),
            scope: ParameterScope::Constant(2.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let cfg_eq = SolverConfig {
        parameter_rules: vec![ParameterRule {
            name: String::from("B1"),
            scope: ParameterScope::Local,
            initial_override: Some(5.0), // overridden safely when bounds fix parameter to 2.0
            lower_bound: Some(2.0),
            upper_bound: Some(2.0),
        }],
        ..SolverConfig::default()
    };

    let fit_const = fit_nonlinear(&model, &series, &HookContext::default(), &cfg_const).unwrap();
    let fit_eq = fit_nonlinear(&model, &series, &HookContext::default(), &cfg_eq).unwrap();

    assert_eq!(fit_const.status, FitStatus::Converged);
    assert_eq!(fit_eq.status, FitStatus::Converged);
    assert_eq!(fit_eq.k_free, 1);
    assert_eq!(fit_eq.df, 4);
    assert_eq!(fit_eq.k_free, fit_const.k_free);
    assert_eq!(fit_eq.df, fit_const.df);
    assert_eq!(fit_eq.parameters.len(), 1);
    assert_eq!(fit_eq.fixed_constants, vec![(String::from("B1"), 2.0, vec![ParameterNote::Constant])]);
    assert_eq!(fit_eq.covariance.covariance_matrix.len(), 1);
    assert_eq!(fit_eq.covariance.covariance_matrix[0].len(), 1);
    assert_approx_eq(fit_eq.parameters[0].estimate, fit_const.parameters[0].estimate, 1e-14, 1e-14, "B0 estimate parity");
    assert_approx_eq(fit_eq.parameters[0].se, fit_const.parameters[0].se, 1e-14, 1e-14, "B0 SE parity");
    assert_approx_eq(fit_eq.aic_c.unwrap(), fit_const.aic_c.unwrap(), 1e-14, 1e-14, "AICc parity");

    // End-to-end ResultSheet also reflects zero free params consumed by equal bounds.
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 5).unwrap();
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number(xs[r])).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(ys[r]);
    }
    let sheet = DataSheet::new("xy-eqb", "EqBounds", TableArchetype::Xy(xy)).unwrap();
    let mut nl_eq = NonlinearFitSpec::new("straight_line");
    nl_eq.parameter_rules = vec![ParameterRule {
        name: String::from("Slope"),
        scope: ParameterScope::Local,
        initial_override: None,
        lower_bound: Some(2.0),
        upper_bound: Some(2.0),
    }];
    let spec_eq = AnalysisSpec::new("res-eqb", "xy-eqb", AnalysisKind::NonlinearRegression(nl_eq)).unwrap();
    let res_eq = execute_analysis(&spec_eq, &sheet, &HookContext::default()).unwrap();
    let gof = res_eq.tabs.iter().find(|t| t.id == "nonlinear_gof").expect("nonlinear_gof tab");
    let df_idx = gof.row_labels.iter().position(|l| l == "Degrees of Freedom").unwrap();
    let ResultCell::Number { value: df_res, .. } = gof.rows[df_idx][0] else {
        panic!("expected Number cell for Degrees of Freedom");
    };
    assert_approx_eq(df_res, 4.0, 1e-12, 1e-12, "ResultSheet df == 4 for equal bounds");
}
