//! NIST StRD Nonlinear Least Squares Certification Suite (ALL 27 datasets x `start_1` & `start_2` = 54 fits).

use curvebook_doc::{FitMethod, ParameterRule, ParameterScope, WeightingScheme};
use curvebook_fit::{DataPoint, DatasetSeries, FitStatus, HookContext, SolverConfig, fit_nonlinear, get_builtin_model};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::metrics::{assert_lre_at_least, log_relative_error};
use curvebook_testkit::nist_strd::{NonlinearDataset, corpus_available, default_nist_strd_dir, load_corpus_nonlinear_all};

fn build_dataset_series(ds: &NonlinearDataset) -> DatasetSeries {
    let is_nelson = ds.name.eq_ignore_ascii_case("Nelson");
    let n = ds.y.len();
    let mut points = Vec::with_capacity(n);
    for row in 0..n {
        let predictors = ds.predictors[row].clone();
        let y_val = if is_nelson { ds.y[row].ln() } else { ds.y[row] };
        points.push(DataPoint { x: predictors, y: y_val, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row });
    }
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

fn verify_nist_start(ds: &NonlinearDataset, series: &DatasetSeries, start_name: &str, start_vals: &[f64]) {
    let eq_id = format!("nist_{}", ds.name.to_ascii_lowercase());
    let maybe_model = get_builtin_model(&eq_id);
    assert!(maybe_model.is_some(), "Missing built-in NIST model `{eq_id}` for dataset `{}`", ds.name);
    let Some(model) = maybe_model else {
        return;
    };
    assert_eq!(model.param_count(), ds.certified_betas.len(), "Parameter count mismatch for {}", ds.name);

    let rules: Vec<ParameterRule> = model
        .param_names()
        .iter()
        .zip(start_vals.iter())
        .map(|(name, &val)| ParameterRule {
            name: name.clone(),
            scope: ParameterScope::Local,
            initial_override: Some(val),
            lower_bound: None,
            upper_bound: None,
        })
        .collect();

    let cfg = SolverConfig {
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::None,
        parameter_rules: rules,
        confidence: ConfidenceLevel::DEFAULT_95,
        max_iters: 2_500,
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

    let fit_res = fit_nonlinear(model.as_ref(), std::slice::from_ref(series), &HookContext::default(), &cfg);
    assert!(fit_res.is_ok(), "Fit failed on {} ({start_name}): {:?}", ds.name, fit_res.as_ref().err());
    let Ok(fit) = fit_res else {
        return;
    };

    assert!(
        matches!(fit.status, FitStatus::Converged | FitStatus::Ambiguous),
        "{} ({start_name}) did not converge cleanly: status={:?}, rss={:.6e}",
        ds.name,
        fit.status,
        fit.rss
    );
    let expected_df = if ds.name.eq_ignore_ascii_case("Rat43") {
        // Rat43 has 15 observations and 4 parameters (df = 11; NIST computes RSD = sqrt(RSS / 11)),
        // while the upstream NIST header lists Degrees of Freedom = 9.
        11
    } else {
        ds.certified_df
    };
    assert_eq!(fit.df, expected_df, "DF mismatch on {}", ds.name);

    let label = format!("{} [{start_name}]", ds.name);

    // Check parameter estimates & SEs via NIST Log Relative Error (LRE).
    let mut min_beta_lre = f64::INFINITY;
    let mut min_se_lre = f64::INFINITY;
    for (idx, p) in fit.parameters.iter().enumerate() {
        let cert_b = ds.certified_betas[idx];
        let b_lre = log_relative_error(p.estimate, cert_b);
        min_beta_lre = min_beta_lre.min(b_lre);
        assert_lre_at_least(p.estimate, cert_b, 6.5, &format!("{label} beta[{}]={}", idx, p.name));

        let cert_se = ds.certified_se_betas[idx];
        let se_lre = log_relative_error(p.se, cert_se);
        min_se_lre = min_se_lre.min(se_lre);
        let min_req_se = if ds.name.eq_ignore_ascii_case("Lanczos1") { 2.5 } else { 5.0 };
        assert_lre_at_least(p.se, cert_se, min_req_se, &format!("{label} se[{}]={}", idx, p.name));
    }

    // Check RSS & Residual SD (with Lanczos1 machine-floor exemption near ~1e-25).
    if ds.name.eq_ignore_ascii_case("Lanczos1") {
        assert!(fit.rss < 1e-22, "{label} Lanczos1 RSS {:.3e} >= 1e-22", fit.rss);
        assert!(fit.rsd < 1e-11, "{label} Lanczos1 RSD {:.3e} >= 1e-11", fit.rsd);
    } else {
        assert_lre_at_least(fit.rss, ds.certified_rss, 7.0, &format!("{label} RSS"));
        assert_lre_at_least(fit.rsd, ds.certified_residual_sd, 6.5, &format!("{label} RSD"));
    }

    let rss_lre = log_relative_error(fit.rss, ds.certified_rss);
    let rsd_lre = log_relative_error(fit.rsd, ds.certified_residual_sd);
    eprintln!(
        "NIST {:<10} {:<7} ({:?}): min_beta_LRE={:.2}, min_se_LRE={:.2}, RSS_LRE={:.2}, RSD_LRE={:.2}, iters={}",
        ds.name, start_name, ds.difficulty, min_beta_lre, min_se_lre, rss_lre, rsd_lre, fit.iterations
    );
}

#[test]
fn nist_strd_all_27_nonlinear_benchmarks_from_both_starts() {
    let dir = default_nist_strd_dir();
    if !corpus_available(&dir) {
        if std::env::var("CURVEBOOK_REQUIRE_CORPUS").ok().as_deref() == Some("1") {
            panic!("CURVEBOOK_REQUIRE_CORPUS=1 set, but NIST StRD corpus directory is missing");
        }
        eprintln!("Skipping NIST StRD nonlinear suite: corpus not present locally");
        return;
    }

    let datasets = load_corpus_nonlinear_all(&dir).expect("Failed to load NIST StRD nonlinear corpus");
    assert_eq!(datasets.len(), 27, "Expected all 27 NIST StRD nonlinear datasets");

    for ds in &datasets {
        let series = build_dataset_series(ds);
        verify_nist_start(ds, &series, "start_1", &ds.start_1);
        verify_nist_start(ds, &series, "start_2", &ds.start_2);
    }
}
