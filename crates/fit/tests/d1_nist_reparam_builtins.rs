//! D1 NIST StRD reparameterization oracles for user-facing built-in fit models.
//!
//! Maps certified NIST StRD NLS and LLS datasets algebraically onto CurveBook's
//! own pharmacology, kinetics, growth, Gaussian, and polynomial built-in model
//! IDs (using `ParameterScope::Constant` where a baseline/offset is fixed to 0).
//!
//! D1 Provenance:
//! - NIST Statistical Reference Datasets (StRD), US government public domain.
//! - Secondary covariance-coupled SEs (`Amplitude`, `Y0`, `LogEC50`/`LogIC50`,
//!   `LogXb`, `PercentFast`, `PercentSlow`) verified by `R 4.5.2 (2025-10-31)`
//!   `stats::nls` via `crates/fit/tests/reference/nist_reparam_oracles.R`
//!   (2026-11-23).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use curvebook_doc::{FitMethod, ParameterRule, ParameterScope, WeightingScheme};
use curvebook_fit::{DataPoint, DatasetSeries, FitStatus, HookContext, NonlinearFitResult, SolverConfig, fit_nonlinear, get_builtin_model};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::assert_approx_eq;
use curvebook_testkit::nist_strd::{
    LinearRegressionDataset, NonlinearDataset, corpus_available, default_nist_strd_dir, load_corpus_linear, load_corpus_nonlinear, wampler1, wampler2,
};

fn resolve_corpus_dir() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("CURVEBOOK_NIST_STRD_DIR") {
        let p = PathBuf::from(custom);
        if corpus_available(&p) {
            return Some(p);
        }
    }
    let dir = default_nist_strd_dir();
    if corpus_available(&dir) {
        return Some(dir);
    }
    if std::env::var("CURVEBOOK_REQUIRE_CORPUS").ok().as_deref() == Some("1") {
        panic!("CURVEBOOK_REQUIRE_CORPUS=1 set, but NIST StRD corpus directory is missing");
    }
    eprintln!("Skipping NIST StRD corpus reparameterization test: corpus not present locally");
    None
}

fn series_from_nls(ds: &NonlinearDataset, negate_x: bool) -> DatasetSeries {
    let points = ds
        .predictors
        .iter()
        .zip(ds.y.iter())
        .enumerate()
        .map(|(row, (xs, &y))| {
            let x = if negate_x { xs.iter().map(|&v| -v).collect() } else { xs.clone() };
            DataPoint { x, y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row }
        })
        .collect();
    DatasetSeries {
        data_set_index: 0,
        label: ds.name.clone(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn series_from_lls(ds: &LinearRegressionDataset) -> DatasetSeries {
    // LLS datasets store the polynomial predictor x in design column 1 (column 0 is the intercept 1.0).
    let points = ds
        .design_rows
        .iter()
        .zip(ds.y.iter())
        .enumerate()
        .map(|(row, (cols, &y))| DataPoint {
            x: vec![cols[1]],
            y,
            sd: None,
            n: None,
            explicit_weight: None,
            source_subcolumn: 0,
            data_set_index: 0,
            row,
        })
        .collect();
    DatasetSeries {
        data_set_index: 0,
        label: ds.name.clone(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn rule_local(name: &str, init: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: Some(init), lower_bound: None, upper_bound: None }
}

fn rule_const(name: &str, val: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Constant(val), initial_override: None, lower_bound: None, upper_bound: None }
}

fn run_fit(model_id: &str, series: &DatasetSeries, rules: Vec<ParameterRule>) -> NonlinearFitResult {
    let model = get_builtin_model(model_id).unwrap_or_else(|| panic!("missing built-in model `{model_id}`"));
    let cfg = SolverConfig {
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::None,
        parameter_rules: rules,
        confidence: ConfidenceLevel::DEFAULT_95,
        max_iters: 3_000,
        compute_profile_ci: false,
        compute_hougaard: false,
        compute_bands: false,
        compute_prediction_bands: false,
        compute_runs_test: false,
        compute_lack_of_fit: false,
        compute_normality: false,
        interpolate_unknowns: false,
        use_simplex_warm_start: true,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(model.as_ref(), std::slice::from_ref(series), &HookContext::default(), &cfg)
        .unwrap_or_else(|e| panic!("fit failed for `{model_id}` on `{}`: {e:?}", series.label));
    assert!(
        matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous),
        "{model_id} on {} did not converge: status={:?}, rss={:.6e}",
        series.label,
        fit.status,
        fit.rss
    );
    fit
}

fn assert_param(fit: &NonlinearFitResult, name: &str, exp_est: f64, exp_se: f64, rel_est: f64, rel_se: f64, ctx: &str) {
    let p = fit.parameters.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("{ctx}: missing param `{name}`"));
    assert_approx_eq(p.estimate, exp_est, 1e-12, rel_est, &format!("{ctx} {name} estimate"));
    assert_approx_eq(p.se, exp_se, 1e-12, rel_se, &format!("{ctx} {name} SE"));
}

#[test]
fn nist_reparam_enzyme_binding_kinetics_and_power() {
    let Some(dir) = resolve_corpus_dir() else {
        return;
    };

    // 1. Misra1d -> michaelis_menten & one_site_specific_binding
    // NIST: y = b1 * b2 * x / (1 + b2 * x)  <=>  Vmax * x / (Km + x) with Vmax = b1, Km = 1/b2.
    let misra1d = load_corpus_nonlinear(&dir, "Misra1d").unwrap();
    let b = &misra1d.certified_betas;
    let s = &misra1d.certified_se_betas;
    let vmax = b[0];
    let km = 1.0 / b[1];
    let se_vmax = s[0];
    let se_km = s[1] / (b[1] * b[1]);
    let ser = series_from_nls(&misra1d, false);

    let mm = run_fit("michaelis_menten", &ser, vec![rule_local("Vmax", 450.0), rule_local("Km", 3000.0)]);
    assert_param(&mm, "Vmax", vmax, se_vmax, 1e-7, 1e-6, "Misra1d>michaelis_menten");
    assert_param(&mm, "Km", km, se_km, 1e-7, 1e-6, "Misra1d>michaelis_menten");
    assert_approx_eq(mm.rss, misra1d.certified_rss, 1e-12, 1e-9, "Misra1d>michaelis_menten RSS");

    let bind = run_fit("one_site_specific_binding", &ser, vec![rule_local("Bmax", 450.0), rule_local("Kd", 3000.0)]);
    assert_param(&bind, "Bmax", vmax, se_vmax, 1e-7, 1e-6, "Misra1d>one_site_specific_binding");
    assert_param(&bind, "Kd", km, se_km, 1e-7, 1e-6, "Misra1d>one_site_specific_binding");
    assert_approx_eq(bind.rss, misra1d.certified_rss, 1e-12, 1e-9, "Misra1d>one_site_specific_binding RSS");

    // 2. BoxBOD & Misra1a -> one_phase_association, exponential_plateau, one_phase_decay (Y0 = 0)
    // NIST: y = b1 * (1 - exp(-b2 * x))
    for (ds_name, init_p, init_k) in [("BoxBOD", 200.0, 0.5), ("Misra1a", 250.0, 5e-4)] {
        let ds = load_corpus_nonlinear(&dir, ds_name).unwrap();
        let ser_exp = series_from_nls(&ds, false);
        let p_cert = ds.certified_betas[0];
        let k_cert = ds.certified_betas[1];
        let p_se = ds.certified_se_betas[0];
        let k_se = ds.certified_se_betas[1];

        let opa = run_fit("one_phase_association", &ser_exp, vec![rule_const("Y0", 0.0), rule_local("Plateau", init_p), rule_local("K", init_k)]);
        assert_param(&opa, "Plateau", p_cert, p_se, 1e-7, 1e-6, &format!("{ds_name}>one_phase_association"));
        assert_param(&opa, "K", k_cert, k_se, 1e-7, 1e-6, &format!("{ds_name}>one_phase_association"));
        assert_approx_eq(opa.rss, ds.certified_rss, 1e-12, 1e-9, &format!("{ds_name}>one_phase_association RSS"));

        let ep = run_fit("exponential_plateau", &ser_exp, vec![rule_local("YM", init_p), rule_const("Y0", 0.0), rule_local("k", init_k)]);
        assert_param(&ep, "YM", p_cert, p_se, 1e-7, 1e-6, &format!("{ds_name}>exponential_plateau"));
        assert_param(&ep, "k", k_cert, k_se, 1e-7, 1e-6, &format!("{ds_name}>exponential_plateau"));
        assert_approx_eq(ep.rss, ds.certified_rss, 1e-12, 1e-9, &format!("{ds_name}>exponential_plateau RSS"));

        let opd = run_fit("one_phase_decay", &ser_exp, vec![rule_const("Y0", 0.0), rule_local("Plateau", init_p), rule_local("K", init_k)]);
        assert_param(&opd, "Plateau", p_cert, p_se, 1e-7, 1e-6, &format!("{ds_name}>one_phase_decay"));
        assert_param(&opd, "K", k_cert, k_se, 1e-7, 1e-6, &format!("{ds_name}>one_phase_decay"));
        assert_approx_eq(opd.rss, ds.certified_rss, 1e-12, 1e-9, &format!("{ds_name}>one_phase_decay RSS"));
        let half_life = opd.derived_scalars[0].iter().find(|d| d.name == "HalfLife").expect("HalfLife derived scalar");
        assert_approx_eq(half_life.value, std::f64::consts::LN_2 / k_cert, 1e-12, 1e-7, &format!("{ds_name}>one_phase_decay HalfLife"));
    }

    // 3. DanWood -> power_law (y = a * x^b)
    let danwood = load_corpus_nonlinear(&dir, "DanWood").unwrap();
    let ser_dw = series_from_nls(&danwood, false);
    let pow = run_fit("power_law", &ser_dw, vec![rule_local("a", 1.0), rule_local("b", 5.0)]);
    assert_param(&pow, "a", danwood.certified_betas[0], danwood.certified_se_betas[0], 1e-7, 1e-6, "DanWood>power_law");
    assert_param(&pow, "b", danwood.certified_betas[1], danwood.certified_se_betas[1], 1e-7, 1e-6, "DanWood>power_law");
    assert_approx_eq(pow.rss, danwood.certified_rss, 1e-12, 1e-9, "DanWood>power_law RSS");
}

#[test]
fn nist_reparam_gaussian_sigmoids_and_three_phase_decay() {
    let Some(dir) = resolve_corpus_dir() else {
        return;
    };

    // 4. Eckerle4 -> gaussian (Amplitude = b1/b2, Mean = b3, SD = b2)
    // Secondary R 4.5.2 nls vcov SE for Amplitude: 3.768303420630896e-3
    let eckerle4 = load_corpus_nonlinear(&dir, "Eckerle4").unwrap();
    let b = &eckerle4.certified_betas;
    let s = &eckerle4.certified_se_betas;
    let ser_e4 = series_from_nls(&eckerle4, false);
    let gauss = run_fit("gaussian", &ser_e4, vec![rule_local("Amplitude", 0.3), rule_local("Mean", 451.0), rule_local("SD", 5.0)]);
    assert_param(&gauss, "Amplitude", b[0] / b[1], 3.768_303_420_630_896e-3, 1e-7, 1e-5, "Eckerle4>gaussian");
    assert_param(&gauss, "Mean", b[2], s[2], 1e-7, 1e-6, "Eckerle4>gaussian");
    assert_param(&gauss, "SD", b[1], s[1], 1e-7, 1e-6, "Eckerle4>gaussian");
    assert_approx_eq(gauss.rss, eckerle4.certified_rss, 1e-12, 1e-9, "Eckerle4>gaussian RSS");

    // 5. Rat42 -> logistic_growth, log_agonist_vs_response_4pl (Bottom=0), log_inhibitor_vs_response_4pl (Bottom=0, -x)
    // NIST: y = b1 / (1 + exp(b2 - b3 * x))
    let rat42 = load_corpus_nonlinear(&dir, "Rat42").unwrap();
    let b = &rat42.certified_betas;
    let s = &rat42.certified_se_betas;
    let y0_cert = b[0] / (1.0 + b[1].exp());
    let log_ec50_cert = b[1] / b[2];
    let hill_cert = b[2] / std::f64::consts::LN_10;
    let se_hill = s[2] / std::f64::consts::LN_10;

    let ser_r42 = series_from_nls(&rat42, false);
    let logi =
        run_fit("logistic_growth", &ser_r42, vec![rule_local("YM", b[0] * 1.05), rule_local("Y0", y0_cert * 1.05), rule_local("k", b[2] * 1.05)]);
    assert_param(&logi, "YM", b[0], s[0], 1e-6, 1e-5, "Rat42>logistic_growth");
    assert_param(&logi, "Y0", y0_cert, 4.709_402_845_203_958e-1, 1e-6, 1e-5, "Rat42>logistic_growth");
    assert_param(&logi, "k", b[2], s[2], 1e-6, 1e-5, "Rat42>logistic_growth");
    assert_approx_eq(logi.rss, rat42.certified_rss, 1e-12, 1e-9, "Rat42>logistic_growth RSS");

    let ag4 = run_fit(
        "log_agonist_vs_response_4pl",
        &ser_r42,
        vec![
            rule_const("Bottom", 0.0),
            rule_local("Top", b[0] * 1.05),
            rule_local("LogEC50", log_ec50_cert * 1.05),
            rule_local("HillSlope", hill_cert * 1.05),
        ],
    );
    assert_param(&ag4, "Top", b[0], s[0], 1e-6, 1e-5, "Rat42>4pl_agonist");
    assert_param(&ag4, "LogEC50", log_ec50_cert, 1.179_440_611_437_669, 1e-6, 1e-5, "Rat42>4pl_agonist");
    assert_param(&ag4, "HillSlope", hill_cert, se_hill, 1e-6, 1e-5, "Rat42>4pl_agonist");
    assert_approx_eq(ag4.rss, rat42.certified_rss, 1e-12, 1e-9, "Rat42>4pl_agonist RSS");

    // Negating X turns agonist (increasing with X) into inhibitor (decreasing with X) with LogIC50 = -LogEC50.
    let ser_r42_neg = series_from_nls(&rat42, true);
    let inh4 = run_fit(
        "log_inhibitor_vs_response_4pl",
        &ser_r42_neg,
        vec![
            rule_const("Bottom", 0.0),
            rule_local("Top", b[0] * 1.05),
            rule_local("LogIC50", -log_ec50_cert * 1.05),
            rule_local("HillSlope", hill_cert * 1.05),
        ],
    );
    assert_param(&inh4, "Top", b[0], s[0], 1e-6, 1e-5, "Rat42>4pl_inhibitor");
    assert_param(&inh4, "LogIC50", -log_ec50_cert, 1.179_440_611_437_669, 1e-6, 1e-5, "Rat42>4pl_inhibitor");
    assert_param(&inh4, "HillSlope", hill_cert, se_hill, 1e-6, 1e-5, "Rat42>4pl_inhibitor");
    assert_approx_eq(inh4.rss, rat42.certified_rss, 1e-12, 1e-9, "Rat42>4pl_inhibitor RSS");

    // 6. Rat43 -> log_agonist_vs_response_5pl (Bottom = 0)
    // NIST: y = b1 / (1 + exp(b2 - b3 * x))^(1/b4)  =>  S = 1/b4, HillSlope = b3/ln(10), LogXb = b2/b3.
    let rat43 = load_corpus_nonlinear(&dir, "Rat43").unwrap();
    let b = &rat43.certified_betas;
    let s = &rat43.certified_se_betas;
    let log_xb = b[1] / b[2];
    let hill5 = b[2] / std::f64::consts::LN_10;
    let s_asym = 1.0 / b[3];
    let ser_r43 = series_from_nls(&rat43, false);
    let five_pl = run_fit(
        "log_agonist_vs_response_5pl",
        &ser_r43,
        vec![
            rule_const("Bottom", 0.0),
            rule_local("Top", b[0] * 1.05),
            rule_local("LogXb", log_xb * 1.05),
            rule_local("HillSlope", hill5 * 1.05),
            rule_local("S", s_asym * 1.05),
        ],
    );
    assert_param(&five_pl, "Top", b[0], s[0], 1e-6, 1e-5, "Rat43>5pl");
    assert_param(&five_pl, "LogXb", log_xb, 1.013_920_603_500_262, 1e-6, 1e-5, "Rat43>5pl");
    assert_param(&five_pl, "HillSlope", hill5, s[2] / std::f64::consts::LN_10, 1e-6, 1e-5, "Rat43>5pl");
    assert_param(&five_pl, "S", s_asym, s[3] / (b[3] * b[3]), 1e-6, 1e-5, "Rat43>5pl");
    assert_approx_eq(five_pl.rss, rat43.certified_rss, 1e-12, 1e-9, "Rat43>5pl RSS");

    // 7. Lanczos3 -> three_phase_decay (Plateau = 0)
    // NIST: y = b1*exp(-b2*x) + b3*exp(-b4*x) + b5*exp(-b6*x), with b6 > b4 > b2.
    let lanczos3 = load_corpus_nonlinear(&dir, "Lanczos3").unwrap();
    let b = &lanczos3.certified_betas;
    let s = &lanczos3.certified_se_betas;
    let y0 = b[0] + b[2] + b[4];
    let pf = 100.0 * b[4] / y0;
    let ps = 100.0 * b[2] / y0;
    let ser_l3 = series_from_nls(&lanczos3, false);
    let tri = run_fit(
        "three_phase_decay",
        &ser_l3,
        vec![
            rule_local("Y0", y0 * 1.02),
            rule_const("Plateau", 0.0),
            rule_local("PercentFast", pf * 1.02),
            rule_local("PercentSlow", ps * 1.02),
            rule_local("KFast", b[5] * 1.02),
            rule_local("KSlow", b[3] * 1.02),
            rule_local("KVerySlow", b[1] * 1.02),
        ],
    );
    assert_param(&tri, "Y0", y0, 2.879_558_676_621_674e-5, 1e-6, 1e-4, "Lanczos3>three_phase_decay");
    assert_param(&tri, "PercentFast", pf, 2.322_573_045_577_274, 1e-5, 1e-4, "Lanczos3>three_phase_decay");
    assert_param(&tri, "PercentSlow", ps, 1.650_549_427_342_901, 1e-5, 1e-4, "Lanczos3>three_phase_decay");
    assert_param(&tri, "KFast", b[5], s[5], 1e-5, 1e-4, "Lanczos3>three_phase_decay");
    assert_param(&tri, "KSlow", b[3], s[3], 1e-5, 1e-4, "Lanczos3>three_phase_decay");
    assert_param(&tri, "KVerySlow", b[1], s[1], 1e-5, 1e-4, "Lanczos3>three_phase_decay");
    assert_approx_eq(tri.rss, lanczos3.certified_rss, 1e-16, 1e-8, "Lanczos3>three_phase_decay RSS");
}

fn verify_polynomial_lls(model_id: &str, ds: &LinearRegressionDataset, est_tol: f64, se_tol: f64) {
    let ser = series_from_lls(ds);
    let fit = run_fit(model_id, &ser, Vec::new());
    for (k, (&cert_b, &cert_se)) in ds.certified_betas.iter().zip(ds.certified_se_betas.iter()).enumerate() {
        let name = format!("B{k}");
        let p = fit.parameters.iter().find(|p| p.name == name).unwrap();
        assert_approx_eq(p.estimate, cert_b, 1e-7, est_tol, &format!("{}>{model_id} {name} est", ds.name));
        if cert_se > 0.0 {
            assert_approx_eq(p.se, cert_se, 1e-7, se_tol, &format!("{}>{model_id} {name} se", ds.name));
        } else {
            assert!(p.se < 1e-8, "{}>{model_id} {name} expected ~0 SE, got {}", ds.name, p.se);
        }
    }
    if ds.certified_ss_res > 0.0 {
        assert_approx_eq(fit.rss, ds.certified_ss_res, 1e-12, 1e-7, &format!("{}>{model_id} RSS", ds.name));
    } else {
        assert!(fit.rss < 1e-16, "{}>{model_id} expected ~0 RSS, got {}", ds.name, fit.rss);
    }
}

#[test]
fn nist_reparam_polynomial_offline_wampler1_and_wampler2() {
    // Runs even without the optional on-disk corpus using testkit's offline Wampler1 & Wampler2.
    verify_polynomial_lls("polynomial_5", &wampler1(), 1e-7, 1e-6);
    verify_polynomial_lls("polynomial_5", &wampler2(), 1e-7, 1e-6);
}

#[test]
fn nist_reparam_polynomial_corpus_pontius_and_wampler1_to_5() {
    let Some(dir) = resolve_corpus_dir() else {
        return;
    };
    let pontius = load_corpus_linear(&dir, "Pontius").unwrap();
    verify_polynomial_lls("polynomial_2", &pontius, 1e-7, 1e-6);

    for name in ["Wampler1", "Wampler2", "Wampler3", "Wampler4", "Wampler5"] {
        let ds = load_corpus_linear(&dir, name).unwrap();
        // Wampler3–5 have high-noise ill-conditioned Vandermonde columns; LM with column-scaled QR
        // matches certified estimates and SEs tightly.
        verify_polynomial_lls("polynomial_5", &ds, 1e-6, 1e-5);
    }
}

#[test]
fn nist_reparam_r18_3_extended_builtins() {
    let Some(dir) = resolve_corpus_dir() else {
        return;
    };

    // 9. Eckerle4 -> gaussian_area (Area = b1 * sqrt(2*pi), Mean = b3, SD = b2)
    let eckerle4 = load_corpus_nonlinear(&dir, "Eckerle4").unwrap();
    let b = &eckerle4.certified_betas;
    let s = &eckerle4.certified_se_betas;
    let sqrt_2pi = (2.0 * std::f64::consts::PI).sqrt();
    let ser_e4 = series_from_nls(&eckerle4, false);
    let ga = run_fit("gaussian_area", &ser_e4, vec![rule_local("Area", b[0] * sqrt_2pi * 1.05), rule_local("Mean", 451.0), rule_local("SD", 5.0)]);
    assert_param(&ga, "Area", b[0] * sqrt_2pi, s[0] * sqrt_2pi, 1e-7, 1e-6, "Eckerle4>gaussian_area");
    assert_param(&ga, "Mean", b[2], s[2], 1e-7, 1e-6, "Eckerle4>gaussian_area");
    assert_param(&ga, "SD", b[1], s[1], 1e-7, 1e-6, "Eckerle4>gaussian_area");
    assert_approx_eq(ga.rss, eckerle4.certified_rss, 1e-12, 1e-9, "Eckerle4>gaussian_area RSS");
    let amp_d = ga.derived_scalars[0].iter().find(|d| d.name == "Amplitude").expect("Amplitude derived scalar");
    assert_approx_eq(amp_d.value, b[0] / b[1], 1e-12, 1e-7, "Eckerle4>gaussian_area derived Amplitude");

    // 10. Rat42 -> boltzmann_sigmoid (Bottom = 0, Top = b1, V50 = b2 / b3, Slope = 1 / b3)
    // Secondary R 4.5.2 nls vcov SE for V50: 1.179440601976244
    let rat42 = load_corpus_nonlinear(&dir, "Rat42").unwrap();
    let b = &rat42.certified_betas;
    let s = &rat42.certified_se_betas;
    let v50_cert = b[1] / b[2];
    let slope_cert = 1.0 / b[2];
    let se_slope = s[2] / (b[2] * b[2]);
    let ser_r42 = series_from_nls(&rat42, false);
    let boltz = run_fit(
        "boltzmann_sigmoid",
        &ser_r42,
        vec![rule_const("Bottom", 0.0), rule_local("Top", b[0] * 1.05), rule_local("V50", v50_cert * 1.05), rule_local("Slope", slope_cert * 1.05)],
    );
    assert_param(&boltz, "Top", b[0], s[0], 1e-6, 1e-5, "Rat42>boltzmann_sigmoid");
    assert_param(&boltz, "V50", v50_cert, 1.179_440_601_976_244, 1e-6, 1e-5, "Rat42>boltzmann_sigmoid");
    assert_param(&boltz, "Slope", slope_cert, se_slope, 1e-6, 1e-5, "Rat42>boltzmann_sigmoid");
    assert_approx_eq(boltz.rss, rat42.certified_rss, 1e-12, 1e-9, "Rat42>boltzmann_sigmoid RSS");

    // 11. BoxBOD -> two_phase_association (Y0 = 0, PercentFast = 100, KSlow = 0.01)
    let boxbod = load_corpus_nonlinear(&dir, "BoxBOD").unwrap();
    let ser_bb = series_from_nls(&boxbod, false);
    let tpa = run_fit(
        "two_phase_association",
        &ser_bb,
        vec![
            rule_const("Y0", 0.0),
            rule_local("Plateau", 200.0),
            rule_const("PercentFast", 100.0),
            rule_local("KFast", 0.5),
            rule_const("KSlow", 0.01),
        ],
    );
    assert_param(&tpa, "Plateau", boxbod.certified_betas[0], boxbod.certified_se_betas[0], 1e-7, 1e-6, "BoxBOD>two_phase_association");
    assert_param(&tpa, "KFast", boxbod.certified_betas[1], boxbod.certified_se_betas[1], 1e-7, 1e-6, "BoxBOD>two_phase_association");
    assert_approx_eq(tpa.rss, boxbod.certified_rss, 1e-12, 1e-9, "BoxBOD>two_phase_association RSS");

    // 12. DanWood -> power_series_two_term (C = 0, D = 1 => y = A * x^B)
    let danwood = load_corpus_nonlinear(&dir, "DanWood").unwrap();
    let ser_dw = series_from_nls(&danwood, false);
    let pst = run_fit("power_series_two_term", &ser_dw, vec![rule_local("A", 1.0), rule_local("B", 5.0), rule_const("C", 0.0), rule_const("D", 1.0)]);
    assert_param(&pst, "A", danwood.certified_betas[0], danwood.certified_se_betas[0], 1e-7, 1e-6, "DanWood>power_series_two_term");
    assert_param(&pst, "B", danwood.certified_betas[1], danwood.certified_se_betas[1], 1e-7, 1e-6, "DanWood>power_series_two_term");
    assert_approx_eq(pst.rss, danwood.certified_rss, 1e-12, 1e-9, "DanWood>power_series_two_term RSS");
}
