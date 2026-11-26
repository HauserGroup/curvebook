#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Stage 3A S1 (`R1-S03-01`, `R1-S03-02`, `R1-S03-03`)
//! and X2 (`R1-X2-02`) curve-fitting and summarized replicate fixes.

use curvebook_doc::{
    Cell, CiBound, DataSheet, FitMethod, LinearRegressionSpec, NonlinearFitSpec, ResultCell, ResultTab, SubcolumnMode, TableArchetype,
    WeightingScheme, XColumnMode, XyTable,
};
use curvebook_fit::{
    DataPoint, DatasetSeries, SimpleLinearOptions, SolverConfig, compare_linear_lines, compute_deviance_profile_likelihood_cis,
    simple_linear_regression,
};
use curvebook_math::ConfidenceLevel;
use curvebook_testkit::metrics::assert_approx_eq;

use crate::common::{dsl_compile, exec_lin, exec_nl, fit_with, mean_sd_n, rule_shared, series_xy, unit_cov_1d, unwrap_finite_ci};

fn poisson_decay_data() -> Vec<DatasetSeries> {
    let xs: Vec<f64> = (0..10).map(f64::from).collect();
    let jitter = [7.0, -9.0, 4.0, 11.0, -6.0, 3.0, -5.0, 2.0, -1.0, 1.0];
    let ys: Vec<f64> = xs.iter().zip(jitter).map(|(&x, j)| (200.0 * (-0.3 * x).exp() + j).round()).collect();
    vec![series_xy(0, "DS0", &xs, &ys)]
}

fn profile_wald_ratios(method: FitMethod) -> Vec<(f64, f64, String, bool)> {
    let fit = fit_with(&dsl_compile("Y = A*exp(-K*X)"), &poisson_decay_data(), &SolverConfig { method, ..SolverConfig::default() });
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
    for method in [FitMethod::LeastSquares, FitMethod::PoissonDeviance] {
        for (lo, hi, _, _) in profile_wald_ratios(method) {
            assert!((0.8..1.25).contains(&lo) && (0.8..1.25).contains(&hi), "{method:?} ratios {lo:.4} {hi:.4}");
        }
    }
}

#[test]
fn r1_s03_01_poisson_zero_deviance_profile_ci_is_nonzero() {
    let xs: Vec<f64> = (0..8).map(f64::from).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| 200.0 * (-0.3 * x).exp()).collect();
    let fit = fit_with(
        &dsl_compile("Y = A*exp(-K*X)"),
        &[series_xy(0, "DS0", &xs, &ys)],
        &SolverConfig { method: FitMethod::PoissonDeviance, ..SolverConfig::default() },
    );
    assert!(fit.weighted_ss < 1e-12);
    for p in &fit.parameters {
        let (lo, hi) = unwrap_finite_ci((p.ci_lower, p.ci_upper), &p.name);
        assert!(lo < p.estimate && p.estimate < hi);
        let sym_half = 0.5 * (p.symmetric_ci_upper - p.symmetric_ci_lower);
        let (r_lo, r_hi) = ((p.estimate - lo) / sym_half, (hi - p.estimate) / sym_half);
        assert!((0.8..1.25).contains(&r_lo) && (0.8..1.25).contains(&r_hi), "{r_lo:.4} {r_hi:.4}");
    }
}

#[test]
fn r1_s03_02_brent_failure_returns_open_not_midpoint() {
    for (lo, hi, notes, finite) in profile_wald_ratios(FitMethod::PoissonDeviance) {
        let exact_mid = (lo - 0.5).abs() < 1e-6 && (hi - 0.5).abs() < 1e-6;
        assert!(!(exact_mid && finite && (notes == "[]" || notes.is_empty())), "midpoint bound with notes {notes}");
    }
    let bounds = compute_deviance_profile_likelihood_cis(&[0.0], 1.0, &unit_cov_1d(), &[(None, None)], ConfidenceLevel::DEFAULT_95, |_k, val, _s| {
        let dist = val.abs();
        ((dist - 1.959_963_984_540_054).abs() > 0.5).then(|| (1.0 + dist * dist, vec![val]))
    });
    assert_eq!(bounds, vec![(CiBound::Open, CiBound::Open)]);
}

#[test]
fn r1_s03_03_rout_independent_curves_use_per_curve_scale_and_shared_pools() {
    let model = dsl_compile("Y = A + B*X");
    let xs: Vec<f64> = (0..12).map(f64::from).collect();
    let small = [0.01, -0.012, 0.008, -0.005, 0.011, -0.009, 0.004, -0.007, 0.006, -0.01, 0.003, -0.002];
    let mut quiet: Vec<f64> = xs.iter().zip(small).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    quiet[5] += 1.0;
    let big = [5.0, -6.0, 4.0, -3.0, 7.0, -5.0, 2.0, -4.0, 6.0, -7.0, 3.0, -2.0];
    let noisy: Vec<f64> = xs.iter().zip(big).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    let cfg = SolverConfig { method: FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }, ..SolverConfig::default() };

    assert_eq!(fit_with(&model, &[series_xy(0, "DS0", &xs, &quiet)], &cfg).n_outliers_excluded, 1);

    let both = fit_with(&model, &[series_xy(0, "DS0", &xs, &quiet), series_xy(1, "DS1", &xs, &noisy)], &cfg);
    assert!(both.n_outliers_excluded >= 1);
    assert!(both.rout.as_ref().unwrap().audit.iter().any(|a| a.data_set_index == 0 && a.row == 5 && a.is_outlier));
    assert_eq!(both.datasets[0].params.len(), 2);
    assert_approx_eq(both.datasets[0].params[0], 1.0, 0.05, 0.05, "quiet intercept");
    assert_approx_eq(both.datasets[0].params[1], 2.0, 0.05, 0.05, "quiet slope");

    let shared_cfg = SolverConfig { parameter_rules: vec![rule_shared("B")], ..cfg };
    assert_eq!(fit_with(&model, &[series_xy(0, "DS0", &xs, &quiet), series_xy(1, "DS1", &xs, &noisy)], &shared_cfg).k_free, 3);
}

fn assert_tabs_match(got: &ResultTab, exp: &ResultTab, max_rows: usize, num_tol: f64, ci_tol: f64, tag: &str) {
    assert_eq!(got.rows.len(), exp.rows.len(), "{tag} row count");
    for (r, (gr, er)) in got.rows.iter().zip(exp.rows.iter()).take(max_rows).enumerate() {
        for (c, (g, e)) in gr.iter().zip(er.iter()).enumerate() {
            match (g, e) {
                (ResultCell::Number { value: a, .. }, ResultCell::Number { value: b, .. }) => {
                    assert_approx_eq(*a, *b, num_tol, num_tol, &format!("{tag}[{r}][{c}]"));
                }
                (
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(al), upper: CiBound::Finite(ah) },
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(bl), upper: CiBound::Finite(bh) },
                ) => {
                    assert_approx_eq(*al, *bl, ci_tol, ci_tol, &format!("{tag} CI lo[{r}]"));
                    assert_approx_eq(*ah, *bh, ci_tol, ci_tol, &format!("{tag} CI hi[{r}]"));
                }
                _ => assert_eq!(g, e, "{tag}[{r}][{c}]"),
            }
        }
    }
}

#[test]
fn r1_x2_02_mean_sd_sem_cv_n_match_raw_replicates_linear_and_nonlinear() {
    let reps: [&[f64]; 5] = [&[1.0, 2.0, 3.0], &[3.0, 4.0, 6.0], &[4.0, 5.0, 7.0], &[6.0, 8.0, 9.0], &[7.0, 9.0, 10.0, 11.0, 12.0, 14.0]];
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let mk_xy = |mode: SubcolumnMode| XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, 5).unwrap();
    let (mut raw, mut sum_sd, mut sum_sem, mut sum_cv) =
        (mk_xy(SubcolumnMode::RawReplicates(6)), mk_xy(SubcolumnMode::MeanSdN), mk_xy(SubcolumnMode::MeanSemN), mk_xy(SubcolumnMode::MeanCvN));
    for (r, &v) in reps.iter().enumerate() {
        for tbl in [&mut raw, &mut sum_sd, &mut sum_sem, &mut sum_cv] {
            tbl.set_x_cell(r, Cell::number(xs[r])).unwrap();
        }
        for (k, &y) in v.iter().enumerate() {
            raw.data_sets[0].cells[r][k] = Cell::number(y);
        }
        let (m, sd, n) = mean_sd_n(v);
        sum_sd.data_sets[0].cells[r] = vec![Cell::number(m), Cell::number(sd), Cell::number(n)];
        sum_sem.data_sets[0].cells[r] = vec![Cell::number(m), Cell::number(sd / n.sqrt()), Cell::number(n)];
        sum_cv.data_sets[0].cells[r] = vec![Cell::number(m), Cell::number(100.0 * sd / m.abs()), Cell::number(n)];
    }
    let raw = DataSheet::new("raw", "raw", TableArchetype::Xy(raw)).unwrap();
    let summaries = [
        ("MeanSdN", DataSheet::new("sum_sd", "sum_sd", TableArchetype::Xy(sum_sd)).unwrap()),
        ("MeanSemN", DataSheet::new("sum_sem", "sum_sem", TableArchetype::Xy(sum_sem)).unwrap()),
        ("MeanCvN", DataSheet::new("sum_cv", "sum_cv", TableArchetype::Xy(sum_cv)).unwrap()),
    ];

    let raw_lin = exec_lin(&raw, LinearRegressionSpec::default());
    let nl_spec = NonlinearFitSpec { weighting: WeightingScheme::None, ..NonlinearFitSpec::new("straight_line") };
    let raw_nl = exec_nl(&raw, nl_spec.clone());
    for (name, sheet) in &summaries {
        let sum_lin = exec_lin(sheet, LinearRegressionSpec::default());
        for key in ["Slope", "YIntercept", "R2"] {
            assert_approx_eq(
                sum_lin.lookup_scalar(key, Some(0)).unwrap(),
                raw_lin.lookup_scalar(key, Some(0)).unwrap(),
                1e-10,
                1e-10,
                &format!("lin {name} {key}"),
            );
        }
        assert_tabs_match(&sum_lin.tabs[0], &raw_lin.tabs[0], 9, 1e-10, 1e-10, &format!("lin {name}"));

        let sum_nl = exec_nl(sheet, nl_spec.clone());
        for (key, ds_idx) in [("YIntercept", Some(0)), ("Slope", Some(0)), ("R2", None), ("RSS", None)] {
            assert_approx_eq(
                sum_nl.lookup_scalar(key, ds_idx).unwrap(),
                raw_nl.lookup_scalar(key, ds_idx).unwrap(),
                1e-7,
                1e-7,
                &format!("nl {name} {key}"),
            );
        }
        assert!(sum_nl.tabs.len() >= 2 && raw_nl.tabs.len() >= 2);
        assert_tabs_match(&sum_nl.tabs[0], &raw_nl.tabs[0], usize::MAX, 1e-6, 1e-5, &format!("nl {name} t0"));
        assert_tabs_match(&sum_nl.tabs[1], &raw_nl.tabs[1], 7, 1e-6, 1e-5, &format!("nl {name} t1"));
    }

    let raw_a: Vec<DataPoint> =
        xs.iter().zip(reps).enumerate().flat_map(|(r, (&x, v))| v.iter().map(move |&y| DataPoint { row: r, ..DataPoint::xy(x, y) })).collect();
    let raw_b: Vec<DataPoint> = raw_a.iter().map(|p| DataPoint { y: p.y + 3.0, data_set_index: 1, ..p.clone() }).collect();
    let sum_a: Vec<DataPoint> = xs
        .iter()
        .zip(reps)
        .enumerate()
        .map(|(r, (&x, v))| {
            let (m, sd, n) = mean_sd_n(v);
            DataPoint { sd: Some(sd), n: Some(n), row: r, ..DataPoint::xy(x, m) }
        })
        .collect();
    let sum_b: Vec<DataPoint> = sum_a.iter().map(|p| DataPoint { y: p.y + 3.0, data_set_index: 1, ..p.clone() }).collect();

    let (lin_r, lin_s) = (
        simple_linear_regression(&raw_a, &SimpleLinearOptions::default()).unwrap(),
        simple_linear_regression(&sum_a, &SimpleLinearOptions::default()).unwrap(),
    );
    assert_eq!((lin_s.n, lin_s.df_residual), (lin_r.n, lin_r.df_residual));
    assert_approx_eq(lin_s.sy_x, lin_r.sy_x, 1e-10, 1e-10, "sy_x");
    assert_approx_eq(lin_s.r_squared, lin_r.r_squared, 1e-10, 1e-10, "r_squared");

    let (cmp_r, cmp_s) = (compare_linear_lines(&[&raw_a, &raw_b], false).unwrap(), compare_linear_lines(&[&sum_a, &sum_b], false).unwrap());
    assert_eq!(cmp_s.df_slopes, cmp_r.df_slopes);
    assert_approx_eq(cmp_s.f_slopes, cmp_r.f_slopes, 1e-10, 1e-10, "ANCOVA f_slopes");
    assert_approx_eq(cmp_s.f_intercepts.unwrap(), cmp_r.f_intercepts.unwrap(), 1e-10, 1e-10, "ANCOVA f_intercepts");

    for bad in [
        [
            DataPoint { sd: Some(0.5), n: Some(1.5), row: 0, ..DataPoint::xy(1.0, 2.0) },
            DataPoint { sd: Some(0.5), n: Some(3.0), row: 1, ..DataPoint::xy(2.0, 4.0) },
        ],
        [
            DataPoint { sd: Some(-0.5), n: Some(3.0), row: 0, ..DataPoint::xy(1.0, 2.0) },
            DataPoint { sd: Some(0.5), n: Some(3.0), row: 1, ..DataPoint::xy(2.0, 4.0) },
        ],
    ] {
        assert!(simple_linear_regression(&bad, &SimpleLinearOptions::default()).is_err());
    }
}
