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

use super::common;

use std::path::PathBuf;

use curvebook_doc::ParameterRule;
use curvebook_fit::{DataPoint, DatasetSeries, FitStatus, NonlinearFitResult, SolverConfig, get_builtin_model};
use curvebook_testkit::assert_approx_eq;
use curvebook_testkit::nist_strd::{
    LinearRegressionDataset, NonlinearDataset, corpus_available, default_nist_strd_dir, load_corpus_linear, load_corpus_nonlinear, wampler1, wampler2,
};

use common::{cfg_fast, find_derived, find_param, fit_with, rule_const, rule_local_init, series_from_pts};

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
    let pts = ds
        .predictors
        .iter()
        .zip(ds.y.iter())
        .enumerate()
        .map(|(row, (xs, &y))| DataPoint { x: if negate_x { xs.iter().map(|&v| -v).collect() } else { xs.clone() }, row, ..DataPoint::xy(0.0, y) })
        .collect();
    series_from_pts(0, &ds.name, pts)
}

fn series_from_lls(ds: &LinearRegressionDataset) -> DatasetSeries {
    let pts = ds.design_rows.iter().zip(ds.y.iter()).enumerate().map(|(row, (cols, &y))| DataPoint { row, ..DataPoint::xy(cols[1], y) }).collect();
    series_from_pts(0, &ds.name, pts)
}

fn run_fit(model_id: &str, series: &DatasetSeries, rules: Vec<ParameterRule>) -> NonlinearFitResult {
    let model = get_builtin_model(model_id).unwrap_or_else(|| panic!("missing built-in model `{model_id}`"));
    let cfg = SolverConfig {
        parameter_rules: rules,
        max_iters: 3_000,
        compute_prediction_bands: false,
        compute_lack_of_fit: false,
        compute_normality: false,
        interpolate_unknowns: false,
        use_simplex_warm_start: true,
        ..cfg_fast()
    };
    let fit = fit_with(model.as_ref(), std::slice::from_ref(series), &cfg);
    assert!(
        matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous),
        "{model_id} on {} did not converge: status={:?}, rss={:.6e}",
        series.label,
        fit.status,
        fit.rss
    );
    fit
}

fn verify_fit(
    model_id: &str,
    series: &DatasetSeries,
    rules: Vec<ParameterRule>,
    rss: (f64, f64, f64),
    params: &[(&str, f64, f64, f64, f64)],
    ctx: &str,
) -> NonlinearFitResult {
    let fit = run_fit(model_id, series, rules);
    for &(name, est, se, r_est, r_se) in params {
        let p = find_param(&fit, name);
        assert_approx_eq(p.estimate, est, 1e-12, r_est, &format!("{ctx} {name} estimate"));
        assert_approx_eq(p.se, se, 1e-12, r_se, &format!("{ctx} {name} SE"));
    }
    assert_approx_eq(fit.rss, rss.0, rss.1, rss.2, &format!("{ctx} RSS"));
    fit
}

#[test]
fn nist_reparam_enzyme_binding_kinetics_and_power() {
    let Some(dir) = resolve_corpus_dir() else { return };

    // 1. Misra1d -> michaelis_menten & one_site_specific_binding
    let misra1d = load_corpus_nonlinear(&dir, "Misra1d").unwrap();
    let (b, s) = (&misra1d.certified_betas, &misra1d.certified_se_betas);
    let (vmax, km, se_vmax, se_km) = (b[0], 1.0 / b[1], s[0], s[1] / (b[1] * b[1]));
    let ser = series_from_nls(&misra1d, false);
    for (id, p1, p2) in [("michaelis_menten", "Vmax", "Km"), ("one_site_specific_binding", "Bmax", "Kd")] {
        verify_fit(
            id,
            &ser,
            vec![rule_local_init(p1, 450.0), rule_local_init(p2, 3000.0)],
            (misra1d.certified_rss, 1e-12, 1e-9),
            &[(p1, vmax, se_vmax, 1e-7, 1e-6), (p2, km, se_km, 1e-7, 1e-6)],
            &format!("Misra1d>{id}"),
        );
    }

    // 2. BoxBOD & Misra1a -> one_phase_association, exponential_plateau, one_phase_decay (Y0 = 0)
    for (ds_name, init_p, init_k) in [("BoxBOD", 200.0, 0.5), ("Misra1a", 250.0, 5e-4)] {
        let ds = load_corpus_nonlinear(&dir, ds_name).unwrap();
        let ser_exp = series_from_nls(&ds, false);
        let (p_cert, k_cert, p_se, k_se) = (ds.certified_betas[0], ds.certified_betas[1], ds.certified_se_betas[0], ds.certified_se_betas[1]);
        for (id, p_name, k_name, check_hl) in
            [("one_phase_association", "Plateau", "K", false), ("exponential_plateau", "YM", "k", false), ("one_phase_decay", "Plateau", "K", true)]
        {
            let fit = verify_fit(
                id,
                &ser_exp,
                vec![rule_const("Y0", 0.0), rule_local_init(p_name, init_p), rule_local_init(k_name, init_k)],
                (ds.certified_rss, 1e-12, 1e-9),
                &[(p_name, p_cert, p_se, 1e-7, 1e-6), (k_name, k_cert, k_se, 1e-7, 1e-6)],
                &format!("{ds_name}>{id}"),
            );
            if check_hl {
                assert_approx_eq(
                    find_derived(&fit, 0, "HalfLife"),
                    std::f64::consts::LN_2 / k_cert,
                    1e-12,
                    1e-7,
                    &format!("{ds_name}>{id} HalfLife"),
                );
            }
        }
    }

    // 3. DanWood -> power_law (y = a * x^b)
    let danwood = load_corpus_nonlinear(&dir, "DanWood").unwrap();
    verify_fit(
        "power_law",
        &series_from_nls(&danwood, false),
        vec![rule_local_init("a", 1.0), rule_local_init("b", 5.0)],
        (danwood.certified_rss, 1e-12, 1e-9),
        &[
            ("a", danwood.certified_betas[0], danwood.certified_se_betas[0], 1e-7, 1e-6),
            ("b", danwood.certified_betas[1], danwood.certified_se_betas[1], 1e-7, 1e-6),
        ],
        "DanWood>power_law",
    );
}

#[test]
fn nist_reparam_gaussian_sigmoids_and_three_phase_decay() {
    let Some(dir) = resolve_corpus_dir() else { return };

    // 4. Eckerle4 -> gaussian
    let eckerle4 = load_corpus_nonlinear(&dir, "Eckerle4").unwrap();
    let (b, s) = (&eckerle4.certified_betas, &eckerle4.certified_se_betas);
    verify_fit(
        "gaussian",
        &series_from_nls(&eckerle4, false),
        vec![rule_local_init("Amplitude", 0.3), rule_local_init("Mean", 451.0), rule_local_init("SD", 5.0)],
        (eckerle4.certified_rss, 1e-12, 1e-9),
        &[("Amplitude", b[0] / b[1], 3.768_303_420_630_896e-3, 1e-7, 1e-5), ("Mean", b[2], s[2], 1e-7, 1e-6), ("SD", b[1], s[1], 1e-7, 1e-6)],
        "Eckerle4>gaussian",
    );

    // 5. Rat42 -> logistic_growth, log_agonist_vs_response_4pl, log_inhibitor_vs_response_4pl
    let rat42 = load_corpus_nonlinear(&dir, "Rat42").unwrap();
    let (b, s) = (&rat42.certified_betas, &rat42.certified_se_betas);
    let (y0_cert, log_ec50_cert, hill_cert, se_hill) =
        (b[0] / (1.0 + b[1].exp()), b[1] / b[2], b[2] / std::f64::consts::LN_10, s[2] / std::f64::consts::LN_10);
    let ser_r42 = series_from_nls(&rat42, false);
    verify_fit(
        "logistic_growth",
        &ser_r42,
        vec![rule_local_init("YM", b[0] * 1.05), rule_local_init("Y0", y0_cert * 1.05), rule_local_init("k", b[2] * 1.05)],
        (rat42.certified_rss, 1e-12, 1e-9),
        &[("YM", b[0], s[0], 1e-6, 1e-5), ("Y0", y0_cert, 4.709_402_845_203_958e-1, 1e-6, 1e-5), ("k", b[2], s[2], 1e-6, 1e-5)],
        "Rat42>logistic_growth",
    );
    verify_fit(
        "log_agonist_vs_response_4pl",
        &ser_r42,
        vec![
            rule_const("Bottom", 0.0),
            rule_local_init("Top", b[0] * 1.05),
            rule_local_init("LogEC50", log_ec50_cert * 1.05),
            rule_local_init("HillSlope", hill_cert * 1.05),
        ],
        (rat42.certified_rss, 1e-12, 1e-9),
        &[
            ("Top", b[0], s[0], 1e-6, 1e-5),
            ("LogEC50", log_ec50_cert, 1.179_440_611_437_669, 1e-6, 1e-5),
            ("HillSlope", hill_cert, se_hill, 1e-6, 1e-5),
        ],
        "Rat42>4pl_agonist",
    );
    verify_fit(
        "log_inhibitor_vs_response_4pl",
        &series_from_nls(&rat42, true),
        vec![
            rule_const("Bottom", 0.0),
            rule_local_init("Top", b[0] * 1.05),
            rule_local_init("LogIC50", -log_ec50_cert * 1.05),
            rule_local_init("HillSlope", hill_cert * 1.05),
        ],
        (rat42.certified_rss, 1e-12, 1e-9),
        &[
            ("Top", b[0], s[0], 1e-6, 1e-5),
            ("LogIC50", -log_ec50_cert, 1.179_440_611_437_669, 1e-6, 1e-5),
            ("HillSlope", hill_cert, se_hill, 1e-6, 1e-5),
        ],
        "Rat42>4pl_inhibitor",
    );

    // 6. Rat43 -> log_agonist_vs_response_5pl (Bottom = 0)
    let rat43 = load_corpus_nonlinear(&dir, "Rat43").unwrap();
    let (b, s) = (&rat43.certified_betas, &rat43.certified_se_betas);
    let (log_xb, hill5, s_asym) = (b[1] / b[2], b[2] / std::f64::consts::LN_10, 1.0 / b[3]);
    verify_fit(
        "log_agonist_vs_response_5pl",
        &series_from_nls(&rat43, false),
        vec![
            rule_const("Bottom", 0.0),
            rule_local_init("Top", b[0] * 1.05),
            rule_local_init("LogXb", log_xb * 1.05),
            rule_local_init("HillSlope", hill5 * 1.05),
            rule_local_init("S", s_asym * 1.05),
        ],
        (rat43.certified_rss, 1e-12, 1e-9),
        &[
            ("Top", b[0], s[0], 1e-6, 1e-5),
            ("LogXb", log_xb, 1.013_920_603_500_262, 1e-6, 1e-5),
            ("HillSlope", hill5, s[2] / std::f64::consts::LN_10, 1e-6, 1e-5),
            ("S", s_asym, s[3] / (b[3] * b[3]), 1e-6, 1e-5),
        ],
        "Rat43>5pl",
    );

    // 7. Lanczos3 -> three_phase_decay (Plateau = 0)
    let lanczos3 = load_corpus_nonlinear(&dir, "Lanczos3").unwrap();
    let (b, s) = (&lanczos3.certified_betas, &lanczos3.certified_se_betas);
    let y0 = b[0] + b[2] + b[4];
    let (pf, ps) = (100.0 * b[4] / y0, 100.0 * b[2] / y0);
    verify_fit(
        "three_phase_decay",
        &series_from_nls(&lanczos3, false),
        vec![
            rule_local_init("Y0", y0 * 1.02),
            rule_const("Plateau", 0.0),
            rule_local_init("PercentFast", pf * 1.02),
            rule_local_init("PercentSlow", ps * 1.02),
            rule_local_init("KFast", b[5] * 1.02),
            rule_local_init("KSlow", b[3] * 1.02),
            rule_local_init("KVerySlow", b[1] * 1.02),
        ],
        (lanczos3.certified_rss, 1e-16, 1e-8),
        &[
            ("Y0", y0, 2.879_558_676_621_674e-5, 1e-6, 1e-4),
            ("PercentFast", pf, 2.322_573_045_577_274, 1e-5, 1e-4),
            ("PercentSlow", ps, 1.650_549_427_342_901, 1e-5, 1e-4),
            ("KFast", b[5], s[5], 1e-5, 1e-4),
            ("KSlow", b[3], s[3], 1e-5, 1e-4),
            ("KVerySlow", b[1], s[1], 1e-5, 1e-4),
        ],
        "Lanczos3>three_phase_decay",
    );
}

fn verify_polynomial_lls(model_id: &str, ds: &LinearRegressionDataset, est_tol: f64, se_tol: f64) {
    let fit = run_fit(model_id, &series_from_lls(ds), Vec::new());
    for (k, (&cert_b, &cert_se)) in ds.certified_betas.iter().zip(ds.certified_se_betas.iter()).enumerate() {
        let name = format!("B{k}");
        let p = find_param(&fit, &name);
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
    verify_polynomial_lls("polynomial_5", &wampler1(), 1e-7, 1e-6);
    verify_polynomial_lls("polynomial_5", &wampler2(), 1e-7, 1e-6);
}

#[test]
fn nist_reparam_polynomial_corpus_pontius_and_wampler1_to_5() {
    let Some(dir) = resolve_corpus_dir() else { return };
    verify_polynomial_lls("polynomial_2", &load_corpus_linear(&dir, "Pontius").unwrap(), 1e-7, 1e-6);
    for name in ["Wampler1", "Wampler2", "Wampler3", "Wampler4", "Wampler5"] {
        verify_polynomial_lls("polynomial_5", &load_corpus_linear(&dir, name).unwrap(), 1e-6, 1e-5);
    }
}

#[test]
fn nist_reparam_r18_3_extended_builtins() {
    let Some(dir) = resolve_corpus_dir() else { return };

    // 9. Eckerle4 -> gaussian_area
    let eckerle4 = load_corpus_nonlinear(&dir, "Eckerle4").unwrap();
    let (b, s, sqrt_2pi) = (&eckerle4.certified_betas, &eckerle4.certified_se_betas, (2.0 * std::f64::consts::PI).sqrt());
    let ga = verify_fit(
        "gaussian_area",
        &series_from_nls(&eckerle4, false),
        vec![rule_local_init("Area", b[0] * sqrt_2pi * 1.05), rule_local_init("Mean", 451.0), rule_local_init("SD", 5.0)],
        (eckerle4.certified_rss, 1e-12, 1e-9),
        &[("Area", b[0] * sqrt_2pi, s[0] * sqrt_2pi, 1e-7, 1e-6), ("Mean", b[2], s[2], 1e-7, 1e-6), ("SD", b[1], s[1], 1e-7, 1e-6)],
        "Eckerle4>gaussian_area",
    );
    assert_approx_eq(find_derived(&ga, 0, "Amplitude"), b[0] / b[1], 1e-12, 1e-7, "Eckerle4>gaussian_area derived Amplitude");

    // 10. Rat42 -> boltzmann_sigmoid
    let rat42 = load_corpus_nonlinear(&dir, "Rat42").unwrap();
    let (b, s) = (&rat42.certified_betas, &rat42.certified_se_betas);
    let (v50_cert, slope_cert, se_slope) = (b[1] / b[2], 1.0 / b[2], s[2] / (b[2] * b[2]));
    verify_fit(
        "boltzmann_sigmoid",
        &series_from_nls(&rat42, false),
        vec![
            rule_const("Bottom", 0.0),
            rule_local_init("Top", b[0] * 1.05),
            rule_local_init("V50", v50_cert * 1.05),
            rule_local_init("Slope", slope_cert * 1.05),
        ],
        (rat42.certified_rss, 1e-12, 1e-9),
        &[("Top", b[0], s[0], 1e-6, 1e-5), ("V50", v50_cert, 1.179_440_601_976_244, 1e-6, 1e-5), ("Slope", slope_cert, se_slope, 1e-6, 1e-5)],
        "Rat42>boltzmann_sigmoid",
    );

    // 11. BoxBOD -> two_phase_association (Y0 = 0, PercentFast = 100, KSlow = 0.01)
    let boxbod = load_corpus_nonlinear(&dir, "BoxBOD").unwrap();
    verify_fit(
        "two_phase_association",
        &series_from_nls(&boxbod, false),
        vec![
            rule_const("Y0", 0.0),
            rule_local_init("Plateau", 200.0),
            rule_const("PercentFast", 100.0),
            rule_local_init("KFast", 0.5),
            rule_const("KSlow", 0.01),
        ],
        (boxbod.certified_rss, 1e-12, 1e-9),
        &[
            ("Plateau", boxbod.certified_betas[0], boxbod.certified_se_betas[0], 1e-7, 1e-6),
            ("KFast", boxbod.certified_betas[1], boxbod.certified_se_betas[1], 1e-7, 1e-6),
        ],
        "BoxBOD>two_phase_association",
    );

    // 12. DanWood -> power_series_two_term (C = 0, D = 1 => y = A * x^B)
    let danwood = load_corpus_nonlinear(&dir, "DanWood").unwrap();
    verify_fit(
        "power_series_two_term",
        &series_from_nls(&danwood, false),
        vec![rule_local_init("A", 1.0), rule_local_init("B", 5.0), rule_const("C", 0.0), rule_const("D", 1.0)],
        (danwood.certified_rss, 1e-12, 1e-9),
        &[
            ("A", danwood.certified_betas[0], danwood.certified_se_betas[0], 1e-7, 1e-6),
            ("B", danwood.certified_betas[1], danwood.certified_se_betas[1], 1e-7, 1e-6),
        ],
        "DanWood>power_series_two_term",
    );
}
