//! Independent non-NIST equation and result-sheet checks. The prediction cases use reductions
//! of published equations. The noisy fits use `reference/non_nist_fits.py` for independent
//! least-squares estimates, predicted responses, residual objectives, and standard errors.

use std::f64::consts::{E, FRAC_PI_2, LN_2};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, NonlinearFitSpec, ParameterRule, ParameterScope, ResultCell, ResultSheet, ResultStatus,
    SubcolumnMode, TableArchetype, WeightingScheme, XyTable,
};
use curvebook_fit::{FitError, HookContext, execute_analysis, get_builtin_model};
use curvebook_testkit::assert_approx_eq;

// Analytical oracles allow rounding in exp, sin, and log operations at ordinary inputs.
const PREDICTION_ABS_TOL: f64 = 2e-13;
// The Gaussian fit stops at a finite nonlinear convergence tolerance and its production
// Jacobian uses finite differences. Compare it to the high-precision stationary solution.
const WEIGHTED_FIT_ABS_TOL: f64 = 5e-7;
// Fixing the knot makes the remaining objective quadratic. Its LM solve should agree more
// closely with the independent linear solve, accounting for its finite-difference Jacobian.
const CONSTRAINED_FIT_ABS_TOL: f64 = 2e-8;

struct PredictionOracle<'a> {
    model: &'static str,
    params: &'a [f64],
    points: &'a [(f64, f64)],
}

#[test]
fn built_in_predictions_match_analytical_references() {
    let cases = [
        // Limpert et al. (2001). Multiples of GeoSD give integer standardized log offsets.
        PredictionOracle {
            model: "log_gaussian",
            params: &[10.0, 8.0, 2.0],
            points: &[(2.0, 10.0 / (E * E)), (4.0, 10.0 / E.sqrt()), (8.0, 10.0), (16.0, 10.0 / E.sqrt()), (32.0, 10.0 / (E * E))],
        },
        // Weibull (1951). At half, one, and two scales the decay is exp(-1/4), exp(-1), exp(-4).
        PredictionOracle {
            model: "weibull_peak",
            params: &[12.0, 3.0, 2.0],
            points: &[(0.0, 0.0), (1.5, 4.0 / E.sqrt().sqrt()), (3.0, 8.0 / E), (6.0, 16.0 / E.powi(4))],
        },
        // Damped sinusoid with quarter-turn samples (Bloomfield 2000 for the sinusoid).
        // A decay constant ln(2) halves displacement amplitude in each unit of time.
        PredictionOracle {
            model: "damped_sine_wave",
            params: &[8.0, 0.25, FRAC_PI_2, LN_2, 3.0],
            points: &[(0.0, 11.0), (1.0, 3.0), (2.0, 1.0), (3.0, 3.0), (4.0, 3.5)],
        },
        // Watts & Bacon (1974). At distance 3 from the join with Delta = 4, the
        // hyperbolic correction is 5 - 4 by the 3-4-5 triangle.
        PredictionOracle { model: "hinge_smooth_hyperbolic", params: &[5.0, 2.0, 1.0, 3.0, 4.0], points: &[(-1.0, 0.0), (2.0, 5.0), (5.0, 12.0)] },
        // Hill (1910). At half, one, and two Kd, occupancy odds are 1:4, 1:1, and 4:1.
        PredictionOracle { model: "one_site_specific_hill", params: &[40.0, 2.0, 2.0], points: &[(0.0, 0.0), (1.0, 8.0), (2.0, 20.0), (4.0, 32.0)] },
        // Michaelis & Menten (1913). At half, one, and three Km the fractions of
        // the maximum rate kcat * Et are 1/3, 1/2, and 3/4.
        PredictionOracle { model: "kcat_enzyme_kinetics", params: &[6.0, 4.0, 3.0], points: &[(0.0, 0.0), (2.0, 6.0), (4.0, 9.0), (12.0, 13.5)] },
        // Haldane (1930), Segel (1975). S = sqrt(Km * Ki) = 4 is the maximum,
        // with matched responses at 2 and 8 and further inhibition at 16.
        PredictionOracle {
            model: "substrate_inhibition",
            params: &[45.0, 2.0, 8.0],
            points: &[(0.0, 0.0), (2.0, 20.0), (4.0, 22.5), (8.0, 20.0), (16.0, 14.4)],
        },
        // Morrison (1969). Choose bound complex C = 0, 1, 2.5, and 4.
        // With apparent Ki = 2, the total inhibitor is C + 2*C/(5-C);
        // the free enzyme fractions are 1, 4/5, 1/2, and 1/5.
        PredictionOracle {
            model: "morrison_tight_binding_ki",
            params: &[75.0, 5.0, 1.0, 4.0, 4.0],
            points: &[(0.0, 75.0), (1.5, 60.0), (4.5, 37.5), (12.0, 15.0)],
        },
    ];

    for case in cases {
        let model = get_builtin_model(case.model).unwrap();
        for &(x, expected) in case.points {
            let label = format!("{} at x={x}", case.model);
            assert_approx_eq(model.eval(&[x], case.params, 0).unwrap_or(f64::NAN), expected, PREDICTION_ABS_TOL, 0.0, &label);
        }
    }
}

fn fit_sheet<const N: usize>(xs: [f64; N], ys: [f64; N], nonlinear: NonlinearFitSpec) -> Result<ResultSheet, FitError> {
    let xy = XyTable::new(SubcolumnMode::Single, 1, N)?;
    let mut sheet = DataSheet::new("fit-oracle-data", "Fit oracle data", TableArchetype::Xy(xy))?;
    for (row, (x, y)) in xs.into_iter().zip(ys).enumerate() {
        sheet.set_x_or_time_cell(row, 0, Cell::number(x))?;
        sheet.set_cell(CellCoord::new(row, 0, 0), Cell::number(y))?;
    }
    let spec = AnalysisSpec::new("fit-oracle", "fit-oracle-data", AnalysisKind::NonlinearRegression(nonlinear))?;
    execute_analysis(&spec, &sheet, &HookContext::default())
}

fn parameter_rule(name: &str, scope: ParameterScope, initial_override: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope, initial_override, lower_bound: None, upper_bound: None }
}

fn result_row<'a>(result: &'a ResultSheet, tab_id: &str, label: &str) -> Option<&'a [ResultCell]> {
    let tab = result.tabs.iter().find(|tab| tab.id == tab_id)?;
    let row = tab.row_labels.iter().position(|name| name == label)?;
    tab.rows.get(row).map(Vec::as_slice)
}

fn assert_number(cell: Option<&ResultCell>, expected: f64, tol: f64, label: &str) {
    let actual = match cell {
        Some(ResultCell::Number { value, .. }) => *value,
        _ => f64::NAN,
    };
    assert_approx_eq(actual, expected, tol, 0.0, label);
}

fn assert_parameter(result: &ResultSheet, name: &str, estimate: f64, se: f64, tol: f64) {
    let row = result_row(result, "nonlinear_parameters", name).unwrap_or(&[]);
    assert_number(row.first(), estimate, tol, &format!("{name} estimate"));
    assert_number(row.get(1), se, tol, &format!("{name} standard error"));
}

fn assert_predictions(result: &ResultSheet, expected: &[f64], tol: f64) {
    let residuals = result.tabs.iter().find(|tab| tab.id == "nonlinear_residuals");
    assert_eq!(residuals.map(|tab| tab.rows.len()), Some(expected.len()));
    for (i, &value) in expected.iter().enumerate() {
        let cell = residuals.and_then(|tab| tab.rows.get(i)).and_then(|row| row.first());
        assert_number(cell, value, tol, &format!("fitted value in row {}", i + 1));
    }
}

fn assert_gof(result: &ResultSheet, label: &str, expected: f64, tol: f64) {
    let cell = result_row(result, "nonlinear_gof", label).and_then(<[ResultCell]>::first);
    assert_number(cell, expected, tol, label);
}

#[test]
fn noisy_weighted_gaussian_matches_independent_fit_and_report() {
    let xs = [1.0, 1.6, 2.3, 3.0, 3.7, 4.4, 5.2, 6.1, 7.0, 8.0, 9.0, 10.25];
    let ys = [2.3, 2.1, 3.4, 4.9, 7.9, 10.2, 9.8, 7.7, 4.1, 2.7, 2.6, 2.2];
    let mut spec = NonlinearFitSpec::new("gaussian_baseline");
    spec.weighting = WeightingScheme::InverseX2;
    spec.profile_likelihood_ci = false;
    spec.parameter_rules = [("Baseline", 2.0), ("Amplitude", 8.0), ("Mean", 5.0), ("SD", 1.5)]
        .into_iter()
        .map(|(name, start)| parameter_rule(name, ParameterScope::Local, Some(start)))
        .collect();
    let result = fit_sheet(xs, ys, spec).unwrap();
    assert_eq!(result.status, ResultStatus::UpToDate);

    let tol = WEIGHTED_FIT_ABS_TOL;
    for (name, estimate, se) in [
        ("Baseline", 2.134_563_135_061_916, 0.106_194_723_486_18),
        ("Amplitude", 8.476_063_151_547_963, 0.326_852_644_588_011),
        ("Mean", 4.845_359_838_298_89, 0.064_877_959_281_731),
        ("SD", 1.255_423_550_948_961, 0.063_387_242_848_533),
    ] {
        assert_parameter(&result, name, estimate, se, tol);
    }
    assert_predictions(
        &result,
        &[
            2.212_353_918_959_983,
            2.434_539_484_308_818,
            3.219_895_638_747_871,
            5.012_081_215_998_543,
            7.725_090_407_868_262,
            10.093_717_675_000_555,
            10.279_095_196_589_164,
            7.278_763_309_115_857,
            4.078_008_602_971_892,
            2.495_208_715_968_952,
            2.170_047_476_020_027,
            2.135_364_327_285_133,
        ],
        tol,
    );
    assert_gof(&result, "Degrees of Freedom", 8.0, 0.0);
    assert_gof(&result, "Sum of Squares (RSS)", 0.844_920_171_796_231, tol);
    assert_gof(&result, "Weighted Sum of Squares", 0.077_989_530_082_215, 1e-12);
    assert_approx_eq(result.lookup_scalar("Weighted SS", None).unwrap(), 0.077_989_530_082_215, 1e-12, 0.0, "weighted objective scalar");
}

#[test]
fn fixed_knot_segmental_fit_matches_independent_fit_and_report() {
    let xs = [-1.0, 0.0, 0.75, 1.75, 2.5, 3.25, 4.0, 5.5, 7.0];
    let ys = [0.4, 1.6, 2.0, 3.4, 4.1, 3.7, 3.5, 2.6, 2.0];
    let mut spec = NonlinearFitSpec::new("segmental_linear");
    spec.profile_likelihood_ci = false;
    spec.parameter_rules = vec![parameter_rule("X0", ParameterScope::Constant(2.5), None)];
    let result = fit_sheet(xs, ys, spec).unwrap();
    assert_eq!(result.status, ResultStatus::UpToDate);

    let tol = CONSTRAINED_FIT_ABS_TOL;
    for (name, estimate, se) in [
        ("Intercept1", 1.455_887_194_903_729, 0.072_988_112_392_408),
        ("Slope1", 1.057_189_893_350_512, 0.043_054_203_492_754),
        ("Slope2", -0.472_168_539_593_921, 0.034_862_867_240_883),
    ] {
        assert_parameter(&result, name, estimate, se, tol);
    }
    let knot_row = result_row(&result, "nonlinear_parameters", "X0 (Constant)").unwrap();
    assert_number(knot_row.first(), 2.5, 0.0, "fixed join estimate");
    assert_eq!(knot_row.get(1), Some(&ResultCell::Empty));
    assert_eq!(knot_row.get(2), Some(&ResultCell::Text("Fixed".to_owned())));
    assert_predictions(
        &result,
        &[
            0.398_697_301_553_217,
            1.455_887_194_903_729,
            2.248_779_614_916_613,
            3.305_969_508_267_125,
            4.098_861_928_280_009,
            3.744_735_523_584_568,
            3.390_609_118_889_127,
            2.682_356_309_498_247,
            1.974_103_500_107_365,
        ],
        tol,
    );
    assert_gof(&result, "Degrees of Freedom", 6.0, 0.0);
    assert_gof(&result, "Sum of Squares (RSS)", 0.112_925_345_358_242, 1e-12);
}
