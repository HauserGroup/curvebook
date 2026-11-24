//! D1 independent R 4.5.2 oracle tests for R18.1 (§1.4 Nonlinear Solver, Weighting & Method Extensions).
//!
//! Reference R script: `crates/fit/tests/reference/r18_1_solver_weighting_rout.R` (R 4.5.2).
//!
//! References:
//! - Carroll, R. J. & Ruppert, D. (1988). *Transformation and Weighting in Regression*. Chapman & Hall.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123. doi:10.1186/1471-2105-7-123
//! - Moré, J. J. (1978). The Levenberg–Marquardt algorithm: implementation and theory. *Lecture Notes in Mathematics* 630, 105–116. doi:10.1007/BFb0067700
//! - Burnham, K. P. & Anderson, D. R. (2002). *Model Selection and Multimodel Inference*, 2nd ed. Springer.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConvergenceStrictness, DataSheet, FitMethod, ModelComparisonSpec, MultipleVariablesTable, NonlinearFitSpec,
    OutlierHandling, ParameterRule, ParameterScope, ReplicateFitMode, ResultStatus, SheetId, SubcolumnMode, TableArchetype, VariableRole,
    WeightingScheme, WeightingSource, XColumnMode, XyTable,
};
use curvebook_fit::{
    DataPoint, DatasetSeries, FitError, FitStatus, HookContext, ModelComparisonOutcome, NonlinearFitResult, SolverConfig,
    SynchronizedComparisonRequest, compare_models_synchronized, execute_analysis, fit_nonlinear, get_builtin_model,
};

fn param_est(fit: &NonlinearFitResult, name: &str) -> f64 {
    fit.parameters.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map_or(f64::NAN, |p| p.estimate)
}

fn param_se(fit: &NonlinearFitResult, name: &str) -> f64 {
    fit.parameters.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map_or(f64::NAN, |p| p.se)
}

fn series_from_xy(idx: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    let points = xs
        .iter()
        .zip(ys.iter())
        .enumerate()
        .map(|(row, (&x, &y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: idx, row })
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

#[test]
fn d1_r18_1_predicted_y_irls_matches_r_oracle_and_handles_zero_y_obs() {
    let model = get_builtin_model("one_phase_decay").unwrap();
    let xs = [0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 10.0, 12.0, 12.0];
    let ys = [114.2, 116.8, 82.1, 59.5, 44.0, 34.2, 27.5, 23.1, 18.4, 0.0, 32.2, 15.8, 16.4];
    let ds = series_from_xy(0, "DecayZeroObs", &xs, &ys);

    // ObservedY must reject y_obs == 0.0 as ImpossibleWeights.
    let obs_cfg = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::ObservedY,
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let obs_err = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &obs_cfg).unwrap_err();
    assert!(matches!(obs_err, FitError::ImpossibleWeights { .. }), "expected ObservedY ImpossibleWeights, got {obs_err:?}");

    // PredictedY IRLS succeeds even with y_obs == 0.0 and matches R 4.5.2 oracle for k = 2.0, 1.0, 1.5.
    let cases = [
        (
            WeightingScheme::RelativeSquare,
            116.378_945_491_601,
            15.011_016_453_335,
            0.419_137_160_580,
            1.898_350_201_419,
            30.928_069_450_198,
            4.268_728_772_349,
            0.154_647_229_125,
        ),
        (
            WeightingScheme::Relative,
            115.853_950_616_323,
            14.912_996_417_020,
            0.414_494_002_727,
            31.501_888_836_633,
            12.454_616_373_306,
            3.980_718_205_241,
            0.097_176_703_163,
        ),
        (
            WeightingScheme::GeneralPowerY(1.5),
            116.052_936_868_724,
            14.963_796_130_504,
            0.416_647_851_611,
            7.733_112_099_059,
            19.699_926_598_624,
            4.117_459_007_806,
            0.122_333_218_056,
        ),
    ];

    for (scheme, r_y0, r_pl, r_k, r_wss, r_se_y0, r_se_pl, r_se_k) in cases {
        let cfg =
            SolverConfig { weighting: scheme, weighting_source: WeightingSource::PredictedY, compute_profile_ci: false, ..SolverConfig::default() };
        let fit = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();
        assert_eq!(fit.status, FitStatus::Converged, "scheme={scheme:?}");
        assert!((param_est(&fit, "Y0") - r_y0).abs() < 1e-5, "Y0 mismatch for {scheme:?}");
        assert!((param_est(&fit, "Plateau") - r_pl).abs() < 1e-5, "Plateau mismatch for {scheme:?}");
        assert!((param_est(&fit, "K") - r_k).abs() < 1e-6, "K mismatch for {scheme:?}");
        assert!((fit.weighted_ss - r_wss).abs() < 1e-5, "WSS mismatch for {scheme:?}");
        assert!((param_se(&fit, "Y0") - r_se_y0).abs() < 1e-4, "SE_Y0 mismatch for {scheme:?}");
        assert!((param_se(&fit, "Plateau") - r_se_pl).abs() < 1e-4, "SE_Pl mismatch for {scheme:?}");
        assert!((param_se(&fit, "K") - r_se_k).abs() < 1e-5, "SE_K mismatch for {scheme:?}");
    }
}

#[test]
fn d1_r18_1_predicted_y_returns_impossible_weights_when_y_hat_is_zero() {
    let model = get_builtin_model("straight_line").unwrap();
    // Exact line y = 2*x passes through y_hat(0) == 0.0 -> must return FitStatus::ImpossibleWeights.
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
    let ds = series_from_xy(0, "ZeroCrossing", &xs, &ys);
    let cfg = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let fit = fit_nonlinear(model.as_ref(), &[ds], &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit.status, FitStatus::ImpossibleWeights);
}

#[test]
fn d1_r18_1_predicted_y_presummarized_sufficient_stats_match_raw_replicates() {
    let model = get_builtin_model("michaelis_menten").unwrap();
    let x_rows = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let y_reps = [[19.2, 21.1, 19.7], [32.5, 34.0, 33.4], [51.2, 49.0, 50.1], [66.1, 68.2, 65.8], [79.5, 81.2, 79.9], [88.0, 89.6, 88.8]];

    let mut raw_pts = Vec::new();
    let mut sum_pts = Vec::new();
    for (row, (&x, reps)) in x_rows.iter().zip(y_reps.iter()).enumerate() {
        let n = reps.len() as f64;
        let mean = reps.iter().sum::<f64>() / n;
        let var = reps.iter().map(|&y| (y - mean) * (y - mean)).sum::<f64>() / (n - 1.0);
        sum_pts.push(DataPoint {
            x: vec![x],
            y: mean,
            sd: Some(var.sqrt()),
            n: Some(n),
            explicit_weight: None,
            source_subcolumn: 0,
            data_set_index: 0,
            row,
        });
        for (sub, &y) in reps.iter().enumerate() {
            raw_pts.push(DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: sub, data_set_index: 0, row });
        }
    }

    let ds_raw = DatasetSeries {
        data_set_index: 0,
        label: "Raw".to_owned(),
        points: raw_pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    };
    let ds_sum = DatasetSeries {
        data_set_index: 0,
        label: "MeanSDN".to_owned(),
        points: sum_pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: true,
        column_title_value: None,
    };

    let cfg = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let fit_raw = fit_nonlinear(model.as_ref(), &[ds_raw], &HookContext::default(), &cfg).unwrap();
    let fit_sum = fit_nonlinear(model.as_ref(), &[ds_sum], &HookContext::default(), &cfg).unwrap();
    assert_eq!(fit_raw.status, FitStatus::Converged);
    assert_eq!(fit_sum.status, FitStatus::Converged);
    assert_eq!(fit_raw.n_obs, fit_sum.n_obs);
    assert_eq!(fit_raw.df, fit_sum.df);
    assert!((param_est(&fit_raw, "Vmax") - param_est(&fit_sum, "Vmax")).abs() < 1e-8);
    assert!((param_est(&fit_raw, "Km") - param_est(&fit_sum, "Km")).abs() < 1e-8);
    assert!((fit_raw.weighted_ss - fit_sum.weighted_ss).abs() < 1e-8);
    assert!((param_se(&fit_raw, "Vmax") - param_se(&fit_sum, "Vmax")).abs() < 1e-8);
}

#[test]
fn d1_r18_1_predicted_y_robust_and_rout_keep_stage1_unweighted() {
    let model = get_builtin_model("straight_line").unwrap();
    let xs: Vec<f64> = (1..=14).map(f64::from).collect();
    let jitter = [0.2, -0.15, 0.18, -0.22, 0.16, -0.19, 0.21, -0.17, 0.14, -0.2, 0.19, -0.16, 0.15, 0.0];
    let mut ys: Vec<f64> = xs.iter().zip(jitter.iter()).map(|(&x, &e)| 5.0 + 2.0 * x + e).collect();
    // Extreme outlier at index 13 (x=14, y_true=33 -> y_obs=95)
    ys[13] = 95.0;
    let ds = series_from_xy(0, "RobustPredY", &xs, &ys);

    // Pure RobustCauchy with PredictedY must produce identical parameter estimates to unweighted RobustCauchy
    // because Stage 1 Cauchy optimization runs unweighted, while applying 1/Y_curve^2 weights to residuals/RSDR.
    let cfg_unw = SolverConfig { method: FitMethod::RobustCauchy, weighting: WeightingScheme::None, ..SolverConfig::default() };
    let cfg_pred = SolverConfig {
        method: FitMethod::RobustCauchy,
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        ..SolverConfig::default()
    };
    let fit_unw = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg_unw).unwrap();
    let fit_pred = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg_pred).unwrap();
    assert!((param_est(&fit_unw, "Slope") - param_est(&fit_pred, "Slope")).abs() < 1e-10);
    assert!((param_est(&fit_unw, "YIntercept") - param_est(&fit_pred, "YIntercept")).abs() < 1e-10);
    // Weighted residuals under PredictedY 1/Y_curve^2 are scaled by 1/|y_hat|, so RSDR is strictly smaller than unweighted.
    assert!(fit_pred.rsdr.unwrap() < fit_unw.rsdr.unwrap());

    // ROUT + PredictedY detects outlier index 13 and refits with PredictedY IRLS on inliers.
    let cfg_rout = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let fit_rout = fit_nonlinear(model.as_ref(), &[ds], &HookContext::default(), &cfg_rout).unwrap();
    assert_eq!(fit_rout.status, FitStatus::Converged);
    assert_eq!(fit_rout.n_outliers_excluded, 1);
    assert_eq!(fit_rout.rout.as_ref().unwrap().outlier_indices, vec![13]);
    assert!((param_est(&fit_rout, "Slope") - 2.0).abs() < 0.05);
    assert!((param_est(&fit_rout, "YIntercept") - 5.0).abs() < 0.25);
}

#[test]
fn d1_r18_1_rout_identify_count_only_and_cleaned_data_tab() {
    let xs: Vec<f64> = (1..=14).map(f64::from).collect();
    let jitter = [0.1, -0.1, 0.05, -0.08, 0.12, -0.05, 0.07, -0.09, 0.06, -0.04, 0.08, -0.07, 0.03, 40.0];
    let ys: Vec<f64> = xs.iter().zip(jitter.iter()).map(|(&x, &e)| 2.0 * x + 5.0 + e).collect();

    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new(SheetId::new("xy-rout"), "ROUT Test", TableArchetype::Xy(xy)).unwrap();

    // 1. IdentifyAndCountOnly: identifies outlier index 13, emits Outliers tab, retains all 14 points in OLS fit.
    let mut nl_count = NonlinearFitSpec::new("straight_line");
    nl_count.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_count.outlier_handling = OutlierHandling::IdentifyAndCountOnly;
    nl_count.profile_likelihood_ci = false;
    let spec_count = AnalysisSpec::new("res-count", SheetId::new("xy-rout"), AnalysisKind::NonlinearRegression(nl_count)).unwrap();
    let rs_count = execute_analysis(&spec_count, &sheet, &HookContext::default()).unwrap();
    assert_eq!(rs_count.status, ResultStatus::UpToDate);
    assert!(rs_count.eliminated_outliers.is_empty(), "IdentifyAndCountOnly must not mark cells eliminated");
    assert!(rs_count.tabs.iter().any(|t| t.id == "nonlinear_outliers" && t.title == "Outliers"));
    let slope_all = rs_count.lookup_scalar("Slope", Some(0)).unwrap();
    let intercept_all = rs_count.lookup_scalar("YIntercept", Some(0)).unwrap();
    let rss_all = rs_count.lookup_scalar("RSS", None).unwrap();
    assert!((intercept_all - (-0.700_989_010_989)).abs() < 1e-6);
    assert!((slope_all - 3.141_846_153_846).abs() < 1e-6);
    assert!((rss_all - 1_188.715_510_329_67).abs() < 1e-4);
    assert_eq!(rs_count.lookup_scalar("ROUT outliers", None), Some(1.0));

    // 2. Eliminate + emit_cleaned_data_tab: excludes outlier, emits "Cleaned data" tab & analyzable_data.
    let mut nl_elim = NonlinearFitSpec::new("straight_line");
    nl_elim.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_elim.outlier_handling = OutlierHandling::Eliminate;
    nl_elim.emit_cleaned_data_tab = true;
    nl_elim.profile_likelihood_ci = false;
    let spec_elim = AnalysisSpec::new("res-elim", SheetId::new("xy-rout"), AnalysisKind::NonlinearRegression(nl_elim)).unwrap();
    let rs_elim = execute_analysis(&spec_elim, &sheet, &HookContext::default()).unwrap();
    assert_eq!(rs_elim.status, ResultStatus::UpToDate);
    assert_eq!(rs_elim.eliminated_outliers.len(), 1);
    assert!(rs_elim.tabs.iter().any(|t| t.id == "cleaned_data" && t.title == "Cleaned data"));
    let cleaned_ds = rs_elim.analyzable_data.as_ref().expect("analyzable Cleaned data sheet");
    assert!(
        matches!(&cleaned_ds.archetype, TableArchetype::Xy(cleaned_xy) if cleaned_xy.data_sets[0].cells[13][0].excluded && !cleaned_xy.data_sets[0].cells[0][0].excluded),
        "outlier row 13 must be excluded in analyzable_data"
    );

    // Secondary analysis bound directly to cleaned analyzable_data matches R clean OLS oracle.
    let mut nl_sec = NonlinearFitSpec::new("straight_line");
    nl_sec.profile_likelihood_ci = false;
    let spec_sec = AnalysisSpec::new("res-sec", cleaned_ds.id.clone(), AnalysisKind::NonlinearRegression(nl_sec)).unwrap();
    let rs_sec = execute_analysis(&spec_sec, cleaned_ds, &HookContext::default()).unwrap();
    assert!((rs_sec.lookup_scalar("YIntercept", Some(0)).unwrap() - 5.013_461_538_462).abs() < 1e-6);
    assert!((rs_sec.lookup_scalar("Slope", Some(0)).unwrap() - 1.998_956_043_956).abs() < 1e-6);
    assert!((rs_sec.lookup_scalar("RSS", None).unwrap() - 0.075_509_340_659).abs() < 1e-6);
}

#[test]
fn d1_r18_1_rout_union_of_outliers_model_comparison_matches_r_oracle() {
    let m1 = get_builtin_model("straight_line").unwrap();
    let m2 = get_builtin_model("quadratic").unwrap();

    let xs: Vec<f64> = (0..21).map(|i| 0.5 * (i as f64)).collect();
    let noise =
        [0.05, -0.04, 0.03, -0.06, 0.02, -0.03, 0.04, -0.05, 0.06, -0.02, 0.03, -0.04, 0.05, -0.03, 0.02, -0.05, 0.04, -0.02, 0.03, -0.04, 0.05];
    let mut ys: Vec<f64> = xs.iter().zip(noise.iter()).map(|(&x, &e)| 2.0 + 1.5 * x + 0.18 * x * x + e).collect();
    // Plant two outliers: index 3 (extreme, caught by both) and index 18 (moderate, caught by quadratic, masked by curvature in straight_line).
    ys[3] += 25.0;
    ys[18] -= 3.2;
    let ds = series_from_xy(0, "UnionCmp", &xs, &ys);

    let cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        outlier_handling: OutlierHandling::Eliminate,
        compute_profile_ci: false,
        ..SolverConfig::default()
    };
    let fit1 = fit_nonlinear(m1.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();
    let fit2 = fit_nonlinear(m2.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap();
    let o1 = &fit1.rout.as_ref().unwrap().outlier_indices;
    let o2 = &fit2.rout.as_ref().unwrap().outlier_indices;
    assert_ne!(o1, o2, "expected distinct ROUT outlier sets O1={o1:?} vs O2={o2:?}");
    assert!(o2.contains(&3) && o2.contains(&18), "expected quadratic to flag both 3 and 18, got {o2:?}");

    // Synchronized comparison removes O1 U O2 = {3, 18} and refits both on the 19 retained points.
    let comp_spec_f = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "quadratic".to_owned() };
    let (eff1, comp_f) = compare_models_synchronized(&SynchronizedComparisonRequest {
        primary_model: m1.as_ref(),
        primary_fit: &fit1,
        alt_model: m2.as_ref(),
        datasets: std::slice::from_ref(&ds),
        hooks: &HookContext::default(),
        config: &cfg,
        spec: &comp_spec_f,
    })
    .unwrap();
    assert_eq!(eff1.n_obs, 19);
    assert_eq!(comp_f.alternative_fit.n_obs, 19);
    assert_eq!(eff1.n_outliers_excluded, 2);
    assert!(
        matches!(&comp_f.outcome, ModelComparisonOutcome::ExtraSumOfSquaresF(f_res) if (f_res.f_stat - 24_553.976_671_347_755).abs() < 1e-3 && f_res.p_value < 1e-20),
        "unexpected F outcome: {:?}",
        comp_f.outcome
    );

    let comp_spec_aicc = ModelComparisonSpec::Aicc { alternative_equation_id: "quadratic".to_owned() };
    let (_, comp_aicc) = compare_models_synchronized(&SynchronizedComparisonRequest {
        primary_model: m1.as_ref(),
        primary_fit: &fit1,
        alt_model: m2.as_ref(),
        datasets: &[ds],
        hooks: &HookContext::default(),
        config: &cfg,
        spec: &comp_spec_aicc,
    })
    .unwrap();
    assert!(
        matches!(&comp_aicc.outcome, ModelComparisonOutcome::Aicc(aicc_res) if (aicc_res.delta_aicc - (-136.140_001_225_017)).abs() < 1e-5),
        "unexpected AICc outcome: {:?}",
        comp_aicc.outcome
    );
}

#[test]
fn d1_r18_1_replicate_fit_mode_row_means_only_matches_r_oracle() {
    let model = get_builtin_model("michaelis_menten").unwrap();
    let x_rows = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let y_reps = [[19.2, 21.1, 19.7], [32.5, 34.0, 33.4], [51.2, 49.0, 50.1], [66.1, 68.2, 65.8], [79.5, 81.2, 79.9], [88.0, 89.6, 88.8]];
    let mut pts = Vec::new();
    for (row, (&x, reps)) in x_rows.iter().zip(y_reps.iter()).enumerate() {
        for (sub, &y) in reps.iter().enumerate() {
            pts.push(DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: sub, data_set_index: 0, row });
        }
    }
    let ds = DatasetSeries {
        data_set_index: 0,
        label: "MMReps".to_owned(),
        points: pts,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    };

    let cfg_ind = SolverConfig { replicate_fit_mode: ReplicateFitMode::IndividualReplicates, compute_profile_ci: false, ..SolverConfig::default() };
    let cfg_means = SolverConfig { replicate_fit_mode: ReplicateFitMode::RowMeansOnly, compute_profile_ci: false, ..SolverConfig::default() };
    let fit_ind = fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg_ind).unwrap();
    let fit_means = fit_nonlinear(model.as_ref(), &[ds], &HookContext::default(), &cfg_means).unwrap();

    assert_eq!(fit_ind.n_obs, 18);
    assert_eq!(fit_ind.df, 16);
    assert!((param_est(&fit_ind, "Vmax") - 100.019_135_480_162).abs() < 1e-6);
    assert!((param_est(&fit_ind, "Km") - 3.993_380_132_104).abs() < 1e-6);
    assert!((fit_ind.rss - 11.928_249_097_406).abs() < 1e-6);

    assert_eq!(fit_means.n_obs, 6);
    assert_eq!(fit_means.df, 4);
    assert!((param_est(&fit_means, "Vmax") - 100.019_135_477_250).abs() < 1e-6);
    assert!((param_est(&fit_means, "Km") - 3.993_380_131_699).abs() < 1e-6);
    assert!((fit_means.rss - 0.049_416_365_802).abs() < 1e-6);
}

#[test]
fn d1_r18_1_convergence_strictness_presets() {
    let model = get_builtin_model("one_phase_decay").unwrap();
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];
    let ys = [102.4, 71.8, 52.1, 38.9, 28.2, 23.1, 18.4, 14.2, 11.9, 10.8];
    let ds = series_from_xy(0, "ConvPresets", &xs, &ys);

    // Provide a slightly displaced initial guess so LM performs several iterations.
    let rules = vec![
        ParameterRule { name: "Y0".to_owned(), scope: ParameterScope::Local, initial_override: Some(150.0), lower_bound: None, upper_bound: None },
        ParameterRule { name: "Plateau".to_owned(), scope: ParameterScope::Local, initial_override: Some(2.0), lower_bound: None, upper_bound: None },
        ParameterRule { name: "K".to_owned(), scope: ParameterScope::Local, initial_override: Some(0.1), lower_bound: None, upper_bound: None },
    ];
    let run_preset = |strictness: ConvergenceStrictness| {
        let cfg = SolverConfig {
            parameter_rules: rules.clone(),
            convergence_strictness: strictness,
            use_simplex_warm_start: false,
            compute_profile_ci: false,
            ..SolverConfig::default()
        };
        fit_nonlinear(model.as_ref(), std::slice::from_ref(&ds), &HookContext::default(), &cfg).unwrap()
    };

    let f_quick = run_preset(ConvergenceStrictness::Quick);
    let f_med = run_preset(ConvergenceStrictness::Medium);
    let f_strict = run_preset(ConvergenceStrictness::Strict);
    let f_default = run_preset(ConvergenceStrictness::Default);

    for f in [&f_quick, &f_med, &f_strict, &f_default] {
        assert_eq!(f.status, FitStatus::Converged);
    }
    assert!(f_quick.iterations <= f_strict.iterations, "Quick ({}) should take <= Strict ({}) iters", f_quick.iterations, f_strict.iterations);
    assert!((param_est(&f_strict, "K") - param_est(&f_default, "K")).abs() < 1e-6);
    assert!((param_est(&f_med, "K") - param_est(&f_default, "K")).abs() < 1e-3);
}

#[test]
fn d1_r18_1_multivar_multi_response_mv_y_columns_matches_r_oracle() {
    let mut mv = MultipleVariablesTable::new(4, 12).unwrap();
    mv.variables[0].title = "Group".to_owned();
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = "X".to_owned();
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = "RespA".to_owned();
    mv.variables[2].lock_role(VariableRole::Continuous);
    mv.variables[3].title = "RespB".to_owned();
    mv.variables[3].lock_role(VariableRole::Continuous);

    let x_mv = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y1_ctrl = [3.1, 4.9, 7.05, 8.95, 11.08, 12.92];
    let y1_trt = [5.9, 9.6, 12.95, 16.55, 19.92, 23.58];
    let y2_ctrl = [10.55, 10.95, 11.52, 11.98, 12.54, 12.96];
    let y2_trt = [13.75, 12.65, 11.38, 10.22, 8.96, 7.84];

    for i in 0..6 {
        mv.variables[0].cells[i] = Cell::text("Control");
        mv.variables[1].cells[i] = Cell::number(x_mv[i]);
        mv.variables[2].cells[i] = Cell::number(y1_ctrl[i]);
        mv.variables[3].cells[i] = Cell::number(y2_ctrl[i]);

        mv.variables[0].cells[6 + i] = Cell::text("Treated");
        mv.variables[1].cells[6 + i] = Cell::number(x_mv[i]);
        mv.variables[2].cells[6 + i] = Cell::number(y1_trt[i]);
        mv.variables[3].cells[6 + i] = Cell::number(y2_trt[i]);
    }
    let sheet = DataSheet::new(SheetId::new("mv-multi"), "MV MultiY", TableArchetype::MultipleVariables(mv)).unwrap();

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.mv_group_columns = vec![0];
    nl.mv_x_columns = vec![1];
    nl.mv_y_columns = vec![2, 3];
    nl.profile_likelihood_ci = false;
    let spec = AnalysisSpec::new("res-mv-multi", SheetId::new("mv-multi"), AnalysisKind::NonlinearRegression(nl)).unwrap();
    let rs = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert_eq!(rs.fitted_curves.len(), 4, "expected M=2 responses x G=2 groups = 4 curves");

    let expected = [
        (0, 1.046_000_000_000, 1.986_857_142_857),
        (1, 2.454_000_000_000, 3.513_142_857_143),
        (2, 10.022_000_000_000, 0.493_714_285_714),
        (3, 14.978_000_000_000, -1.193_714_285_714),
    ];
    for (ds_idx, r_int, r_slope) in expected {
        let int_est = rs.lookup_scalar("YIntercept", Some(ds_idx)).unwrap();
        let slp_est = rs.lookup_scalar("Slope", Some(ds_idx)).unwrap();
        assert!((int_est - r_int).abs() < 1e-8, "ds {ds_idx} intercept {int_est} vs {r_int}");
        assert!((slp_est - r_slope).abs() < 1e-8, "ds {ds_idx} slope {slp_est} vs {r_slope}");
    }
}
