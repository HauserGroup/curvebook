#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, LevelOrderKind, ModelComparisonSpec, MultipleVariablesTable, ResultCell, ResultStatus,
    ResultTab, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_fit::{
    CompiledDslModel, FitError, FitStatus, HookContext, ModelComparisonOutcome, MultivarExtractionSpec, SolverConfig, anderson_darling_test,
    compare_models, compare_models_lrt, execute_analysis, extract_datasets_from_sheet, extract_datasets_from_sheet_with_options, fit_nonlinear,
    get_builtin_model, homoscedasticity_spearman_test, lilliefors_ks_test,
};

use common::{cfg_fast, exec_nl, fit_with, nl_fast, rule_col_title, rule_const, rule_per_ds, series_xy, xy_sheet_single};

fn build_96_curve_xy_sheet() -> (DataSheet, Vec<[f64; 4]>) {
    let log_concs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let n_curves = 96_usize;
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), n_curves, log_concs.len()).unwrap();
    xy.x_title = String::from("Log[Agonist] (M)");

    let mut truth = Vec::with_capacity(n_curves);
    for (r, &lx) in log_concs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(lx);
        for c in 0..n_curves {
            let (bottom, top, log_ec50, hill) =
                (5.0 + ((c % 4) as f64), 95.0 + 2.0 * ((c % 5) as f64), -7.2 + 0.015 * (c as f64), 1.0 + 0.08 * ((c % 4) as f64));
            if r == 0 {
                truth.push([bottom, top, log_ec50, hill]);
            }
            let mean_y = bottom + (top - bottom) / (1.0 + 10.0_f64.powf((log_ec50 - lx) * hill));
            for (rep, jit) in [-0.6, 0.0, 0.6].into_iter().enumerate() {
                xy.data_sets[c].cells[r][rep] = Cell::number(mean_y + jit);
            }
        }
    }
    (DataSheet::new(SheetId::new("dr_96"), "96-Well Dose Response", TableArchetype::Xy(xy)).unwrap(), truth)
}

fn find_tab_kind(tabs: &[ResultTab], kind: ResultTabKind) -> Option<&ResultTab> {
    tabs.iter().find(|t| t.kind == kind)
}

#[test]
fn g12_96_curve_4pl_batch_runs_under_50ms_and_recovers_all_curves() {
    let (ds, truth) = build_96_curve_xy_sheet();
    let datasets = extract_datasets_from_sheet(&ds, &[]).unwrap();
    let model = get_builtin_model("log_agonist_vs_response_4pl").unwrap();
    let cfg = SolverConfig { compute_profile_ci: false, curve_overlay_segments: 256, ..SolverConfig::default() };

    let t0 = Instant::now();
    let fit = fit_nonlinear(model.as_ref(), &datasets, &HookContext::default(), &cfg).unwrap();
    let fit_ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("G12 96-curve fit_nonlinear time: {fit_ms:.2} ms");

    assert!(fit_ms < 50.0, "expected 96-curve fit_nonlinear < 50 ms, took {fit_ms:.2} ms");
    assert_eq!(fit.status, FitStatus::Converged);
    assert_eq!((fit.datasets.len(), fit.k_free, fit.n_obs, fit.covariance.covariance_matrix.len()), (96, 384, 2304, 384));

    for (c, ds_fit) in fit.datasets.iter().enumerate() {
        assert_eq!(ds_fit.data_set_index, c);
        assert_eq!(ds_fit.curve_overlay.as_ref().map_or(0, |o| o.points.len()), 256, "curve {c} should sample 256 overlay points");
        for (p_idx, (&est, &expected)) in ds_fit.params.iter().zip(truth[c].iter()).enumerate() {
            let tol = if p_idx <= 1 { 0.5 } else { 0.05 };
            assert!((est - expected).abs() < tol, "curve {c} param {p_idx}: expected {expected:.4}, got {est:.4}");
        }
    }
    assert_eq!((fit.covariance.covariance_matrix[0][4], fit.covariance.covariance_matrix[4][0]), (0.0, 0.0));

    let mut nl_spec = nl_fast("log_agonist_vs_response_4pl");
    nl_spec.confidence_bands = false;
    let analysis = AnalysisSpec::new(SheetId::new("fit_96"), ds.id.clone(), AnalysisKind::NonlinearRegression(nl_spec)).unwrap();

    let t1 = Instant::now();
    let sheet = execute_analysis(&analysis, &ds, &HookContext::default()).unwrap();
    let exec_ms = t1.elapsed().as_secs_f64() * 1000.0;
    eprintln!("G12 96-curve execute_analysis time: {exec_ms:.2} ms");

    assert!(exec_ms < 50.0, "expected 96-curve execute_analysis < 50 ms, took {exec_ms:.2} ms");
    assert_eq!(sheet.status, ResultStatus::UpToDate);
    assert_eq!(sheet.fitted_curves.len(), 96);
    assert!(find_tab_kind(&sheet.tabs, ResultTabKind::Residuals).is_some());
}

#[test]
fn per_dataset_constant_and_column_title_constant_scopes() {
    let model = get_builtin_model("straight_line").unwrap();
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let mut ds0 = series_xy(0, "10.0", &xs, &xs.map(|x| 2.0 * x + 10.0));
    ds0.column_title_value = Some(10.0);
    let mut ds1 = series_xy(1, "25.0", &xs, &xs.map(|x| 4.0 * x + 25.0));
    ds1.column_title_value = Some(25.0);
    let datasets = [ds0, ds1];

    // Partial PerDatasetConstant: dataset 0 fixed to 10.0, dataset 1 left free.
    let fit_partial =
        fit_with(model.as_ref(), &datasets, &SolverConfig { parameter_rules: vec![rule_per_ds("YIntercept", &[(0, 10.0)])], ..cfg_fast() });
    assert_eq!(fit_partial.k_free, 3);
    assert!((fit_partial.datasets[0].params[0] - 2.0).abs() < 1e-6 && (fit_partial.datasets[0].params[1] - 10.0).abs() < 1e-12);
    assert!((fit_partial.datasets[1].params[0] - 4.0).abs() < 1e-6 && (fit_partial.datasets[1].params[1] - 25.0).abs() < 1e-6);

    // ColumnTitleConstant: each dataset binds YIntercept from its numeric column title.
    let col_title_cfg = SolverConfig { parameter_rules: vec![rule_col_title("YIntercept")], ..cfg_fast() };
    let fit_col = fit_with(model.as_ref(), &datasets, &col_title_cfg);
    assert_eq!(fit_col.k_free, 2);
    assert!((fit_col.datasets[0].params[0] - 2.0).abs() < 1e-6 && (fit_col.datasets[0].params[1] - 10.0).abs() < 1e-12);
    assert!((fit_col.datasets[1].params[0] - 4.0).abs() < 1e-6 && (fit_col.datasets[1].params[1] - 25.0).abs() < 1e-12);

    // Non-numeric column title must return an explicit HookResolutionError.
    let mut bad_datasets = datasets.to_vec();
    bad_datasets[1].column_title_value = None;
    bad_datasets[1].label = String::from("Ctrl");
    assert!(matches!(
        fit_nonlinear(model.as_ref(), &bad_datasets, &HookContext::default(), &col_title_cfg),
        Err(FitError::HookResolutionError { .. })
    ));
}

#[test]
fn range_filters_exclude_out_of_bounds_points() {
    let ds =
        xy_sheet_single("range_ds", "Range Filter Test", "A", &[-5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 20.0], &[999.0, 3.0, 5.0, 500.0, 9.0, 11.0, -50.0]);
    let mut nl = nl_fast("straight_line");
    nl.x_range_filter = Some((Some(0.0), Some(10.0)));
    nl.y_range_filter = Some((Some(0.0), Some(50.0)));
    let sheet = exec_nl(&ds, nl);
    assert_eq!(sheet.status, ResultStatus::UpToDate);
    assert_eq!(sheet.fitted_curves.len(), 1);
    assert!((sheet.lookup_scalar("Slope", Some(0)).unwrap() - 2.0).abs() < 1e-6);
    assert!((sheet.lookup_scalar("YIntercept", Some(0)).unwrap() - 1.0).abs() < 1e-6);
}

#[test]
fn plot_function_only_and_log_x_grid_sampling() {
    let empty_ds =
        DataSheet::new(SheetId::new("empty_xy"), "Empty XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).unwrap())).unwrap();
    let mut nl_plot = nl_fast("agonist_vs_response_4pl");
    nl_plot.plot_function_only = true;
    nl_plot.plot_x_min = Some(1.0);
    nl_plot.plot_x_max = Some(1000.0);
    nl_plot.parameter_rules = vec![rule_const("Bottom", 10.0), rule_const("Top", 110.0), rule_const("EC50", 30.0), rule_const("HillSlope", 1.2)];
    let res = exec_nl(&empty_ds, nl_plot);
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!(res.fitted_curves.len(), 1);
    let pts = &res.fitted_curves[0].points;
    assert_eq!(pts.len(), 256);
    assert!((pts[0].x - 1.0).abs() < 1e-12 && (pts[255].x - 1000.0).abs() < 1e-9);
    let expected_ratio = 1000.0_f64.powf(1.0 / 255.0);
    for w in pts.windows(2) {
        assert!(((w[1].x / w[0].x) - expected_ratio).abs() < 1e-9, "expected constant geometric ratio {expected_ratio}");
    }

    // Contrast: narrow positive span (< 100x) uses linear spacing.
    let narrow_ds = series_xy(0, "Narrow", &[1.0, 10.0, 25.0, 50.0], &[3.0, 21.0, 51.0, 101.0]);
    let narrow_fit =
        fit_with(get_builtin_model("straight_line").unwrap().as_ref(), &[narrow_ds], &SolverConfig { curve_overlay_segments: 100, ..cfg_fast() });
    let overlay = narrow_fit.datasets[0].curve_overlay.as_ref().unwrap();
    let step0 = overlay.points[1].x - overlay.points[0].x;
    for w in overlay.points.windows(2) {
        assert!(((w[1].x - w[0].x) - step0).abs() < 1e-9, "expected linear step {step0}");
    }
}

#[test]
fn residual_homoscedasticity_ad_ks_and_lrt_comparison_parity() {
    let y_hat: Vec<f64> = (1..=20).map(|i| i as f64).collect();
    let homo_res = [0.2, -0.3, 0.1, -0.25, 0.18, -0.15, 0.22, -0.2, 0.12, -0.1, 0.25, -0.22, 0.14, -0.16, 0.19, -0.18, 0.11, -0.12, 0.21, -0.24];
    let hetero_res: Vec<f64> = y_hat.iter().enumerate().map(|(i, &y)| if i % 2 == 0 { y } else { -y }).collect();
    assert!(homoscedasticity_spearman_test(&y_hat, &homo_res, 0.05).unwrap().homoscedastic);
    let hetero = homoscedasticity_spearman_test(&y_hat, &hetero_res, 0.05).unwrap();
    assert!(!hetero.homoscedastic && hetero.spearman_rho > 0.9 && hetero.p_value < 1e-4);

    let normal_sample = [-1.64, -1.28, -0.98, -0.76, -0.56, -0.38, -0.21, -0.05, 0.10, 0.25, 0.41, 0.58, 0.77, 1.00, 1.32, 1.72];
    let skewed_sample = [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.08, 0.10, 0.15, 0.25, 0.60, 2.50, 8.00, 24.0];
    assert!(anderson_darling_test(&normal_sample).unwrap().passed);
    let ad_bad = anderson_darling_test(&skewed_sample).unwrap();
    assert!(!ad_bad.passed && ad_bad.p_value < 0.01);
    assert!(lilliefors_ks_test(&normal_sample).unwrap().passed);
    let ks_bad = lilliefors_ks_test(&skewed_sample).unwrap();
    assert!(!ks_bad.passed && ks_bad.p_value < 0.01);

    let lrt_direct = compare_models_lrt(20.0, 17, 2.0, 16, 20, 0.05).unwrap();
    assert!((lrt_direct.lr_statistic - 20.0 * (10.0_f64).ln()).abs() < 1e-12);
    assert_eq!(lrt_direct.df_diff, 1);
    assert!(lrt_direct.p_value < 1e-6 && lrt_direct.preferred_complex_model);

    // Build 4PL asymmetric-slope data so 4PL decisively beats 3PL via LRT in execute_analysis.
    let log_concs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5, -4.0, -3.5];
    let ys: Vec<f64> = log_concs
        .iter()
        .enumerate()
        .map(|(r, &lx)| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - lx) * 2.5)) + if r % 2 == 0 { 0.05 } else { -0.05 })
        .collect();
    let ds = xy_sheet_single("lrt_ds", "LRT Dose", "A", &log_concs, &ys);

    let mut nl = nl_fast("log_agonist_vs_response_3pl");
    nl.normality_of_residuals = true;
    nl.homoscedasticity_test = true;
    nl.comparison = Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_4pl") });
    let sheet = exec_nl(&ds, nl);
    let comp_tab = find_tab_kind(&sheet.tabs, ResultTabKind::ModelComparison).unwrap();
    assert!(comp_tab.title.contains("Likelihood Ratio") && comp_tab.row_labels.iter().any(|l| l.starts_with("LR statistic")));
    for kw in ["Homoscedasticity", "Anderson", "Lilliefors"] {
        assert!(sheet.checklist.iter().any(|c| c.assumption.contains(kw)), "missing checklist {kw}");
    }

    // Direct solver::compare_models dispatch for LikelihoodRatioTest
    let (m_null, m_alt) = (get_builtin_model("log_agonist_vs_response_3pl").unwrap(), get_builtin_model("log_agonist_vs_response_4pl").unwrap());
    let extracted = extract_datasets_from_sheet(&ds, &[]).unwrap();
    let cmp_cfg = cfg_fast();
    let fit_null = fit_with(m_null.as_ref(), &extracted, &cmp_cfg);
    let (alt_fit, cmp_out) = compare_models(
        &fit_null,
        m_alt.as_ref(),
        &extracted,
        &HookContext::default(),
        &cmp_cfg,
        &ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_4pl") },
    )
    .unwrap();
    assert_eq!(alt_fit.equation_id, "log_agonist_vs_response_4pl");
    let ModelComparisonOutcome::LikelihoodRatioTest(r) = cmp_out else { panic!("expected LikelihoodRatioTest, got {cmp_out:?}") };
    assert!(r.df_diff == 1 && r.preferred_complex_model && r.p_value < 0.01);
}

#[test]
fn open_ci_bounds_on_zero_df_and_ambiguous_covariance() {
    let wald_cfg = cfg_fast();
    let fit_zero_df = fit_with(get_builtin_model("straight_line").unwrap().as_ref(), &[series_xy(0, "ZeroDF", &[1.0, 2.0], &[3.0, 5.0])], &wald_cfg);
    assert_eq!(fit_zero_df.df, 0);
    assert!(fit_zero_df.parameters.iter().all(|p| p.ci_lower == CiBound::Open && p.ci_upper == CiBound::Open));

    let ys_col: Vec<f64> = (0..6).map(|r| 50.0 + ((r % 2) as f64) * 0.1).collect();
    let fit_ambig =
        fit_with(get_builtin_model("log_agonist_vs_response_4pl").unwrap().as_ref(), &[series_xy(0, "IdenticalX", &[-6.0; 6], &ys_col)], &wald_cfg);
    assert!(fit_ambig.covariance.ambiguous, "expected ambiguous=true on identical X");
    assert!(fit_ambig.parameters.iter().all(|p| p.ci_lower == CiBound::Open && p.ci_upper == CiBound::Open));
}

#[test]
fn multivar_cartesian_subgroups_x_columns_and_y_column() {
    let mut mv = MultipleVariablesTable::new(4, 10).unwrap();
    mv.variables[0].title = String::from("Drug");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Sex");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[1].level_order.order = LevelOrderKind::Explicit(vec![String::from("M"), String::from("F")]);
    mv.variables[2].title = String::from("Dose");
    mv.variables[3].title = String::from("Response");

    let records = [
        ("Treated", "F", 1.0, 14.0, false),
        ("Treated", "F", 2.0, 24.0, false),
        ("Ctrl", "F", 1.0, 5.0, false),
        ("Ctrl", "F", 2.0, 9.0, false),
        ("Treated", "M", 1.0, 12.0, false),
        ("Treated", "M", 2.0, 22.0, false),
        ("Ctrl", "M", 1.0, 3.0, false),
        ("Ctrl", "M", 2.0, 5.0, false),
        ("Ctrl", "M", 99.0, 999.0, true),
        ("Treated", "M", 3.0, f64::NAN, false),
    ];
    for (r, &(drug, sex, dose, resp, excl)) in records.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(drug);
        mv.variables[1].cells[r] = Cell::text(sex);
        mv.variables[2].cells[r] = Cell::number(dose);
        mv.variables[3].cells[r] = if resp.is_finite() { Cell { excluded: excl, ..Cell::number(resp) } } else { Cell::empty() };
    }
    let ds = DataSheet::new(SheetId::new("mv_ds"), "MV Cartesian", TableArchetype::MultipleVariables(mv)).unwrap();

    let mv_spec = MultivarExtractionSpec { group_columns: &[0, 1], x_columns: &[2], y_column: Some(3) };
    let series = extract_datasets_from_sheet_with_options(&ds, &[], &mv_spec).unwrap();
    let labels: Vec<&str> = series.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["Treated | M", "Treated | F", "Ctrl | M", "Ctrl | F"]);
    assert_eq!(series[2].points.len(), 2);

    let subset = extract_datasets_from_sheet_with_options(&ds, &[2, 3], &mv_spec).unwrap();
    assert_eq!(subset.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Ctrl | M", "Ctrl | F"]);

    let mut nl = nl_fast("straight_line");
    nl.mv_group_columns = vec![0, 1];
    nl.mv_x_columns = vec![2];
    nl.mv_y_column = Some(3);
    let res = exec_nl(&ds, nl);
    assert_eq!(res.fitted_curves.len(), 4);
    assert!((res.lookup_scalar("Slope", Some(0)).unwrap() - 10.0).abs() < 1e-6);
    assert!((res.lookup_scalar("Slope", Some(2)).unwrap() - 2.0).abs() < 1e-6);
}

#[test]
fn residuals_and_covariance_matrix_tabs_emitted_with_qq_and_studentized_columns() {
    let log_concs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let noise = [0.4, -0.5, 0.3, -0.2, 0.5, -0.4, 0.25, -0.35];
    let ys: Vec<f64> = log_concs.iter().zip(noise).map(|(&lx, e)| 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - lx) * 1.1)) + e).collect();
    let ds = xy_sheet_single("tabs_ds", "Tabs DS", "A", &log_concs, &ys);

    let mut nl_on = nl_fast("log_agonist_vs_response_4pl");
    nl_on.emit_residuals_tab = true;
    nl_on.emit_covariance_tab = true;
    let res_on = exec_nl(&ds, nl_on);

    let res_tab = find_tab_kind(&res_on.tabs, ResultTabKind::Residuals).unwrap();
    assert_eq!(res_tab.column_headers, vec!["Predicted Y", "Residual", "Weighted Residual", "Studentized Residual", "Q-Q Normal Quantile"]);
    assert_eq!(res_tab.rows.len(), log_concs.len());
    let mut qq_vals = Vec::with_capacity(res_tab.rows.len());
    for row in &res_tab.rows {
        assert_eq!(row.len(), 5);
        assert!(row.iter().all(|cell| matches!(cell, ResultCell::Number { value, .. } if value.is_finite())));
        if let ResultCell::Number { value, .. } = row[4] {
            qq_vals.push(value);
        }
    }
    qq_vals.sort_by(f64::total_cmp);
    assert!((qq_vals[0] + qq_vals[qq_vals.len() - 1]).abs() < 1e-9, "Blom Q-Q quantiles should be symmetric");

    let cov_tab = find_tab_kind(&res_on.tabs, ResultTabKind::CovarianceMatrix).unwrap();
    assert_eq!((cov_tab.column_headers.len(), cov_tab.rows.len()), (4, 4));
    for r in 0..4 {
        assert_eq!(cov_tab.rows[r].len(), 4);
        assert!(matches!(cov_tab.rows[r][r], ResultCell::Number { value, .. } if value > 0.0 && value.is_finite()));
    }

    let mut nl_off = nl_fast("log_agonist_vs_response_4pl");
    nl_off.emit_residuals_tab = false;
    nl_off.emit_covariance_tab = false;
    let res_off = exec_nl(&ds, nl_off);
    assert!(find_tab_kind(&res_off.tabs, ResultTabKind::Residuals).is_none());
    assert!(find_tab_kind(&res_off.tabs, ResultTabKind::CovarianceMatrix).is_none());
}

#[test]
fn r1_s03_09_dsl_compilation_scales_linearly_wall_clock() {
    let best_compile_ns = |src: &str| -> f64 {
        CompiledDslModel::compile("scale", src).unwrap();
        (0..5)
            .map(|_| {
                let t0 = Instant::now();
                let _ = CompiledDslModel::compile("scale", src).unwrap();
                t0.elapsed().as_nanos() as f64
            })
            .fold(f64::INFINITY, f64::min)
    };

    let build_transforms = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[Transforms]\nT0 = A + B\n");
        for i in 1..n {
            s.push_str(&format!("T{i} = T{} + 1\n", i - 1));
        }
        s
    };
    let (t1000, t4000) = (best_compile_ns(&build_transforms(1_000)), best_compile_ns(&build_transforms(4_000)));
    assert!(t4000 / t1000.max(1_000_000.0) < 8.0, "transforms 4x scaling too slow: t1000={t1000}ns, t4000={t4000}ns");

    let build_inits = |n: usize| {
        let mut s = String::from("[Equation]\nY = A*X + B\n[InitialValues]\nL0 = YMIN\n");
        for i in 1..n {
            s.push_str(&format!("L{i} = L{} + 0.1\n", i - 1));
        }
        s.push_str(&format!("A = L{}\nB = YMAX\n", n - 1));
        s
    };
    let (init_t1000, init_t4000) = (best_compile_ns(&build_inits(1_000)), best_compile_ns(&build_inits(4_000)));
    assert!(init_t4000 / init_t1000.max(1_000_000.0) < 8.0, "init rules 4x scaling too slow: t1000={init_t1000}ns, t4000={init_t4000}ns");
}
