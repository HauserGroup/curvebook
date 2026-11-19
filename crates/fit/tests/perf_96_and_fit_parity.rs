#![forbid(unsafe_code)]

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, LevelOrderKind, ModelComparisonSpec, MultipleVariablesTable, NonlinearFitSpec,
    ParameterRule, ParameterScope, ResultCell, ResultStatus, ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, FitError, FitStatus, HookContext, ModelComparisonOutcome, MultivarExtractionSpec, SolverConfig,
    anderson_darling_test, compare_models, compare_models_lrt, execute_analysis, extract_datasets_from_sheet,
    extract_datasets_from_sheet_with_options, fit_nonlinear, get_builtin_model, homoscedasticity_spearman_test, lilliefors_ks_test,
};

fn build_96_curve_xy_sheet() -> Result<(DataSheet, Vec<[f64; 4]>), String> {
    let log_concs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let n_curves = 96_usize;
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), n_curves, log_concs.len()).map_err(|e| format!("{e:?}"))?;
    xy.x_title = String::from("Log[Agonist] (M)");

    let mut truth = Vec::with_capacity(n_curves);
    for (r, &lx) in log_concs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(lx);
        for c in 0..n_curves {
            let bottom = 5.0 + ((c % 4) as f64);
            let top = 95.0 + 2.0 * ((c % 5) as f64);
            let log_ec50 = -7.2 + 0.015 * (c as f64);
            let hill = 1.0 + 0.08 * ((c % 4) as f64);
            if r == 0 {
                truth.push([bottom, top, log_ec50, hill]);
            }
            let mean_y = bottom + (top - bottom) / (1.0 + 10.0_f64.powf((log_ec50 - lx) * hill));
            let jitter = [-0.6, 0.0, 0.6];
            for (rep, &jit) in jitter.iter().enumerate() {
                xy.data_sets[c].cells[r][rep] = Cell::number(mean_y + jit);
            }
        }
    }

    let ds = DataSheet::new(SheetId::new("dr_96"), "96-Well Dose Response", TableArchetype::Xy(xy)).map_err(|e| format!("{e:?}"))?;
    Ok((ds, truth))
}

fn find_tab(tabs: &[ResultTab], kind: ResultTabKind) -> Option<&ResultTab> {
    tabs.iter().find(|t| t.kind == kind)
}

#[test]
fn g12_96_curve_4pl_batch_runs_under_50ms_and_recovers_all_curves() {
    let Ok((ds, truth)) = build_96_curve_xy_sheet() else {
        panic_free_fail("failed to build 96-curve dose-response sheet");
        return;
    };
    let Ok(datasets) = extract_datasets_from_sheet(&ds, &[]) else {
        panic_free_fail("failed to extract datasets from 96-curve sheet");
        return;
    };
    let Some(model) = get_builtin_model("log_agonist_vs_response_4pl") else {
        panic_free_fail("missing built-in log_agonist_vs_response_4pl model");
        return;
    };

    let cfg = SolverConfig { compute_profile_ci: false, curve_overlay_segments: 256, ..SolverConfig::default() };

    let t0 = Instant::now();
    let fit_res = fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &cfg);
    let fit_ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("G12 96-curve fit_nonlinear time: {fit_ms:.2} ms");

    let Ok(fit) = fit_res else {
        panic_free_fail(&format!("96-curve fit_nonlinear failed: {:?}", fit_res.err()));
        return;
    };
    assert!(fit_ms < 50.0, "expected 96-curve fit_nonlinear < 50 ms, took {fit_ms:.2} ms");
    assert_eq!(fit.status, FitStatus::Converged);
    assert_eq!(fit.datasets.len(), 96);
    assert_eq!(fit.k_free, 96 * 4);
    assert_eq!(fit.n_obs, 96 * 24);
    assert_eq!(fit.covariance.covariance_matrix.len(), 384);

    for (c, ds_fit) in fit.datasets.iter().enumerate() {
        assert_eq!(ds_fit.data_set_index, c);
        let overlay_len = ds_fit.curve_overlay.as_ref().map_or(0, |o| o.points.len());
        assert_eq!(overlay_len, 256, "curve {c} should sample 256 overlay points");
        for (p_idx, (&est, &expected)) in ds_fit.params.iter().zip(truth[c].iter()).enumerate() {
            let tol = if p_idx <= 1 { 0.5 } else { 0.05 };
            assert!((est - expected).abs() < tol, "curve {c} param {p_idx}: expected {expected:.4}, got {est:.4}");
        }
    }

    // Check block-diagonal cross-dataset covariance is exact zero.
    assert_eq!(fit.covariance.covariance_matrix[0][4], 0.0);
    assert_eq!(fit.covariance.covariance_matrix[4][0], 0.0);

    // End-to-end ResultSheet execution via execute_analysis must also stay well under 50 ms.
    let mut nl_spec = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_spec.profile_likelihood_ci = false;
    nl_spec.confidence_bands = false;
    let Ok(analysis) = AnalysisSpec::new(SheetId::new("fit_96"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl_spec)) else {
        panic_free_fail("failed to build AnalysisSpec");
        return;
    };

    let t1 = Instant::now();
    let res = execute_analysis(&analysis, &ds, &HookContext::default());
    let exec_ms = t1.elapsed().as_secs_f64() * 1000.0;
    eprintln!("G12 96-curve execute_analysis time: {exec_ms:.2} ms");

    let Ok(sheet) = res else {
        panic_free_fail(&format!("execute_analysis failed: {:?}", res.err()));
        return;
    };
    assert!(exec_ms < 50.0, "expected 96-curve execute_analysis < 50 ms, took {exec_ms:.2} ms");
    assert_eq!(sheet.status, ResultStatus::UpToDate);
    assert_eq!(sheet.fitted_curves.len(), 96);
    assert!(find_tab(&sheet.tabs, ResultTabKind::Residuals).is_some());
}

#[test]
fn per_dataset_constant_and_column_title_constant_scopes() {
    let Some(model) = get_builtin_model("straight_line") else {
        panic_free_fail("missing straight_line model");
        return;
    };
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ds0 = DatasetSeries {
        data_set_index: 0,
        label: String::from("10.0"),
        points: xs
            .iter()
            .enumerate()
            .map(|(r, &x)| DataPoint {
                x: vec![x],
                y: 2.0 * x + 10.0,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row: r,
            })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: Some(10.0),
    };
    let ds1 = DatasetSeries {
        data_set_index: 1,
        label: String::from("25.0"),
        points: xs
            .iter()
            .enumerate()
            .map(|(r, &x)| DataPoint {
                x: vec![x],
                y: 4.0 * x + 25.0,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 1,
                row: r,
            })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: Some(25.0),
    };
    let datasets = [ds0, ds1];

    // Partial PerDatasetConstant: dataset 0 fixed to 10.0, dataset 1 left free.
    let partial_cfg = SolverConfig {
        compute_profile_ci: false,
        parameter_rules: vec![ParameterRule {
            name: String::from("YIntercept"),
            scope: ParameterScope::PerDatasetConstant(vec![(0, 10.0)]),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let Ok(fit_partial) = fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &partial_cfg) else {
        panic_free_fail("partial PerDatasetConstant fit failed");
        return;
    };
    assert_eq!(fit_partial.k_free, 3); // Slope (2 datasets) + YIntercept (dataset 1 only)
    assert!((fit_partial.datasets[0].params[0] - 2.0).abs() < 1e-6);
    assert!((fit_partial.datasets[0].params[1] - 10.0).abs() < 1e-12);
    assert!((fit_partial.datasets[1].params[0] - 4.0).abs() < 1e-6);
    assert!((fit_partial.datasets[1].params[1] - 25.0).abs() < 1e-6);

    // ColumnTitleConstant: each dataset binds YIntercept from its numeric column title.
    let col_title_cfg = SolverConfig {
        compute_profile_ci: false,
        parameter_rules: vec![ParameterRule {
            name: String::from("YIntercept"),
            scope: ParameterScope::ColumnTitleConstant,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let Ok(fit_col) = fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &col_title_cfg) else {
        panic_free_fail("ColumnTitleConstant fit failed");
        return;
    };
    assert_eq!(fit_col.k_free, 2); // Slope for dataset 0 and dataset 1
    assert!((fit_col.datasets[0].params[0] - 2.0).abs() < 1e-6);
    assert!((fit_col.datasets[0].params[1] - 10.0).abs() < 1e-12);
    assert!((fit_col.datasets[1].params[0] - 4.0).abs() < 1e-6);
    assert!((fit_col.datasets[1].params[1] - 25.0).abs() < 1e-12);

    // Non-numeric column title must return an explicit HookResolutionError.
    let mut bad_datasets = datasets.to_vec();
    bad_datasets[1].column_title_value = None;
    bad_datasets[1].label = String::from("Ctrl");
    let err = fit_nonlinear(model.as_ref(), &bad_datasets, &HookContext::default(), &col_title_cfg);
    assert!(matches!(err, Err(FitError::HookResolutionError { .. })));
}

#[test]
fn range_filters_exclude_out_of_bounds_points() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 7).ok();
    let Some(ref mut tbl) = xy else {
        panic_free_fail("failed to create XyTable");
        return;
    };
    let raw_pts = [
        (-5.0, 999.0), // excluded by x_range_filter min = 0.0
        (1.0, 3.0),
        (2.0, 5.0),
        (3.0, 500.0), // excluded by y_range_filter max = 50.0
        (4.0, 9.0),
        (5.0, 11.0),
        (20.0, -50.0), // excluded by both filters
    ];
    for (r, &(x, y)) in raw_pts.iter().enumerate() {
        tbl.x_cells[r][0] = Cell::number(x);
        tbl.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let Some(table) = xy.take() else {
        panic_free_fail("unexpected None table");
        return;
    };
    let Ok(ds) = DataSheet::new(SheetId::new("range_ds"), "Range Filter Test", TableArchetype::Xy(table)) else {
        panic_free_fail("failed to wrap DataSheet");
        return;
    };

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.x_range_filter = Some((Some(0.0), Some(10.0)));
    nl.y_range_filter = Some((Some(0.0), Some(50.0)));
    nl.profile_likelihood_ci = false;
    let Ok(spec) = AnalysisSpec::new(SheetId::new("range_fit"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl)) else {
        panic_free_fail("failed to build AnalysisSpec");
        return;
    };
    let Ok(sheet) = execute_analysis(&spec, &ds, &HookContext::default()) else {
        panic_free_fail("execute_analysis with range filters failed");
        return;
    };
    assert_eq!(sheet.status, ResultStatus::UpToDate);
    assert_eq!(sheet.fitted_curves.len(), 1);
    let slope = sheet.lookup_scalar("Slope", Some(0)).unwrap_or(f64::NAN);
    let intercept = sheet.lookup_scalar("YIntercept", Some(0)).unwrap_or(f64::NAN);
    assert!((slope - 2.0).abs() < 1e-6, "expected Slope=2.0 after filter, got {slope}");
    assert!((intercept - 1.0).abs() < 1e-6, "expected YIntercept=1.0 after filter, got {intercept}");
}

#[test]
fn plot_function_only_and_log_x_grid_sampling() {
    // Empty XY table with plot_function_only=true over positive span >= 100x uses log spacing.
    let Ok(xy_empty) = XyTable::new(SubcolumnMode::Single, 1, 2) else {
        panic_free_fail("failed to create empty XyTable");
        return;
    };
    let Ok(empty_ds) = DataSheet::new(SheetId::new("empty_xy"), "Empty XY", TableArchetype::Xy(xy_empty)) else {
        panic_free_fail("failed to create DataSheet");
        return;
    };
    let mut nl_plot = NonlinearFitSpec::new("agonist_vs_response_4pl");
    nl_plot.plot_function_only = true;
    nl_plot.plot_x_min = Some(1.0);
    nl_plot.plot_x_max = Some(1000.0);
    nl_plot.parameter_rules = vec![
        ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::Constant(10.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::Constant(110.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("EC50"),
            scope: ParameterScope::Constant(30.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        ParameterRule {
            name: String::from("HillSlope"),
            scope: ParameterScope::Constant(1.2),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    let Ok(spec) = AnalysisSpec::new(SheetId::new("plot_only"), empty_ds.id.clone(), AnalysisKind::NonlinearRegression(nl_plot)) else {
        panic_free_fail("failed to build AnalysisSpec");
        return;
    };
    let Ok(res) = execute_analysis(&spec, &empty_ds, &HookContext::default()) else {
        panic_free_fail("plot_function_only execution failed on empty table");
        return;
    };
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!(res.fitted_curves.len(), 1);
    let pts = &res.fitted_curves[0].points;
    assert_eq!(pts.len(), 256);
    assert!((pts[0].x - 1.0).abs() < 1e-12);
    assert!((pts[255].x - 1000.0).abs() < 1e-9);
    let ratio0 = pts[1].x / pts[0].x;
    let expected_ratio = 1000.0_f64.powf(1.0 / 255.0);
    assert!((ratio0 - expected_ratio).abs() < 1e-9);
    for w in pts.windows(2) {
        let r = w[1].x / w[0].x;
        assert!((r - expected_ratio).abs() < 1e-9, "expected constant geometric ratio {expected_ratio}, got {r}");
    }

    // Contrast: narrow positive span (< 100x) uses linear spacing.
    let Some(line_model) = get_builtin_model("straight_line") else {
        panic_free_fail("missing straight_line");
        return;
    };
    let narrow_ds = DatasetSeries {
        data_set_index: 0,
        label: String::from("Narrow"),
        points: [1.0, 10.0, 25.0, 50.0]
            .iter()
            .enumerate()
            .map(|(r, &x)| DataPoint {
                x: vec![x],
                y: 2.0 * x + 1.0,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row: r,
            })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    };
    let narrow_cfg = SolverConfig { compute_profile_ci: false, curve_overlay_segments: 100, ..SolverConfig::default() };
    let Ok(narrow_fit) = fit_nonlinear(line_model.as_ref(), &[narrow_ds], &HookContext::default(), &narrow_cfg) else {
        panic_free_fail("narrow linear fit failed");
        return;
    };
    let Some(overlay) = &narrow_fit.datasets[0].curve_overlay else {
        panic_free_fail("missing curve overlay");
        return;
    };
    let step0 = overlay.points[1].x - overlay.points[0].x;
    for w in overlay.points.windows(2) {
        assert!(((w[1].x - w[0].x) - step0).abs() < 1e-9, "expected linear step {step0}");
    }
}

#[test]
fn residual_homoscedasticity_ad_ks_and_lrt_comparison_parity() {
    // 1. Direct homoscedasticity & normality tests
    let y_hat: Vec<f64> = (1..=20).map(|i| i as f64).collect();
    let homo_res: Vec<f64> =
        [0.2, -0.3, 0.1, -0.25, 0.18, -0.15, 0.22, -0.2, 0.12, -0.1, 0.25, -0.22, 0.14, -0.16, 0.19, -0.18, 0.11, -0.12, 0.21, -0.24].to_vec();
    let hetero_res: Vec<f64> = y_hat.iter().enumerate().map(|(i, &y)| if i % 2 == 0 { y } else { -y }).collect();

    let Some(homo_check) = homoscedasticity_spearman_test(&y_hat, &homo_res, 0.05) else {
        panic_free_fail("homoscedasticity test returned None");
        return;
    };
    assert!(homo_check.homoscedastic, "expected homoscedastic=true, p={}", homo_check.p_value);

    let Some(hetero_check) = homoscedasticity_spearman_test(&y_hat, &hetero_res, 0.05) else {
        panic_free_fail("heteroscedasticity test returned None");
        return;
    };
    assert!(!hetero_check.homoscedastic, "expected homoscedastic=false, p={}", hetero_check.p_value);
    assert!(hetero_check.spearman_rho > 0.9);
    assert!(hetero_check.p_value < 1e-4);

    // Normal-looking vs strongly skewed sample
    let normal_sample = [-1.64, -1.28, -0.98, -0.76, -0.56, -0.38, -0.21, -0.05, 0.10, 0.25, 0.41, 0.58, 0.77, 1.00, 1.32, 1.72];
    let skewed_sample = [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.08, 0.10, 0.15, 0.25, 0.60, 2.50, 8.00, 24.0];
    let Some(ad_ok) = anderson_darling_test(&normal_sample) else {
        panic_free_fail("AD normal returned None");
        return;
    };
    let Some(ad_bad) = anderson_darling_test(&skewed_sample) else {
        panic_free_fail("AD skewed returned None");
        return;
    };
    let Some(ks_ok) = lilliefors_ks_test(&normal_sample) else {
        panic_free_fail("KS normal returned None");
        return;
    };
    let Some(ks_bad) = lilliefors_ks_test(&skewed_sample) else {
        panic_free_fail("KS skewed returned None");
        return;
    };
    assert!(ad_ok.passed, "expected normal sample to pass AD: p={}", ad_ok.p_value);
    assert!(!ad_bad.passed && ad_bad.p_value < 0.01, "expected skewed sample to fail AD: p={}", ad_bad.p_value);
    assert!(ks_ok.passed, "expected normal sample to pass Lilliefors KS: p={}", ks_ok.p_value);
    assert!(!ks_bad.passed && ks_bad.p_value < 0.01, "expected skewed sample to fail Lilliefors KS: p={}", ks_bad.p_value);

    // 2. LRT formula + execute_analysis end-to-end
    let Ok(lrt_direct) = compare_models_lrt(20.0, 17, 2.0, 16, 20, 0.05) else {
        panic_free_fail("compare_models_lrt failed");
        return;
    };
    let expected_d = 20.0 * (20.0_f64 / 2.0).ln();
    assert!((lrt_direct.lr_statistic - expected_d).abs() < 1e-12);
    assert_eq!(lrt_direct.df_diff, 1);
    assert!(lrt_direct.p_value < 1e-6);
    assert!(lrt_direct.preferred_complex_model);

    // Build 4PL asymmetric-slope data so 4PL decisively beats 3PL via LRT in execute_analysis.
    let log_concs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0, -3.5];
    let Ok(mut xy) = XyTable::new(SubcolumnMode::Single, 1, log_concs.len()) else {
        panic_free_fail("failed to create LRT XY table");
        return;
    };
    for (r, &lx) in log_concs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(lx);
        let y = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - lx) * 2.5));
        let tiny_jit = if r % 2 == 0 { 0.05 } else { -0.05 };
        xy.data_sets[0].cells[r][0] = Cell::number(y + tiny_jit);
    }
    let Ok(ds) = DataSheet::new(SheetId::new("lrt_ds"), "LRT Dose", TableArchetype::Xy(xy)) else {
        panic_free_fail("failed DataSheet");
        return;
    };
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_3pl");
    nl.profile_likelihood_ci = false;
    nl.normality_of_residuals = true;
    nl.homoscedasticity_test = true;
    nl.comparison = Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_4pl") });
    let Ok(spec) = AnalysisSpec::new(SheetId::new("lrt_spec"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl)) else {
        panic_free_fail("failed AnalysisSpec");
        return;
    };
    let Ok(sheet) = execute_analysis(&spec, &ds, &HookContext::default()) else {
        panic_free_fail("execute_analysis LRT failed");
        return;
    };
    let Some(comp_tab) = find_tab(&sheet.tabs, ResultTabKind::ModelComparison) else {
        panic_free_fail("missing ModelComparison tab");
        return;
    };
    assert!(comp_tab.title.contains("Likelihood Ratio"));
    assert!(comp_tab.row_labels.iter().any(|l| l.starts_with("LR statistic")));
    assert!(sheet.checklist.iter().any(|c| c.assumption.contains("Homoscedasticity")));
    assert!(sheet.checklist.iter().any(|c| c.assumption.contains("Anderson")));
    assert!(sheet.checklist.iter().any(|c| c.assumption.contains("Lilliefors")));

    // Direct solver::compare_models dispatch for LikelihoodRatioTest
    let Some(m_null) = get_builtin_model("log_agonist_vs_response_3pl") else {
        panic_free_fail("missing 3PL");
        return;
    };
    let Some(m_alt) = get_builtin_model("log_agonist_vs_response_4pl") else {
        panic_free_fail("missing 4PL");
        return;
    };
    let Ok(extracted) = extract_datasets_from_sheet(&ds, &[]) else {
        panic_free_fail("extract failed");
        return;
    };
    let cmp_cfg = SolverConfig { compute_profile_ci: false, ..SolverConfig::default() };
    let Ok(fit_null) = fit_nonlinear(m_null.as_ref(), &extracted, &HookContext::default(), &cmp_cfg) else {
        panic_free_fail("primary 3PL fit failed");
        return;
    };
    let Ok((alt_fit, cmp_out)) = compare_models(
        &fit_null,
        m_alt.as_ref(),
        &extracted,
        &HookContext::default(),
        &cmp_cfg,
        &ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_4pl") },
    ) else {
        panic_free_fail("compare_models LRT failed");
        return;
    };
    assert_eq!(alt_fit.equation_id, "log_agonist_vs_response_4pl");
    match cmp_out {
        ModelComparisonOutcome::LikelihoodRatioTest(r) => {
            assert_eq!(r.df_diff, 1);
            assert!(r.preferred_complex_model);
            assert!(r.p_value < 0.01);
        }
        other => panic_free_fail(&format!("expected LikelihoodRatioTest outcome, got {other:?}")),
    }
}

#[test]
fn open_ci_bounds_on_zero_df_and_ambiguous_covariance() {
    let Some(line_model) = get_builtin_model("straight_line") else {
        panic_free_fail("missing straight_line");
        return;
    };
    // Exactly n_obs == 2 == k_free -> df == 0 -> Wald CIs must emit CiBound::Open.
    let ds_zero_df = DatasetSeries {
        data_set_index: 0,
        label: String::from("ZeroDF"),
        points: vec![
            DataPoint { x: vec![1.0], y: 3.0, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 },
            DataPoint { x: vec![2.0], y: 5.0, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 1 },
        ],
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    };
    let wald_cfg = SolverConfig { compute_profile_ci: false, compute_bands: false, ..SolverConfig::default() };
    let Ok(fit_zero_df) = fit_nonlinear(line_model.as_ref(), &[ds_zero_df], &HookContext::default(), &wald_cfg) else {
        panic_free_fail("zero df fit failed");
        return;
    };
    assert_eq!(fit_zero_df.df, 0);
    for p in &fit_zero_df.parameters {
        assert_eq!(p.ci_lower, CiBound::Open, "expected Open lower CI at df=0 for {}", p.name);
        assert_eq!(p.ci_upper, CiBound::Open, "expected Open upper CI at df=0 for {}", p.name);
    }

    // Ambiguous covariance (4PL fit to identical constant X values makes LogEC50/HillSlope unidentifiable).
    let Some(model_4pl) = get_builtin_model("log_agonist_vs_response_4pl") else {
        panic_free_fail("missing 4PL");
        return;
    };
    let ds_collinear = DatasetSeries {
        data_set_index: 0,
        label: String::from("IdenticalX"),
        points: (0..6)
            .map(|r| DataPoint {
                x: vec![-6.0],
                y: 50.0 + ((r % 2) as f64) * 0.1,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row: r,
            })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    };
    let Ok(fit_ambig) = fit_nonlinear(model_4pl.as_ref(), &[ds_collinear], &HookContext::default(), &wald_cfg) else {
        panic_free_fail("ambiguous fit failed");
        return;
    };
    assert!(fit_ambig.covariance.ambiguous, "expected ambiguous=true on identical X");
    for p in &fit_ambig.parameters {
        assert_eq!(p.ci_lower, CiBound::Open, "expected Open lower CI on ambiguous cov for {}", p.name);
        assert_eq!(p.ci_upper, CiBound::Open, "expected Open upper CI on ambiguous cov for {}", p.name);
    }
}

#[test]
fn multivar_cartesian_subgroups_x_columns_and_y_column() {
    // Rows cover 2 Drugs x 2 Sexes + 1 excluded cell + 1 missing Y row.
    // Sex column specifies Explicit order ["M", "F"] so Cartesian groups must appear in deterministic order:
    //   Ctrl × M, Ctrl × F, Treated × M, Treated × F.
    let Ok(mut mv) = MultipleVariablesTable::new(4, 10) else {
        panic_free_fail("failed to create MV table");
        return;
    };
    mv.variables[0].title = String::from("Drug");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Sex");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[1].level_order.order = LevelOrderKind::Explicit(vec![String::from("M"), String::from("F")]);
    mv.variables[2].title = String::from("Dose");
    mv.variables[3].title = String::from("Response");

    // Deliberately insert Treated/F rows earlier than Ctrl/M rows to verify ordered_levels Cartesian sorting!
    let records = [
        ("Treated", "F", 1.0, 14.0, false),
        ("Treated", "F", 2.0, 24.0, false),
        ("Ctrl", "F", 1.0, 5.0, false),
        ("Ctrl", "F", 2.0, 9.0, false),
        ("Treated", "M", 1.0, 12.0, false),
        ("Treated", "M", 2.0, 22.0, false),
        ("Ctrl", "M", 1.0, 3.0, false),
        ("Ctrl", "M", 2.0, 5.0, false),
        ("Ctrl", "M", 99.0, 999.0, true),       // excluded Y cell -> skipped
        ("Treated", "M", 3.0, f64::NAN, false), // empty/invalid below
    ];
    for (r, &(drug, sex, dose, resp, excl)) in records.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(drug);
        mv.variables[1].cells[r] = Cell::text(sex);
        mv.variables[2].cells[r] = Cell::number(dose);
        mv.variables[3].cells[r] = if resp.is_finite() { Cell { excluded: excl, ..Cell::number(resp) } } else { Cell::empty() };
    }

    let Ok(ds) = DataSheet::new(SheetId::new("mv_ds"), "MV Cartesian", TableArchetype::MultipleVariables(mv)) else {
        panic_free_fail("failed MV DataSheet");
        return;
    };

    let mv_spec = MultivarExtractionSpec { group_columns: &[0, 1], x_columns: &[2], y_column: Some(3) };
    let Ok(series) = extract_datasets_from_sheet_with_options(&ds, &[], &mv_spec) else {
        panic_free_fail("extract_datasets_from_sheet_with_options failed");
        return;
    };
    assert_eq!(series.len(), 4);
    assert_eq!(series[0].label, "Treated | M"); // First occurrence in EntryOrder for Drug is Treated; Sex Explicit M then F
    assert_eq!(series[1].label, "Treated | F");
    assert_eq!(series[2].label, "Ctrl | M");
    assert_eq!(series[3].label, "Ctrl | F");
    assert_eq!(series[2].points.len(), 2); // excluded Y skipped

    // Also test selected_data_sets filtering + execute_analysis MV dispatch.
    let Ok(subset) = extract_datasets_from_sheet_with_options(&ds, &[2, 3], &mv_spec) else {
        panic_free_fail("subset extraction failed");
        return;
    };
    assert_eq!(subset.len(), 2);
    assert_eq!(subset[0].label, "Ctrl | M");
    assert_eq!(subset[1].label, "Ctrl | F");

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.profile_likelihood_ci = false;
    nl.mv_group_columns = vec![0, 1];
    nl.mv_x_columns = vec![2];
    nl.mv_y_column = Some(3);
    let Ok(spec) = AnalysisSpec::new(SheetId::new("mv_fit"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl)) else {
        panic_free_fail("failed MV AnalysisSpec");
        return;
    };
    let Ok(res) = execute_analysis(&spec, &ds, &HookContext::default()) else {
        panic_free_fail("execute_analysis MV failed");
        return;
    };
    assert_eq!(res.fitted_curves.len(), 4);
    assert!((res.lookup_scalar("Slope", Some(0)).unwrap_or(0.0) - 10.0).abs() < 1e-6);
    assert!((res.lookup_scalar("Slope", Some(2)).unwrap_or(0.0) - 2.0).abs() < 1e-6);
}

#[test]
fn residuals_and_covariance_matrix_tabs_emitted_with_qq_and_studentized_columns() {
    let log_concs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let Ok(mut xy) = XyTable::new(SubcolumnMode::Single, 1, log_concs.len()) else {
        panic_free_fail("failed to build XyTable");
        return;
    };
    let noise = [0.4, -0.5, 0.3, -0.2, 0.5, -0.4, 0.25, -0.35];
    for (r, &lx) in log_concs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(lx);
        let y = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - lx) * 1.1)) + noise[r];
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let Ok(ds) = DataSheet::new(SheetId::new("tabs_ds"), "Tabs DS", TableArchetype::Xy(xy)) else {
        panic_free_fail("failed DataSheet");
        return;
    };

    let mut nl_on = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_on.profile_likelihood_ci = false;
    nl_on.emit_residuals_tab = true;
    nl_on.emit_covariance_tab = true;
    let Ok(spec_on) = AnalysisSpec::new(SheetId::new("tabs_on"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl_on)) else {
        panic_free_fail("failed AnalysisSpec");
        return;
    };
    let Ok(res_on) = execute_analysis(&spec_on, &ds, &HookContext::default()) else {
        panic_free_fail("execute_analysis tabs_on failed");
        return;
    };

    let Some(res_tab) = find_tab(&res_on.tabs, ResultTabKind::Residuals) else {
        panic_free_fail("missing Residuals tab");
        return;
    };
    assert_eq!(res_tab.column_headers, vec!["Predicted Y", "Residual", "Weighted Residual", "Studentized Residual", "Q-Q Normal Quantile",]);
    assert_eq!(res_tab.rows.len(), log_concs.len());
    // Check Studentized residuals and Blom Q-Q normal quantiles are finite, with Q-Q symmetric around 0.
    let mut qq_vals = Vec::with_capacity(res_tab.rows.len());
    for row in &res_tab.rows {
        assert_eq!(row.len(), 5);
        for cell in row {
            assert!(matches!(cell, ResultCell::Number { value, .. } if value.is_finite()));
        }
        if let ResultCell::Number { value, .. } = row[4] {
            qq_vals.push(value);
        }
    }
    qq_vals.sort_by(f64::total_cmp);
    assert!((qq_vals[0] + qq_vals[qq_vals.len() - 1]).abs() < 1e-9, "Blom Q-Q quantiles should be symmetric");

    let Some(cov_tab) = find_tab(&res_on.tabs, ResultTabKind::CovarianceMatrix) else {
        panic_free_fail("missing CovarianceMatrix tab");
        return;
    };
    assert_eq!(cov_tab.column_headers.len(), 4);
    assert_eq!(cov_tab.rows.len(), 4);
    for r in 0..4 {
        assert_eq!(cov_tab.rows[r].len(), 4);
        if let ResultCell::Number { value, .. } = cov_tab.rows[r][r] {
            assert!(value > 0.0 && value.is_finite(), "variance diagonal must be > 0");
        } else {
            panic_free_fail("expected finite diagonal variance cell in CovarianceMatrix tab");
        }
    }

    // With both flags disabled, neither tab should be emitted.
    let mut nl_off = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_off.profile_likelihood_ci = false;
    nl_off.emit_residuals_tab = false;
    nl_off.emit_covariance_tab = false;
    let Ok(spec_off) = AnalysisSpec::new(SheetId::new("tabs_off"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl_off)) else {
        panic_free_fail("failed AnalysisSpec off");
        return;
    };
    let Ok(res_off) = execute_analysis(&spec_off, &ds, &HookContext::default()) else {
        panic_free_fail("execute_analysis tabs_off failed");
        return;
    };
    assert!(find_tab(&res_off.tabs, ResultTabKind::Residuals).is_none());
    assert!(find_tab(&res_off.tabs, ResultTabKind::CovarianceMatrix).is_none());
}

#[test]
fn r1_s03_09_dsl_compilation_scales_linearly_wall_clock() {
    // Runs in the serial `CI / performance` stage (`binary(*perf*)`) and takes the minimum
    // of 5 warm runs so a single OS scheduling pause on shared runners does not flake while
    // any true O(n^2) (~16x) regression still fails the 8x bound decisively.
    let best_compile_ns = |src: &str| -> Result<f64, String> {
        // Warm up once before timing.
        CompiledDslModel::compile("scale", src).map_err(|e| format!("warmup compile failed: {e:?}"))?;
        let mut best = f64::INFINITY;
        for _ in 0..5 {
            let t0 = Instant::now();
            let _ = CompiledDslModel::compile("scale", src).map_err(|e| format!("timed compile failed: {e:?}"))?;
            best = best.min(t0.elapsed().as_nanos() as f64);
        }
        Ok(best)
    };

    let build_transforms = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[Transforms]\nT0 = A + B\n");
        for i in 1..n {
            s.push_str(&format!("T{i} = T{} + 1\n", i - 1));
        }
        s
    };
    let Ok(t1000) = best_compile_ns(&build_transforms(1_000)) else {
        panic_free_fail("transforms 1000 compile failed");
        return;
    };
    let Ok(t4000) = best_compile_ns(&build_transforms(4_000)) else {
        panic_free_fail("transforms 4000 compile failed");
        return;
    };
    assert!(t4000 / t1000.max(1_000_000.0) < 8.0, "transforms 4x scaling too slow: t1000={t1000}ns, t4000={t4000}ns");

    let build_inits = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[InitialValues]\nL0 = YMIN\n");
        for i in 1..n {
            s.push_str(&format!("L{i} = L{} + 0.1\n", i - 1));
        }
        s.push_str(&format!("A = L{}\nB = YMAX\n", n - 1));
        s
    };
    let Ok(init_t1000) = best_compile_ns(&build_inits(1_000)) else {
        panic_free_fail("init rules 1000 compile failed");
        return;
    };
    let Ok(init_t4000) = best_compile_ns(&build_inits(4_000)) else {
        panic_free_fail("init rules 4000 compile failed");
        return;
    };
    assert!(init_t4000 / init_t1000.max(1_000_000.0) < 8.0, "init rules 4x scaling too slow: t1000={init_t1000}ns, t4000={init_t4000}ns");
}

fn panic_free_fail(msg: &str) {
    assert!(msg.is_empty(), "{msg}");
}
