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
        ("damped_sine_wave", &[8.0, LN_2, 4.0, FRAC_PI_2], &[(0.0, 8.0), (1.0, 0.0), (2.0, -2.0), (3.0, 0.0), (4.0, 0.5)]),
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

#[test]
fn r_4_5_2_noisy_oracles_exponential_growth_hill_mixed_inhibition_and_lq_survival() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `stats::nls(..., algorithm="port")` via
    // `crates/fit/tests/reference/d1_extended_fit_oracles.R` (2026-11-27).
    use curvebook_doc::{AnalysisKind, AnalysisSpec, ParameterRule, ParameterScope, SheetId};
    use curvebook_fit::{HookContext, execute_nonlinear_analysis};

    use crate::common::{rule_shared_bounds, xy_sheet};

    // 1. exponential_growth: Y = Y0 * exp(k * X)
    let t_eg: [f64; 11] = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0];
    let e_eg: [f64; 11] = [0.15, -0.22, 0.18, -0.14, 0.30, -0.25, 0.42, -0.35, 0.50, -0.45, 0.38];
    let y_eg: Vec<f64> = t_eg.iter().zip(e_eg).map(|(&t, e)| 2.5 * (0.45 * t).exp() + e).collect();
    let res_eg = exec_nl(&xy_sheet_single("eg", "ExpGrowth", "Y", &t_eg, &y_eg), nl_fast("exponential_growth"));
    assert_eq!(res_eg.status, ResultStatus::UpToDate);
    assert_num_cell(&tab_row(&res_eg, "nonlinear_parameters", "Y0")[0], 2.503_135_124_786, 1e-6, 1e-6, "exp_growth Y0");
    assert_num_cell(&tab_row(&res_eg, "nonlinear_parameters", "Y0")[1], 0.096_637_652_983, 1e-5, 1e-5, "exp_growth Y0 SE");
    assert_num_cell(&tab_row(&res_eg, "nonlinear_parameters", "k")[0], 0.450_865_405_810, 1e-6, 1e-6, "exp_growth k");
    assert_num_cell(&tab_row(&res_eg, "nonlinear_parameters", "k")[1], 0.009_022_505_504, 1e-5, 1e-5, "exp_growth k SE");
    assert_gof_num(&res_eg, "Degrees of Freedom", 9.0, 0.0, 0.0);
    assert_gof_num(&res_eg, "Sum of Squares (RSS)", 1.133_557_802_767, 1e-6, 1e-6);
    assert_approx_eq(LN_2 / res_eg.lookup_scalar("k", Some(0)).unwrap(), 1.537_370_513_745, 1e-6, 1e-6, "DoublingTime");

    // 2. one_site_specific_hill: Y = Bmax * X^h / (Kd^h + X^h)
    let c_hill: [f64; 12] = [0.1, 0.25, 0.5, 1.0, 2.0, 3.5, 5.0, 8.0, 12.0, 20.0, 35.0, 60.0];
    let e_hill: [f64; 12] = [0.3, -0.4, 0.5, -0.6, 0.8, -0.9, 0.7, -0.8, 0.6, -0.5, 0.4, -0.3];
    let y_hill: Vec<f64> = c_hill.iter().zip(e_hill).map(|(&c, e)| 140.0 * c.powf(1.65) / (4.2_f64.powf(1.65) + c.powf(1.65)) + e).collect();
    let res_hill = exec_nl(&xy_sheet_single("hill", "HillBinding", "Y", &c_hill, &y_hill), nl_fast("one_site_specific_hill"));
    assert_eq!(res_hill.status, ResultStatus::UpToDate);
    assert_num_cell(&tab_row(&res_hill, "nonlinear_parameters", "Bmax")[0], 139.988_536_240_297, 1e-5, 1e-5, "hill Bmax");
    assert_num_cell(&tab_row(&res_hill, "nonlinear_parameters", "Bmax")[1], 0.549_001_053_408, 1e-4, 1e-4, "hill Bmax SE");
    assert_num_cell(&tab_row(&res_hill, "nonlinear_parameters", "Kd")[0], 4.202_640_384_657, 1e-5, 1e-5, "hill Kd");
    assert_num_cell(&tab_row(&res_hill, "nonlinear_parameters", "Kd")[1], 0.038_482_358_565, 1e-4, 1e-4, "hill Kd SE");
    assert_num_cell(&tab_row(&res_hill, "nonlinear_parameters", "h")[0], 1.646_943_000_070, 1e-5, 1e-5, "hill h");
    assert_num_cell(&tab_row(&res_hill, "nonlinear_parameters", "h")[1], 0.022_058_318_078, 1e-4, 1e-4, "hill h SE");
    assert_gof_num(&res_hill, "Degrees of Freedom", 9.0, 0.0, 0.0);
    assert_gof_num(&res_hill, "Sum of Squares (RSS)", 4.271_470_674_671, 1e-5, 1e-5);

    // 3. mixed_model_inhibition (global fit across 4 inhibitor concentrations I = 0, 2, 6, 15 via ColumnTitleValue)
    let s_mix = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let i_concs = [0.0_f64, 2.0, 6.0, 15.0];
    let base_err = [0.45, -0.35, 0.50, -0.40, 0.30, -0.45, 0.35, -0.30];
    let scales = [1.0, -1.0, 0.8, -0.9];
    let mix_cols: Vec<Vec<f64>> = i_concs
        .iter()
        .zip(scales)
        .map(|(&ic, sc)| {
            s_mix.iter().zip(base_err).map(|(&s, eb)| (115.0 * s) / (5.5 * (1.0 + ic / 4.0) + s * (1.0 + ic / (3.2 * 4.0))) + eb * sc).collect()
        })
        .collect();
    let mix_sheet = xy_sheet(
        "sheet-mixed",
        "Mixed Model Inhibition",
        &s_mix,
        &[("0.0 uM", &mix_cols[0]), ("2.0 uM", &mix_cols[1]), ("6.0 uM", &mix_cols[2]), ("15.0 uM", &mix_cols[3])],
    );
    let mix_nl = NonlinearFitSpec {
        parameter_rules: vec![
            rule_shared_bounds("Vmax", Some(110.0), Some(1e-6), None),
            rule_shared_bounds("Km", Some(5.0), Some(1e-6), None),
            rule_shared_bounds("Ki", Some(4.0), Some(1e-6), None),
            rule_shared_bounds("Alpha", Some(3.0), Some(1e-6), None),
            ParameterRule {
                name: "I".to_owned(),
                scope: ParameterScope::ColumnTitleConstant,
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        ..nl_fast("mixed_model_inhibition")
    };
    let sheets = [mix_sheet.clone()];
    let hooks = HookContext { data_sheets: &sheets, ..HookContext::default() };
    let mix_spec = AnalysisSpec::new(SheetId::new("ana-mix"), mix_sheet.id.clone(), AnalysisKind::NonlinearRegression(mix_nl.clone())).unwrap();
    let res_mix = execute_nonlinear_analysis(&mix_spec, &mix_nl, &mix_sheet, &hooks).unwrap();
    assert_eq!(res_mix.status, ResultStatus::UpToDate);
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Vmax")[0], 114.891_577_618_823, 1e-4, 1e-5, "mixed Vmax");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Vmax")[1], 0.343_953_711_532, 1e-4, 1e-4, "mixed Vmax SE");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Km")[0], 5.485_555_470_056, 1e-4, 1e-5, "mixed Km");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Km")[1], 0.058_138_155_343, 1e-4, 1e-4, "mixed Km SE");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Ki")[0], 3.982_108_793_574, 1e-4, 1e-5, "mixed Ki");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Ki")[1], 0.088_140_320_371, 1e-4, 1e-4, "mixed Ki SE");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Alpha")[0], 3.232_365_590_623, 1e-4, 1e-5, "mixed Alpha");
    assert_num_cell(&tab_row(&res_mix, "nonlinear_parameters", "Alpha")[1], 0.113_983_005_096, 1e-4, 1e-4, "mixed Alpha SE");
    assert_gof_num(&res_mix, "Degrees of Freedom", 28.0, 0.0, 0.0);
    assert_gof_num(&res_mix, "Sum of Squares (RSS)", 4.256_888_126_034, 1e-5, 1e-5);

    // 4. linear_quadratic_survival: S = exp(-(Alpha * D + Beta * D^2))
    let d_lq: [f64; 9] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];
    let e_lq: [f64; 9] = [0.005, -0.012, 0.010, -0.008, 0.006, -0.004, 0.003, -0.0015, 0.0005];
    let y_lq: Vec<f64> = d_lq.iter().zip(e_lq).map(|(&d, e)| (-(0.28 * d + 0.035 * d * d)).exp() + e).collect();
    let res_lq = exec_nl(&xy_sheet_single("lq", "LQSurvival", "S", &d_lq, &y_lq), nl_fast("linear_quadratic_survival"));
    assert_eq!(res_lq.status, ResultStatus::UpToDate);
    assert_num_cell(&tab_row(&res_lq, "nonlinear_parameters", "Alpha")[0], 0.285_609_768_765, 1e-6, 1e-6, "LQ Alpha");
    assert_num_cell(&tab_row(&res_lq, "nonlinear_parameters", "Alpha")[1], 0.009_491_270_218, 1e-5, 1e-5, "LQ Alpha SE");
    assert_num_cell(&tab_row(&res_lq, "nonlinear_parameters", "Beta")[0], 0.033_347_202_384, 1e-6, 1e-6, "LQ Beta");
    assert_num_cell(&tab_row(&res_lq, "nonlinear_parameters", "Beta")[1], 0.003_037_273_518, 1e-5, 1e-5, "LQ Beta SE");
    assert_gof_num(&res_lq, "Degrees of Freedom", 7.0, 0.0, 0.0);
    assert_gof_num(&res_lq, "Sum of Squares (RSS)", 0.000_377_592_278, 1e-9, 1e-6);
    assert_approx_eq(
        res_lq.lookup_scalar("Alpha", Some(0)).unwrap() / res_lq.lookup_scalar("Beta", Some(0)).unwrap(),
        8.564_729_522_912,
        1e-5,
        1e-5,
        "LQ Alpha/Beta",
    );
}
