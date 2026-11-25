//! Independent non-NIST equation and result-sheet checks (`reference/non_nist_fits.py`).

use std::f64::consts::{E, FRAC_PI_2, LN_2};

use curvebook_doc::{NonlinearFitSpec, ResultCell, ResultSheet, ResultStatus, WeightingScheme};
use curvebook_fit::get_builtin_model;
use curvebook_testkit::assert_approx_eq;

use crate::common::{assert_gof_num, assert_num_cell, exec_nl, find_tab, nl_fast, rule_const, rule_local_init, tab_row, xy_sheet_single};

const PREDICTION_ABS_TOL: f64 = 2e-13;
const WEIGHTED_FIT_ABS_TOL: f64 = 5e-7;
const CONSTRAINED_FIT_ABS_TOL: f64 = 2e-8;

type PredCase<'a> = (&'a str, &'a [f64], &'a [(f64, f64)]);

#[test]
fn built_in_predictions_match_analytical_references() {
    let cases: &[PredCase<'_>] = &[
        (
            "log_gaussian",
            &[10.0, 8.0, 2.0],
            &[(2.0, 10.0 / (E * E)), (4.0, 10.0 / E.sqrt()), (8.0, 10.0), (16.0, 10.0 / E.sqrt()), (32.0, 10.0 / (E * E))],
        ),
        ("weibull_peak", &[12.0, 3.0, 2.0], &[(0.0, 0.0), (1.5, 4.0 / E.sqrt().sqrt()), (3.0, 8.0 / E), (6.0, 16.0 / E.powi(4))]),
        ("damped_sine_wave", &[8.0, 0.25, FRAC_PI_2, LN_2, 3.0], &[(0.0, 11.0), (1.0, 3.0), (2.0, 1.0), (3.0, 3.0), (4.0, 3.5)]),
        ("hinge_smooth_hyperbolic", &[5.0, 2.0, 1.0, 3.0, 4.0], &[(-1.0, 0.0), (2.0, 5.0), (5.0, 12.0)]),
        ("one_site_specific_hill", &[40.0, 2.0, 2.0], &[(0.0, 0.0), (1.0, 8.0), (2.0, 20.0), (4.0, 32.0)]),
        ("kcat_enzyme_kinetics", &[6.0, 4.0, 3.0], &[(0.0, 0.0), (2.0, 6.0), (4.0, 9.0), (12.0, 13.5)]),
        ("substrate_inhibition", &[45.0, 2.0, 8.0], &[(0.0, 0.0), (2.0, 20.0), (4.0, 22.5), (8.0, 20.0), (16.0, 14.4)]),
        ("morrison_tight_binding_ki", &[75.0, 5.0, 1.0, 4.0, 4.0], &[(0.0, 75.0), (1.5, 60.0), (4.5, 37.5), (12.0, 15.0)]),
    ];
    for &(model_id, params, points) in cases {
        let model = get_builtin_model(model_id).unwrap();
        for &(x, expected) in points {
            assert_approx_eq(model.eval(&[x], params, 0).unwrap(), expected, PREDICTION_ABS_TOL, 0.0, &format!("{model_id} x={x}"));
        }
    }
}

fn assert_params_and_preds(result: &ResultSheet, params: &[(&str, f64, f64)], preds: &[f64], tol: f64) {
    assert_eq!(result.status, ResultStatus::UpToDate);
    for &(name, est, se) in params {
        let row = tab_row(result, "nonlinear_parameters", name);
        assert_num_cell(&row[0], est, tol, 0.0, &format!("{name} est"));
        assert_num_cell(&row[1], se, tol, 0.0, &format!("{name} SE"));
    }
    let res_tab = find_tab(result, "nonlinear_residuals");
    assert_eq!(res_tab.rows.len(), preds.len());
    for (i, &val) in preds.iter().enumerate() {
        assert_num_cell(&res_tab.rows[i][0], val, tol, 0.0, &format!("pred row {}", i + 1));
    }
}

#[test]
fn noisy_weighted_gaussian_matches_independent_fit_and_report() {
    let sheet = xy_sheet_single(
        "fit-oracle-data",
        "Fit oracle data",
        "Y",
        &[1.0, 1.6, 2.3, 3.0, 3.7, 4.4, 5.2, 6.1, 7.0, 8.0, 9.0, 10.25],
        &[2.3, 2.1, 3.4, 4.9, 7.9, 10.2, 9.8, 7.7, 4.1, 2.7, 2.6, 2.2],
    );
    let nl = NonlinearFitSpec {
        weighting: WeightingScheme::InverseX2,
        parameter_rules: [("Baseline", 2.0), ("Amplitude", 8.0), ("Mean", 5.0), ("SD", 1.5)]
            .into_iter()
            .map(|(n, s)| rule_local_init(n, s))
            .collect(),
        ..nl_fast("gaussian_baseline")
    };
    let result = exec_nl(&sheet, nl);
    assert_params_and_preds(
        &result,
        &[
            ("Baseline", 2.134_563_135_061_916, 0.106_194_723_486_18),
            ("Amplitude", 8.476_063_151_547_963, 0.326_852_644_588_011),
            ("Mean", 4.845_359_838_298_89, 0.064_877_959_281_731),
            ("SD", 1.255_423_550_948_961, 0.063_387_242_848_533),
        ],
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
        WEIGHTED_FIT_ABS_TOL,
    );
    assert_gof_num(&result, "Degrees of Freedom", 8.0, 0.0, 0.0);
    assert_gof_num(&result, "Sum of Squares (RSS)", 0.844_920_171_796_231, WEIGHTED_FIT_ABS_TOL, 0.0);
    assert_gof_num(&result, "Weighted Sum of Squares", 0.077_989_530_082_215, 1e-12, 0.0);
    assert_approx_eq(result.lookup_scalar("Weighted SS", None).unwrap(), 0.077_989_530_082_215, 1e-12, 0.0, "weighted scalar");
}

#[test]
fn fixed_knot_segmental_fit_matches_independent_fit_and_report() {
    let sheet = xy_sheet_single(
        "fit-oracle-data",
        "Fit oracle data",
        "Y",
        &[-1.0, 0.0, 0.75, 1.75, 2.5, 3.25, 4.0, 5.5, 7.0],
        &[0.4, 1.6, 2.0, 3.4, 4.1, 3.7, 3.5, 2.6, 2.0],
    );
    let result = exec_nl(&sheet, NonlinearFitSpec { parameter_rules: vec![rule_const("X0", 2.5)], ..nl_fast("segmental_linear") });
    assert_params_and_preds(
        &result,
        &[
            ("Intercept1", 1.455_887_194_903_729, 0.072_988_112_392_408),
            ("Slope1", 1.057_189_893_350_512, 0.043_054_203_492_754),
            ("Slope2", -0.472_168_539_593_921, 0.034_862_867_240_883),
        ],
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
        CONSTRAINED_FIT_ABS_TOL,
    );
    let knot_row = tab_row(&result, "nonlinear_parameters", "X0 (Constant)");
    assert_num_cell(&knot_row[0], 2.5, 0.0, 0.0, "fixed join");
    assert_eq!((&knot_row[1], &knot_row[2]), (&ResultCell::Empty, &ResultCell::Text("Fixed".to_owned())));
    assert_gof_num(&result, "Degrees of Freedom", 6.0, 0.0, 0.0);
    assert_gof_num(&result, "Sum of Squares (RSS)", 0.112_925_345_358_242, 1e-12, 0.0);
}
