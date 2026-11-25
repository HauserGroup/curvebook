#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Reproducer and D1 reference-oracle suite for `R4.6` solver & results fixes in
//! [`curvebook_fit::solver`] and [`curvebook_fit::result`].
//! References: Akaike (1974); Hurvich & Tsai (1989); Schwarz (1978); Burnham & Anderson (2002);
//! McCullagh & Nelder (1989); Draper & Smith (1998); Seber & Wild (1989); Seber & Lee (2003);
//! Motulsky & Brown (2006); Benjamini & Hochberg (1995); Moré (1978).

use curvebook_doc::{
    Cell, CiBound, DataSheet, FitMethod, LinearRegressionSpec, ModelComparisonSpec, NonlinearFitSpec, ResultCell, SubcolumnMode, TableArchetype,
    WeightingScheme, XyTable,
};
use curvebook_fit::{
    DataPoint, DatasetSeries, FitError, FitStatus, HookContext, ModelComparisonOutcome, SolverConfig, compare_models, compare_models_checked,
    get_builtin_model,
};
use curvebook_testkit::assert_approx_eq as assert_close;

use crate::common::{
    cfg_rules, dsl_compile, exec_lin, exec_nl, find_tab, fit_default, fit_with, mean_sd_n, nl_fast, param_val, rule_const, rule_local,
    rule_local_bounds, series_from_pts, series_xy, unwrap_finite_ci,
};

#[test]
fn r46_2_poisson_aic_aicc_bic_and_model_comparison_reproducer_and_oracle() {
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];
    let ys = [100.0, 74.0, 55.0, 41.0, 30.0, 22.0, 16.0, 9.0, 5.0, 3.0];
    let ds = [series_xy(0, "Counts", &xs, &ys)];

    let exp2 = dsl_compile("Y = A * exp(-K * X)");
    let decay3 = get_builtin_model("one_phase_decay").unwrap();
    let pois_cfg = SolverConfig { method: FitMethod::PoissonDeviance, compute_profile_ci: false, ..Default::default() };
    let fit2 = fit_with(&exp2, &ds, &pois_cfg);
    assert_eq!(fit2.status, FitStatus::Converged);

    let ll: f64 =
        ys.iter().zip(&fit2.datasets[0].y_hat).map(|(&y, &mu)| y * mu.ln() - mu - curvebook_math::special::ln_gamma(y + 1.0).unwrap()).sum();
    let (neg2_ll, k, n) = (-2.0 * ll, 2.0_f64, 10.0_f64);
    let exp_aicc = neg2_ll + 2.0 * k + (2.0 * k * (k + 1.0)) / (n - k - 1.0);
    let exp_bic = neg2_ll + k * n.ln();
    assert!(fit2.aic_c.unwrap() > 50.0);
    assert_close(fit2.aic_c.unwrap(), exp_aicc, 1e-10, 1e-10, "Poisson AICc");
    assert_close(fit2.bic.unwrap(), exp_bic, 1e-10, 1e-10, "Poisson BIC");

    let hooks = HookContext::default();
    let f_spec = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "one_phase_decay".to_owned() };
    assert!(matches!(
        compare_models(&fit2, decay3.as_ref(), &ds, &hooks, &pois_cfg, &f_spec),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));
    assert!(matches!(
        compare_models_checked(&fit2, decay3.as_ref(), &ds, &hooks, &pois_cfg, &f_spec),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));

    let ls_cfg = SolverConfig { method: FitMethod::LeastSquares, compute_profile_ci: false, ..Default::default() };
    let lrt_spec = ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: "one_phase_decay".to_owned() };
    assert!(matches!(
        compare_models(&fit2, decay3.as_ref(), &ds, &hooks, &ls_cfg, &lrt_spec),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));

    let (fit3, aicc_cmp) = compare_models(
        &fit2,
        decay3.as_ref(),
        &ds,
        &hooks,
        &pois_cfg,
        &ModelComparisonSpec::Aicc { alternative_equation_id: "one_phase_decay".to_owned() },
    )
    .unwrap();
    let ModelComparisonOutcome::Aicc(a) = aicc_cmp else { panic!("expected Aicc") };
    let exp_aicc_3 = fit3.aic_c.unwrap();
    assert_close(a.aicc_a, exp_aicc, 1e-10, 1e-10, "compare AICc A");
    assert_close(a.aicc_b, exp_aicc_3, 1e-10, 1e-10, "compare AICc B");
    assert_close(a.delta_aicc, exp_aicc_3 - exp_aicc, 1e-10, 1e-10, "delta AICc");
    assert_eq!(a.preferred_index, 0);

    let ds_plat = [series_xy(0, "CountsPlateau", &xs, &[120.0, 94.0, 75.0, 61.0, 50.0, 42.0, 36.0, 29.0, 25.0, 23.0])];
    let fit2_p = fit_with(&exp2, &ds_plat, &pois_cfg);
    let (fit3_p, lrt_cmp) = compare_models(&fit2_p, decay3.as_ref(), &ds_plat, &hooks, &pois_cfg, &lrt_spec).unwrap();
    let ModelComparisonOutcome::LikelihoodRatioTest(lrt) = lrt_cmp else { panic!("expected LRT") };
    assert_eq!(lrt.df_diff, 1);
    assert_close(lrt.lr_statistic, (fit2_p.weighted_ss - fit3_p.weighted_ss).max(0.0), 1e-10, 1e-10, "Deviance LRT");
    assert!(lrt.preferred_complex_model && lrt.p_value < 0.01);
}

#[test]
fn r46_3_replicates_lack_of_fit_weighted_and_presummarized_oracles() {
    let line = get_builtin_model("straight_line").unwrap();
    let rep_xs = [1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 8.0, 8.0, 8.0];
    let cfg_wls =
        SolverConfig { weighting: WeightingScheme::RelativeSquare, compute_lack_of_fit: true, compute_profile_ci: false, ..Default::default() };

    let fit_c = fit_with(
        line.as_ref(),
        &[series_xy(0, "WlsCurved", &rep_xs, &[126.5, 127.0, 127.5, 203.0, 204.0, 205.0, 506.0, 508.0, 510.0, 1710.0, 1716.0, 1722.0])],
        &cfg_wls,
    );
    let lof_c = fit_c.lack_of_fit.unwrap();
    assert_eq!((lof_c.df_pure_error, lof_c.df_lack_of_fit), (8, 2));
    assert_close(lof_c.ss_pure_error, 0.000_134_510_441, 1e-8, 1e-6, "curved ss_pe_w");
    assert_close(lof_c.ss_lack_of_fit, 0.610_531_757_608, 1e-8, 1e-6, "curved ss_lof_w");
    assert_close(lof_c.ss_pure_error + lof_c.ss_lack_of_fit, fit_c.weighted_ss, 1e-10, 1e-10, "curved partition");
    assert_close(lof_c.f_stat, 18_155.668_852, 1e-2, 1e-5, "curved F");
    assert!(lof_c.significant_lack_of_fit && lof_c.p_value < 1e-12);

    let fit_l = fit_with(
        line.as_ref(),
        &[series_xy(0, "WlsLinear", &rep_xs, &[14.5, 15.0, 15.5, 24.0, 25.0, 26.0, 43.0, 45.0, 47.0, 81.0, 85.0, 89.0])],
        &cfg_wls,
    );
    let lof_l = fit_l.lack_of_fit.unwrap();
    assert_close(lof_l.ss_pure_error, 0.013_810_289_595, 1e-9, 1e-6, "lin ss_pe_w");
    assert_close(lof_l.ss_lack_of_fit, 8.549_263_765_673e-9, 1e-11, 1e-4, "lin ss_lof_w");
    assert_close(lof_l.ss_pure_error + lof_l.ss_lack_of_fit, fit_l.weighted_ss, 1e-12, 1e-12, "lin partition");
    assert!(!lof_l.significant_lack_of_fit);

    let raw_groups: [(f64, &[f64]); 4] = [(1.0, &[2.0, 4.0, 6.0]), (1.0, &[3.0, 5.0]), (2.0, &[8.0, 9.0, 10.0]), (4.0, &[16.0, 18.0, 20.0])];
    let (mut raw_pts, mut sum_pts) = (Vec::new(), Vec::new());
    for (r, (x, ys)) in raw_groups.into_iter().enumerate() {
        for &y in ys {
            raw_pts.push(DataPoint { row: raw_pts.len(), ..DataPoint::xy(x, y) });
        }
        let (m, sd, n) = mean_sd_n(ys);
        sum_pts.push(DataPoint { sd: Some(sd), n: Some(n), row: r, ..DataPoint::xy(x, m) });
    }
    let mut ds_sum = series_from_pts(0, "Sum", sum_pts);
    ds_sum.is_pre_summarized = true;
    let cfg_unw = SolverConfig { compute_lack_of_fit: true, compute_profile_ci: false, ..Default::default() };
    let lof_raw = fit_with(line.as_ref(), &[series_from_pts(0, "Raw", raw_pts)], &cfg_unw).lack_of_fit.unwrap();
    let lof_sum = fit_with(line.as_ref(), &[ds_sum], &cfg_unw).lack_of_fit.unwrap();
    assert_eq!((lof_sum.df_pure_error, lof_sum.df_lack_of_fit), (lof_raw.df_pure_error, lof_raw.df_lack_of_fit));
    assert_close(lof_sum.ss_pure_error, lof_raw.ss_pure_error, 1e-10, 1e-10, "sum vs raw ss_pe");
    assert_close(lof_sum.ss_lack_of_fit, lof_raw.ss_lack_of_fit, 1e-10, 1e-10, "sum vs raw ss_lof");
    assert_close(lof_sum.f_stat, lof_raw.f_stat, 1e-10, 1e-10, "sum vs raw F");
    assert_close(lof_sum.p_value, lof_raw.p_value, 1e-10, 1e-10, "sum vs raw p");

    let sum_uniq: Vec<DataPoint> = [(1.0, 4.0, 1.5, 3.0), (2.0, 9.0, 1.0, 3.0), (3.0, 14.0, 1.2, 3.0), (4.0, 19.0, 1.8, 3.0)]
        .into_iter()
        .enumerate()
        .map(|(r, (x, y, sd, n))| DataPoint { sd: Some(sd), n: Some(n), row: r, ..DataPoint::xy(x, y) })
        .collect();
    let mut ds_obs = series_from_pts(0, "ObsSd", sum_uniq);
    ds_obs.is_pre_summarized = true;
    assert!(fit_with(line.as_ref(), &[ds_obs], &SolverConfig { weighting: WeightingScheme::ObservedSd, ..cfg_unw }).lack_of_fit.is_none());
}

#[test]
fn r46_4_rout_motulsky_brown_2006_top_30_percent_bh_reproducer_and_oracle() {
    let mut is_elevated = [false; 100];
    for k in 0..15 {
        is_elevated[k] = true;
        is_elevated[99 - k] = true;
    }
    is_elevated[49] = true;
    let xs: Vec<f64> = (0..100).map(|i| (i as f64) - 49.5).collect();
    let ys: Vec<f64> = (0..100)
        .map(|i| {
            let eps = if is_elevated[i] {
                if i == 49 {
                    1.320
                } else if i < 50 {
                    if i % 2 == 0 { 1.320 } else { -1.320 }
                } else if (99 - i) % 2 == 0 {
                    -1.320
                } else {
                    1.320
                }
            } else if i % 2 == 0 {
                0.50
            } else {
                -0.50
            };
            10.0 + 2.0 * xs[i] + eps
        })
        .collect();

    let line = get_builtin_model("straight_line").unwrap();
    let cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 3.6 },
        compute_profile_ci: false,
        compute_bands: false,
        ..Default::default()
    };
    let fit = fit_with(line.as_ref(), &[series_xy(0, "Rout31pct", &xs, &ys)], &cfg);
    assert_eq!(fit.n_outliers_excluded, 0);
    assert!(fit.rout.as_ref().unwrap().audit.iter().all(|a| !a.is_outlier));

    let mut ys_mixed = ys;
    ys_mixed[0] += 50.0;
    ys_mixed[1] -= 50.0;
    ys_mixed[99] += 60.0;
    let fit_m = fit_with(line.as_ref(), &[series_xy(0, "RoutMixed", &xs, &ys_mixed)], &cfg);
    let rout_m = fit_m.rout.as_ref().unwrap();
    assert_eq!((fit_m.n_outliers_excluded, &rout_m.outlier_indices), (3, &vec![0, 1, 99]));
    assert!(rout_m.audit.iter().filter(|a| (a.q_value - 1.0).abs() < 1e-12 && !a.is_outlier).count() >= 70);
}

#[test]
fn r46_6_inverse_interpolation_calibration_vs_mean_response_and_deming_note() {
    let (cal_x, cal_y) = ([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], [3.1, 4.8, 7.3, 8.7, 11.4, 12.6, 15.2, 16.9]);
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, cal_x.len() + 2).unwrap();
    for (r, (&x, &y)) in cal_x.iter().zip(&cal_y).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    xy.data_sets[0].cells[8][0] = Cell::number(10.0);
    xy.set_x_cell(9, Cell::number(4.5)).unwrap();
    let sheet = DataSheet::new("cal-sheet", "Calibration", TableArchetype::Xy(xy)).unwrap();

    let lin_res = exec_lin(&sheet, LinearRegressionSpec { interpolate_unknowns: true, ..Default::default() });
    let lin_tab = find_tab(&lin_res, "linear_interpolated");
    assert_eq!(lin_tab.column_headers, vec!["Input", "Interpolated", "Mean-Response CI", "Calibration / Prediction Interval"]);
    let (
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(l_cl), upper: CiBound::Finite(l_ch) },
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(l_pl), upper: CiBound::Finite(l_ph) },
    ) = (&lin_tab.rows[0][2], &lin_tab.rows[0][3])
    else {
        panic!("finite linear CI/PI")
    };
    assert_close(*l_cl, 4.362_750_432_716_67, 1e-10, 1e-10, "lin CI lo");
    assert_close(*l_ch, 4.637_249_567_283_33, 1e-10, 1e-10, "lin CI hi");
    assert_close(*l_pl, 4.088_251_298_150_02, 1e-10, 1e-10, "lin PI lo");
    assert_close(*l_ph, 4.911_748_701_849_98, 1e-10, 1e-10, "lin PI hi");
    assert!(*l_pl < *l_cl && *l_ch < *l_ph);

    let nl_res = exec_nl(&sheet, NonlinearFitSpec { interpolate_unknowns: true, ..nl_fast("straight_line") });
    let nl_tab = find_tab(&nl_res, "nonlinear_interpolated");
    assert_eq!(nl_tab.column_headers, vec!["Input", "Interpolated", "Mean-Response CI", "Calibration / Prediction Interval", "Direction", "Dataset"]);
    let (
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(n_cl), upper: CiBound::Finite(n_ch) },
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(n_pl), upper: CiBound::Finite(n_ph) },
    ) = (&nl_tab.rows[0][2], &nl_tab.rows[0][3])
    else {
        panic!("finite nl CI/PI")
    };
    assert_close(*n_cl, *l_cl, 1e-8, 1e-8, "nl CI lo");
    assert_close(*n_ch, *l_ch, 1e-8, 1e-8, "nl CI hi");
    assert_close(*n_pl, *l_pl, 1e-8, 1e-8, "nl PI lo");
    assert_close(*n_ph, *l_ph, 1e-8, 1e-8, "nl PI hi");

    let dem_res = exec_lin(&sheet, LinearRegressionSpec { deming_model_ii: true, ..Default::default() });
    assert!(find_tab(&dem_res, "deming_summary").footnotes.iter().any(|f| f.contains("1.0") && f.contains("lambda")));

    let line_m = get_builtin_model("straight_line").unwrap();
    let mut ds_w = series_xy(0, "WlsCal", &[1.0, 2.0, 4.0, 8.0, 16.0], &[5.1, 9.8, 20.5, 39.2, 81.4]);
    ds_w.unknowns_y.push((5, 30.0).into());
    let cfg_w = SolverConfig {
        weighting: WeightingScheme::RelativeSquare,
        interpolate_unknowns: true,
        compute_profile_ci: false,
        compute_bands: true,
        compute_prediction_bands: true,
        ..Default::default()
    };
    let fit_w = fit_with(line_m.as_ref(), std::slice::from_ref(&ds_w), &cfg_w);
    let unk = &fit_w.interpolated_x[0];
    let (ci_lo, ci_hi) = unwrap_finite_ci(unk.ci_bounds_option().unwrap(), "WLS CI");
    let (pi_lo, pi_hi) = unwrap_finite_ci(unk.pred_bounds_option().unwrap(), "WLS PI");
    assert!(pi_lo < ci_lo && ci_lo < unk.output_estimate && unk.output_estimate < ci_hi && ci_hi < pi_hi);

    let ds_fwd = DatasetSeries { unknowns_y: Vec::new(), unknowns_x: vec![(0, pi_lo), (1, pi_hi)], ..ds_w };
    let fit_fwd = fit_with(line_m.as_ref(), &[ds_fwd], &cfg_w);
    let (_, fwd_hi) = unwrap_finite_ci(fit_fwd.interpolated_y[0].pred_bounds_option().unwrap(), "fwd hi");
    let (fwd_lo, _) = unwrap_finite_ci(fit_fwd.interpolated_y[1].pred_bounds_option().unwrap(), "fwd lo");
    assert_close(fwd_hi, 30.0, 1e-6, 1e-6, "WLS lower duality");
    assert_close(fwd_lo, 30.0, 1e-6, 1e-6, "WLS upper duality");
}

#[test]
fn r46_7_centered_poly_default_parameter_rules_wiring_oracle() {
    let skew_xs = [100.0, 100.5, 101.0, 101.5, 102.0, 102.6, 103.3, 104.1, 105.0, 106.0, 107.2, 108.6];
    let mean_x = skew_xs.iter().sum::<f64>() / (skew_xs.len() as f64);
    let coeffs = [12.5, -1.8, 0.25, -0.02, 0.005, -0.0005, 0.0001];

    for deg in 1..=6 {
        let id = format!("centered_poly_{deg}");
        let model = get_builtin_model(&id).unwrap();
        let ys: Vec<f64> = skew_xs.iter().map(|&x| (0..=deg).fold(0.0, |acc, k| acc + coeffs[k] * (x - mean_x).powi(k as i32))).collect();
        let fit = fit_default(model.as_ref(), &[series_xy(0, &id, &skew_xs, &ys)]);
        assert_eq!(
            (fit.status, fit.k_free, fit.covariance.numerical_rank, fit.covariance.ambiguous),
            (FitStatus::Converged, deg + 1, deg + 1, false)
        );
        assert!(fit.parameters.iter().all(|p| p.se.is_finite()));
        assert!(fit.fixed_constants.iter().any(|(n, v, _)| n == "XCenter" && (*v - mean_x).abs() < 1e-10));
        for (k, &exp) in coeffs.iter().enumerate().take(deg + 1) {
            assert_close(param_val(&fit, &format!("B{k}")), exp, 1e-5, 1e-5, &format!("{id} B{k}"));
        }
    }

    let poly2 = get_builtin_model("centered_poly_2").unwrap();
    let ys_custom: Vec<f64> = skew_xs.iter().map(|&x| 4.0 + 1.5 * (x - 120.0) - 0.05 * (x - 120.0).powi(2)).collect();
    let ds_c = [series_xy(0, "CustomXCenter", &skew_xs, &ys_custom)];

    let fit_c = fit_with(poly2.as_ref(), &ds_c, &cfg_rules(vec![rule_const("XCenter", 120.0)]));
    assert_eq!(fit_c.status, FitStatus::Converged);
    assert!(fit_c.fixed_constants.iter().any(|(n, v, _)| n == "XCenter" && (*v - 120.0).abs() < 1e-12));
    assert_close(param_val(&fit_c, "B0"), 4.0, 1e-6, 1e-6, "override B0");

    let fit_l = fit_with(poly2.as_ref(), &ds_c, &cfg_rules(vec![rule_local("XCenter")]));
    assert_eq!((fit_l.status, fit_l.k_free), (FitStatus::Ambiguous, 4));
    assert!(fit_l.covariance.numerical_rank < 4);
}

#[test]
fn r46_8_more_newton_trust_region_lm_convergence_oracles() {
    let model = get_builtin_model("one_site_specific_binding").unwrap();
    let xs = [1e-4, 5e-4, 1e-3, 2e-3, 4e-3, 8e-3, 2e-2, 5e-2, 0.1, 0.5];
    let (bmax, kd) = (250_000.0_f64, 0.004_f64);
    let ys: Vec<f64> = xs.iter().map(|&x| bmax * x / (kd + x)).collect();
    let cfg = SolverConfig {
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_bounds("Bmax", Some(10_000.0), Some(0.0), None), rule_local_bounds("Kd", Some(0.2), Some(1e-9), None)],
        ..Default::default()
    };
    let fit = fit_with(model.as_ref(), &[series_xy(0, "MultiScale", &xs, &ys)], &cfg);
    assert_eq!(fit.status, FitStatus::Converged);
    assert_close(param_val(&fit, "Bmax"), bmax, 1e-4, 1e-8, "Moré Bmax");
    assert_close(param_val(&fit, "Kd"), kd, 1e-10, 1e-8, "Moré Kd");
    assert!(fit.rss < 1e-8);

    let line = get_builtin_model("straight_line").unwrap();
    let cfg_b = SolverConfig {
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_bounds("YIntercept", Some(5.0), Some(2.0), None)],
        ..Default::default()
    };
    let fit_b = fit_with(line.as_ref(), &[series_xy(0, "Constrained", &[0.0, 1.0, 2.0, 3.0, 4.0], &[1.0, 3.0, 5.0, 7.0, 9.0])], &cfg_b);
    assert_eq!(fit_b.status, FitStatus::Converged);
    assert_close(param_val(&fit_b, "YIntercept"), 2.0, 1e-10, 1e-10, "bounded intercept");
    assert_close(param_val(&fit_b, "Slope"), 5.0 / 3.0, 1e-10, 1e-10, "constrained slope");
}
