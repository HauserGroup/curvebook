#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Phase 3 SEC-07 regression tests (`FA-SEC07-01` .. `FA-SEC07-16`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, DataSheet, ErrorCode, FitMethod, GroupedTable, LinearRegressionSpec,
    ModelComparisonSpec, MultipleVariablesTable, NonlinearFitSpec, ParameterRule, ParameterScope, ResultCell, ResultStatus, SheetId, SubcolumnMode,
    TableArchetype, VariableRole, WeightingScheme, XColumnMode, XyTable,
};
use curvebook_fit::diagnostics::{compare_models_f_test, compute_information_criteria, rout_detect_outliers};
use curvebook_fit::error::{FitError, FitIdentifiability, FitStatus};
use curvebook_fit::linear::{DemingOptions, SimpleLinearOptions, compare_linear_lines, deming_regression, simple_linear_regression};
use curvebook_fit::result::execute_analysis;
use curvebook_fit::solver::{NonlinearFitResult, SolverConfig, compare_models, compare_models_checked, fit_nonlinear};
use curvebook_fit::tex_format::format_model_equation_tex;
use curvebook_fit::types::{
    DataPoint, DatasetSeries, HookContext, MultivarExtractionSpec, extract_datasets_from_sheet, extract_datasets_from_sheet_with_options,
};
use curvebook_fit::{CompiledDslModel, get_builtin_model};

fn series_xy(data_set_index: usize, label: &str, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    let points = xs.iter().zip(ys.iter()).enumerate().map(|(row, (&x, &y))| DataPoint { data_set_index, row, ..DataPoint::xy(x, y) }).collect();
    DatasetSeries {
        data_set_index,
        label: label.to_owned(),
        points,
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn make_xy_sheet(id: &str, title: &str, cols: &[(&str, &[f64], &[f64])]) -> DataSheet {
    let rows = cols.iter().map(|(_, xs, _)| xs.len()).max().unwrap_or(0);
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, cols.len(), rows).unwrap();
    for (c, (col_title, xs, ys)) in cols.iter().enumerate() {
        xy.data_sets[c].title = (*col_title).to_owned();
        for r in 0..xs.len() {
            if c == 0 {
                xy.set_x_cell(r, Cell::number(xs[r])).unwrap();
            }
            xy.data_sets[c].cells[r][0] = Cell::number(ys[r]);
        }
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(xy)).unwrap()
}

fn find_param_val(fit: &NonlinearFitResult, name: &str) -> f64 {
    fit.parameters.iter().find(|p| p.name == name).unwrap().estimate
}

fn assert_close(actual: f64, expected: f64, atol: f64, rtol: f64, ctx: &str) {
    let tol = atol.max(rtol * expected.abs());
    assert!((actual - expected).abs() <= tol, "{ctx}: actual={actual}, expected={expected}, diff={}", (actual - expected).abs());
}

#[test]
fn fa_sec07_01_unconverged_and_not_assessed_identifiability() {
    let xs = [0.0_f64, 1.0, 2.0, 4.0, 8.0, 12.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 10.0 + 90.0 * (-0.35 * x).exp()).collect();
    let sheet = make_xy_sheet("xy-01", "Decay", &[("Obs", &xs, &ys)]);

    // 1. max_iters = 1 forces HitMaxIters: ResultSheet must be Failed and never Pass identifiability.
    let mut nl_capped = NonlinearFitSpec::new("one_phase_decay");
    nl_capped.max_iters = 1;
    nl_capped.identify_unstable_parameters = true;
    nl_capped.profile_likelihood_ci = false;
    let spec_capped = AnalysisSpec::new("fit-01-cap", "xy-01", AnalysisKind::NonlinearRegression(nl_capped)).unwrap();
    let rs_capped = execute_analysis(&spec_capped, &sheet, &HookContext::default()).unwrap();
    assert!(matches!(rs_capped.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }), "got {:?}", rs_capped.status);
    assert!(rs_capped.checklist.iter().any(|c| c.assumption == "Solver convergence" && c.status == ChecklistStatus::Violated));
    assert!(
        !rs_capped.checklist.iter().any(|c| c.assumption == "Parameter identifiability" && c.status == ChecklistStatus::Pass),
        "HitMaxIters must not emit Parameter identifiability Pass: {:?}",
        rs_capped.checklist
    );

    // 2. Converged fit with identify_unstable_parameters=false must emit Info, never false Pass.
    let mut nl_no_id = NonlinearFitSpec::new("one_phase_decay");
    nl_no_id.identify_unstable_parameters = false;
    nl_no_id.profile_likelihood_ci = false;
    let spec_no_id = AnalysisSpec::new("fit-01-noid", "xy-01", AnalysisKind::NonlinearRegression(nl_no_id)).unwrap();
    let rs_no_id = execute_analysis(&spec_no_id, &sheet, &HookContext::default()).unwrap();
    assert_eq!(rs_no_id.status, ResultStatus::UpToDate);
    assert!(rs_no_id.checklist.iter().any(|c| c.assumption == "Parameter identifiability" && c.status == ChecklistStatus::Info));

    // 3. Direct diverged fit must have NotAssessed identifiability and Failed ResultStatus on sheet.
    // `sqrt(1.0 - A)` evaluates at initial `A = 1.0 - 1e-6` while positive descent toward true `A = 3.0` crosses domain wall `A > 1.0`.
    let div_model = CompiledDslModel::compile("wall_div", "Y = A * X + 0.0 * sqrt(1.0 - A)").unwrap();
    let div_ds = vec![series_xy(0, "Stalled", &[1.0, 2.0, 3.0, 4.0], &[3.0, 6.0, 9.0, 12.0])];
    let div_cfg = SolverConfig {
        identify_unstable_parameters: true,
        compute_profile_ci: false,
        use_simplex_warm_start: false,
        parameter_rules: vec![ParameterRule {
            name: "A".to_owned(),
            scope: ParameterScope::Local,
            initial_override: Some(0.99),
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let div_fit = fit_nonlinear(&div_model, &div_ds, &HookContext::default(), &div_cfg).unwrap();
    assert_eq!(div_fit.status, FitStatus::Diverged);
    assert_eq!(div_fit.identifiability, FitIdentifiability::NotAssessed);

    // 4. Multi-dataset independent fit where DS0 converges clean (Identifiable) and DS1 hits Diverged (NotAssessed):
    // combined identifiability must downgrade Identifiable -> NotAssessed rather than staying Identifiable.
    let multi_ds = vec![
        series_xy(0, "CleanA", &[1.0, 2.0, 3.0, 4.0, 5.0], &[0.5, 1.0, 1.5, 2.0, 2.5]),
        series_xy(1, "WallA", &[1.0, 2.0, 3.0, 4.0, 5.0], &[3.0, 6.0, 9.0, 12.0, 15.0]),
    ];
    let multi_fit = fit_nonlinear(&div_model, &multi_ds, &HookContext::default(), &div_cfg).unwrap();
    assert_eq!(multi_fit.status, FitStatus::Diverged);
    assert_eq!(multi_fit.identifiability, FitIdentifiability::NotAssessed);
}

#[test]
fn fa_sec07_02_irls_outer_convergence_iterations_and_poisson_deviance_guard() {
    let hooks = HookContext::default();

    // 1. Cauchy IRLS with max_iters = 1 must report HitMaxIters and accumulate iterations > 1.
    let xs_c = [0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let ys_c = [100.0, 72.0, 55.0, 31.0, 20.0, 14.0, 11.5, 10.2];
    let decay = get_builtin_model("one_phase_decay").unwrap();
    let ds_c = vec![series_xy(0, "CauchyCap", &xs_c, &ys_c)];
    let cfg_cauchy_cap = SolverConfig {
        method: FitMethod::RobustCauchy,
        max_iters: 1,
        use_simplex_warm_start: false,
        parameter_rules: vec![ParameterRule {
            name: "Y0".to_owned(),
            scope: ParameterScope::Local,
            initial_override: Some(25.0),
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let res_cauchy_cap = fit_nonlinear(decay.as_ref(), &ds_c, &hooks, &cfg_cauchy_cap).unwrap();
    assert_eq!(res_cauchy_cap.status, FitStatus::HitMaxIters);
    assert!(res_cauchy_cap.iterations >= 2, "expected accumulated IRLS iterations >= 2, got {}", res_cauchy_cap.iterations);

    // 2. Poisson IRLS on curved 4-parameter model with max_iters = 1 per pass exhausts IRLS budget
    // with FitStatus::HitMaxIters and accumulated iterations > 1, then converges under full budget.
    let sigmoid_p = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let xs_p = [-8.0_f64, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let ys_p = [5.0, 8.0, 22.0, 52.0, 82.0, 95.0, 99.0];
    let ds_p = vec![series_xy(0, "Counts", &xs_p, &ys_p)];
    let cfg_pois_cap = SolverConfig {
        method: FitMethod::PoissonDeviance,
        max_iters: 1,
        use_simplex_warm_start: false,
        parameter_rules: vec![
            ParameterRule {
                name: "Bottom".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(1.0),
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "Top".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(40.0),
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "LogEC50".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(-7.5),
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "HillSlope".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(0.5),
                lower_bound: None,
                upper_bound: None,
            },
        ],
        ..SolverConfig::default()
    };
    let res_pois_cap = fit_nonlinear(sigmoid_p.as_ref(), &ds_p, &hooks, &cfg_pois_cap).unwrap();
    assert_eq!(res_pois_cap.status, FitStatus::HitMaxIters);
    assert!(res_pois_cap.iterations >= 2, "expected accumulated Poisson IRLS iterations >= 2, got {}", res_pois_cap.iterations);

    let cfg_pois_ok = SolverConfig { method: FitMethod::PoissonDeviance, ..SolverConfig::default() };
    let res_pois_ok = fit_nonlinear(sigmoid_p.as_ref(), &ds_p, &hooks, &cfg_pois_ok).unwrap();
    assert_eq!(res_pois_ok.status, FitStatus::Converged);
    assert!(res_pois_ok.weighted_ss.is_finite() && res_pois_ok.weighted_ss >= 0.0);
}

#[test]
fn fa_sec07_03_per_dataset_default_parameter_rules_xcenter_preserved() {
    let xa = [0.0, 1.0, 2.0, 3.0];
    let ya = [2.0, 4.0, 6.0, 8.0];
    let xb = [10.0, 11.0, 12.0, 13.0];
    let yb = [100.0, 103.0, 106.0, 109.0];

    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 8).unwrap();
    xy.data_sets[0].title = "LowX".to_owned();
    xy.data_sets[1].title = "HighX".to_owned();
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number(xa[r])).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(ya[r]);
        xy.set_x_cell(4 + r, Cell::number(xb[r])).unwrap();
        xy.data_sets[1].cells[4 + r][0] = Cell::number(yb[r]);
    }
    let sheet = DataSheet::new(SheetId::new("xy-xc"), "CenteredMulti", TableArchetype::Xy(xy)).unwrap();

    let model = get_builtin_model("centered_poly_1").unwrap();
    let ds = extract_datasets_from_sheet(&sheet, &[]).unwrap();
    let fit = fit_nonlinear(model.as_ref(), &ds, &HookContext::default(), &SolverConfig::default()).unwrap();
    assert_eq!(fit.status, FitStatus::Converged);

    // Must preserve per-dataset fixed_constants `[DS1]`=1.5 and `[DS2]`=11.5 (not pooled 6.5).
    assert!(
        fit.fixed_constants.iter().any(|(n, v, _)| n == "XCenter [DS1]" && (*v - 1.5).abs() < 1e-12),
        "expected XCenter [DS1]=1.5, got {:?}",
        fit.fixed_constants
    );
    assert!(
        fit.fixed_constants.iter().any(|(n, v, _)| n == "XCenter [DS2]" && (*v - 11.5).abs() < 1e-12),
        "expected XCenter [DS2]=11.5, got {:?}",
        fit.fixed_constants
    );

    let mut nl = NonlinearFitSpec::new("centered_poly_1");
    nl.profile_likelihood_ci = false;
    let spec = AnalysisSpec::new("fit-xc", "xy-xc", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let rs = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_close(rs.lookup_scalar("XCenter", Some(0)).unwrap(), 1.5, 1e-12, 1e-12, "DS0 XCenter scalar");
    assert_close(rs.lookup_scalar("XCenter", Some(1)).unwrap(), 11.5, 1e-12, 1e-12, "DS1 XCenter scalar");
}

#[test]
fn fa_sec07_04_robust_cauchy_suppresses_weighted_ss_row_and_footnote() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let ys = [3.0, 5.0, 7.0, 40.0, 11.0, 13.0, 15.0];
    let sheet = make_xy_sheet("xy-04", "Outlier", &[("Y", &xs, &ys)]);

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.method = FitMethod::RobustCauchy;
    nl.weighting = WeightingScheme::None;
    let spec = AnalysisSpec::new("fit-04", "xy-04", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let rs = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    assert_eq!(rs.status, ResultStatus::UpToDate);

    let param_tab = rs.tabs.iter().find(|t| t.id == "nonlinear_parameters").unwrap();
    assert!(param_tab.footnotes.iter().all(|f| !f.contains("Weighted SS")), "unexpected Weighted SS footnote: {:?}", param_tab.footnotes);
    let gof_tab = rs.tabs.iter().find(|t| t.id == "nonlinear_gof").unwrap();
    assert!(!gof_tab.row_labels.iter().any(|l| l == "Weighted Sum of Squares"), "unexpected Weighted SS row: {:?}", gof_tab.row_labels);
}

#[test]
fn fa_sec07_05_pre_summarized_weighting_honors_n_and_sd_moments() {
    let xs = [1.0, 2.0, 4.0, 8.0];
    let rep_ys = [[2.1, 1.9, 2.0], [4.2, 3.8, 4.0], [8.3, 7.7, 8.0], [15.6, 16.4, 16.0]];

    let mut xy_raw =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(3), 1, 4).unwrap();
    let mut xy_sum = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 1, 4).unwrap();
    for (r, (&x, reps)) in xs.iter().zip(rep_ys.iter()).enumerate() {
        xy_raw.set_x_cell(r, Cell::number(x)).unwrap();
        xy_sum.set_x_cell(r, Cell::number(x)).unwrap();
        for (c, &yv) in reps.iter().enumerate() {
            xy_raw.data_sets[0].cells[r][c] = Cell::number(yv);
        }
        let mean = reps.iter().sum::<f64>() / 3.0;
        let sd = (reps.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 2.0).sqrt();
        xy_sum.data_sets[0].cells[r][0] = Cell::number(mean);
        xy_sum.data_sets[0].cells[r][1] = Cell::number(sd);
        xy_sum.data_sets[0].cells[r][2] = Cell::number(3.0);
    }

    let ds_raw = extract_datasets_from_sheet(&DataSheet::new(SheetId::new("raw-05"), "Raw", TableArchetype::Xy(xy_raw)).unwrap(), &[]).unwrap();
    let ds_sum = extract_datasets_from_sheet(&DataSheet::new(SheetId::new("sum-05"), "Sum", TableArchetype::Xy(xy_sum)).unwrap(), &[]).unwrap();
    let model = get_builtin_model("straight_line").unwrap();
    let hooks = HookContext::default();

    // Predictor-only weighting (InverseX, InverseX2) preserves exact RawReplicates parity;
    // response weighting (RelativeSquare, Relative) still honors N, unweighted RSS, and weighted (N-1)*SD^2.
    for scheme in [WeightingScheme::InverseX, WeightingScheme::InverseX2] {
        let cfg = SolverConfig { weighting: scheme, compute_lack_of_fit: true, compute_profile_ci: false, ..SolverConfig::default() };
        let f_raw = fit_nonlinear(model.as_ref(), &ds_raw, &hooks, &cfg).unwrap();
        let f_sum = fit_nonlinear(model.as_ref(), &ds_sum, &hooks, &cfg).unwrap();
        assert_eq!(f_sum.n_obs, f_raw.n_obs, "scheme={scheme:?}");
        assert_eq!(f_sum.df, f_raw.df, "scheme={scheme:?}");
        assert_close(find_param_val(&f_sum, "Slope"), find_param_val(&f_raw, "Slope"), 1e-10, 1e-10, &format!("{scheme:?} Slope"));
        assert_close(find_param_val(&f_sum, "YIntercept"), find_param_val(&f_raw, "YIntercept"), 1e-10, 1e-10, &format!("{scheme:?} Intercept"));
        assert_close(f_sum.rss, f_raw.rss, 1e-10, 1e-10, &format!("{scheme:?} RSS"));
        assert_close(f_sum.r_squared, f_raw.r_squared, 1e-10, 1e-10, &format!("{scheme:?} R2"));
        assert_close(f_sum.weighted_ss, f_raw.weighted_ss, 1e-10, 1e-10, &format!("{scheme:?} wSS"));
        assert_close(f_sum.datasets[0].ss_residual, f_raw.datasets[0].ss_residual, 1e-10, 1e-10, &format!("{scheme:?} ds SS"));
        assert_close(f_sum.datasets[0].sy_x, f_raw.datasets[0].sy_x, 1e-10, 1e-10, &format!("{scheme:?} ds sy_x"));
        let lof_raw = f_raw.lack_of_fit.as_ref().unwrap();
        let lof_sum = f_sum.lack_of_fit.as_ref().unwrap();
        assert_eq!(lof_sum.df_pure_error, lof_raw.df_pure_error);
        assert_close(lof_sum.ss_pure_error, lof_raw.ss_pure_error, 1e-10, 1e-10, &format!("{scheme:?} LOF pure error"));
    }

    let cfg_y2 = SolverConfig { weighting: WeightingScheme::RelativeSquare, compute_profile_ci: false, ..SolverConfig::default() };
    let f_sum_y2 = fit_nonlinear(model.as_ref(), &ds_sum, &hooks, &cfg_y2).unwrap();
    assert_eq!(f_sum_y2.n_obs, 12);
    assert_eq!(f_sum_y2.df, 10);
    // Row means follow exact line Y = 2*X with N=3 and within-row SD > 0; weighted_ss equals sum w_row*(N-1)*SD^2.
    let exp_weighted_within: f64 = ds_sum[0].points.iter().map(|pt| 2.0 * pt.sd.unwrap().powi(2) / (pt.y * pt.y)).sum();
    let exp_raw_within: f64 = ds_sum[0].points.iter().map(|pt| 2.0 * pt.sd.unwrap().powi(2)).sum();
    assert_close(f_sum_y2.weighted_ss, exp_weighted_within, 1e-10, 1e-10, "RelativeSquare MeanSdN weighted_ss");
    assert_close(f_sum_y2.rss, exp_raw_within, 1e-10, 1e-10, "RelativeSquare MeanSdN unweighted rss");
    assert_close(f_sum_y2.datasets[0].ss_residual, exp_raw_within, 1e-10, 1e-10, "RelativeSquare MeanSdN ds ss_residual");
}

#[test]
fn fa_sec07_06_compare_models_rejects_unconverged_and_objective_reversal() {
    let hooks = HookContext::default();
    let xs = [0.0_f64, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 10.0 + 90.0 * (-0.5 * x).exp()).collect();
    let ds = vec![series_xy(0, "Decay", &xs, &ys)];

    let line = get_builtin_model("straight_line").unwrap();
    let poly2 = get_builtin_model("polynomial_2").unwrap();
    let one_phase = get_builtin_model("one_phase_decay").unwrap();
    let two_phase = get_builtin_model("two_phase_decay").unwrap();

    // 1. Unconverged primary fit must be rejected by compare_models and compare_models_checked.
    let cfg_cap = SolverConfig { max_iters: 1, use_simplex_warm_start: false, ..SolverConfig::default() };
    let fit_cap = fit_nonlinear(one_phase.as_ref(), &ds, &hooks, &cfg_cap).unwrap();
    assert_eq!(fit_cap.status, FitStatus::HitMaxIters);
    let comp_f = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() };
    let err_primary = compare_models(&fit_cap, two_phase.as_ref(), &ds, &hooks, &SolverConfig::default(), &comp_f).unwrap_err();
    assert!(matches!(err_primary, FitError::InvalidParameter { name: "comparison", .. }));
    let err_checked = compare_models_checked(&fit_cap, two_phase.as_ref(), &ds, &hooks, &SolverConfig::default(), &comp_f).unwrap_err();
    assert!(matches!(err_checked, FitError::InvalidParameter { name: "comparison", .. }));

    // 2. Unconverged alternative fit under capped max_iters must also be rejected.
    let fit_line = fit_nonlinear(line.as_ref(), &ds, &hooks, &SolverConfig::default()).unwrap();
    let comp_poly = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "one_phase_decay".to_owned() };
    let mut fit_line_same_cap = fit_line.clone();
    fit_line_same_cap.status = FitStatus::Converged;
    let err_alt = compare_models(&fit_line_same_cap, one_phase.as_ref(), &ds, &hooks, &cfg_cap, &comp_poly).unwrap_err();
    assert!(matches!(err_alt, FitError::InvalidParameter { name: "comparison", .. }));

    // 3. Reversed nested SS (higher k_free / lower df achieves materially higher SS) rejected in compare_models.
    let mut bad_complex_src = fit_nonlinear(poly2.as_ref(), &ds, &hooks, &SolverConfig::default()).unwrap();
    bad_complex_src.weighted_ss = fit_line.weighted_ss + 50.0;
    let comp_back = ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "straight_line".to_owned() };
    let err_rev = compare_models(&bad_complex_src, line.as_ref(), &ds, &hooks, &SolverConfig::default(), &comp_back).unwrap_err();
    assert!(matches!(err_rev, FitError::InvalidParameter { name: "comparison", .. }));

    // 4. Diagnostics helper itself still preserves F=0 clamping tested in d1_edge_matrix.
    let f_clamp = compare_models_f_test(4.0, 8, 5.0, 7, 0.05).unwrap();
    assert_eq!(f_clamp.f_stat, 0.0);

    // 5. execute_analysis returns Failed ResultSheet when primary doesn't converge even if comparison is configured.
    let sheet = make_xy_sheet("xy-06", "Cmp", &[("D", &xs, &ys)]);
    let mut nl_cmp = NonlinearFitSpec::new("one_phase_decay");
    nl_cmp.max_iters = 1;
    nl_cmp.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: "two_phase_decay".to_owned() });
    let rs_cmp = execute_analysis(&AnalysisSpec::new("fit-06", "xy-06", AnalysisKind::NonlinearRegression(nl_cmp)).unwrap(), &sheet, &hooks).unwrap();
    assert!(matches!(rs_cmp.status, ResultStatus::Failed { code: ErrorCode::SolverFailed, .. }));
    assert!(rs_cmp.checklist.iter().any(|c| c.assumption == "Model comparison" && c.status == ChecklistStatus::Violated));
}

#[test]
fn fa_sec07_07_mv_column_disjointness_and_numeric_role_guards() {
    let mut mv = MultipleVariablesTable::new(4, 4).unwrap();
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].lock_role(VariableRole::Continuous);
    mv.variables[3].lock_role(VariableRole::Continuous);
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::text(format!("ID{r}"));
        mv.variables[1].cells[r] = Cell::text(if r < 2 { "A" } else { "B" });
        mv.variables[2].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[3].cells[r] = Cell::number((2 * (r + 1)) as f64);
    }
    let sheet = DataSheet::new(SheetId::new("mv-07"), "MV", TableArchetype::MultipleVariables(mv)).unwrap();

    // Overlap: X == Y, X in group, Y in group; duplicate X / group; non-numeric Y / X.
    for spec in [
        MultivarExtractionSpec { group_columns: &[], x_columns: &[2], y_column: Some(2) },
        MultivarExtractionSpec { group_columns: &[1], x_columns: &[1, 2], y_column: Some(3) },
        MultivarExtractionSpec { group_columns: &[1], x_columns: &[2], y_column: Some(1) },
        MultivarExtractionSpec { group_columns: &[], x_columns: &[2, 2], y_column: Some(3) },
        MultivarExtractionSpec { group_columns: &[1, 1], x_columns: &[2], y_column: Some(3) },
        MultivarExtractionSpec { group_columns: &[], x_columns: &[2], y_column: Some(0) },
        MultivarExtractionSpec { group_columns: &[], x_columns: &[1], y_column: Some(3) },
    ] {
        let err = extract_datasets_from_sheet_with_options(&sheet, &[], &spec).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { .. }), "expected InvalidParameter, got {err:?}");
    }

    // Only 1 numeric column + 1 Categorical column must fail with InsufficientData instead of falling back to Categorical.
    let mut mv_1num = MultipleVariablesTable::new(2, 4).unwrap();
    mv_1num.variables[0].lock_role(VariableRole::Categorical);
    mv_1num.variables[1].lock_role(VariableRole::Continuous);
    for r in 0..4 {
        mv_1num.variables[0].cells[r] = Cell::number(r as f64);
        mv_1num.variables[1].cells[r] = Cell::number((2 * r + 1) as f64);
    }
    let sheet_1num = DataSheet::new(SheetId::new("mv-1n"), "OneNum", TableArchetype::MultipleVariables(mv_1num)).unwrap();
    let err = extract_datasets_from_sheet(&sheet_1num, &[]).unwrap_err();
    assert!(matches!(err, FitError::InsufficientData { required: 2, actual: 1 }), "got {err:?}");
}

#[test]
fn fa_sec07_08_deming_rejects_pre_summarized_and_survives_degenerate_loo() {
    // 1. Direct points with sd/n rejected with RequiresRawReplicates.
    let pts_sd = vec![
        DataPoint { sd: Some(0.2), n: Some(3.0), ..DataPoint::xy(1.0, 2.0) },
        DataPoint { sd: Some(0.2), n: Some(3.0), ..DataPoint::xy(2.0, 4.0) },
        DataPoint { sd: Some(0.2), n: Some(3.0), ..DataPoint::xy(3.0, 6.0) },
    ];
    assert_eq!(deming_regression(&pts_sd, &DemingOptions::default()).unwrap_err(), FitError::RequiresRawReplicates);

    // 2. execute_analysis with deming_model_ii on MeanLowHigh table rejected with RequiresRawReplicates.
    let mut xy_mlh =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanLowHigh, 1, 3).unwrap();
    for (r, (x, y)) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)].into_iter().enumerate() {
        xy_mlh.set_x_cell(r, Cell::number(x)).unwrap();
        xy_mlh.data_sets[0].cells[r][0] = Cell::number(y);
        xy_mlh.data_sets[0].cells[r][1] = Cell::number(y - 0.5);
        xy_mlh.data_sets[0].cells[r][2] = Cell::number(y + 0.5);
    }
    let sheet_mlh = DataSheet::new(SheetId::new("xy-mlh"), "MLH", TableArchetype::Xy(xy_mlh)).unwrap();
    let lin_spec = AnalysisSpec::new(
        "dem-mlh",
        "xy-mlh",
        AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() }),
    )
    .unwrap();
    assert_eq!(execute_analysis(&lin_spec, &sheet_mlh, &HookContext::default()).unwrap_err(), FitError::RequiresRawReplicates);

    // 3. n = 3 with replicate points (LOO drops distinct point -> collinear/degenerate subset) still fits slope/intercept.
    let pts_rep = vec![DataPoint::xy(1.0, 2.0), DataPoint::xy(1.0, 2.0), DataPoint::xy(3.0, 6.0)];
    let dem = deming_regression(&pts_rep, &DemingOptions::default()).unwrap();
    assert_close(dem.slope, 2.0, 1e-12, 1e-12, "n=3 replicate Deming slope");
    assert_close(dem.intercept, 0.0, 1e-12, 1e-12, "n=3 replicate Deming intercept");
    assert!(dem.se_slope.is_nan() && dem.se_intercept.is_nan());
    assert!(dem.ci_slope.0.is_nan() && dem.ci_slope.1.is_nan());

    let sheet_rep = make_xy_sheet("xy-dem3", "Dem3", &[("M", &[1.0, 1.0, 3.0], &[2.0, 2.0, 6.0])]);
    let spec_dem3 = AnalysisSpec::new(
        "dem-3",
        "xy-dem3",
        AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii: true, ..LinearRegressionSpec::default() }),
    )
    .unwrap();
    let rs = execute_analysis(&spec_dem3, &sheet_rep, &HookContext::default()).unwrap();
    assert!(rs.validate().is_ok());
    let tab = rs.tabs.iter().find(|t| t.id == "deming_summary").unwrap();
    assert_eq!(tab.rows[4][0], ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open });
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
        let c = format_model_equation_tex(canonical, None, &[], false, 4);
        assert_eq!(a, c, "alias {alias} drifted from {canonical}");
        assert!(!a.starts_with("Y = f_{"), "alias {alias} fell back: {a}");
    }

    let unk = format_model_equation_tex("my_custom_eq_id", None, &[], false, 4);
    assert_eq!(unk, "Y = f_{\\text{my\\_custom\\_eq\\_id}}(X)");

    // Unparseable custom DSL with underscores, whitespace before `^`, and incomplete `sqrt(`:
    let dsl_fb = format_model_equation_tex("custom_dsl", Some("Y = a_1 ^ 2 + unknown_sym_2 + sqrt(X"), &[("a_1".to_owned(), -2.0)], true, 4);
    assert!(dsl_fb.contains("(-2) ^ 2"), "missing parens before spaced caret: {dsl_fb}");
    assert!(dsl_fb.contains("unknown\\_sym\\_2"), "missing escaped underscore in fallback: {dsl_fb}");
    assert!(dsl_fb.contains("\\text{sqrt}("), "expected safe sqrt fallback: {dsl_fb}");
}

#[test]
fn fa_sec07_10_compute_jacobian_one_sided_domain_retry_at_upper_edge() {
    // Domain y = sqrt(10 - K * X) over X in [1..=5]: valid only for K <= 2.0!
    // Put K at upper edge 2.0 - 1e-7 where +h_c crosses 2.0 (invalid domain) so LM must retry -h_f.
    let model = CompiledDslModel::compile("sqrt_edge", "Y = A * sqrt(10.0 - K * X)").unwrap();
    let xs = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let ys: Vec<f64> = xs.iter().map(|&x| 3.0 * (10.0 - 1.999_999 * x).sqrt()).collect();
    let ds = vec![series_xy(0, "Edge", &xs, &ys)];
    let cfg = SolverConfig {
        use_simplex_warm_start: false,
        compute_profile_ci: false,
        parameter_rules: vec![
            ParameterRule { name: "A".to_owned(), scope: ParameterScope::Local, initial_override: Some(2.9), lower_bound: None, upper_bound: None },
            ParameterRule {
                name: "K".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(1.999_999_9),
                lower_bound: None,
                upper_bound: Some(2.0),
            },
        ],
        ..SolverConfig::default()
    };
    let res = fit_nonlinear(&model, &ds, &HookContext::default(), &cfg).unwrap();
    assert_eq!(res.status, FitStatus::Converged);
    assert!(find_param_val(&res, "A").is_finite() && (find_param_val(&res, "A") - 3.0).abs() < 1e-4);
    assert!(res.parameters[0].se.is_finite() && res.parameters[1].se.is_finite());
}

#[test]
fn fa_sec07_11_curve_overlay_delta_grad_handles_domain_and_bound_edge() {
    // Custom DSL without closed-form grad: optimal A = 2.0 at lb = 2.0, B = 2.0 - 1e-7 near domain sqrt(2.0 - B).
    let model = CompiledDslModel::compile("band_edge", "Y = A * X + sqrt(2.0 - B)").unwrap();
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [2.1, 4.0, 6.2, 7.9, 10.1];
    let ds = vec![series_xy(0, "Band", &xs, &ys)];
    let cfg = SolverConfig {
        compute_bands: true,
        compute_profile_ci: false,
        use_simplex_warm_start: false,
        parameter_rules: vec![
            ParameterRule {
                name: "A".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(2.0),
                lower_bound: Some(2.0),
                upper_bound: None,
            },
            ParameterRule {
                name: "B".to_owned(),
                scope: ParameterScope::Local,
                initial_override: Some(1.999_999_9),
                lower_bound: None,
                upper_bound: Some(2.0),
            },
        ],
        ..SolverConfig::default()
    };
    let res = fit_nonlinear(&model, &ds, &HookContext::default(), &cfg).unwrap();
    let overlay = res.datasets[0].curve_overlay.as_ref().unwrap();
    let mid = &overlay.points[overlay.points.len() / 2];
    let (lo, hi) = (mid.ci_lower.unwrap(), mid.ci_upper.unwrap());
    assert!(hi > lo, "expected positive confidence band width via one-sided delta_grad, got [{lo}, {hi}]");
}

#[test]
fn fa_sec07_12_pre_summarized_non_moment_modes_extract_unknowns() {
    for mode in [SubcolumnMode::MeanLowHigh, SubcolumnMode::MedianIqr] {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, 5).unwrap();
        for (r, (x, y)) in [(1.0, 3.0), (2.0, 5.0), (3.0, 7.0)].into_iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x)).unwrap();
            xy.data_sets[0].cells[r][0] = Cell::number(y);
            xy.data_sets[0].cells[r][1] = Cell::number(y - 0.5);
            xy.data_sets[0].cells[r][2] = Cell::number(y + 0.5);
        }
        // Row 3: blank X, Y=9.0 -> unknown X from Y (3.0)
        xy.data_sets[0].cells[3][0] = Cell::number(9.0);
        xy.data_sets[0].cells[3][1] = Cell::number(8.5);
        xy.data_sets[0].cells[3][2] = Cell::number(9.5);
        // Row 4: X=5.0, blank Y subcells -> unknown Y from X (11.0)
        xy.set_x_cell(4, Cell::number(5.0)).unwrap();

        let sheet = DataSheet::new(SheetId::new("xy-12"), "NonMoment", TableArchetype::Xy(xy)).unwrap();
        let extracted = extract_datasets_from_sheet(&sheet, &[]).unwrap();
        assert_eq!(extracted[0].unknowns_y.len(), 1, "mode={mode:?}");
        assert_eq!(extracted[0].unknowns_x, vec![(4, 5.0)], "mode={mode:?}");

        let mut nl = NonlinearFitSpec::new("straight_line");
        nl.interpolate_unknowns = true;
        nl.profile_likelihood_ci = false;
        let rs =
            execute_analysis(&AnalysisSpec::new("fit-12", "xy-12", AnalysisKind::NonlinearRegression(nl)).unwrap(), &sheet, &HookContext::default())
                .unwrap();
        assert!(rs.tabs.iter().any(|t| t.id == "nonlinear_interpolated"));
    }
}

#[test]
fn fa_sec07_13_selected_data_sets_rejects_oob_and_duplicates() {
    let sheet_xy = make_xy_sheet("xy-13", "XY", &[("A", &[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]), ("B", &[1.0, 2.0, 3.0], &[3.0, 5.0, 7.0])]);
    for bad in [&[99][..], &[0, 0][..]] {
        let err = extract_datasets_from_sheet(&sheet_xy, bad).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { name: "selected_data_sets", .. }), "xy bad={bad:?}: {err:?}");
    }

    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 3).unwrap();
    for r in 0..3 {
        gt.data_sets[0].cells[r][0] = Cell::number(r as f64);
        gt.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let sheet_g = DataSheet::new(SheetId::new("g-13"), "G", TableArchetype::Grouped(gt)).unwrap();
    for bad in [&[5][..], &[1, 1][..]] {
        let err = extract_datasets_from_sheet(&sheet_g, bad).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { name: "selected_data_sets", .. }), "grouped bad={bad:?}: {err:?}");
    }

    let mut mv = MultipleVariablesTable::new(2, 3).unwrap();
    mv.variables[0].lock_role(VariableRole::Continuous);
    mv.variables[1].lock_role(VariableRole::Continuous);
    for r in 0..3 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[1].cells[r] = Cell::number((2 * (r + 1)) as f64);
    }
    let sheet_mv = DataSheet::new(SheetId::new("mv-13"), "MV", TableArchetype::MultipleVariables(mv)).unwrap();
    for bad in [&[9][..], &[0, 0][..]] {
        let err = extract_datasets_from_sheet(&sheet_mv, bad).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { name: "selected_data_sets", .. }), "mv bad={bad:?}: {err:?}");
    }
}

#[test]
fn fa_sec07_14_plot_function_only_validates_eval_and_computes_gof() {
    let hooks = HookContext::default();
    let ds = vec![series_xy(0, "Line", &[1.0, 2.0, 3.0, 4.0], &[3.0, 5.0, 7.0, 9.0])];

    // 1. All-None evaluation on non-empty dataset must fail instead of returning fake R^2 = 1.0.
    let undef_model = CompiledDslModel::compile("always_none", "Y = A * sqrt(-1.0 - X * X)").unwrap();
    let cfg_plot = SolverConfig { plot_function_only: true, ..SolverConfig::default() };
    let err = fit_nonlinear(&undef_model, &ds, &hooks, &cfg_plot).unwrap_err();
    assert!(matches!(err, FitError::BadInitialValues { .. }), "got {err:?}");

    // 2. Deliberate bad constants on non-empty dataset report real dataset R^2 != 1 and sy_x > 0.
    let line = get_builtin_model("straight_line").unwrap();
    let cfg_bad_line = SolverConfig {
        plot_function_only: true,
        parameter_rules: vec![
            ParameterRule {
                name: "Slope".to_owned(),
                scope: ParameterScope::Constant(0.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "YIntercept".to_owned(),
                scope: ParameterScope::Constant(0.0),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ],
        ..SolverConfig::default()
    };
    let fit_plot = fit_nonlinear(line.as_ref(), &ds, &hooks, &cfg_bad_line).unwrap();
    let raw_rss = 9.0 + 25.0 + 49.0 + 81.0; // = 164
    let tss = 9.0 + 1.0 + 1.0 + 9.0; // = 20
    assert_close(fit_plot.datasets[0].r_squared, 1.0 - raw_rss / tss, 1e-12, 1e-12, "plot_only ds R2");
    assert_close(fit_plot.datasets[0].sy_x, (raw_rss / 4.0_f64).sqrt(), 1e-12, 1e-12, "plot_only ds sy_x");

    // 3. Empty dataset plot_function_only still succeeds cleanly.
    let empty_ds = vec![series_xy(0, "Empty", &[], &[])];
    let empty_fit = fit_nonlinear(line.as_ref(), &empty_ds, &hooks, &cfg_bad_line).unwrap();
    assert_eq!(empty_fit.n_obs, 0);
}

#[test]
fn fa_sec07_15_linear_and_ic_contracts_match_docstrings() {
    let pts = vec![DataPoint::xy(1.0, 2.1), DataPoint::xy(2.0, 3.9), DataPoint::xy(3.0, 6.1), DataPoint::xy(4.0, 8.0)];
    let orig = simple_linear_regression(&pts, &SimpleLinearOptions { force_through_origin: true, ..SimpleLinearOptions::default() }).unwrap();
    assert!(orig.r.is_nan(), "expected r=NaN when force_through_origin=true, got {}", orig.r);
    assert!(orig.x_intercept.is_none(), "expected x_intercept=None when force_through_origin=true");

    let flat = vec![DataPoint::xy(1.0, 5.0), DataPoint::xy(2.0, 5.0), DataPoint::xy(3.0, 5.0), DataPoint::xy(4.0, 5.0)];
    let flat_fit = simple_linear_regression(&flat, &SimpleLinearOptions::default()).unwrap();
    assert!(flat_fit.r.is_nan(), "expected r=NaN on flat zero-variance Y, got {}", flat_fit.r);

    // compare_linear_lines must omit ANCOVA intercept fields when slopes differ significantly.
    let line_a = vec![DataPoint::xy(1.0, 2.0), DataPoint::xy(2.0, 4.0), DataPoint::xy(3.0, 6.0), DataPoint::xy(4.0, 8.0)];
    let line_b = vec![DataPoint::xy(1.0, 10.1), DataPoint::xy(2.0, 20.0), DataPoint::xy(3.0, 29.9), DataPoint::xy(4.0, 40.1)];
    let cmp = compare_linear_lines(&[&line_a, &line_b], false).unwrap();
    assert!(cmp.slopes_significantly_different);
    assert!(cmp.f_intercepts.is_none());
    assert!(cmp.df_intercepts.is_none());
    assert!(cmp.p_intercepts.is_none());
    assert!(cmp.intercepts_significantly_different.is_none());

    // compute_information_criteria rejects negative / NaN / infinite ss cleanly.
    for bad_ss in [-1.0, f64::NAN, f64::INFINITY] {
        let err = compute_information_criteria(bad_ss, 10, 2).unwrap_err();
        assert!(matches!(err, FitError::InvalidParameter { name: "ss", .. }), "bad_ss={bad_ss}: {err:?}");
    }
}

#[test]
fn fa_sec07_16_unified_rout_pipeline_parity() {
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let mut ys: Vec<f64> = xs.iter().map(|&x| 2.0 + 3.0 * x).collect();
    ys[10] += 35.0;
    let ds = series_xy(0, "ROUT", &xs, &ys);
    let residuals: Vec<f64> = ds.points.iter().map(|p| p.y - (2.0 + 3.0 * p.x0())).collect();
    let rout = rout_detect_outliers(&ds.points, &residuals, 2, 1.0).unwrap();
    assert_eq!(rout.outlier_indices, vec![10]);
}
