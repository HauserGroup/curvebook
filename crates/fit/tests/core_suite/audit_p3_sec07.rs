#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Phase 3 SEC-07 regression tests (`FA-SEC07-01` .. `FA-SEC07-16`).

use curvebook_doc::{
    Cell, ChecklistStatus, CiBound, DataSheet, ErrorCode, FitMethod, GroupedTable, LinearRegressionSpec, ModelComparisonSpec, MultipleVariablesTable,
    NonlinearFitSpec, ResultCell, ResultStatus, SheetId, SubcolumnMode, TableArchetype, VariableRole, WeightingScheme, XColumnMode, XyTable,
};
use curvebook_fit::diagnostics::{compare_models_f_test, compute_information_criteria, rout_detect_outliers};
use curvebook_fit::error::{FitError, FitIdentifiability, FitStatus};
use curvebook_fit::get_builtin_model;
use curvebook_fit::linear::{DemingOptions, SimpleLinearOptions, compare_linear_lines, deming_regression, simple_linear_regression};
use curvebook_fit::solver::{SolverConfig, compare_models, compare_models_checked, fit_nonlinear};
use curvebook_fit::tex_format::format_model_equation_tex;
use curvebook_fit::types::{DataPoint, HookContext, MultivarExtractionSpec, extract_datasets_from_sheet, extract_datasets_from_sheet_with_options};
use curvebook_testkit::assert_approx_eq as assert_close;

use crate::common::{
    dsl_compile, exec_lin, exec_lin_res, exec_nl, find_tab, fit_default, fit_with, mean_sd_n, mv_sheet_num, nl_fast, param_val, rule_const,
    rule_local_bounds, rule_local_init, series_xy, xy_pts, xy_sheet, xy_sheet_single, xy_sheet_staggered,
};

#[test]
fn fa_sec07_01_unconverged_and_not_assessed_identifiability() {
    let xs = [0.0_f64, 1.0, 2.0, 4.0, 8.0, 12.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 10.0 + 90.0 * (-0.35 * x).exp()).collect();
    let sheet = xy_sheet_single("xy-01", "Decay", "Obs", &xs, &ys);

    let rs_capped = exec_nl(&sheet, NonlinearFitSpec { max_iters: 1, identify_unstable_parameters: true, ..nl_fast("one_phase_decay") });
    assert!(matches!(rs_capped.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }), "got {:?}", rs_capped.status);
    assert!(rs_capped.checklist.iter().any(|c| c.assumption == "Solver convergence" && c.status == ChecklistStatus::Violated));
    assert!(!rs_capped.checklist.iter().any(|c| c.assumption == "Parameter identifiability" && c.status == ChecklistStatus::Pass));

    let rs_no_id = exec_nl(&sheet, NonlinearFitSpec { identify_unstable_parameters: false, ..nl_fast("one_phase_decay") });
    assert_eq!(rs_no_id.status, ResultStatus::UpToDate);
    assert!(rs_no_id.checklist.iter().any(|c| c.assumption == "Parameter identifiability" && c.status == ChecklistStatus::Info));

    let div_model = dsl_compile("Y = A * X + 0.0 * sqrt(1.0 - A)");
    let div_cfg = SolverConfig {
        identify_unstable_parameters: true,
        compute_profile_ci: false,
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_init("A", 0.99)],
        ..SolverConfig::default()
    };
    let div_fit = fit_with(&div_model, &[series_xy(0, "Stalled", &[1.0, 2.0, 3.0, 4.0], &[3.0, 6.0, 9.0, 12.0])], &div_cfg);
    assert_eq!((div_fit.status, div_fit.identifiability), (FitStatus::Diverged, FitIdentifiability::NotAssessed));

    let multi_fit = fit_with(
        &div_model,
        &[
            series_xy(0, "CleanA", &[1.0, 2.0, 3.0, 4.0, 5.0], &[0.5, 1.0, 1.5, 2.0, 2.5]),
            series_xy(1, "WallA", &[1.0, 2.0, 3.0, 4.0, 5.0], &[3.0, 6.0, 9.0, 12.0, 15.0]),
        ],
        &div_cfg,
    );
    assert_eq!((multi_fit.status, multi_fit.identifiability), (FitStatus::Diverged, FitIdentifiability::NotAssessed));
}

#[test]
fn fa_sec07_02_irls_outer_convergence_iterations_and_poisson_deviance_guard() {
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let ds_c = [series_xy(0, "CauchyCap", &[0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[100.0, 72.0, 55.0, 31.0, 20.0, 14.0, 11.5, 10.2])];
    let cfg_cauchy = SolverConfig {
        method: FitMethod::RobustCauchy,
        max_iters: 1,
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_init("Y0", 25.0)],
        ..SolverConfig::default()
    };
    let res_c = fit_with(decay.as_ref(), &ds_c, &cfg_cauchy);
    assert_eq!(res_c.status, FitStatus::HitMaxIters);
    assert!(res_c.iterations >= 2, "got {}", res_c.iterations);

    let sigmoid = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let ds_p = [series_xy(0, "Counts", &[-8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0], &[5.0, 8.0, 22.0, 52.0, 82.0, 95.0, 99.0])];
    let cfg_pois_cap = SolverConfig {
        method: FitMethod::PoissonDeviance,
        max_iters: 1,
        use_simplex_warm_start: false,
        parameter_rules: [("Bottom", 1.0), ("Top", 40.0), ("LogEC50", -7.5), ("HillSlope", 0.5)]
            .into_iter()
            .map(|(n, v)| rule_local_init(n, v))
            .collect(),
        ..SolverConfig::default()
    };
    let res_p_cap = fit_with(sigmoid.as_ref(), &ds_p, &cfg_pois_cap);
    assert_eq!(res_p_cap.status, FitStatus::HitMaxIters);
    assert!(res_p_cap.iterations >= 2, "got {}", res_p_cap.iterations);

    let res_p_ok = fit_with(sigmoid.as_ref(), &ds_p, &SolverConfig { method: FitMethod::PoissonDeviance, ..SolverConfig::default() });
    assert_eq!(res_p_ok.status, FitStatus::Converged);
    assert!(res_p_ok.weighted_ss.is_finite() && res_p_ok.weighted_ss >= 0.0);
}

#[test]
fn fa_sec07_03_per_dataset_default_parameter_rules_xcenter_preserved() {
    let sheet = xy_sheet_staggered(
        "xy-xc",
        "CenteredMulti",
        &[("LowX", &[0.0, 1.0, 2.0, 3.0], &[2.0, 4.0, 6.0, 8.0]), ("HighX", &[10.0, 11.0, 12.0, 13.0], &[100.0, 103.0, 106.0, 109.0])],
    );
    let model = get_builtin_model("centered_poly_1").unwrap();
    let ds = extract_datasets_from_sheet(&sheet, &[]).unwrap();
    let fit = fit_default(model.as_ref(), &ds);
    assert_eq!(fit.status, FitStatus::Converged);
    assert!(fit.fixed_constants.iter().any(|(n, v, _)| n == "XCenter [DS1]" && (*v - 1.5).abs() < 1e-12));
    assert!(fit.fixed_constants.iter().any(|(n, v, _)| n == "XCenter [DS2]" && (*v - 11.5).abs() < 1e-12));

    let rs = exec_nl(&sheet, nl_fast("centered_poly_1"));
    assert_close(rs.lookup_scalar("XCenter", Some(0)).unwrap(), 1.5, 1e-12, 1e-12, "DS0 XCenter scalar");
    assert_close(rs.lookup_scalar("XCenter", Some(1)).unwrap(), 11.5, 1e-12, 1e-12, "DS1 XCenter scalar");
}

#[test]
fn fa_sec07_04_robust_cauchy_suppresses_weighted_ss_row_and_footnote() {
    let sheet = xy_sheet_single("xy-04", "Outlier", "Y", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], &[3.0, 5.0, 7.0, 40.0, 11.0, 13.0, 15.0]);
    let rs = exec_nl(
        &sheet,
        NonlinearFitSpec { method: FitMethod::RobustCauchy, weighting: WeightingScheme::None, ..NonlinearFitSpec::new("straight_line") },
    );
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert!(find_tab(&rs, "nonlinear_parameters").footnotes.iter().all(|f| !f.contains("Weighted SS")));
    assert!(!find_tab(&rs, "nonlinear_gof").row_labels.iter().any(|l| l == "Weighted Sum of Squares"));
}

#[test]
fn fa_sec07_05_pre_summarized_weighting_honors_n_and_sd_moments() {
    let xs = [1.0, 2.0, 4.0, 8.0];
    let rep_ys = [[2.1, 1.9, 2.0], [4.2, 3.8, 4.0], [8.3, 7.7, 8.0], [15.6, 16.4, 16.0]];
    let mut xy_raw = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 4).unwrap();
    let mut xy_sum = XyTable::new(SubcolumnMode::MeanSdN, 1, 4).unwrap();
    for (r, (&x, reps)) in xs.iter().zip(rep_ys.iter()).enumerate() {
        xy_raw.set_x_cell(r, Cell::number(x)).unwrap();
        xy_sum.set_x_cell(r, Cell::number(x)).unwrap();
        for (c, &yv) in reps.iter().enumerate() {
            xy_raw.data_sets[0].cells[r][c] = Cell::number(yv);
        }
        let (mean, sd, n) = mean_sd_n(reps);
        xy_sum.data_sets[0].cells[r][0] = Cell::number(mean);
        xy_sum.data_sets[0].cells[r][1] = Cell::number(sd);
        xy_sum.data_sets[0].cells[r][2] = Cell::number(n);
    }
    let ds_raw = extract_datasets_from_sheet(&DataSheet::new(SheetId::new("raw-05"), "Raw", TableArchetype::Xy(xy_raw)).unwrap(), &[]).unwrap();
    let ds_sum = extract_datasets_from_sheet(&DataSheet::new(SheetId::new("sum-05"), "Sum", TableArchetype::Xy(xy_sum)).unwrap(), &[]).unwrap();
    let model = get_builtin_model("straight_line").unwrap();

    for scheme in [WeightingScheme::InverseX, WeightingScheme::InverseX2] {
        let cfg = SolverConfig { weighting: scheme, compute_lack_of_fit: true, compute_profile_ci: false, ..SolverConfig::default() };
        let f_raw = fit_with(model.as_ref(), &ds_raw, &cfg);
        let f_sum = fit_with(model.as_ref(), &ds_sum, &cfg);
        assert_eq!((f_sum.n_obs, f_sum.df), (f_raw.n_obs, f_raw.df), "scheme={scheme:?}");
        for (tag, got, exp) in [
            ("Slope", param_val(&f_sum, "Slope"), param_val(&f_raw, "Slope")),
            ("Intercept", param_val(&f_sum, "YIntercept"), param_val(&f_raw, "YIntercept")),
            ("RSS", f_sum.rss, f_raw.rss),
            ("R2", f_sum.r_squared, f_raw.r_squared),
            ("wSS", f_sum.weighted_ss, f_raw.weighted_ss),
            ("ds SS", f_sum.datasets[0].ss_residual, f_raw.datasets[0].ss_residual),
            ("ds sy_x", f_sum.datasets[0].sy_x, f_raw.datasets[0].sy_x),
        ] {
            assert_close(got, exp, 1e-10, 1e-10, &format!("{scheme:?} {tag}"));
        }
        let (lof_raw, lof_sum) = (f_raw.lack_of_fit.as_ref().unwrap(), f_sum.lack_of_fit.as_ref().unwrap());
        assert_eq!(lof_sum.df_pure_error, lof_raw.df_pure_error);
        assert_close(lof_sum.ss_pure_error, lof_raw.ss_pure_error, 1e-10, 1e-10, &format!("{scheme:?} LOF pure error"));
    }

    let cfg_y2 = SolverConfig { weighting: WeightingScheme::RelativeSquare, compute_profile_ci: false, ..SolverConfig::default() };
    let f_sum_y2 = fit_with(model.as_ref(), &ds_sum, &cfg_y2);
    assert_eq!((f_sum_y2.n_obs, f_sum_y2.df), (12, 10));
    let exp_w: f64 = ds_sum[0].points.iter().map(|pt| 2.0 * pt.sd.unwrap().powi(2) / (pt.y * pt.y)).sum();
    let exp_raw: f64 = ds_sum[0].points.iter().map(|pt| 2.0 * pt.sd.unwrap().powi(2)).sum();
    assert_close(f_sum_y2.weighted_ss, exp_w, 1e-10, 1e-10, "RelativeSquare MeanSdN weighted_ss");
    assert_close(f_sum_y2.rss, exp_raw, 1e-10, 1e-10, "RelativeSquare MeanSdN unweighted rss");
    assert_close(f_sum_y2.datasets[0].ss_residual, exp_raw, 1e-10, 1e-10, "RelativeSquare MeanSdN ds ss_residual");
}

#[test]
fn fa_sec07_06_compare_models_rejects_unconverged_and_objective_reversal() {
    let hooks = HookContext::default();
    let xs = [0.0_f64, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 10.0 + 90.0 * (-0.5 * x).exp()).collect();
    let ds = [series_xy(0, "Decay", &xs, &ys)];

    let (line, poly2) = (get_builtin_model("straight_line").unwrap(), get_builtin_model("polynomial_2").unwrap());
    let (one_phase, two_phase) = (get_builtin_model("one_phase_decay").unwrap(), get_builtin_model("two_phase_decay").unwrap());
    let cfg_cap = SolverConfig { max_iters: 1, use_simplex_warm_start: false, ..SolverConfig::default() };

    let fit_cap = fit_with(one_phase.as_ref(), &ds, &cfg_cap);
    assert_eq!(fit_cap.status, FitStatus::HitMaxIters);
    let comp_f = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() };
    assert!(matches!(
        compare_models(&fit_cap, two_phase.as_ref(), &ds, &hooks, &SolverConfig::default(), &comp_f),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));
    assert!(matches!(
        compare_models_checked(&fit_cap, two_phase.as_ref(), &ds, &hooks, &SolverConfig::default(), &comp_f),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));

    let fit_line = fit_default(line.as_ref(), &ds);
    let mut fit_line_ok = fit_line.clone();
    fit_line_ok.status = FitStatus::Converged;
    let comp_1p = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "one_phase_decay".to_owned() };
    assert!(matches!(
        compare_models(&fit_line_ok, one_phase.as_ref(), &ds, &hooks, &cfg_cap, &comp_1p),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));

    let mut bad_complex = fit_default(poly2.as_ref(), &ds);
    bad_complex.weighted_ss = fit_line.weighted_ss + 50.0;
    let comp_back = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "straight_line".to_owned() };
    assert!(matches!(
        compare_models(&bad_complex, line.as_ref(), &ds, &hooks, &SolverConfig::default(), &comp_back),
        Err(FitError::InvalidParameter { name: "comparison", .. })
    ));

    assert_eq!(compare_models_f_test(4.0, 8, 5.0, 7, 0.05).unwrap().f_stat, 0.0);

    let sheet = xy_sheet_single("xy-06", "Cmp", "D", &xs, &ys);
    let rs_cmp = exec_nl(
        &sheet,
        NonlinearFitSpec {
            max_iters: 1,
            comparison: Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() }),
            ..NonlinearFitSpec::new("one_phase_decay")
        },
    );
    assert!(matches!(rs_cmp.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }));
    assert!(rs_cmp.checklist.iter().any(|c| c.assumption == "Model comparison" && c.status == ChecklistStatus::Violated));
}

#[test]
fn fa_sec07_07_mv_column_disjointness_and_numeric_role_guards() {
    let mut mv = MultipleVariablesTable::new(4, 4).unwrap();
    for (c, role) in [VariableRole::Label, VariableRole::Categorical, VariableRole::Continuous, VariableRole::Continuous].into_iter().enumerate() {
        mv.variables[c].lock_role(role);
    }
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::text(format!("ID{r}"));
        mv.variables[1].cells[r] = Cell::text(if r < 2 { "A" } else { "B" });
        mv.variables[2].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[3].cells[r] = Cell::number((2 * (r + 1)) as f64);
    }
    let sheet = DataSheet::new(SheetId::new("mv-07"), "MV", TableArchetype::MultipleVariables(mv)).unwrap();
    for spec in [
        MultivarExtractionSpec { group_columns: &[], x_columns: &[2], y_column: Some(2) },
        MultivarExtractionSpec { group_columns: &[1], x_columns: &[1, 2], y_column: Some(3) },
        MultivarExtractionSpec { group_columns: &[1], x_columns: &[2], y_column: Some(1) },
        MultivarExtractionSpec { group_columns: &[], x_columns: &[2, 2], y_column: Some(3) },
        MultivarExtractionSpec { group_columns: &[1, 1], x_columns: &[2], y_column: Some(3) },
        MultivarExtractionSpec { group_columns: &[], x_columns: &[2], y_column: Some(0) },
        MultivarExtractionSpec { group_columns: &[], x_columns: &[1], y_column: Some(3) },
    ] {
        assert!(matches!(extract_datasets_from_sheet_with_options(&sheet, &[], &spec), Err(FitError::InvalidParameter { .. })));
    }

    let mut mv_1num = MultipleVariablesTable::new(2, 4).unwrap();
    mv_1num.variables[0].lock_role(VariableRole::Categorical);
    mv_1num.variables[1].lock_role(VariableRole::Continuous);
    for r in 0..4 {
        mv_1num.variables[0].cells[r] = Cell::number(r as f64);
        mv_1num.variables[1].cells[r] = Cell::number((2 * r + 1) as f64);
    }
    let sheet_1num = DataSheet::new(SheetId::new("mv-1n"), "OneNum", TableArchetype::MultipleVariables(mv_1num)).unwrap();
    assert!(matches!(extract_datasets_from_sheet(&sheet_1num, &[]), Err(FitError::InsufficientData { required: 2, actual: 1 })));
}

#[test]
fn fa_sec07_08_deming_rejects_pre_summarized_and_survives_degenerate_loo() {
    let pts_sd: Vec<DataPoint> =
        [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)].into_iter().map(|(x, y)| DataPoint { sd: Some(0.2), n: Some(3.0), ..DataPoint::xy(x, y) }).collect();
    assert_eq!(deming_regression(&pts_sd, &DemingOptions::default()).unwrap_err(), FitError::RequiresRawReplicates);

    let mut xy_mlh = XyTable::new(SubcolumnMode::MeanLowHigh, 1, 3).unwrap();
    for (r, (x, y)) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)].into_iter().enumerate() {
        xy_mlh.set_x_cell(r, Cell::number(x)).unwrap();
        xy_mlh.data_sets[0].cells[r] = vec![Cell::number(y), Cell::number(y - 0.5), Cell::number(y + 0.5)];
    }
    let sheet_mlh = DataSheet::new(SheetId::new("xy-mlh"), "MLH", TableArchetype::Xy(xy_mlh)).unwrap();
    let dem_lin = LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() };
    assert_eq!(exec_lin_res(&sheet_mlh, dem_lin).unwrap_err(), FitError::RequiresRawReplicates);

    let dem = deming_regression(&xy_pts(&[1.0, 1.0, 3.0], &[2.0, 2.0, 6.0]), &DemingOptions::default()).unwrap();
    assert_close(dem.slope, 2.0, 1e-12, 1e-12, "n=3 replicate Deming slope");
    assert_close(dem.intercept, 0.0, 1e-12, 1e-12, "n=3 replicate Deming intercept");
    assert!(dem.se_slope.is_nan() && dem.se_intercept.is_nan() && dem.ci_slope.0.is_nan() && dem.ci_slope.1.is_nan());

    let sheet_rep = xy_sheet_single("xy-dem3", "Dem3", "M", &[1.0, 1.0, 3.0], &[2.0, 2.0, 6.0]);
    let rs = exec_lin(&sheet_rep, dem_lin);
    assert!(rs.validate().is_ok());
    assert_eq!(find_tab(&rs, "deming_summary").rows[4][0], ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open });
}

#[test]
fn fa_sec07_09_tex_aliases_unknown_underscores_and_fallback_escaping() {
    for (alias, canonical) in [
        ("centered_polynomial_1", "centered_poly_1"),
        ("centered_polynomial_2", "centered_poly_2"),
        ("centered_polynomial_3", "centered_poly_3"),
        ("centered_polynomial_4", "centered_poly_4"),
        ("centered_polynomial_5", "centered_poly_5"),
        ("centered_polynomial_6", "centered_poly_6"),
        ("semilog_line", "semi_log_line"),
        ("loglog_line", "log_log_line"),
    ] {
        let a = format_model_equation_tex(alias, None, &[], false, 4);
        assert_eq!(a, format_model_equation_tex(canonical, None, &[], false, 4), "alias {alias}");
        assert!(!a.starts_with("Y = f_{"), "alias {alias} fell back: {a}");
    }
    assert_eq!(format_model_equation_tex("my_custom_eq_id", None, &[], false, 4), "Y = f_{\\text{my\\_custom\\_eq\\_id}}(X)");

    let dsl_fb = format_model_equation_tex("custom_dsl", Some("Y = a_1 ^ 2 + unknown_sym_2 + sqrt(X"), &[("a_1".to_owned(), -2.0)], true, 4);
    assert!(dsl_fb.contains("(-2) ^ 2") && dsl_fb.contains("unknown\\_sym\\_2") && dsl_fb.contains("\\text{sqrt}("), "got: {dsl_fb}");
}

#[test]
fn fa_sec07_10_compute_jacobian_one_sided_domain_retry_at_upper_edge() {
    let model = dsl_compile("Y = A * sqrt(10.0 - K * X)");
    let xs = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 3.0 * (10.0 - 1.999_999 * x).sqrt()).collect();
    let cfg = SolverConfig {
        use_simplex_warm_start: false,
        compute_profile_ci: false,
        parameter_rules: vec![rule_local_init("A", 2.9), rule_local_bounds("K", Some(1.999_999_9), None, Some(2.0))],
        ..SolverConfig::default()
    };
    let res = fit_with(&model, &[series_xy(0, "Edge", &xs, &ys)], &cfg);
    assert_eq!(res.status, FitStatus::Converged);
    assert!(param_val(&res, "A").is_finite() && (param_val(&res, "A") - 3.0).abs() < 1e-4);
    assert!(res.parameters[0].se.is_finite() && res.parameters[1].se.is_finite());
}

#[test]
fn fa_sec07_11_curve_overlay_delta_grad_handles_domain_and_bound_edge() {
    let model = dsl_compile("Y = A * X + sqrt(2.0 - B)");
    let cfg = SolverConfig {
        compute_bands: true,
        compute_profile_ci: false,
        use_simplex_warm_start: false,
        parameter_rules: vec![rule_local_bounds("A", Some(2.0), Some(2.0), None), rule_local_bounds("B", Some(1.999_999_9), None, Some(2.0))],
        ..SolverConfig::default()
    };
    let res = fit_with(&model, &[series_xy(0, "Band", &[1.0, 2.0, 3.0, 4.0, 5.0], &[2.1, 4.0, 6.2, 7.9, 10.1])], &cfg);
    let overlay = res.datasets[0].curve_overlay.as_ref().unwrap();
    let mid = &overlay.points[overlay.points.len() / 2];
    assert!(mid.ci_upper.unwrap() > mid.ci_lower.unwrap(), "expected positive confidence band width");
}

#[test]
fn fa_sec07_12_pre_summarized_non_moment_modes_extract_unknowns() {
    for mode in [SubcolumnMode::MeanLowHigh, SubcolumnMode::MedianIqr] {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, 5).unwrap();
        for (r, (x, y)) in [(1.0, 3.0), (2.0, 5.0), (3.0, 7.0)].into_iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x)).unwrap();
            xy.data_sets[0].cells[r] = vec![Cell::number(y), Cell::number(y - 0.5), Cell::number(y + 0.5)];
        }
        xy.data_sets[0].cells[3] = vec![Cell::number(9.0), Cell::number(8.5), Cell::number(9.5)];
        xy.set_x_cell(4, Cell::number(5.0)).unwrap();

        let sheet = DataSheet::new(SheetId::new("xy-12"), "NonMoment", TableArchetype::Xy(xy)).unwrap();
        let extracted = extract_datasets_from_sheet(&sheet, &[]).unwrap();
        assert_eq!(extracted[0].unknowns_y.len(), 1, "mode={mode:?}");
        assert_eq!(extracted[0].unknowns_x, vec![(4, 5.0)], "mode={mode:?}");

        let rs = exec_nl(&sheet, NonlinearFitSpec { interpolate_unknowns: true, ..nl_fast("straight_line") });
        assert!(rs.tabs.iter().any(|t| t.id == "nonlinear_interpolated"));
    }
}

#[test]
fn fa_sec07_13_selected_data_sets_rejects_oob_and_duplicates() {
    let sheet_xy = xy_sheet("xy-13", "XY", &[1.0, 2.0, 3.0], &[("A", &[2.0, 4.0, 6.0]), ("B", &[3.0, 5.0, 7.0])]);
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 3).unwrap();
    for r in 0..3 {
        gt.data_sets[0].cells[r][0] = Cell::number(r as f64);
        gt.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let sheet_g = DataSheet::new(SheetId::new("g-13"), "G", TableArchetype::Grouped(gt)).unwrap();
    let sheet_mv = mv_sheet_num("mv-13", "MV", &[("X", &[1.0, 2.0, 3.0]), ("Y", &[2.0, 4.0, 6.0])]);

    for (tag, sheet, bads) in
        [("xy", &sheet_xy, [&[99][..], &[0, 0][..]]), ("grouped", &sheet_g, [&[5][..], &[1, 1][..]]), ("mv", &sheet_mv, [&[9][..], &[0, 0][..]])]
    {
        for bad in bads {
            let err = extract_datasets_from_sheet(sheet, bad).unwrap_err();
            assert!(matches!(err, FitError::InvalidParameter { name: "selected_data_sets", .. }), "{tag} bad={bad:?}: {err:?}");
        }
    }
}

#[test]
fn fa_sec07_14_plot_function_only_validates_eval_and_computes_gof() {
    let ds = [series_xy(0, "Line", &[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0])];
    let undef = dsl_compile("Y = A * sqrt(-1.0 - X * X)");
    let cfg_plot = SolverConfig { plot_function_only: true, ..SolverConfig::default() };
    assert!(matches!(fit_nonlinear(&undef, &ds, &HookContext::default(), &cfg_plot), Err(FitError::BadInitialValues { .. })));

    let line = get_builtin_model("straight_line").unwrap();
    let cfg_bad = SolverConfig {
        plot_function_only: true,
        parameter_rules: vec![rule_const("Slope", 0.0), rule_const("YIntercept", 0.0)],
        ..SolverConfig::default()
    };
    let fit_plot = fit_with(line.as_ref(), &ds, &cfg_bad);
    assert_close(fit_plot.datasets[0].r_squared, 1.0 - 164.0 / 20.0, 1e-12, 1e-12, "plot_only ds R2");
    assert_close(fit_plot.datasets[0].sy_x, (164.0 / 4.0_f64).sqrt(), 1e-12, 1e-12, "plot_only ds sy_x");
    assert_eq!(fit_with(line.as_ref(), &[series_xy(0, "Empty", &[], &[])], &cfg_bad).n_obs, 0);
}

#[test]
fn fa_sec07_15_linear_and_ic_contracts_match_docstrings() {
    let orig = simple_linear_regression(
        &xy_pts(&[1.0, 2.0, 3.0, 4.0], &[2.1, 3.9, 6.1, 8.0]),
        &SimpleLinearOptions { force_through_origin: true, ..Default::default() },
    )
    .unwrap();
    assert!(orig.r.is_nan() && orig.x_intercept.is_none());

    let flat = simple_linear_regression(&xy_pts(&[1.0, 2.0, 3.0, 4.0], &[5.0; 4]), &SimpleLinearOptions::default()).unwrap();
    assert!(flat.r.is_nan());

    let line_a = xy_pts(&[1.0, 2.0, 3.0, 4.0], &[2.0, 4.0, 6.0, 8.0]);
    let line_b = xy_pts(&[1.0, 2.0, 3.0, 4.0], &[10.1, 20.0, 29.9, 40.1]);
    let cmp = compare_linear_lines(&[&line_a, &line_b], false).unwrap();
    assert!(cmp.slopes_significantly_different);
    assert!(
        cmp.f_intercepts.is_none() && cmp.df_intercepts.is_none() && cmp.p_intercepts.is_none() && cmp.intercepts_significantly_different.is_none()
    );

    for bad_ss in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(compute_information_criteria(bad_ss, 10, 2), Err(FitError::InvalidParameter { name: "ss", .. })));
    }
}

#[test]
fn fa_sec07_16_unified_rout_pipeline_parity() {
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let mut ys: Vec<f64> = xs.iter().map(|&x| 2.0 + 3.0 * x).collect();
    ys[10] += 35.0;
    let ds = series_xy(0, "ROUT", &xs, &ys);
    let residuals: Vec<f64> = ds.points.iter().map(|p| p.y - (2.0 + 3.0 * p.x0())).collect();
    assert_eq!(rout_detect_outliers(&ds.points, &residuals, 2, 1.0).unwrap().outlier_indices, vec![10]);
}
