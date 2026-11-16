#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Stage 3A S1 (`R1-S03-01`, `R1-S03-02`, `R1-S03-03`)
//! and X2 (`R1-X2-02`) curve-fitting and summarized replicate fixes.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, DataSheet, FitMethod, LinearRegressionSpec, NonlinearFitSpec, ParameterRule, ParameterScope,
    ResultCell, SubcolumnMode, TableArchetype, WeightingScheme, XColumnMode, XyTable,
};
use curvebook_fit::diagnostics::CovarianceDiagnostics;
use curvebook_fit::{
    CompiledDslModel, DataPoint, DatasetSeries, HookContext, ParameterNote, SimpleLinearOptions, SolverConfig, compare_linear_lines,
    compute_deviance_profile_likelihood_cis, execute_analysis, fit_nonlinear, simple_linear_regression,
};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::metrics::assert_approx_eq;

fn ds(idx: usize, xs: &[f64], ys: &[f64]) -> DatasetSeries {
    DatasetSeries {
        data_set_index: idx,
        label: format!("DS{idx}"),
        points: xs
            .iter()
            .zip(ys)
            .enumerate()
            .map(|(row, (&x, &y))| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: idx, row })
            .collect(),
        unknowns_y: Vec::new(),
        unknowns_x: Vec::new(),
        is_pre_summarized: false,
        column_title_value: None,
    }
}

fn poisson_decay_data() -> Vec<DatasetSeries> {
    let xs: Vec<f64> = (0..10).map(f64::from).collect();
    let jitter = [7.0, -9.0, 4.0, 11.0, -6.0, 3.0, -5.0, 2.0, -1.0, 1.0];
    let ys: Vec<f64> = xs.iter().zip(jitter).map(|(&x, j)| (200.0 * (-0.3 * x).exp() + j).round()).collect();
    vec![ds(0, &xs, &ys)]
}

fn profile_wald_ratios(method: FitMethod) -> Vec<(f64, f64, String, bool)> {
    let model = CompiledDslModel::compile("decay", "Y = A*exp(-K*X)").unwrap();
    let cfg = SolverConfig { method, ..SolverConfig::default() };
    let fit = fit_nonlinear(&model, &poisson_decay_data(), &HookContext::default(), &cfg).unwrap();
    fit.parameters
        .iter()
        .map(|p| {
            let sym_half = 0.5 * (p.symmetric_ci_upper - p.symmetric_ci_lower);
            let (lo, hi) = match (p.ci_lower, p.ci_upper) {
                (CiBound::Finite(a), CiBound::Finite(b)) => (a, b),
                _ => (f64::NAN, f64::NAN),
            };
            ((p.estimate - lo) / sym_half, (hi - p.estimate) / sym_half, format!("{:?}", p.notes), lo.is_finite())
        })
        .collect()
}

#[test]
fn r1_s03_01_poisson_profile_ci_matches_wald_scale() {
    for (lo, hi, _, _) in profile_wald_ratios(FitMethod::LeastSquares) {
        assert!((0.8..1.25).contains(&lo) && (0.8..1.25).contains(&hi), "LS control ratio {lo} {hi}");
    }
    for (lo, hi, _, _) in profile_wald_ratios(FitMethod::PoissonDeviance) {
        assert!((0.8..1.25).contains(&lo) && (0.8..1.25).contains(&hi), "Poisson profile/Wald ratios {lo:.4} {hi:.4} out of 0.8..1.25");
    }
}

#[test]
fn r1_s03_01_poisson_zero_deviance_profile_ci_is_nonzero() {
    let xs: Vec<f64> = (0..8).map(f64::from).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| 200.0 * (-0.3 * x).exp()).collect();
    let model = CompiledDslModel::compile("decay_exact", "Y = A*exp(-K*X)").unwrap();
    let cfg = SolverConfig { method: FitMethod::PoissonDeviance, ..SolverConfig::default() };
    let fit = fit_nonlinear(&model, &[ds(0, &xs, &ys)], &HookContext::default(), &cfg).unwrap();
    assert!(fit.weighted_ss < 1e-12);
    for p in &fit.parameters {
        let (CiBound::Finite(lo), CiBound::Finite(hi)) = (p.ci_lower, p.ci_upper) else {
            panic!("expected finite Poisson profile CIs at exact zero deviance for {}", p.name);
        };
        assert!(lo < p.estimate && p.estimate < hi, "expected non-degenerate CI around {} for {}", p.estimate, p.name);
        let sym_half = 0.5 * (p.symmetric_ci_upper - p.symmetric_ci_lower);
        let r_lo = (p.estimate - lo) / sym_half;
        let r_hi = (hi - p.estimate) / sym_half;
        assert!((0.8..1.25).contains(&r_lo) && (0.8..1.25).contains(&r_hi), "exact Poisson profile/Wald ratios {r_lo:.4} {r_hi:.4}");
    }
}

#[test]
fn r1_s03_01_robust_cauchy_omits_profile_cis_and_formats_not_applicable() {
    let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let ys = [1.0, 3.1, 4.9, 25.0, 9.1, 10.9, 13.0, 15.1];
    let model = CompiledDslModel::compile("line_cauchy", "Y = A + B*X").unwrap();
    let cfg = SolverConfig { method: FitMethod::RobustCauchy, ..SolverConfig::default() };
    let fit = fit_nonlinear(&model, &[ds(0, &xs, &ys)], &HookContext::default(), &cfg).unwrap();
    for p in &fit.parameters {
        assert_eq!(p.ci_lower, CiBound::Open, "RobustCauchy must leave ci_lower Open for {}", p.name);
        assert_eq!(p.ci_upper, CiBound::Open, "RobustCauchy must leave ci_upper Open for {}", p.name);
        assert!(!p.notes.contains(&ParameterNote::UnboundedCi), "RobustCauchy must omit UnboundedCi note");
    }

    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("d-cauchy", "Cauchy", TableArchetype::Xy(xy)).unwrap();
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.method = FitMethod::RobustCauchy;
    let spec = AnalysisSpec::new("res-cauchy", "d-cauchy", AnalysisKind::NonlinearRegression(nl)).unwrap();
    let res = execute_analysis(&spec, &sheet, &HookContext::default()).unwrap();
    let param_tab = res.tabs.first().expect("parameter summary tab");
    assert_eq!(param_tab.rows[0][2], ResultCell::Text("Not applicable".to_owned()));
}

#[test]
fn r1_s03_02_brent_failure_returns_open_not_midpoint() {
    for (lo_ratio, hi_ratio, notes, finite) in profile_wald_ratios(FitMethod::PoissonDeviance) {
        let is_exact_midpoint = (lo_ratio - 0.5).abs() < 1e-6 && (hi_ratio - 0.5).abs() < 1e-6;
        assert!(
            !(is_exact_midpoint && finite && (notes == "[]" || notes.is_empty())),
            "bound is exact bracket midpoint reported finite with notes {notes}"
        );
    }

    // Directly exercise Brent root failure inside compute_deviance_profile_likelihood_cis:
    // evaluation succeeds at bracket endpoints but returns None in the interior so Brent fails.
    let cov = CovarianceDiagnostics {
        standard_errors: vec![1.0],
        symmetric_ci_lower: vec![-1.96],
        symmetric_ci_upper: vec![1.96],
        covariance_matrix: vec![vec![1.0]],
        correlation_matrix: vec![vec![1.0]],
        dependencies: vec![0.0],
        numerical_rank: 1,
        ambiguous: false,
    };
    let bounds = compute_deviance_profile_likelihood_cis(&[0.0], 1.0, &cov, &[(None, None)], ConfidenceLevel::DEFAULT_95, |_k, val, _seed| {
        let dist = val.abs();
        // Bracket at base_step ~ 1.96 succeeds with deviance above threshold, while interior evals fail.
        if (dist - 1.959_963_984_540_054).abs() > 0.5 { Some((1.0 + dist * dist, vec![val])) } else { None }
    });
    assert_eq!(bounds, vec![(CiBound::Open, CiBound::Open)]);
}

#[test]
fn r1_s03_03_rout_independent_curves_use_per_curve_scale_and_shared_pools() {
    let model = CompiledDslModel::compile("line", "Y = A + B*X").unwrap();
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let small = [0.01, -0.012, 0.008, -0.005, 0.011, -0.009, 0.004, -0.007, 0.006, -0.01, 0.003, -0.002];
    let mut quiet: Vec<f64> = xs.iter().zip(small).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    quiet[5] += 1.0;
    let big = [5.0, -6.0, 4.0, -3.0, 7.0, -5.0, 2.0, -4.0, 6.0, -7.0, 3.0, -2.0];
    let noisy: Vec<f64> = xs.iter().zip(big).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    let cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };

    let alone = fit_nonlinear(&model, &[ds(0, &xs, &quiet)], &HookContext::default(), &cfg).unwrap();
    assert_eq!(alone.n_outliers_excluded, 1);

    let both = fit_nonlinear(&model, &[ds(0, &xs, &quiet), ds(1, &xs, &noisy)], &HookContext::default(), &cfg).unwrap();
    assert!(both.n_outliers_excluded >= 1, "expected quiet curve's outlier to be excluded (got {})", both.n_outliers_excluded);
    let rout = both.rout.as_ref().expect("rout summary");
    assert!(rout.audit.iter().any(|a| a.data_set_index == 0 && a.row == 5 && a.is_outlier));
    assert_eq!(both.datasets[0].params.len(), 2);
    assert_approx_eq(both.datasets[0].params[0], 1.0, 0.05, 0.05, "quiet intercept after outlier removal");
    assert_approx_eq(both.datasets[0].params[1], 2.0, 0.05, 0.05, "quiet slope after outlier removal");

    // Global shared-parameter fit still runs pooled ROUT without splitting curves.
    let shared_cfg = SolverConfig {
        method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        parameter_rules: vec![ParameterRule {
            name: "B".to_owned(),
            scope: ParameterScope::SharedGlobal,
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }],
        ..SolverConfig::default()
    };
    let shared_fit = fit_nonlinear(&model, &[ds(0, &xs, &quiet), ds(1, &xs, &noisy)], &HookContext::default(), &shared_cfg).unwrap();
    assert_eq!(shared_fit.k_free, 3);
}

fn replicate_groups() -> Vec<Vec<f64>> {
    vec![vec![1.0, 2.0, 3.0], vec![3.0, 4.0, 6.0], vec![4.0, 5.0, 7.0], vec![6.0, 8.0, 9.0], vec![7.0, 9.0, 10.0, 11.0, 12.0, 14.0]]
}

fn mean_sd_n(v: &[f64]) -> (f64, f64, f64) {
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    (m, sd, n)
}

fn build_rep_and_summary_tables() -> (DataSheet, DataSheet, DataSheet, DataSheet) {
    let reps = replicate_groups();
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let mut raw =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(6), 1, 5).unwrap();
    let mut sum_sd = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 1, 5).unwrap();
    let mut sum_sem =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSemN, 1, 5).unwrap();
    let mut sum_cv = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanCvN, 1, 5).unwrap();

    for (r, v) in reps.iter().enumerate() {
        raw.set_x_cell(r, Cell::number(xs[r])).unwrap();
        sum_sd.set_x_cell(r, Cell::number(xs[r])).unwrap();
        sum_sem.set_x_cell(r, Cell::number(xs[r])).unwrap();
        sum_cv.set_x_cell(r, Cell::number(xs[r])).unwrap();
        for (k, &y) in v.iter().enumerate() {
            raw.data_sets[0].cells[r][k] = Cell::number(y);
        }
        let (m, sd, n) = mean_sd_n(v);
        sum_sd.data_sets[0].cells[r][0] = Cell::number(m);
        sum_sd.data_sets[0].cells[r][1] = Cell::number(sd);
        sum_sd.data_sets[0].cells[r][2] = Cell::number(n);

        sum_sem.data_sets[0].cells[r][0] = Cell::number(m);
        sum_sem.data_sets[0].cells[r][1] = Cell::number(sd / n.sqrt());
        sum_sem.data_sets[0].cells[r][2] = Cell::number(n);

        sum_cv.data_sets[0].cells[r][0] = Cell::number(m);
        sum_cv.data_sets[0].cells[r][1] = Cell::number(100.0 * sd / m.abs());
        sum_cv.data_sets[0].cells[r][2] = Cell::number(n);
    }
    (
        DataSheet::new("raw", "raw", TableArchetype::Xy(raw)).unwrap(),
        DataSheet::new("sum_sd", "sum_sd", TableArchetype::Xy(sum_sd)).unwrap(),
        DataSheet::new("sum_sem", "sum_sem", TableArchetype::Xy(sum_sem)).unwrap(),
        DataSheet::new("sum_cv", "sum_cv", TableArchetype::Xy(sum_cv)).unwrap(),
    )
}

#[test]
fn r1_x2_02_mean_sd_sem_cv_n_match_raw_replicates_linear_and_nonlinear() {
    let (raw, sum_sd, sum_sem, sum_cv) = build_rep_and_summary_tables();
    let hooks = HookContext::default();

    // Linear regression end-to-end across RawReplicates vs MeanSdN, MeanSemN, MeanCvN.
    let raw_lin =
        execute_analysis(&AnalysisSpec::new("r-raw", "raw", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap(), &raw, &hooks)
            .unwrap();
    for (name, ds_sheet) in [("MeanSdN", &sum_sd), ("MeanSemN", &sum_sem), ("MeanCvN", &sum_cv)] {
        let spec = AnalysisSpec::new("r-sum", ds_sheet.id.as_str(), AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
        let sum_lin = execute_analysis(&spec, ds_sheet, &hooks).unwrap();
        for scalar_key in ["Slope", "YIntercept", "R2"] {
            let expected = raw_lin.lookup_scalar(scalar_key, Some(0)).unwrap();
            let actual = sum_lin.lookup_scalar(scalar_key, Some(0)).unwrap();
            assert_approx_eq(actual, expected, 1e-10, 1e-10, &format!("linear {name} {scalar_key}"));
        }
        assert_eq!(sum_lin.tabs[0].rows.len(), raw_lin.tabs[0].rows.len());
        // Rows 0..=8 cover Slope, Y-intercept, SE of Slope, SE of Intercept, 95% CI Slope, R2, Sy.x, F, P value
        // (row 9 is Runs test P value, which inspects per-replicate residuals vs per-mean residuals).
        for (row_idx, (got_row, exp_row)) in sum_lin.tabs[0].rows.iter().zip(raw_lin.tabs[0].rows.iter()).take(9).enumerate() {
            for (col_idx, (got, exp)) in got_row.iter().zip(exp_row.iter()).enumerate() {
                match (got, exp) {
                    (ResultCell::Number { value: a, .. }, ResultCell::Number { value: b, .. }) => {
                        assert_approx_eq(*a, *b, 1e-10, 1e-10, &format!("linear {name} tab0[{row_idx}][{col_idx}]"));
                    }
                    (
                        ResultCell::ConfidenceInterval { lower: CiBound::Finite(a_lo), upper: CiBound::Finite(a_hi) },
                        ResultCell::ConfidenceInterval { lower: CiBound::Finite(b_lo), upper: CiBound::Finite(b_hi) },
                    ) => {
                        assert_approx_eq(*a_lo, *b_lo, 1e-10, 1e-10, &format!("linear {name} CI lower[{row_idx}]"));
                        assert_approx_eq(*a_hi, *b_hi, 1e-10, 1e-10, &format!("linear {name} CI upper[{row_idx}]"));
                    }
                    _ => assert_eq!(got, exp, "linear {name} cell[{row_idx}][{col_idx}]"),
                }
            }
        }
    }

    // Nonlinear unweighted LS regression end-to-end across RawReplicates vs MeanSdN, MeanSemN, MeanCvN.
    let mut nl_spec = NonlinearFitSpec::new("straight_line");
    nl_spec.weighting = WeightingScheme::None;
    let raw_nl =
        execute_analysis(&AnalysisSpec::new("nl-raw", "raw", AnalysisKind::NonlinearRegression(nl_spec.clone())).unwrap(), &raw, &hooks).unwrap();
    for (name, ds_sheet) in [("MeanSdN", &sum_sd), ("MeanSemN", &sum_sem), ("MeanCvN", &sum_cv)] {
        let spec = AnalysisSpec::new("nl-sum", ds_sheet.id.as_str(), AnalysisKind::NonlinearRegression(nl_spec.clone())).unwrap();
        let sum_nl = execute_analysis(&spec, ds_sheet, &hooks).unwrap();
        for (scalar_key, ds_idx) in [("YIntercept", Some(0)), ("Slope", Some(0)), ("R2", None), ("RSS", None)] {
            let expected = raw_nl.lookup_scalar(scalar_key, ds_idx).unwrap();
            let actual = sum_nl.lookup_scalar(scalar_key, ds_idx).unwrap();
            assert_approx_eq(actual, expected, 1e-7, 1e-7, &format!("nonlinear {name} {scalar_key}"));
        }
        assert!(sum_nl.tabs.len() >= 2 && raw_nl.tabs.len() >= 2);
        // tab 0: parameter estimates, SEs, CIs; tab 1 rows 0..=6: DF, R2, RSS, Sy.x, AICc, BIC, N.
        for (tab_idx, max_rows) in [(0usize, usize::MAX), (1usize, 7usize)] {
            assert_eq!(sum_nl.tabs[tab_idx].rows.len(), raw_nl.tabs[tab_idx].rows.len());
            for (row_idx, (got_row, exp_row)) in sum_nl.tabs[tab_idx].rows.iter().zip(raw_nl.tabs[tab_idx].rows.iter()).take(max_rows).enumerate() {
                for (col_idx, (got, exp)) in got_row.iter().zip(exp_row.iter()).enumerate() {
                    match (got, exp) {
                        (ResultCell::Number { value: a, .. }, ResultCell::Number { value: b, .. }) => {
                            assert_approx_eq(*a, *b, 1e-6, 1e-6, &format!("nonlinear {name} tab{tab_idx}[{row_idx}][{col_idx}]"));
                        }
                        (
                            ResultCell::ConfidenceInterval { lower: CiBound::Finite(a_lo), upper: CiBound::Finite(a_hi) },
                            ResultCell::ConfidenceInterval { lower: CiBound::Finite(b_lo), upper: CiBound::Finite(b_hi) },
                        ) => {
                            assert_approx_eq(*a_lo, *b_lo, 1e-5, 1e-5, &format!("nonlinear {name} tab{tab_idx} CI lower[{row_idx}]"));
                            assert_approx_eq(*a_hi, *b_hi, 1e-5, 1e-5, &format!("nonlinear {name} tab{tab_idx} CI upper[{row_idx}]"));
                        }
                        _ => assert_eq!(got, exp, "nonlinear {name} tab{tab_idx} cell[{row_idx}][{col_idx}]"),
                    }
                }
            }
        }
    }

    // Direct linear ANCOVA comparison (RawReplicates vs MeanSdN across 2 datasets) matches identically.
    let reps = replicate_groups();
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let raw_pts_a: Vec<DataPoint> = xs
        .iter()
        .zip(reps.iter())
        .enumerate()
        .flat_map(|(r, (&x, v))| {
            v.iter().map(move |&y| DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: 0, row: r })
        })
        .collect();
    let raw_pts_b: Vec<DataPoint> = raw_pts_a.iter().map(|p| DataPoint { y: p.y + 3.0, data_set_index: 1, ..p.clone() }).collect();
    let sum_pts_a: Vec<DataPoint> = xs
        .iter()
        .zip(reps.iter())
        .enumerate()
        .map(|(r, (&x, v))| {
            let (m, sd, n) = mean_sd_n(v);
            DataPoint { x: vec![x], y: m, sd: Some(sd), n: Some(n), explicit_weight: None, data_set_index: 0, row: r }
        })
        .collect();
    let sum_pts_b: Vec<DataPoint> = sum_pts_a.iter().map(|p| DataPoint { y: p.y + 3.0, data_set_index: 1, ..p.clone() }).collect();

    let lin_raw = simple_linear_regression(&raw_pts_a, &SimpleLinearOptions::default()).unwrap();
    let lin_sum = simple_linear_regression(&sum_pts_a, &SimpleLinearOptions::default()).unwrap();
    assert_eq!(lin_sum.n, lin_raw.n);
    assert_eq!(lin_sum.df_residual, lin_raw.df_residual);
    assert_approx_eq(lin_sum.sy_x, lin_raw.sy_x, 1e-10, 1e-10, "sy_x");
    assert_approx_eq(lin_sum.r_squared, lin_raw.r_squared, 1e-10, 1e-10, "r_squared");

    let cmp_raw = compare_linear_lines(&[&raw_pts_a, &raw_pts_b], false).unwrap();
    let cmp_sum = compare_linear_lines(&[&sum_pts_a, &sum_pts_b], false).unwrap();
    assert_eq!(cmp_sum.df_slopes, cmp_raw.df_slopes);
    assert_approx_eq(cmp_sum.f_slopes, cmp_raw.f_slopes, 1e-10, 1e-10, "ANCOVA f_slopes");
    assert_approx_eq(cmp_sum.f_intercepts.unwrap(), cmp_raw.f_intercepts.unwrap(), 1e-10, 1e-10, "ANCOVA f_intercepts");

    // Hostile non-integer / invalid n rejects safely rather than panicking or silently wrapping.
    let bad_n = vec![
        DataPoint { x: vec![1.0], y: 2.0, sd: Some(0.5), n: Some(1.5), explicit_weight: None, data_set_index: 0, row: 0 },
        DataPoint { x: vec![2.0], y: 4.0, sd: Some(0.5), n: Some(3.0), explicit_weight: None, data_set_index: 0, row: 1 },
    ];
    assert!(simple_linear_regression(&bad_n, &SimpleLinearOptions::default()).is_err());
    let bad_sd = vec![
        DataPoint { x: vec![1.0], y: 2.0, sd: Some(-0.5), n: Some(3.0), explicit_weight: None, data_set_index: 0, row: 0 },
        DataPoint { x: vec![2.0], y: 4.0, sd: Some(0.5), n: Some(3.0), explicit_weight: None, data_set_index: 0, row: 1 },
    ];
    assert!(simple_linear_regression(&bad_sd, &SimpleLinearOptions::default()).is_err());
}
