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

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::common;

use curvebook_doc::{
    Cell, CiBound, ConvergenceStrictness, DataSheet, FitMethod, ModelComparisonMethod, ModelComparisonQuestion, ModelComparisonSpec,
    MultipleVariablesTable, OutlierHandling, ReplicateFitMode, ResultStatus, SheetId, TableArchetype, VariableRole, WeightingScheme, WeightingSource,
};
use curvebook_fit::{
    DataPoint, DatasetSeries, FitError, FitStatus, HookContext, ModelComparisonOutcome, SolverConfig, SynchronizedComparisonRequest, UnknownResponse,
    compare_models, compare_models_question, compare_models_synchronized, compute_rsdr, fit_nonlinear, get_builtin_model,
};
use curvebook_testkit::assert_approx_eq;

use common::{
    assert_converged, cfg_fast, exec_nl, find_param, fit_with, mean_sd_n, nl_fast, param_val, rule_local_init, rule_shared, series_from_pts,
    series_replicates, series_xy, xy_sheet_replicates, xy_sheet_single,
};

#[test]
fn d1_r18_1_predicted_y_irls_matches_r_oracle_and_handles_zero_y_obs() {
    let model = get_builtin_model("one_phase_decay").unwrap();
    let xs = [0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 10.0, 12.0, 12.0];
    let ys = [114.2, 116.8, 82.1, 59.5, 44.0, 34.2, 27.5, 23.1, 18.4, 0.0, 32.2, 15.8, 16.4];
    let ds = series_xy(0, "DecayZeroObs", &xs, &ys);

    // ObservedY must reject y_obs == 0.0 as ImpossibleWeights.
    let obs_cfg = SolverConfig { weighting: WeightingScheme::RelativeSquare, weighting_source: WeightingSource::ObservedY, ..cfg_fast() };
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
        let cfg = SolverConfig { weighting: scheme, weighting_source: WeightingSource::PredictedY, ..cfg_fast() };
        let fit = fit_with(model.as_ref(), std::slice::from_ref(&ds), &cfg);
        assert_converged(&fit, &format!("scheme={scheme:?}"));
        assert!((param_val(&fit, "Y0") - r_y0).abs() < 1e-5, "Y0 mismatch for {scheme:?}");
        assert!((param_val(&fit, "Plateau") - r_pl).abs() < 1e-5, "Plateau mismatch for {scheme:?}");
        assert!((param_val(&fit, "K") - r_k).abs() < 1e-6, "K mismatch for {scheme:?}");
        assert!((fit.weighted_ss - r_wss).abs() < 1e-5, "WSS mismatch for {scheme:?}");
        assert!((find_param(&fit, "Y0").se - r_se_y0).abs() < 1e-4, "SE_Y0 mismatch for {scheme:?}");
        assert!((find_param(&fit, "Plateau").se - r_se_pl).abs() < 1e-4, "SE_Pl mismatch for {scheme:?}");
        assert!((find_param(&fit, "K").se - r_se_k).abs() < 1e-5, "SE_K mismatch for {scheme:?}");
    }
}

#[test]
fn d1_r18_1_predicted_y_returns_impossible_weights_when_y_hat_is_zero() {
    let model = get_builtin_model("straight_line").unwrap();
    let ds = series_xy(0, "ZeroCrossing", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    let cfg = SolverConfig { weighting: WeightingScheme::RelativeSquare, weighting_source: WeightingSource::PredictedY, ..cfg_fast() };
    assert_eq!(fit_with(model.as_ref(), &[ds], &cfg).status, FitStatus::ImpossibleWeights);
}

#[test]
fn d1_r18_1_predicted_y_presummarized_sufficient_stats_match_raw_replicates() {
    let model = get_builtin_model("michaelis_menten").unwrap();
    let x_rows = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    let y_reps = [[19.2, 21.1, 19.7], [32.5, 34.0, 33.4], [51.2, 49.0, 50.1], [66.1, 68.2, 65.8], [79.5, 81.2, 79.9], [88.0, 89.6, 88.8]];
    let ds_raw = series_replicates(0, "Raw", &x_rows, &y_reps);
    let sum_pts = x_rows
        .iter()
        .zip(&y_reps)
        .enumerate()
        .map(|(row, (&x, reps))| {
            let (m, sd, n) = mean_sd_n(reps);
            DataPoint { sd: Some(sd), n: Some(n), row, ..DataPoint::xy(x, m) }
        })
        .collect();
    let mut ds_sum = series_from_pts(0, "MeanSDN", sum_pts);
    ds_sum.is_pre_summarized = true;

    let cfg = SolverConfig { weighting: WeightingScheme::RelativeSquare, weighting_source: WeightingSource::PredictedY, ..cfg_fast() };
    let fit_raw = fit_with(model.as_ref(), &[ds_raw], &cfg);
    let fit_sum = fit_with(model.as_ref(), &[ds_sum], &cfg);
    assert_converged(&fit_raw, "raw");
    assert_converged(&fit_sum, "sum");
    assert_eq!((fit_raw.n_obs, fit_raw.df), (fit_sum.n_obs, fit_sum.df));
    assert!((param_val(&fit_raw, "Vmax") - param_val(&fit_sum, "Vmax")).abs() < 1e-8);
    assert!((param_val(&fit_raw, "Km") - param_val(&fit_sum, "Km")).abs() < 1e-8);
    assert!((fit_raw.weighted_ss - fit_sum.weighted_ss).abs() < 1e-8);
    assert!((find_param(&fit_raw, "Vmax").se - find_param(&fit_sum, "Vmax").se).abs() < 1e-8);
}

#[test]
fn d1_r18_1_predicted_y_robust_and_rout_keep_stage1_unweighted() {
    let model = get_builtin_model("straight_line").unwrap();
    let xs: Vec<f64> = (1..=14).map(f64::from).collect();
    let jitter = [0.2, -0.15, 0.18, -0.22, 0.16, -0.19, 0.21, -0.17, 0.14, -0.2, 0.19, -0.16, 0.15, 0.0];
    let mut ys: Vec<f64> = xs.iter().zip(jitter).map(|(&x, e)| 5.0 + 2.0 * x + e).collect();
    ys[13] = 95.0; // extreme outlier at index 13
    let ds = series_xy(0, "RobustPredY", &xs, &ys);

    let cfg_unw = SolverConfig { method: FitMethod::RobustCauchy, weighting: WeightingScheme::None, ..SolverConfig::default() };
    let cfg_pred = SolverConfig {
        method: FitMethod::RobustCauchy,
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        ..SolverConfig::default()
    };
    let fit_unw = fit_with(model.as_ref(), std::slice::from_ref(&ds), &cfg_unw);
    let fit_pred = fit_with(model.as_ref(), std::slice::from_ref(&ds), &cfg_pred);
    assert!((param_val(&fit_unw, "Slope") - param_val(&fit_pred, "Slope")).abs() < 1e-10);
    assert!((param_val(&fit_unw, "YIntercept") - param_val(&fit_pred, "YIntercept")).abs() < 1e-10);
    assert!(fit_pred.rsdr.unwrap() < fit_unw.rsdr.unwrap());

    let cfg_rout = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        weighting: WeightingScheme::RelativeSquare,
        weighting_source: WeightingSource::PredictedY,
        ..cfg_fast()
    };
    let fit_rout = fit_with(model.as_ref(), &[ds], &cfg_rout);
    assert_converged(&fit_rout, "rout_pred");
    assert_eq!(fit_rout.n_outliers_excluded, 1);
    assert_eq!(fit_rout.rout.as_ref().unwrap().outlier_indices, vec![13]);
    assert!((param_val(&fit_rout, "Slope") - 2.0).abs() < 0.05);
    assert!((param_val(&fit_rout, "YIntercept") - 5.0).abs() < 0.25);
}

#[test]
fn d1_r18_1_rout_identify_count_only_and_cleaned_data_tab() {
    let xs: Vec<f64> = (1..=14).map(f64::from).collect();
    let jitter = [0.1, -0.1, 0.05, -0.08, 0.12, -0.05, 0.07, -0.09, 0.06, -0.04, 0.08, -0.07, 0.03, 40.0];
    let ys: Vec<f64> = xs.iter().zip(jitter).map(|(&x, e)| 2.0 * x + 5.0 + e).collect();
    let sheet = xy_sheet_single("xy-rout", "ROUT Test", "A", &xs, &ys);

    // 1. IdentifyAndCountOnly: identifies outlier index 13, emits Outliers tab, retains all 14 points in OLS fit.
    let mut nl_count = nl_fast("straight_line");
    nl_count.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_count.outlier_handling = OutlierHandling::IdentifyAndCountOnly;
    let rs_count = exec_nl(&sheet, nl_count);
    assert_eq!(rs_count.status, ResultStatus::UpToDate);
    assert!(rs_count.eliminated_outliers.is_empty(), "IdentifyAndCountOnly must not mark cells eliminated");
    assert!(rs_count.tabs.iter().any(|t| t.id == "nonlinear_outliers" && t.title == "Outliers"));
    assert!((rs_count.lookup_scalar("YIntercept", Some(0)).unwrap() - (-0.700_989_010_989)).abs() < 1e-6);
    assert!((rs_count.lookup_scalar("Slope", Some(0)).unwrap() - 3.141_846_153_846).abs() < 1e-6);
    assert!((rs_count.lookup_scalar("RSS", None).unwrap() - 1_188.715_510_329_67).abs() < 1e-4);
    assert_eq!(rs_count.lookup_scalar("ROUT outliers", None), Some(1.0));

    // 2. Eliminate + emit_cleaned_data_tab: excludes outlier, emits "Cleaned data" tab & analyzable_data.
    let mut nl_elim = nl_fast("straight_line");
    nl_elim.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_elim.outlier_handling = OutlierHandling::Eliminate;
    nl_elim.emit_cleaned_data_tab = true;
    let rs_elim = exec_nl(&sheet, nl_elim);
    assert_eq!(rs_elim.status, ResultStatus::UpToDate);
    assert_eq!(rs_elim.eliminated_outliers.len(), 1);
    assert!(rs_elim.tabs.iter().any(|t| t.id == "cleaned_data" && t.title == "Cleaned data"));
    let cleaned_ds = rs_elim.analyzable_data.as_ref().expect("analyzable Cleaned data sheet");
    assert!(
        matches!(&cleaned_ds.archetype, TableArchetype::Xy(cleaned_xy) if cleaned_xy.data_sets[0].cells[13][0].excluded && !cleaned_xy.data_sets[0].cells[0][0].excluded),
        "outlier row 13 must be excluded in analyzable_data"
    );

    // Secondary analysis bound directly to cleaned analyzable_data matches R clean OLS oracle.
    let rs_sec = exec_nl(cleaned_ds, nl_fast("straight_line"));
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
    let mut ys: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 2.0 + 1.5 * x + 0.18 * x * x + e).collect();
    ys[3] += 25.0;
    ys[18] -= 3.2;
    let ds = series_xy(0, "UnionCmp", &xs, &ys);

    let cfg =
        SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, outlier_handling: OutlierHandling::Eliminate, ..cfg_fast() };
    let fit1 = fit_with(m1.as_ref(), std::slice::from_ref(&ds), &cfg);
    let fit2 = fit_with(m2.as_ref(), std::slice::from_ref(&ds), &cfg);
    let (o1, o2) = (&fit1.rout.as_ref().unwrap().outlier_indices, &fit2.rout.as_ref().unwrap().outlier_indices);
    assert_ne!(o1, o2, "expected distinct ROUT outlier sets O1={o1:?} vs O2={o2:?}");
    assert!(o2.contains(&3) && o2.contains(&18), "expected quadratic to flag both 3 and 18, got {o2:?}");

    let sync_cmp = |spec: &ModelComparisonSpec| {
        compare_models_synchronized(&SynchronizedComparisonRequest {
            primary_model: m1.as_ref(),
            primary_fit: &fit1,
            alt_model: m2.as_ref(),
            datasets: std::slice::from_ref(&ds),
            hooks: &HookContext::default(),
            config: &cfg,
            spec,
        })
        .unwrap()
    };
    let (eff1, comp_f) = sync_cmp(&ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "quadratic".to_owned() });
    assert_eq!((eff1.n_obs, comp_f.alternative_fit.n_obs, eff1.n_outliers_excluded), (19, 19, 2));
    assert!(
        matches!(&comp_f.outcome, ModelComparisonOutcome::ExtraSumOfSquaresF(f) if (f.f_stat - 24_553.976_671_347_755).abs() < 1e-3 && f.p_value < 1e-20),
        "unexpected F outcome: {:?}",
        comp_f.outcome
    );

    let (_, comp_aicc) = sync_cmp(&ModelComparisonSpec::Aicc { alternative_equation_id: "quadratic".to_owned() });
    assert!(
        matches!(&comp_aicc.outcome, ModelComparisonOutcome::Aicc(a) if (a.delta_aicc - (-136.140_001_225_017)).abs() < 1e-5),
        "unexpected AICc outcome: {:?}",
        comp_aicc.outcome
    );
}

#[test]
fn d1_r18_1_replicate_fit_mode_row_means_only_matches_r_oracle() {
    let model = get_builtin_model("michaelis_menten").unwrap();
    let ds = series_replicates(
        0,
        "MMReps",
        &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
        &[[19.2, 21.1, 19.7], [32.5, 34.0, 33.4], [51.2, 49.0, 50.1], [66.1, 68.2, 65.8], [79.5, 81.2, 79.9], [88.0, 89.6, 88.8]],
    );
    for (mode, exp_n, exp_df, exp_vmax, exp_km, exp_rss) in [
        (ReplicateFitMode::IndividualReplicates, 18, 16, 100.019_135_480_162, 3.993_380_132_104, 11.928_249_097_406),
        (ReplicateFitMode::RowMeansOnly, 6, 4, 100.019_135_477_250, 3.993_380_131_699, 0.049_416_365_802),
    ] {
        let fit = fit_with(model.as_ref(), std::slice::from_ref(&ds), &SolverConfig { replicate_fit_mode: mode, ..cfg_fast() });
        assert_eq!((fit.n_obs, fit.df), (exp_n, exp_df));
        assert!((param_val(&fit, "Vmax") - exp_vmax).abs() < 1e-6);
        assert!((param_val(&fit, "Km") - exp_km).abs() < 1e-6);
        assert!((fit.rss - exp_rss).abs() < 1e-6);
    }
}

#[test]
fn d1_r18_1_convergence_strictness_presets() {
    let model = get_builtin_model("one_phase_decay").unwrap();
    let ds = series_xy(
        0,
        "ConvPresets",
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0],
        &[102.4, 71.8, 52.1, 38.9, 28.2, 23.1, 18.4, 14.2, 11.9, 10.8],
    );
    let rules = vec![rule_local_init("Y0", 150.0), rule_local_init("Plateau", 2.0), rule_local_init("K", 0.1)];
    let run_preset = |strictness| {
        let cfg = SolverConfig { parameter_rules: rules.clone(), convergence_strictness: strictness, use_simplex_warm_start: false, ..cfg_fast() };
        fit_with(model.as_ref(), std::slice::from_ref(&ds), &cfg)
    };
    let (f_quick, f_med, f_strict, f_default) = (
        run_preset(ConvergenceStrictness::Quick),
        run_preset(ConvergenceStrictness::Medium),
        run_preset(ConvergenceStrictness::Strict),
        run_preset(ConvergenceStrictness::Default),
    );
    for f in [&f_quick, &f_med, &f_strict, &f_default] {
        assert_converged(f, "preset");
    }
    assert!(f_quick.iterations <= f_strict.iterations, "Quick ({}) should take <= Strict ({}) iters", f_quick.iterations, f_strict.iterations);
    assert!((param_val(&f_strict, "K") - param_val(&f_default, "K")).abs() < 1e-6);
    assert!((param_val(&f_med, "K") - param_val(&f_default, "K")).abs() < 1e-3);
}

#[test]
fn d1_r18_1_multivar_multi_response_mv_y_columns_matches_r_oracle() {
    let mut mv = MultipleVariablesTable::new(4, 12).unwrap();
    for (c, (title, role)) in [
        ("Group", VariableRole::Categorical),
        ("X", VariableRole::Continuous),
        ("RespA", VariableRole::Continuous),
        ("RespB", VariableRole::Continuous),
    ]
    .into_iter()
    .enumerate()
    {
        mv.variables[c].title = title.to_owned();
        mv.variables[c].lock_role(role);
    }
    let x_mv = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let groups = [
        ("Control", [3.1, 4.9, 7.05, 8.95, 11.08, 12.92], [10.55, 10.95, 11.52, 11.98, 12.54, 12.96]),
        ("Treated", [5.9, 9.6, 12.95, 16.55, 19.92, 23.58], [13.75, 12.65, 11.38, 10.22, 8.96, 7.84]),
    ];
    for (g, (lbl, y1, y2)) in groups.into_iter().enumerate() {
        for i in 0..6 {
            let r = 6 * g + i;
            mv.variables[0].cells[r] = Cell::text(lbl);
            mv.variables[1].cells[r] = Cell::number(x_mv[i]);
            mv.variables[2].cells[r] = Cell::number(y1[i]);
            mv.variables[3].cells[r] = Cell::number(y2[i]);
        }
    }
    let sheet = DataSheet::new(SheetId::new("mv-multi"), "MV MultiY", TableArchetype::MultipleVariables(mv)).unwrap();
    let mut nl = nl_fast("straight_line");
    nl.mv_group_columns = vec![0];
    nl.mv_x_columns = vec![1];
    nl.mv_y_columns = vec![2, 3];
    let rs = exec_nl(&sheet, nl);
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert_eq!(rs.fitted_curves.len(), 4, "expected M=2 responses x G=2 groups = 4 curves");

    for (ds_idx, r_int, r_slope) in [
        (0, 1.046_000_000_000, 1.986_857_142_857),
        (1, 2.454_000_000_000, 3.513_142_857_143),
        (2, 10.022_000_000_000, 0.493_714_285_714),
        (3, 14.978_000_000_000, -1.193_714_285_714),
    ] {
        assert!((rs.lookup_scalar("YIntercept", Some(ds_idx)).unwrap() - r_int).abs() < 1e-8);
        assert!((rs.lookup_scalar("Slope", Some(ds_idx)).unwrap() - r_slope).abs() < 1e-8);
    }
}

#[test]
fn aud_03_07_09_12_23_predicted_y_jacobian_observed_sd_and_zero_optimum() {
    let m_lin = get_builtin_model("straight_line").unwrap();
    let m_quad = get_builtin_model("quadratic").unwrap();

    // AUD-03-07: independent ROUT PredictedY ImpossibleWeights aborts cleanly without fabricating rout audit.
    let fit_imp = fit_with(
        m_lin.as_ref(),
        &[
            series_xy(0, "Pos", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[10.0, 12.0, 14.0, 16.0, 18.0, 20.0]),
            series_xy(1, "ZeroHit", &[-2.0, -1.0, 0.0, 1.0, 2.0, 3.0], &[-4.0, -2.0, 0.0, 2.0, 4.0, 6.0]),
        ],
        &SolverConfig {
            method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
            outlier_handling: OutlierHandling::Eliminate,
            weighting: WeightingScheme::RelativeSquare,
            weighting_source: WeightingSource::PredictedY,
            ..cfg_fast()
        },
    );
    assert_eq!((fit_imp.status, fit_imp.rout.is_none(), fit_imp.n_outliers_excluded), (FitStatus::ImpossibleWeights, true, 0));

    // AUD-03-09: PredictedY AICc & LRT include exact -sum ln w_i = sum 2 ln|y_hat_i| Jacobian term.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let noise = [0.1, -0.2, 0.15, -0.1, 0.25, -0.3, 0.2, -0.4, 0.3, -0.2];
    let ys: Vec<f64> = xs.iter().zip(noise).map(|(&x, e)| 5.0 + 2.0 * x + 0.15 * x * x + e * (1.0 + 0.1 * x)).collect();
    let ds = [series_xy(0, "Curve", &xs, &ys)];
    let cfg_py = SolverConfig { weighting: WeightingScheme::RelativeSquare, weighting_source: WeightingSource::PredictedY, ..cfg_fast() };
    let (fit_lin, fit_quad) = (fit_with(m_lin.as_ref(), &ds, &cfg_py), fit_with(m_quad.as_ref(), &ds, &cfg_py));
    let (nlw_lin, nlw_quad) = (fit_lin.predicted_y_neg_log_weight_sum.unwrap(), fit_quad.predicted_y_neg_log_weight_sum.unwrap());
    let exp_nlw: f64 = fit_lin.datasets[0].y_hat.iter().map(|yh| 2.0 * yh.abs().ln()).sum();
    assert_approx_eq(nlw_lin, exp_nlw, 1e-10, 1e-10, "neg_log_w_sum");
    let (n, k_ic) = (fit_lin.n_obs as f64, (fit_lin.k_free + 1) as f64);
    let exp_aicc = n * (fit_lin.weighted_ss / n).ln() + nlw_lin + 2.0 * k_ic + (2.0 * k_ic * (k_ic + 1.0)) / (n - k_ic - 1.0);
    assert_approx_eq(fit_lin.aic_c.unwrap(), exp_aicc, 1e-9, 1e-9, "PredictedY AICc");

    let (_, ModelComparisonOutcome::Aicc(aicc)) = compare_models(
        &fit_lin,
        m_quad.as_ref(),
        &ds,
        &HookContext::default(),
        &cfg_py,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "quadratic".into() },
    )
    .unwrap() else {
        panic!("expected Aicc");
    };
    assert_approx_eq(aicc.aicc_a, fit_lin.aic_c.unwrap(), 1e-9, 1e-9, "compare aicc_a");
    assert_approx_eq(aicc.aicc_b, fit_quad.aic_c.unwrap(), 1e-9, 1e-9, "compare aicc_b");
    let (_, ModelComparisonOutcome::LikelihoodRatioTest(lrt)) = compare_models(
        &fit_lin,
        m_quad.as_ref(),
        &ds,
        &HookContext::default(),
        &cfg_py,
        &ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "quadratic".into() },
    )
    .unwrap() else {
        panic!("expected LRT");
    };
    let exp_d = (n * (fit_lin.weighted_ss / n).ln() + nlw_lin) - (n * (fit_quad.weighted_ss / n).ln() + nlw_quad);
    assert_approx_eq(lrt.lr_statistic, exp_d.max(0.0), 1e-8, 1e-8, "PredictedY LRT");

    // AUD-03-12: ObservedSd keeps CI bands while omitting dimensionally invalid prediction bands.
    let pts: Vec<DataPoint> = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
        .iter()
        .enumerate()
        .map(|(row, &x)| DataPoint {
            row,
            sd: Some(50.0),
            n: Some(3.0),
            ..DataPoint::xy(x, 1000.0 + 200.0 * x + if row % 2 == 0 { 15.0 } else { -15.0 })
        })
        .collect();
    let mut ds_sd = series_from_pts(0, "ObsSd", pts);
    ds_sd.is_pre_summarized = true;
    ds_sd.unknowns_y.push(UnknownResponse::Single((6, 1700.0)));
    let fit_sd = fit_with(
        m_lin.as_ref(),
        &[ds_sd],
        &SolverConfig {
            weighting: WeightingScheme::ObservedSd,
            compute_bands: true,
            compute_prediction_bands: true,
            interpolate_unknowns: true,
            ..cfg_fast()
        },
    );
    let ov = fit_sd.datasets[0].curve_overlay.as_ref().unwrap();
    assert!(ov.points.iter().any(|p| p.ci_lower.is_some() && p.ci_upper.is_some()));
    assert!(ov.points.iter().all(|p| p.pred_lower.is_none() && p.pred_upper.is_none()));
    assert_eq!((fit_sd.interpolated_x[0].pred_lower, fit_sd.interpolated_x[0].pred_upper), (CiBound::Open, CiBound::Open));

    // AUD-03-23: IRLS converges when optimal YIntercept == 0.0 under PredictedY RelativeSquare & RobustCauchy.
    let xs_z = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let ds_z = [series_xy(0, "ZeroInt", &xs_z, &xs_z.map(|x| 2.0 * x))];
    for (method, weighting, source) in [
        (FitMethod::LeastSquares, WeightingScheme::RelativeSquare, WeightingSource::PredictedY),
        (FitMethod::RobustCauchy, WeightingScheme::None, WeightingSource::ObservedY),
    ] {
        let fit = fit_with(m_lin.as_ref(), &ds_z, &SolverConfig { method, weighting, weighting_source: source, ..cfg_fast() });
        assert_converged(&fit, "zero-optimum IRLS");
        assert_approx_eq(param_val(&fit, "YIntercept"), 0.0, 1e-6, 1e-6, "YIntercept");
        assert_approx_eq(param_val(&fit, "Slope"), 2.0, 1e-6, 1e-6, "Slope");
    }
}

#[test]
fn aud_03_21_24_35_rout_row_means_union_and_shared_robust_rsdr() {
    let (m_lin, m_quad) = (get_builtin_model("straight_line").unwrap(), get_builtin_model("quadratic").unwrap());

    // AUD-03-21: ROUT RowMeansOnly rejects pre-summarized and eliminates/excludes all 3 replicate subcols on outlier row.
    let mut pre = series_xy(0, "Pre", &[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.0, 6.0, 8.0, 10.0]);
    pre.is_pre_summarized = true;
    assert!(matches!(
        fit_nonlinear(
            m_lin.as_ref(),
            &[pre],
            &HookContext::default(),
            &SolverConfig {
                method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
                outlier_handling: OutlierHandling::Eliminate,
                replicate_fit_mode: ReplicateFitMode::RowMeansOnly,
                ..cfg_fast()
            },
        ),
        Err(FitError::RequiresRawReplicates)
    ));
    let xs8 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let reps: [[f64; 3]; 8] = std::array::from_fn(|r| {
        let base = if r == 4 { 500.0 } else { 2.0 * xs8[r] + 1.0 };
        [base - 0.05, base, base + 0.05]
    });
    let mut nl_r = nl_fast("straight_line");
    nl_r.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    nl_r.outlier_handling = OutlierHandling::Eliminate;
    nl_r.replicate_fit_mode = ReplicateFitMode::RowMeansOnly;
    nl_r.emit_cleaned_data_tab = true;
    let res_r = exec_nl(&xy_sheet_replicates("xy-rep-rout", "RepRout", &xs8, &[("Rep", &reps)]), nl_r);
    assert_eq!(res_r.status, ResultStatus::UpToDate);
    assert_eq!(res_r.eliminated_outliers.iter().filter(|c| c.row == 4 && c.col == 0).map(|c| c.subcol).collect::<Vec<_>>(), vec![0, 1, 2]);
    let Some(DataSheet { archetype: TableArchetype::Xy(clean_xy), .. }) = &res_r.analyzable_data else { panic!("expected cleaned XY") };
    assert!((0..3).all(|s| clean_xy.data_sets[0].cells[4][s].excluded));

    // AUD-03-24: IdentifyAndCountOnly permits compare_models; Eliminate via compare_models_question synchronizes O_1 U O_2.
    let xs11 = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let mut ys11: Vec<f64> = xs11.iter().map(|&x| 1.0 + 0.5 * x + 0.4 * x * x).collect();
    ys11[5] += 80.0;
    let ds11 = [series_xy(0, "Q", &xs11, &ys11)];
    let cfg_id = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        outlier_handling: OutlierHandling::IdentifyAndCountOnly,
        ..cfg_fast()
    };
    let primary_id = fit_with(m_lin.as_ref(), &ds11, &cfg_id);
    assert_eq!(primary_id.n_outliers_excluded, 0);
    let (_, out_id) = compare_models(
        &primary_id,
        m_quad.as_ref(),
        &ds11,
        &HookContext::default(),
        &cfg_id,
        &ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "quadratic".into() },
    )
    .unwrap();
    assert!(matches!(out_id, ModelComparisonOutcome::ExtraSumOfSquaresF(_)));
    let cfg_el =
        SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, outlier_handling: OutlierHandling::Eliminate, ..cfg_fast() };
    let primary_el = fit_with(m_lin.as_ref(), &ds11, &cfg_el);
    let q_res = compare_models_question(
        m_lin.as_ref(),
        &primary_el,
        &ds11,
        &HookContext::default(),
        &cfg_el,
        ModelComparisonMethod::ExtraSumOfSquaresF,
        &ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: "quadratic".into() },
        false,
        None,
    )
    .unwrap();
    assert_eq!((q_res.model_a_fit.n_obs, q_res.model_a_fit.n_outliers_excluded), (q_res.model_b_fit.n_obs, q_res.model_b_fit.n_outliers_excluded));

    // AUD-03-35: shared global RobustCauchy apportions k_eff_d in ds_rsdr and retains Cauchy weight under PredictedY.
    let xs5 = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ds4: Vec<DatasetSeries> = (0..4)
        .map(|d| {
            series_xy(
                d,
                &format!("D{d}"),
                &xs5,
                &xs5.iter().enumerate().map(|(i, &x)| 2.0 * x + 5.0 + if i == 2 { 0.5 } else { 0.1 * ((i + d) as f64 - 2.0) }).collect::<Vec<_>>(),
            )
        })
        .collect();
    let fit_sh = fit_with(
        m_lin.as_ref(),
        &ds4,
        &SolverConfig { method: FitMethod::RobustCauchy, parameter_rules: vec![rule_shared("Slope"), rule_shared("YIntercept")], ..cfg_fast() },
    );
    for d in &fit_sh.datasets {
        let p68 = compute_rsdr(&d.residuals.iter().map(|r| r.abs()).collect::<Vec<_>>(), 0);
        assert_approx_eq(d.rsdr.unwrap(), p68 * (5.0 / 4.5), 1e-12, 1e-12, "apportioned ds_rsdr");
    }
    let mut ys_out = [3.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0, 17.0];
    ys_out[7] = 200.0;
    let fit_py = fit_with(
        m_lin.as_ref(),
        &[series_xy(0, "Out", &xs8, &ys_out)],
        &SolverConfig {
            method: FitMethod::RobustCauchy,
            weighting: WeightingScheme::Relative,
            weighting_source: WeightingSource::PredictedY,
            ..cfg_fast()
        },
    );
    let base_w = fit_py.datasets[0].residuals[7].abs() / fit_py.datasets[0].y_hat[7].abs().sqrt();
    assert!(fit_py.datasets[0].weighted_residuals[7].abs() < base_w * 0.5, "Cauchy weight must downweight outlier");
}
