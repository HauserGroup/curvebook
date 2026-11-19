//! D1 public-oracle verification suite for `curvebook-fit`.
//!
//! Oracles and literature references verified here:
//! - NIST Statistical Reference Datasets (StRD), Nonlinear Least Squares:
//!   certified parameter estimates, standard errors, residual sum of squares,
//!   and residual standard deviations across Lower, Average, and Higher tiers.
//! - Motulsky & Christopoulos (2004), *Fitting Models to Biological Data Using
//!   Linear and Nonlinear Regression*: 4PL/5PL dose-response curves, shared
//!   global fitting, weighting (`1/Y^2`, `1/|Y|`, `1/|X|`, `1/X^2`, `ObservedSd`),
//!   robust Cauchy regression, extra-sum-of-squares F test, and AICc comparison.
//! - Arunlakshana & Schild (1959) / Gaddum-Schild competitive antagonism (`pA2`).
//! - Cheng & Prusoff (1973) competitive receptor binding (`Ki`).
//! - Michaelis-Menten, Hill allosteric, and competitive / noncompetitive /
//!   uncompetitive enzyme inhibition kinetics (Cornish-Bowden 1974).
//! - One-, two-, and three-phase exponential decay; association-dissociation;
//!   logistic (Verhulst 1838) and Gompertz (1825) growth models.
//! - Venzon & Moolgavkar (1988) profile-likelihood confidence intervals vs
//!   symmetric Wald intervals and unbounded (`CiBound::Open`) plateaus.
//! - Deming (1943) Model II orthogonal & weighted total least squares.
//! - Motulsky & Brown (2006) ROUT outlier elimination (`Q = 1%`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CiBound, ConstantValue, DataSheet, FitMethod, HookSource, InfoSheet, LinearRegressionSpec,
    ModelComparisonSpec, NonlinearFitSpec, ParameterRule, ParameterScope, ResultSheet, SheetId, SubcolumnMode, TableArchetype, WeightingScheme,
    XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, DemingOptions, FitError, FitStatus, FittedParameterSummary, HookContext, ModelComparisonOutcome,
    NonlinearFitResult, ParameterNote, ParametricModel, SolverConfig, compare_models, compare_models_aicc, compare_models_f_test, deming_regression,
    execute_linear_analysis, execute_nonlinear_analysis, fit_nonlinear, get_builtin_model, rout_detect_outliers, simple_linear_regression,
};
use curvebook_testkit::nist_strd::{NistDifficulty, default_nist_strd_dir, load_corpus_nonlinear_all};
use curvebook_testkit::{assert_approx_eq, assert_lre_at_least, log_relative_error};

fn nist_strd_dir() -> std::path::PathBuf {
    default_nist_strd_dir()
}

fn series_idx(idx: usize, label: &str, x: Vec<f64>, y: Vec<f64>) -> DatasetSeries {
    assert_eq!(x.len(), y.len());
    let points = x
        .into_iter()
        .zip(y)
        .enumerate()
        .map(|(row, (xv, yv))| DataPoint {
            x: vec![xv],
            y: yv,
            sd: None,
            n: None,
            explicit_weight: None,
            source_subcolumn: 0,
            data_set_index: idx,
            row,
        })
        .collect();
    DatasetSeries {
        data_set_index: idx,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn series(label: &str, x: Vec<f64>, y: Vec<f64>) -> DatasetSeries {
    series_idx(0, label, x, y)
}

fn rule_local(name: &str, init: Option<f64>, lb: Option<f64>, ub: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Local, initial_override: init, lower_bound: lb, upper_bound: ub }
}

fn rule_const(name: &str, val: f64) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::Constant(val), initial_override: None, lower_bound: None, upper_bound: None }
}

fn rule_shared(name: &str, init: Option<f64>, lb: Option<f64>, ub: Option<f64>) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::SharedGlobal, initial_override: init, lower_bound: lb, upper_bound: ub }
}

fn find_param<'a>(res: &'a NonlinearFitResult, ds_idx: usize, name: &str) -> &'a FittedParameterSummary {
    let found = res.parameters.iter().find(|p| p.name.eq_ignore_ascii_case(name) && (p.dataset_index == Some(ds_idx) || p.dataset_index.is_none()));
    assert!(found.is_some(), "missing parameter `{name}` for dataset {ds_idx}");
    found.unwrap_or(&res.parameters[0])
}

fn find_ds_param(res: &NonlinearFitResult, ds_idx: usize, name: &str) -> f64 {
    let pos = res.datasets[ds_idx].param_names.iter().position(|p| p.eq_ignore_ascii_case(name));
    assert!(pos.is_some(), "missing dataset param `{name}` on dataset {ds_idx}");
    pos.map_or(f64::NAN, |i| res.datasets[ds_idx].params[i])
}

fn find_derived(res: &NonlinearFitResult, ds_idx: usize, name: &str) -> f64 {
    let val = res.derived_scalars[ds_idx].iter().find(|d| d.name.eq_ignore_ascii_case(name)).map(|d| d.value);
    assert!(val.is_some(), "missing derived scalar `{name}` on dataset {ds_idx}");
    val.unwrap_or(f64::NAN)
}

fn eval_model_at(model: &dyn ParametricModel, res: &NonlinearFitResult, ds_idx: usize, x: f64) -> f64 {
    let val = model.eval(&[x], &res.datasets[ds_idx].params, ds_idx);
    assert!(val.is_some(), "model evaluation returned None at finite x={x}");
    val.unwrap_or(f64::NAN)
}

fn build_xy_sheet(sheet_id: &str, title: &str, x: &[f64], columns: &[(&str, &[f64])]) -> Result<DataSheet, curvebook_doc::DocError> {
    let mut xy = XyTable::new(SubcolumnMode::Single, columns.len(), x.len())?;
    for (c_idx, (col_title, _)) in columns.iter().enumerate() {
        xy.data_sets[c_idx].title = (*col_title).to_owned();
    }
    let mut sheet = DataSheet::new(sheet_id, title, TableArchetype::Xy(xy))?;
    for (r_idx, &xv) in x.iter().enumerate() {
        sheet.set_x_or_time_cell(r_idx, 0, Cell::number(xv))?;
        for (c_idx, (_, y_vals)) in columns.iter().enumerate() {
            sheet.set_cell(CellCoord::new(r_idx, c_idx, 0), Cell::number(y_vals[r_idx]))?;
        }
    }
    Ok(sheet)
}

fn assert_converged(status: FitStatus, label: &str) {
    assert_eq!(status, FitStatus::Converged, "{label}: expected FitStatus::Converged, got {status:?}");
}

fn assert_nist_converged(status: FitStatus, label: &str) {
    assert!(matches!(status, FitStatus::Converged | FitStatus::Ambiguous), "{label}: expected Converged or Ambiguous, got {status:?}");
}

fn assert_normal_equations_small(model: &dyn ParametricModel, res: &NonlinearFitResult, ds: &DatasetSeries, free_param_names: &[&str], tol: f64) {
    let p_names = model.param_names();
    let full_params = res.datasets[0].params.clone();
    let eps = 1e-7;
    for &fname in free_param_names {
        let maybe_idx = p_names.iter().position(|p| p.eq_ignore_ascii_case(fname));
        assert!(maybe_idx.is_some(), "missing parameter `{fname}` in normal equations check");
        let Some(p_idx) = maybe_idx else {
            return;
        };
        let mut plus = full_params.clone();
        let mut minus = full_params.clone();
        let h = eps * (1.0 + full_params[p_idx].abs());
        plus[p_idx] += h;
        minus[p_idx] -= h;
        let mut grad = 0.0_f64;
        for pt in &ds.points {
            let xi = pt.x[0];
            let fx = model.eval(&[xi], &full_params, 0);
            let fp = model.eval(&[xi], &plus, 0);
            let fm = model.eval(&[xi], &minus, 0);
            assert!(fx.is_some() && fp.is_some() && fm.is_some(), "finite-diff eval failed at x={xi}");
            let (Some(fx), Some(fp), Some(fm)) = (fx, fp, fm) else {
                return;
            };
            let r = pt.y - fx;
            let df = (fp - fm) / (2.0 * h);
            grad += r * df;
        }
        assert!(grad.abs() <= tol, "normal equation |J^T r| for `{fname}` = {grad:.3e} > tol {tol:.3e}");
    }
}

// ============================================================================
// SECTION A: NIST StRD Nonlinear Least Squares Oracles
// ============================================================================

fn verify_nist_tier(difficulty: NistDifficulty, expected_count: usize) {
    let dir = nist_strd_dir();
    if !dir.exists() {
        assert_ne!(std::env::var("CURVEBOOK_REQUIRE_CORPUS").ok().as_deref(), Some("1"), "missing NIST StRD corpus at {}", dir.display());
        return;
    }

    let load_res = load_corpus_nonlinear_all(&dir);
    assert!(load_res.is_ok(), "load NIST StRD corpus failed: {:?}", load_res.as_ref().err());
    let Ok(all) = load_res else {
        return;
    };
    let tier_ds: Vec<_> = all.into_iter().filter(|d| d.difficulty == difficulty).collect();
    assert_eq!(tier_ds.len(), expected_count, "unexpected NIST dataset count for {difficulty:?}");

    for ds in &tier_ds {
        let model_id = format!("nist_{}", ds.name.to_ascii_lowercase());
        let maybe_model = get_builtin_model(&model_id);
        assert!(maybe_model.is_some(), "missing builtin `{model_id}`");
        let Some(model) = maybe_model else {
            return;
        };
        let is_nelson = ds.name.eq_ignore_ascii_case("Nelson");
        let n_obs = ds.y.len();
        let points: Vec<DataPoint> = (0..n_obs)
            .map(|row| DataPoint {
                x: ds.predictors[row].clone(),
                y: if is_nelson { ds.y[row].ln() } else { ds.y[row] },
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row,
            })
            .collect();
        let input = DatasetSeries {
            data_set_index: 0,
            label: ds.name.clone(),
            points,
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: None,
        };

        for (start_label, starts) in [("start1", &ds.start_1), ("start2", &ds.start_2)] {
            let rules: Vec<ParameterRule> =
                model.param_names().iter().zip(starts.iter()).map(|(name, &val)| rule_local(name, Some(val), None, None)).collect();

            let cfg = SolverConfig { parameter_rules: rules, compute_profile_ci: false, ..SolverConfig::default() };
            let fit_res = fit_nonlinear(model.as_ref(), std::slice::from_ref(&input), &HookContext::default(), &cfg);
            assert!(fit_res.is_ok(), "{} ({start_label}) failed: {:?}", ds.name, fit_res.as_ref().err());
            let Ok(res) = fit_res else {
                return;
            };

            assert_nist_converged(res.status, &format!("{} ({start_label})", ds.name));

            let expected_df = n_obs - model.param_names().len();
            assert_eq!(res.df, expected_df, "{} df mismatch", ds.name);
            if ds.name != "Rat43" {
                assert_eq!(ds.certified_df, expected_df);
            }

            for (idx, ((&cert_b, &cert_se), p)) in ds.certified_betas.iter().zip(ds.certified_se_betas.iter()).zip(res.parameters.iter()).enumerate()
            {
                let tag = format!("{} ({start_label}) b{}", ds.name, idx + 1);
                let beta_floor = match difficulty {
                    NistDifficulty::Lower => 7.0,
                    NistDifficulty::Average => {
                        if ds.name == "Lanczos1" || ds.name == "ENSO" {
                            6.5
                        } else {
                            7.0
                        }
                    }
                    NistDifficulty::Higher => 6.5,
                };
                assert_lre_at_least(p.estimate, cert_b, beta_floor, &format!("{tag} beta"));

                let se_floor = if ds.name == "Lanczos1" {
                    2.5
                } else if difficulty == NistDifficulty::Lower {
                    5.5
                } else {
                    5.0
                };
                assert_lre_at_least(p.se, cert_se, se_floor, &format!("{tag} SE"));
            }

            if ds.name == "Lanczos1" {
                assert!(res.rss < 1e-22, "Lanczos1 RSS too high: {}", res.rss);
                assert!(res.rsd < 1e-11, "Lanczos1 RSD too high: {}", res.rsd);
            } else {
                let rss_lre = log_relative_error(res.rss, ds.certified_rss);
                let rsd_lre = log_relative_error(res.rsd, ds.certified_residual_sd);
                let rss_floor = match difficulty {
                    NistDifficulty::Lower => 8.0,
                    NistDifficulty::Average => 7.5,
                    NistDifficulty::Higher => 7.0,
                };
                assert!(rss_lre >= rss_floor, "{} ({start_label}) RSS LRE {rss_lre:.2} < {rss_floor}", ds.name);
                assert!(rsd_lre >= 6.5, "{} ({start_label}) RSD LRE {rsd_lre:.2} < 6.5", ds.name);
            }
        }
    }
}

#[test]
fn nist_strd_lower_difficulty_8_datasets() {
    verify_nist_tier(NistDifficulty::Lower, 8);
}

#[test]
fn nist_strd_average_difficulty_11_datasets() {
    verify_nist_tier(NistDifficulty::Average, 11);
}

#[test]
fn nist_strd_higher_difficulty_8_datasets() {
    verify_nist_tier(NistDifficulty::Higher, 8);
}

#[test]
fn nist_strd_inline_certified_danwood_and_boxbod_oracles() {
    // Self-contained NIST StRD oracles: DanWood (Lower) from default initial guess
    // and BoxBOD (Higher) from Start 2.
    let dw_x = vec![1.309, 1.471, 1.490, 1.565, 1.611, 1.680];
    let dw_y = vec![2.138, 3.421, 3.597, 4.340, 4.882, 5.660];
    let dw_cert_b = [0.76886226176, 3.8604055871];
    let dw_cert_se = [0.018281973860, 0.051726610913];
    let dw_cert_rss = 0.0043173084083;

    let dw_model = get_builtin_model("nist_danwood").expect("nist_danwood");
    let dw_res = fit_nonlinear(dw_model.as_ref(), &[series("DanWood", dw_x, dw_y)], &HookContext::default(), &SolverConfig::default())
        .expect("DanWood auto-init fit");
    assert_converged(dw_res.status, "DanWood auto-init");
    for i in 0..2 {
        assert_lre_at_least(dw_res.parameters[i].estimate, dw_cert_b[i], 9.0, &format!("DanWood b{}", i + 1));
        assert_lre_at_least(dw_res.parameters[i].se, dw_cert_se[i], 7.0, &format!("DanWood SE{}", i + 1));
    }
    assert_lre_at_least(dw_res.rss, dw_cert_rss, 9.0, "DanWood RSS");

    // BoxBOD (Higher): y = b1 * (1 - exp(-b2 * x)), Start 2 (100, 0.75)
    let bb_x = vec![1.0, 2.0, 3.0, 5.0, 7.0, 10.0];
    let bb_y = vec![109.0, 149.0, 149.0, 191.0, 213.0, 224.0];
    let bb_cert_b = [213.80940889, 0.54723748542];
    let bb_cert_se = [12.354515176, 0.10455993237];
    let bb_cert_rss = 1168.0088766;

    let bb_model = get_builtin_model("nist_boxbod").expect("nist_boxbod");
    let bb_cfg = SolverConfig {
        parameter_rules: vec![rule_local("b1", Some(100.0), None, None), rule_local("b2", Some(0.75), None, None)],
        ..SolverConfig::default()
    };
    let bb_res = fit_nonlinear(bb_model.as_ref(), &[series("BoxBOD", bb_x, bb_y)], &HookContext::default(), &bb_cfg).expect("BoxBOD fit");
    assert_converged(bb_res.status, "BoxBOD");
    for i in 0..2 {
        assert_lre_at_least(bb_res.parameters[i].estimate, bb_cert_b[i], 8.0, &format!("BoxBOD b{}", i + 1));
        assert_lre_at_least(bb_res.parameters[i].se, bb_cert_se[i], 7.0, &format!("BoxBOD SE{}", i + 1));
    }
    assert_lre_at_least(bb_res.rss, bb_cert_rss, 8.0, "BoxBOD RSS");
}

// ============================================================================
// SECTION B: Dose-Response Pharmacology (4PL, 5PL, Gaddum-Schild, Cheng-Prusoff)
// ============================================================================

#[test]
fn pharma_4pl_agonist_and_inhibitor_oracles() {
    // 1) Agonist 4PL: Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))
    let log_c: Vec<f64> = (-20..=-7).map(|i| i as f64 * 0.5).collect();
    let true_b = 12.5_f64;
    let true_t = 98.0_f64;
    let true_log_ec50 = -6.75_f64;
    let true_h = 1.35_f64;
    let y_ag: Vec<f64> = log_c.iter().map(|&x| true_b + (true_t - true_b) / (1.0 + 10_f64.powf((true_log_ec50 - x) * true_h))).collect();

    let ag_model = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL agonist");
    let ag_res = fit_nonlinear(ag_model.as_ref(), &[series("Agonist", log_c, y_ag)], &HookContext::default(), &SolverConfig::default())
        .expect("4PL agonist fit");
    assert_converged(ag_res.status, "4PL agonist");
    assert_approx_eq(find_param(&ag_res, 0, "Bottom").estimate, true_b, 1e-6, 1e-6, "4PL ag Bottom");
    assert_approx_eq(find_param(&ag_res, 0, "Top").estimate, true_t, 1e-6, 1e-6, "4PL ag Top");
    assert_approx_eq(find_param(&ag_res, 0, "LogEC50").estimate, true_log_ec50, 1e-6, 1e-6, "4PL ag LogEC50");
    assert_approx_eq(find_param(&ag_res, 0, "HillSlope").estimate, true_h, 1e-6, 1e-6, "4PL ag HillSlope");
    assert_approx_eq(find_derived(&ag_res, 0, "EC50"), 10_f64.powf(true_log_ec50), 1e-12, 1e-6, "4PL ag derived EC50");
    assert_approx_eq(find_derived(&ag_res, 0, "Span"), true_t - true_b, 1e-6, 1e-6, "4PL ag derived Span");
    assert_approx_eq(
        eval_model_at(ag_model.as_ref(), &ag_res, 0, true_log_ec50),
        0.5 * (true_b + true_t),
        1e-6,
        1e-6,
        "4PL midpoint response == (Bottom + Top)/2",
    );

    // 2) Noisy inhibitor 4PL with normal equations & SE validation
    let log_i: Vec<f64> = (-20..=-6).map(|i| i as f64 * 0.5).collect();
    let inh_b = 8.0_f64;
    let inh_t = 104.0_f64;
    let inh_log_ic50 = -7.2_f64;
    let inh_h = 1.15_f64;
    let noise = [0.4, -0.6, 0.5, -0.3, 0.8, -0.7, 0.2, -0.5, 0.6, -0.4, 0.3, -0.2, 0.5, -0.4, 0.2];
    let y_inh: Vec<f64> =
        log_i.iter().zip(noise.iter()).map(|(&x, &e)| inh_b + (inh_t - inh_b) / (1.0 + 10_f64.powf((x - inh_log_ic50) * inh_h)) + e).collect();
    let inh_ds = series("Inhibitor", log_i, y_inh);
    let inh_model = get_builtin_model("log_inhibitor_vs_response_4pl").expect("4PL inhibitor");
    let inh_res = fit_nonlinear(inh_model.as_ref(), std::slice::from_ref(&inh_ds), &HookContext::default(), &SolverConfig::default())
        .expect("4PL inhibitor fit");
    assert_converged(inh_res.status, "4PL inhibitor");
    assert_normal_equations_small(inh_model.as_ref(), &inh_res, &inh_ds, &["Bottom", "Top", "LogIC50", "HillSlope"], 1e-5);
    for (name, truth) in [("Bottom", inh_b), ("Top", inh_t), ("LogIC50", inh_log_ic50), ("HillSlope", inh_h)] {
        let p = find_param(&inh_res, 0, name);
        assert!((p.estimate - truth).abs() <= 2.5 * p.se, "noisy 4PL `{name}` est={} outside 2.5*SE ({}) of {truth}", p.estimate, p.se);
    }
}

#[test]
fn pharma_5pl_asymmetric_richards_oracle() {
    // Richards 5PL: Y = Bottom + (Top - Bottom) / (1 + 10^((LogXb - X)*HillSlope))^S
    // Half-maximal response satisfies (1 + 10^((LogXb - LogEC50)*HillSlope))^S = 2
    // => LogEC50 = LogXb - log10(2^(1/S) - 1) / HillSlope
    let log_c: Vec<f64> = (-22..=-8).map(|i| i as f64 * 0.5).collect();
    let model = get_builtin_model("log_agonist_vs_response_5pl").expect("5PL agonist");

    for (true_s, tag) in [(2.4_f64, "S=2.4"), (0.45_f64, "S=0.45")] {
        let true_b = 5.0_f64;
        let true_t = 105.0_f64;
        let true_log_xb = -7.4_f64;
        let true_h = 1.25_f64;

        let y: Vec<f64> = log_c.iter().map(|&x| true_b + (true_t - true_b) / (1.0 + 10_f64.powf((true_log_xb - x) * true_h)).powf(true_s)).collect();

        let cfg = SolverConfig {
            parameter_rules: vec![
                rule_local("Bottom", Some(5.0), None, None),
                rule_local("Top", Some(105.0), None, None),
                rule_local("LogXb", Some(-7.2), None, None),
                rule_local("HillSlope", Some(1.0), Some(0.05), Some(10.0)),
                rule_local("S", Some(1.0), Some(1e-3), Some(20.0)),
            ],
            max_iters: 2000,
            ..SolverConfig::default()
        };

        let res = fit_nonlinear(model.as_ref(), &[series(tag, log_c.clone(), y)], &HookContext::default(), &cfg)
            .unwrap_or_else(|e| panic!("5PL {tag} failed: {e}"));
        assert_converged(res.status, &format!("5PL {tag}"));

        assert_approx_eq(find_param(&res, 0, "Bottom").estimate, true_b, 1e-5, 1e-6, &format!("{tag} Bottom"));
        assert_approx_eq(find_param(&res, 0, "Top").estimate, true_t, 1e-5, 1e-6, &format!("{tag} Top"));
        assert_approx_eq(find_param(&res, 0, "LogXb").estimate, true_log_xb, 1e-5, 1e-6, &format!("{tag} LogXb"));
        assert_approx_eq(find_param(&res, 0, "HillSlope").estimate, true_h, 1e-5, 1e-6, &format!("{tag} HillSlope"));
        assert_approx_eq(find_param(&res, 0, "S").estimate, true_s, 1e-5, 1e-6, &format!("{tag} S"));

        let expected_log_ec50 = true_log_xb - (2_f64.powf(1.0 / true_s) - 1.0).log10() / true_h;
        assert_approx_eq(find_derived(&res, 0, "LogEC50"), expected_log_ec50, 1e-6, 1e-6, &format!("{tag} derived LogEC50"));
        assert_approx_eq(find_derived(&res, 0, "EC50"), 10_f64.powf(expected_log_ec50), 1e-11, 1e-5, &format!("{tag} derived EC50"));
        let y_at_ec50 = eval_model_at(model.as_ref(), &res, 0, expected_log_ec50);
        assert_approx_eq(y_at_ec50, 0.5 * (true_b + true_t), 1e-4, 1e-5, &format!("{tag} Y(EC50)==50%"));
        let y_at_xb = eval_model_at(model.as_ref(), &res, 0, true_log_xb);
        assert!((y_at_xb - 0.5 * (true_b + true_t)).abs() > 5.0, "{tag}: LogXb must differ from LogEC50 when S != 1");
    }
}

#[test]
fn pharma_gaddum_schild_competitive_antagonism_pa2_builtin_and_global() {
    // Arunlakshana & Schild (1959): LogDR = log10(1 + (AntagConc * 10^pA2)^SchildSlope)
    let bottom = 10.0_f64;
    let top = 100.0_f64;
    let log_ec50 = -8.0_f64;
    let hill = 1.1_f64;
    let pa2 = 7.4_f64;
    let schild_slope = 1.0_f64;

    let log_ag: Vec<f64> = (-20..=-6).map(|i| i as f64 * 0.5).collect();
    let gen_curve = |antag_m: f64| -> Vec<f64> {
        let log_dr = if antag_m > 0.0 { (1.0 + (antag_m * 10_f64.powf(pa2)).powf(schild_slope)).log10() } else { 0.0 };
        let shifted = log_ec50 + log_dr;
        log_ag.iter().map(|&x| bottom + (top - bottom) / (1.0 + 10_f64.powf((shifted - x) * hill))).collect()
    };

    // Part 1: Built-in `ec50_shift_gaddum_schild` anchored to vehicle LogEC50 at [B] = 1e-6 M
    let shift_model = get_builtin_model("ec50_shift_gaddum_schild").expect("ec50_shift_gaddum_schild");
    let cfg_single = SolverConfig {
        parameter_rules: vec![
            rule_const("LogEC50", -8.0),
            rule_const("SchildSlope", 1.0),
            rule_const("AntagConc", 1e-6),
            rule_local("pA2", Some(7.0), Some(3.0), Some(12.0)),
        ],
        ..SolverConfig::default()
    };
    let single_res = fit_nonlinear(shift_model.as_ref(), &[series("[B]=1uM", log_ag.clone(), gen_curve(1e-6))], &HookContext::default(), &cfg_single)
        .expect("single-concentration Gaddum-Schild");
    assert_converged(single_res.status, "single Gaddum-Schild");
    let pa2_est = find_param(&single_res, 0, "pA2").estimate;
    assert_approx_eq(pa2_est, pa2, 1e-6, 1e-6, "builtin pA2");
    assert_approx_eq(10_f64.powf(-pa2_est), 10_f64.powf(-pa2), 1e-13, 1e-6, "builtin Kb_M = 10^-pA2");

    // Part 2: Simultaneous 4-curve Global Schild regression via CompiledDslModel (<A>.. <D>)
    let dsl_src = "\
<A> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))
<B> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + (1e-7 * 10^pA2)^SchildSlope) - X) * HillSlope))
<C> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + (1e-6 * 10^pA2)^SchildSlope) - X) * HillSlope))
<D> Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 + log10(1 + (1e-5 * 10^pA2)^SchildSlope) - X) * HillSlope))
";
    let dsl = CompiledDslModel::compile("global_gaddum_schild", dsl_src).expect("compile global Gaddum-Schild DSL");
    let y_10u = gen_curve(1e-5);
    let datasets = vec![
        series_idx(0, "Vehicle", log_ag.clone(), gen_curve(0.0)),
        series_idx(1, "100 nM", log_ag.clone(), gen_curve(1e-7)),
        series_idx(2, "1 uM", log_ag.clone(), gen_curve(1e-6)),
        series_idx(3, "10 uM", log_ag, y_10u),
    ];
    let global_rules: Vec<ParameterRule> = dsl
        .param_names()
        .iter()
        .map(|name| {
            let (init, lb, ub) = match name.as_str() {
                "Bottom" => (10.0, None, None),
                "Top" => (100.0, None, None),
                "LogEC50" => (-8.0, None, None),
                "HillSlope" => (1.0, Some(0.1), Some(5.0)),
                "pA2" => (7.0, None, None),
                "SchildSlope" => (1.0, Some(0.1), Some(5.0)),
                other => panic!("unexpected Gaddum-Schild DSL parameter: {other}"),
            };
            rule_shared(name, Some(init), lb, ub)
        })
        .collect();
    let global_cfg = SolverConfig { parameter_rules: global_rules, max_iters: 2000, ..SolverConfig::default() };
    let global_res = fit_nonlinear(&dsl, &datasets, &HookContext::default(), &global_cfg).expect("global Gaddum-Schild fit");
    assert_converged(global_res.status, "global Gaddum-Schild");
    assert_eq!(global_res.k_free, 6, "global Gaddum-Schild has 6 shared free params");
    assert_approx_eq(find_param(&global_res, 0, "pA2").estimate, pa2, 1e-6, 1e-6, "global pA2");
    assert_approx_eq(find_param(&global_res, 0, "SchildSlope").estimate, schild_slope, 1e-6, 1e-6, "global SchildSlope");
    assert_approx_eq(find_param(&global_res, 0, "LogEC50").estimate, log_ec50, 1e-6, 1e-6, "global LogEC50");
    assert_approx_eq(find_param(&global_res, 0, "HillSlope").estimate, hill, 1e-6, 1e-6, "global HillSlope");
}

#[test]
fn pharma_cheng_prusoff_one_site_and_two_sites_ki_oracle() {
    // Cheng & Prusoff (1973): IC50 = Ki * (1 + [Hot] / Kd_hot)
    // [Hot] = 2.4 nM, Kd_hot = 0.8 nM => factor = 4.0 => IC50 = 4 * Ki
    let hot_conc = 2.4_f64;
    let hot_kd = 0.8_f64;
    let true_ki = 5e-9_f64;
    let true_log_ki = true_ki.log10();
    let expected_ic50 = true_ki * (1.0 + hot_conc / hot_kd);
    let log_ic50_true = true_log_ki + (1.0 + hot_conc / hot_kd).log10();

    let log_comp: Vec<f64> = (-22..=-8).map(|i| i as f64 * 0.5).collect();
    let true_top = 1200.0_f64;
    let true_bot = 150.0_f64;
    let y1: Vec<f64> = log_comp.iter().map(|&x| true_bot + (true_top - true_bot) / (1.0 + 10_f64.powf(x - log_ic50_true))).collect();

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Conditions").expect("InfoSheet");
    info.set_constant("HotConc_nM", ConstantValue::Number(hot_conc), None).unwrap();
    info.set_constant("HotKd_nM", ConstantValue::Number(hot_kd), None).unwrap();
    let infos = [info];
    let hooks = HookContext { info_sheets: &infos, ..HookContext::default() };

    let hook_rules = vec![
        ParameterRule {
            name: "HotConc_nM".to_owned(),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: "HotConc_nM".to_owned(),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: "HotKd_nM".to_owned(),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: "HotKd_nM".to_owned(),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];

    let cfg1 = SolverConfig { parameter_rules: hook_rules.clone(), ..SolverConfig::default() };
    let m1 = get_builtin_model("one_site_fit_ki").expect("one_site_fit_ki");
    let res1 = fit_nonlinear(m1.as_ref(), &[series("Competition", log_comp, y1)], &hooks, &cfg1).expect("one_site_fit_ki fit");
    assert_converged(res1.status, "one_site_fit_ki");
    assert_approx_eq(find_param(&res1, 0, "Top").estimate, true_top, 1e-5, 1e-6, "Ki Top");
    assert_approx_eq(find_param(&res1, 0, "Bottom").estimate, true_bot, 1e-5, 1e-6, "Ki Bottom");
    let log_ki_est = find_param(&res1, 0, "LogKi").estimate;
    assert_approx_eq(log_ki_est, true_log_ki, 1e-6, 1e-6, "LogKi");
    let ki_est = 10_f64.powf(log_ki_est);
    let ic50_est = ki_est * (1.0 + hot_conc / hot_kd);
    assert_approx_eq(ki_est, true_ki, 1e-14, 1e-6, "Ki == 10^LogKi");
    assert_approx_eq(ic50_est, expected_ic50, 1e-14, 1e-6, "Cheng-Prusoff IC50 == 4*Ki");

    // Two-site competition binding: FractionHi = 0.40, Ki_1 = 0.5 nM, Ki_2 = 200 nM
    let ki1 = 5e-10_f64;
    let ki2 = 2e-7_f64;
    let log_ki1 = ki1.log10();
    let log_ki2 = ki2.log10();
    let frac_hi = 0.40_f64;
    let shift = (1.0 + hot_conc / hot_kd).log10();
    let log_ic50_1 = log_ki1 + shift;
    let log_ic50_2 = log_ki2 + shift;
    let log_comp_wide: Vec<f64> = (-24..=-6).map(|i| i as f64 * 0.5).collect();
    let y2: Vec<f64> = log_comp_wide
        .iter()
        .map(|&x| {
            let p1 = 1.0 / (1.0 + 10_f64.powf(x - log_ic50_1));
            let p2 = 1.0 / (1.0 + 10_f64.powf(x - log_ic50_2));
            true_bot + (true_top - true_bot) * (frac_hi * p1 + (1.0 - frac_hi) * p2)
        })
        .collect();

    let mut rules2 = hook_rules;
    rules2.extend([
        rule_local("FractionHi", Some(0.5), Some(0.0), Some(1.0)),
        rule_local("LogKi_1", Some(-9.5), None, None),
        rule_local("LogKi_2", Some(-6.5), None, None),
    ]);
    let cfg2 = SolverConfig { parameter_rules: rules2, max_iters: 2000, ..SolverConfig::default() };
    let m2 = get_builtin_model("two_sites_fit_ki").expect("two_sites_fit_ki");
    let res2 = fit_nonlinear(m2.as_ref(), &[series("TwoSites", log_comp_wide, y2)], &hooks, &cfg2).expect("two_sites_fit_ki fit");
    assert_converged(res2.status, "two_sites_fit_ki");
    assert_approx_eq(find_param(&res2, 0, "FractionHi").estimate, frac_hi, 1e-5, 1e-5, "FractionHi");
    assert_approx_eq(find_param(&res2, 0, "LogKi_1").estimate, log_ki1, 1e-5, 1e-5, "LogKi_1");
    assert_approx_eq(find_param(&res2, 0, "LogKi_2").estimate, log_ki2, 1e-5, 1e-5, "LogKi_2");
}

// ============================================================================
// SECTION C: Enzyme Kinetics (Michaelis-Menten, Allosteric, Inhibition Modes)
// ============================================================================

#[test]
fn enzyme_michaelis_menten_and_allosteric_sigmoidal_oracles() {
    let s = vec![0.25, 0.5, 1.0, 2.25, 4.5, 9.0, 18.0, 36.0, 72.0];

    // 1) Michaelis-Menten: V = Vmax * [S] / (Km + [S])
    let vmax = 120.0_f64;
    let km = 4.5_f64;
    let v_mm: Vec<f64> = s.iter().map(|&x| vmax * x / (km + x)).collect();
    let mm = get_builtin_model("michaelis_menten").expect("michaelis_menten");
    let mm_res = fit_nonlinear(mm.as_ref(), &[series("MM", s.clone(), v_mm)], &HookContext::default(), &SolverConfig::default()).expect("MM fit");
    assert_converged(mm_res.status, "Michaelis-Menten");
    assert_approx_eq(find_param(&mm_res, 0, "Vmax").estimate, vmax, 1e-7, 1e-7, "MM Vmax");
    assert_approx_eq(find_param(&mm_res, 0, "Km").estimate, km, 1e-7, 1e-7, "MM Km");
    assert_approx_eq(eval_model_at(mm.as_ref(), &mm_res, 0, km), 0.5 * vmax, 1e-7, 1e-7, "v(Km) == Vmax/2");

    // 2) Allosteric Sigmoidal: V = Vmax * [S]^h / (Khalf^h + [S]^h)
    let allo_vmax = 85.0_f64;
    let khalf = 12.0_f64;
    let h = 2.4_f64;
    let v_allo: Vec<f64> = s.iter().map(|&x| allo_vmax * x.powf(h) / (khalf.powf(h) + x.powf(h))).collect();
    let allo = get_builtin_model("allosteric_sigmoidal_kinetics").expect("allosteric_sigmoidal");
    let allo_res = fit_nonlinear(allo.as_ref(), &[series("Allosteric", s, v_allo)], &HookContext::default(), &SolverConfig::default())
        .expect("Allosteric sigmoidal fit");
    assert_converged(allo_res.status, "Allosteric sigmoidal");
    assert_approx_eq(find_param(&allo_res, 0, "Vmax").estimate, allo_vmax, 1e-6, 1e-6, "Allo Vmax");
    let khalf_est = find_param(&allo_res, 0, "Khalf").estimate;
    let h_est = find_param(&allo_res, 0, "h").estimate;
    assert_approx_eq(khalf_est, khalf, 1e-6, 1e-6, "Allo Khalf");
    assert_approx_eq(h_est, h, 1e-6, 1e-6, "Allo h");
    assert_approx_eq(khalf_est.powf(h_est), khalf.powf(h), 1e-4, 1e-6, "Allo Kprime = Khalf^h");
    assert_approx_eq(eval_model_at(allo.as_ref(), &allo_res, 0, khalf), 0.5 * allo_vmax, 1e-6, 1e-6, "v(Khalf) == Vmax/2");
}

#[test]
fn enzyme_competitive_noncompetitive_uncompetitive_inhibition_oracles() {
    let s = vec![0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let vmax = 100.0_f64;
    let km = 5.0_f64;
    let ki = 3.0_f64;
    let inh_concs = [0.0_f64, 4.0, 12.0];

    let comp_y = |i: f64| -> Vec<f64> { s.iter().map(|&x| vmax * x / (km * (1.0 + i / ki) + x)).collect() };
    let noncomp_y = |i: f64| -> Vec<f64> { s.iter().map(|&x| (vmax / (1.0 + i / ki)) * x / (km + x)).collect() };
    let uncomp_y = |i: f64| -> Vec<f64> { s.iter().map(|&x| (vmax / (1.0 + i / ki)) * x / (km / (1.0 + i / ki) + x)).collect() };

    // Part 1: Built-in single-[I] models with anchored Vmax & Km and constant I = 6.0
    let fixed_i = 6.0;
    for (eq_id, ki_name, y_vec) in [
        ("competitive_inhibition", "Ki", comp_y(fixed_i)),
        ("noncompetitive_inhibition", "Ki", noncomp_y(fixed_i)),
        ("uncompetitive_inhibition", "alphaKi", uncomp_y(fixed_i)),
    ] {
        let model = get_builtin_model(eq_id).unwrap();
        let cfg = SolverConfig {
            parameter_rules: vec![
                rule_const("Vmax", vmax),
                rule_const("Km", km),
                rule_const("I", fixed_i),
                rule_local(ki_name, Some(5.0), Some(1e-6), None),
            ],
            ..SolverConfig::default()
        };
        let res = fit_nonlinear(model.as_ref(), &[series(eq_id, s.clone(), y_vec)], &HookContext::default(), &cfg)
            .unwrap_or_else(|e| panic!("{eq_id} failed: {e}"));
        assert_converged(res.status, eq_id);
        assert_approx_eq(find_param(&res, 0, ki_name).estimate, ki, 1e-6, 1e-6, &format!("{eq_id} {ki_name}"));
    }

    // Part 2: Global multi-[I] fit across [I] = 0, 4, 12 simultaneously recovering Vmax, Km, Ki
    let comp_dsl = CompiledDslModel::compile(
        "global_comp",
        "\
<A> Y = Vmax * X / (Km * (1 + 0 / Ki) + X)
<B> Y = Vmax * X / (Km * (1 + 4 / Ki) + X)
<C> Y = Vmax * X / (Km * (1 + 12 / Ki) + X)
",
    )
    .expect("compile global competitive DSL");

    let noncomp_dsl = CompiledDslModel::compile(
        "global_noncomp",
        "\
<A> Y = (Vmax / (1 + 0 / Ki)) * X / (Km + X)
<B> Y = (Vmax / (1 + 4 / Ki)) * X / (Km + X)
<C> Y = (Vmax / (1 + 12 / Ki)) * X / (Km + X)
",
    )
    .expect("compile global noncompetitive DSL");

    let uncomp_dsl = CompiledDslModel::compile(
        "global_uncomp",
        "\
<A> Y = (Vmax / (1 + 0 / alphaKi)) * X / (Km / (1 + 0 / alphaKi) + X)
<B> Y = (Vmax / (1 + 4 / alphaKi)) * X / (Km / (1 + 4 / alphaKi) + X)
<C> Y = (Vmax / (1 + 12 / alphaKi)) * X / (Km / (1 + 12 / alphaKi) + X)
",
    )
    .expect("compile global uncompetitive DSL");

    let make_global_cfg = |names: &[&str]| SolverConfig {
        parameter_rules: names
            .iter()
            .map(|&n| {
                let init = match n {
                    "Vmax" => 80.0,
                    "Km" | "Ki" | "alphaKi" => 4.0,
                    other => panic!("unexpected enzyme parameter: {other}"),
                };
                rule_shared(n, Some(init), Some(1e-6), None)
            })
            .collect(),
        ..SolverConfig::default()
    };

    let ds_comp: Vec<_> = inh_concs.iter().enumerate().map(|(k, &ci)| series_idx(k, &format!("I={ci}"), s.clone(), comp_y(ci))).collect();
    let res_gc = fit_nonlinear(&comp_dsl, &ds_comp, &HookContext::default(), &make_global_cfg(&["Vmax", "Km", "Ki"])).expect("global comp fit");
    assert_approx_eq(find_param(&res_gc, 0, "Vmax").estimate, vmax, 1e-6, 1e-6, "global comp Vmax");
    assert_approx_eq(find_param(&res_gc, 0, "Km").estimate, km, 1e-6, 1e-6, "global comp Km");
    assert_approx_eq(find_param(&res_gc, 0, "Ki").estimate, ki, 1e-6, 1e-6, "global comp Ki");

    let ds_nc: Vec<_> = inh_concs.iter().enumerate().map(|(k, &ci)| series_idx(k, &format!("I={ci}"), s.clone(), noncomp_y(ci))).collect();
    let res_gnc = fit_nonlinear(&noncomp_dsl, &ds_nc, &HookContext::default(), &make_global_cfg(&["Vmax", "Km", "Ki"])).expect("global noncomp fit");
    assert_approx_eq(find_param(&res_gnc, 0, "Vmax").estimate, vmax, 1e-6, 1e-6, "global noncomp Vmax");
    assert_approx_eq(find_param(&res_gnc, 0, "Km").estimate, km, 1e-6, 1e-6, "global noncomp Km");
    assert_approx_eq(find_param(&res_gnc, 0, "Ki").estimate, ki, 1e-6, 1e-6, "global noncomp Ki");

    let ds_uc: Vec<_> = inh_concs.iter().enumerate().map(|(k, &ci)| series_idx(k, &format!("I={ci}"), s.clone(), uncomp_y(ci))).collect();
    let res_guc =
        fit_nonlinear(&uncomp_dsl, &ds_uc, &HookContext::default(), &make_global_cfg(&["Vmax", "Km", "alphaKi"])).expect("global uncomp fit");
    assert_approx_eq(find_param(&res_guc, 0, "Vmax").estimate, vmax, 1e-6, 1e-6, "global uncomp Vmax");
    assert_approx_eq(find_param(&res_guc, 0, "Km").estimate, km, 1e-6, 1e-6, "global uncomp Km");
    assert_approx_eq(find_param(&res_guc, 0, "alphaKi").estimate, ki, 1e-6, 1e-6, "global uncomp alphaKi");

    // Mechanism discrimination via AICc on slightly noisy competitive dataset
    let ds_comp_noisy: Vec<_> = inh_concs
        .iter()
        .enumerate()
        .map(|(k, &ci)| {
            let y: Vec<f64> = comp_y(ci).into_iter().enumerate().map(|(j, yv)| yv + if (j + k) % 2 == 0 { 0.4 } else { -0.4 }).collect();
            series_idx(k, &format!("I={ci}"), s.clone(), y)
        })
        .collect();
    let cfg_comp = make_global_cfg(&["Vmax", "Km", "Ki"]);
    let fit_c = fit_nonlinear(&comp_dsl, &ds_comp_noisy, &HookContext::default(), &cfg_comp).unwrap();
    let (fit_nc, cmp_vs_nc) = compare_models(
        &fit_c,
        &noncomp_dsl,
        &ds_comp_noisy,
        &HookContext::default(),
        &cfg_comp,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "global_noncomp".to_owned() },
    )
    .expect("AICc comp vs noncomp");
    let ModelComparisonOutcome::Aicc(aicc) = cmp_vs_nc else {
        panic!("expected Aicc outcome");
    };
    assert_eq!(aicc.preferred_index, 0, "competitive data must prefer competitive over noncompetitive");
    assert!(fit_c.rss < fit_nc.rss);
}

// ============================================================================
// SECTION D: PK / Exponential Decay, Binding Kinetics, Growth Models
// ============================================================================

#[test]
fn pk_one_two_three_phase_exponential_decay_oracles() {
    // 1) One-phase decay
    let t1: Vec<f64> = (0..=12).map(|i| i as f64 * 1.5).collect();
    let (y0_1, plat_1, k1) = (100.0_f64, 10.0_f64, 0.25_f64);
    let y1: Vec<f64> = t1.iter().map(|&t| plat_1 + (y0_1 - plat_1) * (-k1 * t).exp()).collect();
    let m1 = get_builtin_model("one_phase_decay").expect("one_phase_decay");
    let res1 =
        fit_nonlinear(m1.as_ref(), &[series("1Phase", t1, y1)], &HookContext::default(), &SolverConfig::default()).expect("one_phase_decay fit");
    assert_converged(res1.status, "one_phase_decay");
    assert_approx_eq(find_param(&res1, 0, "Y0").estimate, y0_1, 1e-6, 1e-6, "1P Y0");
    assert_approx_eq(find_param(&res1, 0, "Plateau").estimate, plat_1, 1e-6, 1e-6, "1P Plateau");
    assert_approx_eq(find_param(&res1, 0, "K").estimate, k1, 1e-7, 1e-7, "1P K");
    assert_approx_eq(find_derived(&res1, 0, "HalfLife"), 2_f64.ln() / k1, 1e-6, 1e-6, "1P HalfLife");
    assert_approx_eq(find_derived(&res1, 0, "Tau"), 1.0 / k1, 1e-6, 1e-6, "1P Tau");
    assert_approx_eq(find_derived(&res1, 0, "Span"), y0_1 - plat_1, 1e-6, 1e-6, "1P Span");

    // 2) Two-phase decay
    let t2 = vec![0.0, 0.2, 0.4, 0.8, 1.2, 1.8, 2.5, 4.0, 6.0, 9.0, 14.0, 20.0, 28.0, 40.0];
    let (y0_2, plat_2, pfast_2, kfast_2, kslow_2) = (100.0_f64, 5.0_f64, 65.0_f64, 1.4_f64, 0.12_f64);
    let span2 = y0_2 - plat_2;
    let y2: Vec<f64> = t2
        .iter()
        .map(|&t| plat_2 + span2 * 0.01 * pfast_2 * (-kfast_2 * t).exp() + span2 * 0.01 * (100.0 - pfast_2) * (-kslow_2 * t).exp())
        .collect();
    let m2 = get_builtin_model("two_phase_decay").expect("two_phase_decay");
    let cfg2 = SolverConfig {
        parameter_rules: vec![
            rule_local("Y0", Some(100.0), None, None),
            rule_local("Plateau", Some(5.0), None, None),
            rule_local("PercentFast", Some(50.0), Some(0.0), Some(100.0)),
            rule_local("KFast", Some(1.0), Some(0.2), None),
            rule_local("KSlow", Some(0.1), Some(1e-4), None),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let res2 = fit_nonlinear(m2.as_ref(), &[series("2Phase", t2, y2)], &HookContext::default(), &cfg2).expect("two_phase_decay fit");
    assert_converged(res2.status, "two_phase_decay");
    assert_approx_eq(find_param(&res2, 0, "Y0").estimate, y0_2, 1e-5, 1e-6, "2P Y0");
    assert_approx_eq(find_param(&res2, 0, "Plateau").estimate, plat_2, 1e-5, 1e-6, "2P Plateau");
    assert_approx_eq(find_param(&res2, 0, "PercentFast").estimate, pfast_2, 1e-4, 1e-6, "2P PercentFast");
    let kfast_est = find_param(&res2, 0, "KFast").estimate;
    let kslow_est = find_param(&res2, 0, "KSlow").estimate;
    assert_approx_eq(kfast_est, kfast_2, 1e-5, 1e-6, "2P KFast");
    assert_approx_eq(kslow_est, kslow_2, 1e-5, 1e-6, "2P KSlow");
    assert_approx_eq(2_f64.ln() / kfast_est, 2_f64.ln() / kfast_2, 1e-5, 1e-6, "2P HalfLifeFast = ln(2)/KFast");
    assert_approx_eq(2_f64.ln() / kslow_est, 2_f64.ln() / kslow_2, 1e-5, 1e-6, "2P HalfLifeSlow = ln(2)/KSlow");

    // 3) Three-phase decay (explicit bounds prevent phase swapping)
    let t3 = vec![0.0, 0.1, 0.2, 0.4, 0.7, 1.0, 1.5, 2.5, 4.0, 6.0, 9.0, 14.0, 22.0, 35.0, 55.0, 80.0, 120.0];
    let (y0_3, plat_3, pf3, ps3, kf3, ks3, kvs3) = (120.0_f64, 4.0_f64, 45.0_f64, 35.0_f64, 2.5_f64, 0.35_f64, 0.03_f64);
    let span3 = y0_3 - plat_3;
    let y3: Vec<f64> = t3
        .iter()
        .map(|&t| {
            let pvs = 100.0 - pf3 - ps3;
            plat_3 + span3 * (0.01 * pf3 * (-kf3 * t).exp() + 0.01 * ps3 * (-ks3 * t).exp() + 0.01 * pvs * (-kvs3 * t).exp())
        })
        .collect();
    let m3 = get_builtin_model("three_phase_decay").expect("three_phase_decay");
    let cfg3 = SolverConfig {
        parameter_rules: vec![
            rule_local("Y0", Some(120.0), None, None),
            rule_local("Plateau", Some(4.0), Some(0.0), None),
            rule_local("PercentFast", Some(40.0), Some(0.0), Some(100.0)),
            rule_local("PercentSlow", Some(35.0), Some(0.0), Some(100.0)),
            rule_local("KFast", Some(2.0), Some(1.0), None),
            rule_local("KSlow", Some(0.3), Some(0.1), Some(1.0)),
            rule_local("KVerySlow", Some(0.04), Some(1e-4), Some(0.1)),
        ],
        max_iters: 3000,
        ..SolverConfig::default()
    };
    let res3 = fit_nonlinear(m3.as_ref(), &[series("3Phase", t3, y3)], &HookContext::default(), &cfg3).expect("three_phase_decay fit");
    assert_converged(res3.status, "three_phase_decay");
    assert_approx_eq(find_param(&res3, 0, "Y0").estimate, y0_3, 1e-4, 1e-5, "3P Y0");
    assert_approx_eq(find_param(&res3, 0, "Plateau").estimate, plat_3, 1e-4, 1e-5, "3P Plateau");
    assert_approx_eq(find_param(&res3, 0, "PercentFast").estimate, pf3, 1e-3, 1e-4, "3P PercentFast");
    assert_approx_eq(find_param(&res3, 0, "PercentSlow").estimate, ps3, 1e-3, 1e-4, "3P PercentSlow");
    assert_approx_eq(find_param(&res3, 0, "KFast").estimate, kf3, 1e-4, 1e-5, "3P KFast");
    assert_approx_eq(find_param(&res3, 0, "KSlow").estimate, ks3, 1e-4, 1e-5, "3P KSlow");
    assert_approx_eq(find_param(&res3, 0, "KVerySlow").estimate, kvs3, 1e-5, 1e-5, "3P KVerySlow");
}

#[test]
fn binding_assoc_then_dissoc_and_growth_gompertz_logistic_oracles() {
    // 1) Association then dissociation (piecewise exact kinetic solution)
    let bmax = 250.0_f64;
    let kon = 4.0e6_f64;
    let koff = 0.08_f64;
    let lig = 2.0e-8_f64;
    let time0 = 30.0_f64;
    let kobs = kon * lig + koff;
    let kd = koff / kon;
    let eq = bmax * lig / (lig + kd);
    let y_wash = eq * (1.0 - (-kobs * time0).exp());

    let t_ad: Vec<f64> = (0..=24).map(|i| i as f64 * 2.5).collect();
    let y_ad: Vec<f64> =
        t_ad.iter().map(|&t| if t < time0 { eq * (1.0 - (-kobs * t).exp()) } else { y_wash * (-koff * (t - time0)).exp() }).collect();

    let ad_model = get_builtin_model("assoc_then_dissoc").expect("assoc_then_dissoc");
    let ad_cfg = SolverConfig {
        parameter_rules: vec![
            rule_local("Bmax", Some(200.0), Some(1.0), None),
            rule_local("Kon", Some(2.0e6), Some(1.0), None),
            rule_local("Koff", Some(0.05), Some(1e-6), None),
            rule_const("LigandConc", lig),
            rule_const("Time0", time0),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let ad_res =
        fit_nonlinear(ad_model.as_ref(), &[series("AssocDissoc", t_ad, y_ad)], &HookContext::default(), &ad_cfg).expect("assoc_then_dissoc fit");
    assert_converged(ad_res.status, "assoc_then_dissoc");
    assert_approx_eq(find_param(&ad_res, 0, "Bmax").estimate, bmax, 1e-4, 1e-6, "AD Bmax");
    let kon_est = find_param(&ad_res, 0, "Kon").estimate;
    let koff_est = find_param(&ad_res, 0, "Koff").estimate;
    assert_lre_at_least(kon_est, kon, 7.0, "AD Kon");
    assert_approx_eq(koff_est, koff, 1e-6, 1e-6, "AD Koff");
    assert_approx_eq(koff_est / kon_est, kd, 1e-13, 1e-6, "AD Kd_M = Koff / Kon");

    // 2) Logistic growth: Y = YM * Y0 / (Y0 + (YM - Y0) * exp(-k * t))
    // Inflection point t_infl = ln((YM - Y0) / Y0) / k where Y(t_infl) == YM / 2
    let t_g: Vec<f64> = (0..=20).map(|i| i as f64).collect();
    let (log_ym, log_y0, log_k) = (500.0_f64, 12.0_f64, 0.42_f64);
    let y_log: Vec<f64> = t_g.iter().map(|&t| log_ym * log_y0 / (log_y0 + (log_ym - log_y0) * (-log_k * t).exp())).collect();
    let m_log = get_builtin_model("logistic_growth").expect("logistic_growth");
    let res_log = fit_nonlinear(m_log.as_ref(), &[series("Logistic", t_g.clone(), y_log)], &HookContext::default(), &SolverConfig::default())
        .expect("logistic_growth fit");
    assert_converged(res_log.status, "logistic_growth");
    assert_approx_eq(find_param(&res_log, 0, "YM").estimate, log_ym, 1e-5, 1e-6, "Logistic YM");
    assert_approx_eq(find_param(&res_log, 0, "Y0").estimate, log_y0, 1e-5, 1e-6, "Logistic Y0");
    assert_approx_eq(find_param(&res_log, 0, "k").estimate, log_k, 1e-6, 1e-6, "Logistic k");
    let t_infl_log = ((log_ym - log_y0) / log_y0).ln() / log_k;
    assert_approx_eq(eval_model_at(m_log.as_ref(), &res_log, 0, t_infl_log), 0.5 * log_ym, 1e-5, 1e-6, "Logistic inflection Y == YM / 2");

    // 3) Gompertz growth: Y = YM * exp(-ln(YM / Y0) * exp(-k * t))
    // Inflection point t_infl = ln(ln(YM / Y0)) / k where Y(t_infl) == YM / e
    let (gom_ym, gom_y0, gom_k) = (400.0_f64, 8.0_f64, 0.28_f64);
    let y_gom: Vec<f64> = t_g.iter().map(|&t| gom_ym * (-(gom_ym / gom_y0).ln() * (-gom_k * t).exp()).exp()).collect();
    let m_gom = get_builtin_model("gompertz_growth").expect("gompertz_growth");
    let res_gom = fit_nonlinear(m_gom.as_ref(), &[series("Gompertz", t_g, y_gom)], &HookContext::default(), &SolverConfig::default())
        .expect("gompertz_growth fit");
    assert_converged(res_gom.status, "gompertz_growth");
    assert_approx_eq(find_param(&res_gom, 0, "YM").estimate, gom_ym, 1e-5, 1e-6, "Gompertz YM");
    assert_approx_eq(find_param(&res_gom, 0, "Y0").estimate, gom_y0, 1e-5, 1e-6, "Gompertz Y0");
    assert_approx_eq(find_param(&res_gom, 0, "k").estimate, gom_k, 1e-6, 1e-6, "Gompertz k");
    let t_infl_gom = ((gom_ym / gom_y0).ln()).ln() / gom_k;
    assert_approx_eq(
        eval_model_at(m_gom.as_ref(), &res_gom, 0, t_infl_gom),
        gom_ym / std::f64::consts::E,
        1e-5,
        1e-6,
        "Gompertz inflection Y == YM / e",
    );
}

// ============================================================================
// SECTION E: Shared Global, Constraints & Hooks, Profile CI, F/AICc, Deming/WLS/ROUT
// ============================================================================

#[test]
fn global_shared_parameters_multi_dataset_oracle() {
    let t: Vec<f64> = (0..=10).map(|i| i as f64 * 1.5).collect();
    let k_shared = 0.32_f64;
    let plat_shared = 7.5_f64;
    let y0s = [130.0_f64, 85.0_f64, 48.0_f64];
    let noise_offsets = [0.4, -0.5, 0.3, -0.2, 0.4, -0.3, 0.2, -0.4, 0.3, -0.2, 0.0];

    let curves: Vec<DatasetSeries> = y0s
        .iter()
        .enumerate()
        .map(|(ds_i, &y0)| {
            let y: Vec<f64> = t
                .iter()
                .zip(noise_offsets.iter())
                .map(|(&ti, &e)| plat_shared + (y0 - plat_shared) * (-k_shared * ti).exp() + if ds_i % 2 == 0 { e } else { -e })
                .collect();
            series_idx(ds_i, &format!("Curve{ds_i}"), t.clone(), y)
        })
        .collect();

    let model = get_builtin_model("one_phase_decay").expect("one_phase_decay");
    let cfg = SolverConfig {
        parameter_rules: vec![rule_local("Y0", None, None, None), rule_shared("Plateau", None, None, None), rule_shared("K", None, Some(1e-5), None)],
        ..SolverConfig::default()
    };

    let res = fit_nonlinear(model.as_ref(), &curves, &HookContext::default(), &cfg).expect("shared global fit");
    assert_converged(res.status, "global shared one_phase_decay");
    assert_eq!(res.k_free, 5);
    assert_eq!(res.n_obs, 33);
    assert_eq!(res.df, 28);

    let k0 = find_param(&res, 0, "K");
    let p0 = find_param(&res, 0, "Plateau");
    assert!(k0.notes.contains(&ParameterNote::SharedGlobal));
    assert!(p0.notes.contains(&ParameterNote::SharedGlobal));
    for ds_i in 0..3 {
        assert_approx_eq(find_ds_param(&res, ds_i, "K"), k0.estimate, 1e-12, 1e-12, "shared K identical across dataset param vectors");
        assert_approx_eq(find_ds_param(&res, ds_i, "Plateau"), p0.estimate, 1e-12, 1e-12, "shared Plateau identical across dataset param vectors");
    }
    assert!((k0.estimate - k_shared).abs() <= 2.5 * k0.se);
    assert!((p0.estimate - plat_shared).abs() <= 2.5 * p0.se);
    for (ds_i, &y0_true) in y0s.iter().enumerate() {
        let y0_p = find_param(&res, ds_i, "Y0");
        assert!(!y0_p.notes.contains(&ParameterNote::SharedGlobal));
        assert!((y0_p.estimate - y0_true).abs() <= 2.5 * y0_p.se);
    }
}

#[test]
fn box_constraints_and_hooked_constants_end_to_end_oracle() {
    // 1) Box constraint hitting LowerBound and UpperBound
    let t: Vec<f64> = (0..=8).map(|i| i as f64 * 1.5).collect();
    let y_neg_plat: Vec<f64> = t.iter().map(|&ti| -12.0 + 112.0 * (-0.2 * ti).exp()).collect();
    let m1 = get_builtin_model("one_phase_decay").expect("one_phase_decay");
    let res_lb = fit_nonlinear(
        m1.as_ref(),
        &[series("LowerBound", t.clone(), y_neg_plat)],
        &HookContext::default(),
        &SolverConfig { parameter_rules: vec![rule_local("Plateau", Some(5.0), Some(0.0), None)], ..SolverConfig::default() },
    )
    .expect("lower-bound constrained fit");
    let plat_lb = find_param(&res_lb, 0, "Plateau");
    assert_approx_eq(plat_lb.estimate, 0.0, 1e-10, 1e-10, "HitLowerBound clamps Plateau at 0.0");
    assert!(plat_lb.notes.contains(&ParameterNote::HitLowerBound));

    let y_fast_k: Vec<f64> = t.iter().map(|&ti| 10.0 + 90.0 * (-0.35 * ti).exp()).collect();
    let res_ub = fit_nonlinear(
        m1.as_ref(),
        &[series("UpperBound", t, y_fast_k)],
        &HookContext::default(),
        &SolverConfig { parameter_rules: vec![rule_local("K", Some(0.1), Some(1e-4), Some(0.15))], ..SolverConfig::default() },
    )
    .expect("upper-bound constrained fit");
    let k_ub = find_param(&res_ub, 0, "K");
    assert_approx_eq(k_ub.estimate, 0.15, 1e-10, 1e-10, "HitUpperBound clamps K at 0.15");
    assert!(k_ub.notes.contains(&ParameterNote::HitUpperBound));

    // 2) End-to-end `execute_nonlinear_analysis` verifying ALL 3 `HookSource` variants:
    //    - InfoConstant (`HotConc_nM`, `HotKd_nM`)
    //    - ResultScalar (`Bottom` pulled from an upstream baseline analysis ResultSheet)
    //    - ColumnTitleValue (`I` parsed from leading number of DataSheet column title `"25.0 uM"`)
    let log_x: Vec<f64> = (-20..=-8).map(|i| i as f64 * 0.5).collect();
    let y_baseline: Vec<f64> = log_x.iter().map(|&x| 42.0 + (220.0 - 42.0) / (1.0 + 10_f64.powf((-7.0 - x) * 1.0))).collect();
    let baseline_sheet = build_xy_sheet("sheet-base", "Baseline 4PL", &log_x, &[("Agonist", &y_baseline)]).expect("baseline_sheet");
    let base_nl = NonlinearFitSpec {
        equation_id: "log_agonist_vs_response_4pl".to_owned(),
        custom_dsl: None,
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::None,
        parameter_rules: vec![],
        profile_likelihood_ci: false,
        comparison: None,
        confidence_bands: false,
        prediction_bands: false,
        interpolate_unknowns: false,
        replicates_lack_of_fit_test: false,
        normality_of_residuals: false,
        max_iters: 1000,
        ..NonlinearFitSpec::new("log_agonist_vs_response_4pl")
    };
    let baseline_analysis_spec =
        AnalysisSpec::new(SheetId::new("ana-base"), baseline_sheet.id.clone(), AnalysisKind::NonlinearRegression(base_nl.clone()))
            .expect("AnalysisSpec::new");
    let baseline_result: ResultSheet =
        execute_nonlinear_analysis(&baseline_analysis_spec, &base_nl, &baseline_sheet, &HookContext::default()).expect("upstream ResultSheet fit");

    let true_ki = 1e-8_f64;
    let true_log_ic50 = true_ki.log10() + (1.0 + 3.0 / 1.5_f64).log10();
    let y_comp: Vec<f64> = log_x.iter().map(|&x| 42.0 + (500.0 - 42.0) / (1.0 + 10_f64.powf(x - true_log_ic50))).collect();
    let comp_sheet = build_xy_sheet("sheet-comp", "Competition", &log_x, &[("Radioligand", &y_comp)]).expect("comp_sheet");

    let mut info = InfoSheet::new_preseeded("info-assay", "Assay Info").expect("InfoSheet");
    info.set_constant("HotConc_nM", ConstantValue::Number(3.0), None).unwrap();
    info.set_constant("HotKd_nM", ConstantValue::Number(1.5), None).unwrap();

    let s_vals = vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let i_col = 25.0_f64;
    let ki_enz = 5.0_f64;
    let v_enz: Vec<f64> = s_vals.iter().map(|&sv| 100.0 * sv / (4.0 * (1.0 + i_col / ki_enz) + sv)).collect();
    let enz_sheet = build_xy_sheet("sheet-enz", "Enzyme", &s_vals, &[("25.0 uM", &v_enz)]).expect("enz_sheet");

    let infos = [info];
    let results = [baseline_result];
    let sheets = [enz_sheet.clone()];
    let hooks = HookContext { info_sheets: &infos, result_sheets: &results, data_sheets: &sheets, ..HookContext::default() };

    let ki_nl = NonlinearFitSpec {
        equation_id: "one_site_fit_ki".to_owned(),
        custom_dsl: None,
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::None,
        parameter_rules: vec![
            ParameterRule {
                name: "HotConc_nM".to_owned(),
                scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                    info_sheet_id: Some(SheetId::new("info-assay")),
                    constant_name: "HotConc_nM".to_owned(),
                }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "HotKd_nM".to_owned(),
                scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                    info_sheet_id: Some(SheetId::new("info-assay")),
                    constant_name: "HotKd_nM".to_owned(),
                }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "Bottom".to_owned(),
                scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
                    result_sheet_id: SheetId::new("ana-base"),
                    parameter_key: "Bottom".to_owned(),
                    data_set_index: Some(0),
                }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        profile_likelihood_ci: false,
        comparison: None,
        confidence_bands: false,
        prediction_bands: false,
        interpolate_unknowns: false,
        replicates_lack_of_fit_test: false,
        normality_of_residuals: false,
        max_iters: 1000,
        ..NonlinearFitSpec::new("one_site_fit_ki")
    };
    let ki_spec = AnalysisSpec::new(SheetId::new("ana-ki"), comp_sheet.id.clone(), AnalysisKind::NonlinearRegression(ki_nl.clone()))
        .expect("AnalysisSpec::new");
    let ki_result = execute_nonlinear_analysis(&ki_spec, &ki_nl, &comp_sheet, &hooks).expect("hooked Ki ResultSheet");
    let log_ki_cell = ki_result.lookup_scalar("LogKi", Some(0)).expect("LogKi scalar in ResultSheet");
    assert_approx_eq(log_ki_cell, true_ki.log10(), 1e-5, 1e-5, "ResultSheet hooked LogKi");
    let bot_cell = ki_result.lookup_scalar("Bottom", None).expect("Bottom scalar in ResultSheet");
    assert_approx_eq(bot_cell, 42.0, 1e-6, 1e-6, "ResultScalar-hooked Bottom == 42.0");

    // ColumnTitleValue hook (`competitive_inhibition` reading [I] = 25.0 from `"25.0 uM"`)
    let enz_nl = NonlinearFitSpec {
        equation_id: "competitive_inhibition".to_owned(),
        custom_dsl: None,
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::None,
        parameter_rules: vec![
            rule_const("Vmax", 100.0),
            rule_const("Km", 4.0),
            ParameterRule {
                name: "I".to_owned(),
                scope: ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("sheet-enz"), data_set_index: 0 }),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        profile_likelihood_ci: false,
        comparison: None,
        confidence_bands: false,
        prediction_bands: false,
        interpolate_unknowns: false,
        replicates_lack_of_fit_test: false,
        normality_of_residuals: false,
        max_iters: 1000,
        ..NonlinearFitSpec::new("competitive_inhibition")
    };
    let enz_spec = AnalysisSpec::new(SheetId::new("ana-enz"), enz_sheet.id.clone(), AnalysisKind::NonlinearRegression(enz_nl.clone()))
        .expect("AnalysisSpec::new");
    let enz_result = execute_nonlinear_analysis(&enz_spec, &enz_nl, &enz_sheet, &hooks).expect("ColumnTitleValue fit");
    assert_approx_eq(enz_result.lookup_scalar("Ki", Some(0)).expect("Ki scalar"), ki_enz, 1e-6, 1e-6, "ColumnTitleValue hooked Ki");
}

#[test]
fn profile_likelihood_vs_wald_asymmetric_ci_and_open_bound_oracle() {
    // Venzon & Moolgavkar (1988) profile-likelihood CI vs symmetric Wald CI on Michaelis-Menten
    let s = vec![1.0, 2.0, 4.0, 6.0, 10.0, 15.0, 25.0, 40.0, 70.0, 100.0];
    let noise = [3.2, -2.8, 4.1, -3.5, 2.7, -4.4, 3.8, -3.0, 2.5, -2.6];
    let y: Vec<f64> = s.iter().zip(noise.iter()).map(|(&sv, &e)| 120.0 * sv / (12.0 + sv) + e).collect();

    let model = get_builtin_model("michaelis_menten").expect("michaelis_menten");
    let cfg = SolverConfig { compute_profile_ci: true, ..SolverConfig::default() };
    let res = fit_nonlinear(model.as_ref(), &[series("MM CI", s.clone(), y.clone())], &HookContext::default(), &cfg).expect("MM profile CI fit");
    assert_converged(res.status, "MM profile CI");

    let km = find_param(&res, 0, "Km");
    let half_sym_lo = km.estimate - km.symmetric_ci_lower;
    let half_sym_hi = km.symmetric_ci_upper - km.estimate;
    assert_approx_eq(half_sym_lo, half_sym_hi, 1e-10, 1e-10, "Wald CI must be symmetric");

    let (CiBound::Finite(p_lo), CiBound::Finite(p_hi)) = (km.ci_lower, km.ci_upper) else {
        panic!("expected finite profile CI for well-determined Km, got {:?}..{:?}", km.ci_lower, km.ci_upper);
    };
    let half_prof_lo = km.estimate - p_lo;
    let half_prof_hi = p_hi - km.estimate;
    assert!(half_prof_hi > half_prof_lo + 0.05, "Km profile CI must be noticeably right-skewed: lower={half_prof_lo:.4}, upper={half_prof_hi:.4}");
    assert!((half_prof_hi - half_sym_hi).abs() > 0.05, "profile upper bound must differ meaningfully from symmetric Wald");

    // Exact Venzon-Moolgavkar critical SS ratio at both profile endpoints:
    // t_{df=8, 0.975} = 2.3060041350333704
    let df = res.df as f64;
    assert_eq!(df, 8.0);
    let t_crit_8 = 2.3060041350333704_f64;
    let expected_ss_crit = res.rss * (1.0 + (t_crit_8 * t_crit_8) / df);
    let profile_rss_at_km = |km_test: f64| -> f64 {
        let f: Vec<f64> = s.iter().map(|&sv| sv / (km_test + sv)).collect();
        let num: f64 = y.iter().zip(f.iter()).map(|(&yi, &fi)| yi * fi).sum();
        let den: f64 = f.iter().map(|&fi| fi * fi).sum();
        let vmax_star = num / den;
        y.iter()
            .zip(f.iter())
            .map(|(&yi, &fi)| {
                let r = yi - vmax_star * fi;
                r * r
            })
            .sum()
    };
    assert_lre_at_least(profile_rss_at_km(p_lo), expected_ss_crit, 4.0, "profile RSS at lower endpoint == Venzon-Moolgavkar SS_crit");
    assert_lre_at_least(profile_rss_at_km(p_hi), expected_ss_crit, 4.0, "profile RSS at upper endpoint == Venzon-Moolgavkar SS_crit");

    // 2) Incomplete saturation plateau ([S] < Km in Michaelis-Menten) produces CiBound::Open / UnboundedCi
    let s_partial = vec![0.3, 0.7, 1.2, 1.9, 2.8, 3.8];
    let noise_partial = [-1.8, 2.2, -2.4, 2.6, -2.0, -0.4];
    let v_partial: Vec<f64> = s_partial.iter().zip(noise_partial.iter()).map(|(&x, &eps)| 120.0 * x / (12.0 + x) + eps).collect();
    let mm_open = get_builtin_model("michaelis_menten").expect("michaelis_menten");
    let res_open = fit_nonlinear(
        mm_open.as_ref(),
        &[series("IncompleteMM", s_partial, v_partial)],
        &HookContext::default(),
        &SolverConfig {
            compute_profile_ci: true,
            identify_unstable_parameters: true,
            parameter_rules: vec![rule_local("Vmax", Some(120.0), Some(0.0), None), rule_local("Km", Some(12.0), Some(0.0), None)],
            ..SolverConfig::default()
        },
    )
    .expect("partial curve profile CI");
    let vmax_p = find_param(&res_open, 0, "Vmax");
    assert!(res_open.identifiability.is_ambiguous());
    assert!(vmax_p.notes.contains(&ParameterNote::Unstable));
    let is_open =
        matches!(vmax_p.ci_upper, CiBound::Open) || matches!(vmax_p.ci_lower, CiBound::Open) || vmax_p.notes.contains(&ParameterNote::UnboundedCi);
    assert!(
        is_open,
        "incomplete saturation curve must flag Open/Unbounded profile CI for Vmax, got {:?}..{:?}, notes={:?}",
        vmax_p.ci_lower, vmax_p.ci_upper, vmax_p.notes
    );
}

#[test]
fn model_comparison_extra_f_test_and_aicc_oracles() {
    let t = vec![0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0, 8.0, 12.0, 18.0, 26.0, 36.0];
    let noise = [0.5, -0.4, 0.6, -0.5, 0.3, -0.6, 0.4, -0.3, 0.5, -0.4, 0.2, -0.3, 0.2];
    let m1 = get_builtin_model("one_phase_decay").expect("1P");
    let m2 = get_builtin_model("two_phase_decay").expect("2P");

    // Case 1: True mono-exponential data + scatter -> simpler 1-phase wins F-test
    let y_mono: Vec<f64> = t.iter().zip(noise.iter()).map(|(&ti, &e)| 8.0 + 92.0 * (-0.25_f64 * ti).exp() + e).collect();
    let ds_mono = series("Mono", t.clone(), y_mono);
    let fit_m1_mono =
        fit_nonlinear(m1.as_ref(), std::slice::from_ref(&ds_mono), &HookContext::default(), &SolverConfig::default()).expect("1P fit on mono");
    let (_p1, cmp_mono) = compare_models(
        &fit_m1_mono,
        m2.as_ref(),
        &[ds_mono],
        &HookContext::default(),
        &SolverConfig::default(),
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() },
    )
    .expect("F-test on mono");
    let ModelComparisonOutcome::ExtraSumOfSquaresF(f_mono) = cmp_mono else {
        panic!("expected ExtraSumOfSquaresF");
    };
    assert!(
        !f_mono.preferred_complex_model && f_mono.p_value > 0.05,
        "mono-exponential data must prefer simpler 1-phase model (p={})",
        f_mono.p_value
    );

    // Case 2: True bi-exponential data -> extra-SS F-test strongly prefers 2-phase
    let y_bi: Vec<f64> = t.iter().zip(noise.iter()).map(|(&ti, &e)| 5.0 + 55.0 * (-2.0_f64 * ti).exp() + 45.0 * (-0.1_f64 * ti).exp() + e).collect();
    let ds_bi = series("BiExp", t, y_bi);
    let fit_m1_bi =
        fit_nonlinear(m1.as_ref(), std::slice::from_ref(&ds_bi), &HookContext::default(), &SolverConfig::default()).expect("1P fit on bi");
    let cfg_2p = SolverConfig {
        parameter_rules: vec![
            rule_local("PercentFast", Some(55.0), Some(0.0), Some(100.0)),
            rule_local("KFast", Some(1.5), Some(0.2), None),
            rule_local("KSlow", Some(0.1), Some(1e-4), None),
        ],
        max_iters: 2000,
        ..SolverConfig::default()
    };
    let (_p2, cmp_bi) = compare_models(
        &fit_m1_bi,
        m2.as_ref(),
        &[ds_bi],
        &HookContext::default(),
        &cfg_2p,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() },
    )
    .expect("F-test on bi");
    let ModelComparisonOutcome::ExtraSumOfSquaresF(f_bi) = cmp_bi else {
        panic!("expected ExtraSumOfSquaresF");
    };
    assert!(f_bi.preferred_complex_model && f_bi.p_value < 1e-4);

    // Exact closed-form verification of F-test & AICc formulas (Burnham & Anderson 2002)
    let f_direct = compare_models_f_test(120.0, 20, 40.0, 18, 0.05).expect("direct F test");
    assert_approx_eq(f_direct.f_stat, 18.0, 1e-12, 1e-12, "exact F-stat");
    assert_eq!(f_direct.df_num, 2);
    assert_eq!(f_direct.df_den, 18);
    assert!(f_direct.p_value < 1e-4 && f_direct.preferred_complex_model);

    let aicc_direct = compare_models_aicc(30.0, 3, 26.0, 5, 15).expect("direct AICc");
    let aicc_a_ref = 15.0 * (30.0 / 15.0_f64).ln() + 8.0 + (2.0 * 4.0 * 5.0) / (15.0 - 4.0 - 1.0);
    let aicc_b_ref = 15.0 * (26.0 / 15.0_f64).ln() + 12.0 + (2.0 * 6.0 * 7.0) / (15.0 - 6.0 - 1.0);
    assert_approx_eq(aicc_direct.aicc_a, aicc_a_ref, 1e-12, 1e-12, "exact AICc_A");
    assert_approx_eq(aicc_direct.aicc_b, aicc_b_ref, 1e-12, 1e-12, "exact AICc_B");
    assert_approx_eq(aicc_direct.weight_a + aicc_direct.weight_b, 1.0, 1e-12, 1e-12, "Akaike weights sum to 1");
    let expected_er = (0.5 * (aicc_direct.aicc_a - aicc_direct.aicc_b).abs()).exp();
    assert_approx_eq(aicc_direct.evidence_ratio, expected_er, 1e-12, 1e-12, "AICc evidence ratio = exp(|dAICc|/2)");
    assert_eq!(aicc_direct.preferred_index, 0, "model A should win when B adds 2 params for small SS drop");

    // Integrated non-nested AICc comparison (`allosteric_sigmoidal_kinetics` vs `michaelis_menten`)
    let s_vals = vec![0.5_f64, 1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 14.0, 20.0, 30.0, 50.0, 80.0];
    let y_hill: Vec<f64> = s_vals
        .iter()
        .enumerate()
        .map(|(i, &sv)| 100.0 * sv.powf(2.5) / (8.0_f64.powf(2.5) + sv.powf(2.5)) + if i % 2 == 0 { 0.3 } else { -0.3 })
        .collect();
    let ds_hill = series("Hill", s_vals, y_hill);
    let mm = get_builtin_model("michaelis_menten").expect("MM");
    let allo = get_builtin_model("allosteric_sigmoidal_kinetics").expect("Allo");
    let fit_mm = fit_nonlinear(mm.as_ref(), std::slice::from_ref(&ds_hill), &HookContext::default(), &SolverConfig::default()).expect("MM on Hill");
    let (_p_aicc, cmp_aicc) = compare_models(
        &fit_mm,
        allo.as_ref(),
        &[ds_hill],
        &HookContext::default(),
        &SolverConfig::default(),
        &ModelComparisonSpec::Aicc { alternative_equation_id: "allosteric_sigmoidal_kinetics".to_owned() },
    )
    .expect("AICc MM vs Allosteric");
    let ModelComparisonOutcome::Aicc(cmp_h) = cmp_aicc else {
        panic!("expected Aicc");
    };
    assert_eq!(cmp_h.preferred_index, 1, "Hill sigmoidal data must prefer allosteric over MM");
}

#[test]
fn deming_model_ii_and_weighting_and_poisson_and_robust_oracles() {
    // 1) Deming (1943) Orthogonal Model II regression closed-form & attenuation-bias oracle
    // Symmetric orthogonal cross perturbations around y = 1 + 2x make Deming(lambda=1) exact (2.0) while OLS attenuates (< 1.95).
    let d = 1.5_f64;
    let mut dem_pts = Vec::with_capacity(16);
    for xk in [-3.0_f64, -1.0, 1.0, 3.0] {
        let yk = 1.0 + 2.0 * xk;
        dem_pts.push(DataPoint::xy(xk + d, yk));
        dem_pts.push(DataPoint::xy(xk - d, yk));
        dem_pts.push(DataPoint::xy(xk, yk + d));
        dem_pts.push(DataPoint::xy(xk, yk - d));
    }

    let opts = DemingOptions { variance_ratio_lambda: 1.0, ..DemingOptions::default() };
    let dem = deming_regression(&dem_pts, &opts).expect("orthogonal Deming");

    let n = dem_pts.len() as f64;
    let x_bar = dem_pts.iter().map(|p| p.x[0]).sum::<f64>() / n;
    let y_bar = dem_pts.iter().map(|p| p.y).sum::<f64>() / n;
    let s_xx: f64 = dem_pts.iter().map(|p| (p.x[0] - x_bar).powi(2)).sum();
    let s_yy: f64 = dem_pts.iter().map(|p| (p.y - y_bar).powi(2)).sum();
    let s_xy: f64 = dem_pts.iter().map(|p| (p.x[0] - x_bar) * (p.y - y_bar)).sum();
    let expected_slope = ((s_yy - s_xx) + ((s_yy - s_xx).powi(2) + 4.0 * s_xy * s_xy).sqrt()) / (2.0 * s_xy);
    let expected_intercept = y_bar - expected_slope * x_bar;
    assert_approx_eq(dem.slope, expected_slope, 1e-12, 1e-12, "exact closed-form Deming slope");
    assert_approx_eq(dem.intercept, expected_intercept, 1e-12, 1e-12, "exact closed-form Deming intercept");

    let ols = simple_linear_regression(&dem_pts, &Default::default()).expect("OLS");
    assert!(
        ols.slope < 1.95 && (dem.slope - 2.0).abs() < (ols.slope - 2.0).abs(),
        "Deming ({:.4}) must avoid OLS attenuation ({:.4}) relative to truth 2.0",
        dem.slope,
        ols.slope
    );
    assert!(dem.se_slope > 0.0 && dem.ci_slope.0 < dem.slope && dem.slope < dem.ci_slope.1);

    let dem_x: Vec<f64> = dem_pts.iter().map(|p| p.x[0]).collect();
    let dem_y: Vec<f64> = dem_pts.iter().map(|p| p.y).collect();
    let dem_sheet = build_xy_sheet("sheet-dem", "Deming", &dem_x, &[("Y", &dem_y)]).expect("dem_sheet");
    let lin_spec = LinearRegressionSpec {
        deming_model_ii: true,
        force_through_origin: false,
        confidence_bands: false,
        prediction_bands: false,
        ..LinearRegressionSpec::default()
    };
    let dem_ana =
        AnalysisSpec::new(SheetId::new("ana-dem"), dem_sheet.id.clone(), AnalysisKind::LinearRegression(lin_spec)).expect("AnalysisSpec::new");
    let dem_rs = execute_linear_analysis(&dem_ana, &lin_spec, &dem_sheet).expect("Deming ResultSheet");
    assert_approx_eq(
        dem_rs.lookup_scalar("Slope", Some(0)).expect("Deming ResultSheet Slope"),
        dem.slope,
        1e-10,
        1e-10,
        "execute_linear_analysis Deming slope matches direct",
    );

    // 2) Weighted Least Squares: 1/Y^2, 1/|Y|, 1/|X|, 1/X^2, and ObservedSd (MeanSdN XyTable)
    let c_vals = vec![0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0];
    let rel_err = [0.05, -0.05, 0.05, -0.05, 0.05, -0.05, 0.05, -0.05, 0.05, -0.05];
    let (true_bmax, true_kd) = (220.0_f64, 6.0_f64);
    let y_cv: Vec<f64> = c_vals.iter().zip(rel_err.iter()).map(|(&c, &r)| (true_bmax * c / (true_kd + c)) * (1.0 + r)).collect();
    let bind_model = get_builtin_model("one_site_specific_binding").expect("binding");
    let ds_cv = series("ConstCV", c_vals.clone(), y_cv);

    let res_unw = fit_nonlinear(bind_model.as_ref(), std::slice::from_ref(&ds_cv), &HookContext::default(), &SolverConfig::default())
        .expect("unweighted binding");
    let res_w_y2 = fit_nonlinear(
        bind_model.as_ref(),
        std::slice::from_ref(&ds_cv),
        &HookContext::default(),
        &SolverConfig { weighting: WeightingScheme::RelativeSquare, ..SolverConfig::default() },
    )
    .expect("1/Y^2 weighted binding");
    let kd_unw = find_param(&res_unw, 0, "Kd").estimate;
    let kd_w = find_param(&res_w_y2, 0, "Kd").estimate;
    assert!((kd_w - true_kd).abs() < (kd_unw - true_kd).abs(), "1/Y^2 Kd ({kd_w:.4}) must be closer to {true_kd} than unweighted Kd ({kd_unw:.4})");
    for scheme in [WeightingScheme::Relative, WeightingScheme::InverseX, WeightingScheme::InverseX2] {
        let w_res = fit_nonlinear(
            bind_model.as_ref(),
            std::slice::from_ref(&ds_cv),
            &HookContext::default(),
            &SolverConfig { weighting: scheme, ..SolverConfig::default() },
        )
        .unwrap_or_else(|e| panic!("weighting {scheme:?} failed: {e}"));
        assert_converged(w_res.status, &format!("{scheme:?}"));
    }

    // ObservedSd weighting from MeanSdN XyTable
    let mut xy_msd = XyTable::new(SubcolumnMode::MeanSdN, 1, c_vals.len()).expect("XyTable");
    xy_msd.data_sets[0].title = "PreSummarized".to_owned();
    let mut sheet_msd = DataSheet::new("sheet-msd", "MeanSdN", TableArchetype::Xy(xy_msd)).expect("DataSheet");
    for (r, &c) in c_vals.iter().enumerate() {
        sheet_msd.set_x_or_time_cell(r, 0, Cell::number(c)).unwrap();
        let mean_y = true_bmax * c / (true_kd + c) + rel_err[r] * (true_bmax * c / (true_kd + c));
        let sd_y = 0.05 * (true_bmax * c / (true_kd + c));
        sheet_msd.set_cell(CellCoord::new(r, 0, 0), Cell::number(mean_y)).unwrap();
        sheet_msd.set_cell(CellCoord::new(r, 0, 1), Cell::number(sd_y)).unwrap();
        sheet_msd.set_cell(CellCoord::new(r, 0, 2), Cell::number(3.0)).unwrap();
    }
    let msd_nl = NonlinearFitSpec {
        equation_id: "one_site_specific_binding".to_owned(),
        custom_dsl: None,
        method: FitMethod::LeastSquares,
        weighting: WeightingScheme::ObservedSd,
        parameter_rules: vec![],
        profile_likelihood_ci: false,
        comparison: None,
        confidence_bands: false,
        prediction_bands: false,
        interpolate_unknowns: false,
        replicates_lack_of_fit_test: false,
        normality_of_residuals: false,
        max_iters: 1000,
        ..NonlinearFitSpec::new("one_site_specific_binding")
    };
    let msd_spec = AnalysisSpec::new(SheetId::new("ana-msd"), sheet_msd.id.clone(), AnalysisKind::NonlinearRegression(msd_nl.clone()))
        .expect("AnalysisSpec::new");
    let msd_rs = execute_nonlinear_analysis(&msd_spec, &msd_nl, &sheet_msd, &HookContext::default()).expect("ObservedSd MeanSdN fit");
    let kd_msd = msd_rs.lookup_scalar("Kd", Some(0)).expect("ObservedSd Kd");
    assert!((kd_msd - true_kd).abs() < 0.2, "ObservedSd Kd ({kd_msd:.4}) should closely recover {true_kd}");

    // 3) Poisson Deviance regression on count decay curve
    let t_p: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let y_counts: Vec<f64> = t_p.iter().map(|&ti| (20.0 + 500.0 * (-0.3_f64 * ti).exp()).round()).collect();
    let dec_m = get_builtin_model("one_phase_decay").expect("decay");
    let res_pois = fit_nonlinear(
        dec_m.as_ref(),
        &[series("Counts", t_p.clone(), y_counts.clone())],
        &HookContext::default(),
        &SolverConfig { method: FitMethod::PoissonDeviance, ..SolverConfig::default() },
    )
    .expect("PoissonDeviance fit");
    assert_converged(res_pois.status, "PoissonDeviance");
    assert!((find_param(&res_pois, 0, "K").estimate - 0.3).abs() < 0.02);
    let expected_dev: f64 = t_p
        .iter()
        .zip(y_counts.iter())
        .map(|(&ti, &yi)| {
            let mu = eval_model_at(dec_m.as_ref(), &res_pois, 0, ti);
            2.0 * (yi * (yi / mu).ln() - (yi - mu))
        })
        .sum();
    assert_approx_eq(res_pois.weighted_ss, expected_dev, 1e-5, 1e-6, "PoissonDeviance objective == 2*sum(y*ln(y/mu)-(y-mu))");

    // 4) Robust Cauchy regression resisting a severe outlier spike (+80 on lower limb)
    let log_d: Vec<f64> = (-20..=-8).map(|i| i as f64 * 0.5).collect();
    let mut y_rob: Vec<f64> = log_d.iter().map(|&x| 10.0 + (100.0 - 10.0) / (1.0 + 10_f64.powf((-7.0 - x) * 1.2))).collect();
    y_rob[3] += 80.0;
    let ag_m = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL");
    let ds_rob = series("Outlier", log_d, y_rob);
    let fit_ols =
        fit_nonlinear(ag_m.as_ref(), std::slice::from_ref(&ds_rob), &HookContext::default(), &SolverConfig::default()).expect("OLS on outlier");
    let fit_cauchy = fit_nonlinear(
        ag_m.as_ref(),
        &[ds_rob],
        &HookContext::default(),
        &SolverConfig { method: FitMethod::RobustCauchy, ..SolverConfig::default() },
    )
    .expect("RobustCauchy on outlier");
    let ec_ols = find_param(&fit_ols, 0, "LogEC50").estimate;
    let ec_rob = find_param(&fit_cauchy, 0, "LogEC50").estimate;
    assert!(
        (ec_rob - (-7.0)).abs() < 0.05 && (ec_rob - (-7.0)).abs() < (ec_ols - (-7.0)).abs(),
        "RobustCauchy LogEC50 ({ec_rob:.4}) must stay near -7.0 while OLS ({ec_ols:.4}) is corrupted"
    );
}

#[test]
fn rout_outlier_elimination_q1_percent_and_guards_oracle() {
    // Motulsky & Brown (2006) ROUT outlier detection at Q = 1%
    let log_c: Vec<f64> = (-22..=-8).map(|i| i as f64 * 0.5).collect();
    let tiny_jitter = [0.15, -0.18, 0.12, -0.14, 0.20, 0.0, -0.16, 0.14, -0.12, 0.18, -0.15, 0.0, 0.12, -0.14, 0.16];
    let mut y: Vec<f64> = log_c.iter().zip(tiny_jitter.iter()).map(|(&x, &e)| 10.0 + 90.0 / (1.0 + 10_f64.powf((-7.5 - x) * 1.1)) + e).collect();
    y[5] += 85.0;
    y[11] -= 55.0;

    let ag_m = get_builtin_model("log_agonist_vs_response_4pl").expect("4PL");
    let res = fit_nonlinear(
        ag_m.as_ref(),
        &[series("ROUT", log_c, y)],
        &HookContext::default(),
        &SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() },
    )
    .expect("ROUT Q=1% fit");

    let rout = res.rout.as_ref().expect("ROUT outcome populated");
    assert_approx_eq(rout.q_threshold_percent, 1.0, 1e-12, 1e-12, "ROUT Q = 1%");
    assert_eq!(rout.outlier_indices, vec![5, 11], "exact outlier indices detected");
    assert_eq!(res.n_outliers_excluded, 2);
    assert_eq!(res.df, 13 - 4, "df must reflect 2 excluded outliers");
    assert_approx_eq(find_param(&res, 0, "Bottom").estimate, 10.0, 0.3, 1e-2, "ROUT clean Bottom");
    assert_approx_eq(find_param(&res, 0, "Top").estimate, 100.0, 0.3, 1e-2, "ROUT clean Top");
    assert_approx_eq(find_param(&res, 0, "LogEC50").estimate, -7.5, 0.02, 1e-2, "ROUT clean LogEC50");
    assert_approx_eq(find_param(&res, 0, "HillSlope").estimate, 1.1, 0.03, 1e-2, "ROUT clean HillSlope");

    // Direct `rout_detect_outliers`: clean data yields 0 false positives at Q = 1%
    let clean_pts: Vec<DataPoint> = (0..20).map(|i| DataPoint::xy(i as f64, 2.0 * i as f64 + 5.0)).collect();
    let clean_res: Vec<f64> = (0..20).map(|i| if i % 2 == 0 { 0.08 } else { -0.08 }).collect();
    let rout_clean = rout_detect_outliers(&clean_pts, &clean_res, 2, 1.0).expect("ROUT clean");
    assert!(rout_clean.outlier_indices.is_empty(), "clean data must yield 0 outliers at Q=1%, got {:?}", rout_clean.outlier_indices);

    // Direct `rout_detect_outliers` with planted residual spikes
    let mut dirty_res = clean_res;
    dirty_res[4] = 15.0;
    dirty_res[14] = -18.0;
    let rout_dirty = rout_detect_outliers(&clean_pts, &dirty_res, 2, 1.0).expect("ROUT dirty");
    assert_eq!(rout_dirty.outlier_indices, vec![4, 14]);

    // Pre-summarized Mean/SD/N table must be rejected with `FitError::RequiresRawReplicates`
    let mut pre_ds = series("MeanSdN", vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![10.0; 6]);
    pre_ds.is_pre_summarized = true;
    for pt in &mut pre_ds.points {
        pt.sd = Some(0.5);
        pt.n = Some(3.0);
    }
    let err = fit_nonlinear(
        ag_m.as_ref(),
        &[pre_ds],
        &HookContext::default(),
        &SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() },
    )
    .expect_err("ROUT on pre-summarized data must return RequiresRawReplicates");
    assert!(matches!(err, FitError::RequiresRawReplicates), "expected RequiresRawReplicates, got {err:?}");
}
