//! NIST StRD numerical accuracy, rank-deficient linear algebra, tail-safe distribution, and
//! degenerate-input robustness integration suite for `curvebook-math` + `curvebook-testkit`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::dist::{
    chi2_isf, chi2_sf, dunnett_one_sided_crit, dunnett_one_sided_p, dunnett_two_sided_crit, dunnett_two_sided_p, f_isf, f_sf, norm_cdf, norm_sf,
    student_t_isf, student_t_sf, student_t_two_sided_p, studentized_range_isf, studentized_range_sf,
};
use curvebook_math::sum::{NeumaierSum, SummaryMoments, covariance, lag1_autocorrelation, pearson_r, sample_sd};
use curvebook_math::{
    AxisScale, ConfidenceLevel, DenseMatrix, DeterministicRng, LogitDomain, PValueFormat, cholesky, noncentral_chi2_ci, noncentral_f_ci,
    noncentral_t_ci, parameter_dependencies, qr_col_pivoted, solve_linear_least_squares, svd,
};
use curvebook_testkit::nist_strd::{
    all_offline_anova_benchmarks, all_offline_linear_regression_benchmarks, all_offline_univariate_benchmarks, binary_stiff_1e8, corpus_available,
    default_nist_strd_dir, load_corpus_anova_all, load_corpus_linear_all, load_corpus_univariate_all, numacc4,
};
use curvebook_testkit::{DoubleDouble, assert_approx_eq, assert_lre_at_least, oracle_univariate_f64};

#[test]
fn nist_strd_univariate_stiffness_and_permutation_invariance() {
    for ds in all_offline_univariate_benchmarks() {
        let moments = SummaryMoments::from_slice(&ds.values).unwrap();
        let sd = moments.sample_sd().unwrap();
        let r1 = lag1_autocorrelation(&ds.values).unwrap();

        // Against exact 106-bit DoubleDouble oracle of the IEEE-754 f64 input vector, compensated
        // two-pass moments retain >= 14.5 digits on every dataset including NumAcc4 & BinaryStiff1e8!
        let (oracle_mean, oracle_sd, oracle_r1) = oracle_univariate_f64(&ds.values);
        assert_lre_at_least(moments.mean, oracle_mean, 14.5, &format!("{} mean (f64 oracle)", ds.name));
        assert_lre_at_least(sd, oracle_sd, 14.5, &format!("{} sd (f64 oracle)", ds.name));
        assert_approx_eq(r1, oracle_r1, 1e-14, 1e-13, &format!("{} r1 (f64 oracle)", ds.name));

        // Against decimal NIST certified targets:
        // NumAcc1, NumAcc2, and BinaryStiff1e8 have exact binary inputs (>= 14.5 digits);
        // NumAcc3 (~9.63 digits) and NumAcc4 (~8.25 digits) hit the theoretical IEEE-754 input-rounding ceiling.
        let min_decimal_sd_lre = match ds.name.as_str() {
            "NumAcc3" => 9.5,
            "NumAcc4" => 8.0,
            _ => 14.5,
        };
        assert_lre_at_least(moments.mean, ds.certified_mean, 14.5, &format!("{} decimal mean", ds.name));
        assert_lre_at_least(sd, ds.certified_sd, min_decimal_sd_lre, &format!("{} decimal sd", ds.name));
        if ds.name == "NumAcc1" {
            assert_eq!(ds.values, vec![10_000_001.0, 10_000_003.0, 10_000_002.0]);
            assert_eq!(ds.certified_r1, -0.5);
            assert_lre_at_least(r1, ds.certified_r1, 14.5, "NumAcc1 decimal r1");
        }
    }

    // Row-permutation invariance under deterministic shuffling on stiff series (mean / sd = 8e8):
    let mut rng = DeterministicRng::from_seed(0x5EED_CAFE_2026);
    for base_ds in [numacc4(), binary_stiff_1e8()] {
        let ref_moments = SummaryMoments::from_slice(&base_ds.values).unwrap();
        let ref_sd = ref_moments.sample_sd().unwrap();
        let mut perm = base_ds.values.clone();
        for _ in 0..5 {
            rng.shuffle(&mut perm);
            let m = SummaryMoments::from_slice(&perm).unwrap();
            assert_lre_at_least(m.mean, ref_moments.mean, 14.5, &format!("{} shuffled mean", base_ds.name));
            assert_lre_at_least(m.sample_sd().unwrap(), ref_sd, 14.5, &format!("{} shuffled sd", base_ds.name));
        }
    }
}

#[test]
fn nist_strd_linear_regression_wampler_noint_and_vandermonde() {
    for ds in all_offline_linear_regression_benchmarks() {
        let n = ds.design_rows.len();
        let p = ds.certified_betas.len();
        let mut flat = Vec::with_capacity(n * p);
        for row in &ds.design_rows {
            flat.extend_from_slice(row);
        }
        let x = DenseMatrix::from_row_major(n, p, flat).unwrap();
        let (fit, cov_unscaled) = solve_linear_least_squares(&x, &ds.y).unwrap();
        assert_eq!(fit.rank, p, "{} expected full rank {p}, got {}", ds.name, fit.rank);

        let min_beta_lre = match ds.name.as_str() {
            "FilipScaleVandermonde" => 6.0, // kappa(X) ~ 1e10 -> ~6-7 digits in IEEE-754 f64 (0 digits for X^T X!)
            "WamplerOrtho3" => 11.0,        // Bjorck augmented Dot2 refinement recovers 11+ digits even at s = 10_000!
            "WamplerOrtho2" => 13.0,
            _ => 13.5,
        };
        for (j, (&actual_b, &cert_b)) in fit.coefficients.iter().zip(ds.certified_betas.iter()).enumerate() {
            assert_lre_at_least(actual_b, cert_b, min_beta_lre, &format!("{} B{j}", ds.name));
        }

        if ds.certified_ss_res > 0.0 {
            let min_rss_lre = if ds.name == "FilipScaleVandermonde" { 9.0 } else { 12.0 };
            assert_lre_at_least(fit.rss, ds.certified_ss_res, min_rss_lre, &format!("{} RSS", ds.name));
            let df_res = fit.df_residual as f64;
            let sigma2 = fit.rss / df_res;
            let rsd = sigma2.sqrt();
            assert_lre_at_least(rsd, ds.certified_residual_sd, min_rss_lre, &format!("{} RSD", ds.name));

            for (j, &cert_se) in ds.certified_se_betas.iter().enumerate() {
                let se = (sigma2 * cov_unscaled.get(j, j).unwrap()).sqrt();
                let min_se_lre = if ds.name == "FilipScaleVandermonde" { 8.0 } else { 11.0 };
                assert_lre_at_least(se, cert_se, min_se_lre, &format!("{} SE(B{j})", ds.name));
            }
        } else {
            assert!(fit.rss < 1e-16, "{} exact polynomial RSS={:.3e}", ds.name, fit.rss);
        }

        // Cross-check unscaled SVD pseudoinverse solve (kappa(X) ~ 1.9e6 -> ~9.3 digits unscaled):
        if ds.name != "FilipScaleVandermonde" {
            let svd_beta = svd(&x).unwrap().solve_least_squares(&ds.y, None).unwrap();
            let svd_floor = match ds.name.as_str() {
                "WamplerOrtho3" => 6.0,
                "WamplerOrtho2" => 8.0,
                _ => 9.0,
            };
            for (j, (&sb, &cert_b)) in svd_beta.iter().zip(ds.certified_betas.iter()).enumerate() {
                assert_lre_at_least(sb, cert_b, svd_floor, &format!("{} SVD B{j}", ds.name));
            }
        }
    }
}

#[test]
fn nist_strd_one_way_anova_stiffness_retains_14_plus_digits() {
    for ds in all_offline_anova_benchmarks() {
        let mut max_group = 0_usize;
        for &(g, _) in &ds.observations {
            max_group = max_group.max(g);
        }
        let mut groups: Vec<Vec<f64>> = vec![Vec::new(); max_group];
        let mut all_vals = Vec::with_capacity(ds.observations.len());
        for &(g, y) in &ds.observations {
            groups[g - 1].push(y);
            all_vals.push(y);
        }

        let grand = SummaryMoments::from_slice(&all_vals).unwrap();
        let mut ss_between_acc = NeumaierSum::new();
        let mut ss_within_acc = NeumaierSum::new();
        for grp in &groups {
            let gm = SummaryMoments::from_slice(grp).unwrap();
            let diff = gm.mean - grand.mean;
            ss_between_acc.add((gm.count as f64) * diff * diff);
            ss_within_acc.add(gm.css);
        }

        let ss_between = ss_between_acc.total();
        let ss_within = ss_within_acc.total();
        let df_b = ds.certified_df_between as f64;
        let df_w = ds.certified_df_within as f64;
        let ms_b = ss_between / df_b;
        let ms_w = ss_within / df_w;
        let f_stat = ms_b / ms_w;
        let r_sq = ss_between / (ss_between + ss_within);
        let rsd = ms_w.sqrt();

        assert_lre_at_least(ss_between, ds.certified_ss_between, 14.5, &format!("{} SS_between", ds.name));
        assert_lre_at_least(ss_within, ds.certified_ss_within, 14.5, &format!("{} SS_within", ds.name));
        assert_lre_at_least(f_stat, ds.certified_f, 14.5, &format!("{} F", ds.name));
        assert_lre_at_least(r_sq, ds.certified_r_squared, 14.5, &format!("{} R^2", ds.name));
        assert_lre_at_least(rsd, ds.certified_residual_sd, 14.5, &format!("{} RSD", ds.name));

        // Exercise tail-safe F p-value and noncentral-F CI (95% confidence_percent) on the ANOVA statistic:
        let p_val = f_sf(f_stat, df_b, df_w).unwrap();
        assert!(p_val > 0.0 && p_val < 1.0);
        let (lam_lo, lam_hi) = noncentral_f_ci(f_stat, df_b, df_w, 95.0).unwrap();
        assert!(lam_lo >= 0.0 && lam_hi > lam_lo);
    }
}

#[test]
fn rank_deficient_design_matrices_detected_cleanly_without_nan_or_panic() {
    // Exact collinearity: column 2 = 2 * column 0 + 3 * column 1!
    let mut rows = Vec::new();
    let mut y = Vec::new();
    for i in 0..12 {
        let x1 = 1.0_f64;
        let x2 = f64::from(i);
        let x3 = 2.0 * x1 + 3.0 * x2;
        rows.extend_from_slice(&[x1, x2, x3]);
        y.push(4.0 + 1.5 * x2);
    }
    let x = DenseMatrix::from_row_major(12, 3, rows).unwrap();
    let pqr = qr_col_pivoted(&x).unwrap();
    assert_eq!(pqr.rank(), 2);
    assert_eq!(pqr.aliased_columns().len(), 1);

    let (ls, cov_rank_def) = solve_linear_least_squares(&x, &y).unwrap();
    assert_eq!(ls.rank, 2);
    assert_eq!(ls.aliased_columns.len(), 1);
    assert!(ls.rss < 1e-24);
    assert!(cov_rank_def.get(0, 0).unwrap().is_finite());
    for &b in &ls.coefficients {
        assert!(b.is_finite());
    }

    // Near-collinear 2-column Jacobian J whose Gram matrix J^T J = [[1, r], [r, 1]] with r = 0.999
    // has exact parameter dependency r^2 = 0.999^2:
    let r = 0.999_f64;
    let j_mat = DenseMatrix::from_row_major(2, 2, vec![1.0, r, 0.0, (1.0 - r * r).sqrt()]).unwrap();
    let deps = parameter_dependencies(&j_mat).unwrap();
    assert!((deps[0] - r * r).abs() < 1e-11, "deps={deps:?}");
}

#[test]
fn tail_safe_distributions_and_multiple_comparison_integrals_sweep() {
    // Deep upper tails where 1.0 - cdf(x) collapses identically to 0.0:
    let z = 25.0;
    assert_eq!(1.0 - norm_cdf(z), 0.0);
    assert!(norm_sf(z) > 0.0 && norm_sf(z) < 1e-130);

    // Scale invariance and quantile round-trips across fractional df:
    for &df in &[0.75, 2.5, 18.4, 500.0] {
        for &q in &[0.25, 0.01, 1e-8, 1e-25] {
            let t_q = student_t_isf(q, df).unwrap();
            assert_approx_eq(student_t_sf(t_q, df).unwrap(), q, 0.0, 1e-11, "student_t_isf roundtrip");

            let f_q = f_isf(q, 3.5, df).unwrap();
            assert_approx_eq(f_sf(f_q, 3.5, df).unwrap(), q, 0.0, 1e-11, "f_isf roundtrip");

            let c_q = chi2_isf(q, df).unwrap();
            assert_approx_eq(chi2_sf(c_q, df).unwrap(), q, 0.0, 1e-11, "chi2_isf roundtrip");
        }
    }

    // Studentized range monotonicity and quantile inversion for 2 <= k <= 200:
    let mut prev_crit = 0.0_f64;
    for &k in &[2_usize, 3, 5, 10, 25, 50, 100, 200] {
        let q_crit = studentized_range_isf(0.05, k, 24.5).unwrap();
        assert!(q_crit > prev_crit, "q_crit must increase strictly with k: k={k}");
        let sf = studentized_range_sf(q_crit, k, 24.5).unwrap();
        assert_approx_eq(sf, 0.05, 1e-10, 1e-9, &format!("studentized_range k={k}"));
        prev_crit = q_crit;
    }

    // Dunnett unequal group sizes (one-sided and two-sided) + 99.9% CI:
    let ci_999 = ConfidenceLevel::from_percent(99.9).unwrap();
    let d2 = dunnett_two_sided_crit(ci_999.alpha(), 19.3, 10.0, &[4.0, 7.0, 12.0, 6.0]).unwrap();
    assert_approx_eq(
        dunnett_two_sided_p(d2, 19.3, 10.0, &[4.0, 7.0, 12.0, 6.0]).unwrap(),
        ci_999.alpha(),
        1e-12,
        1e-9,
        "dunnett two-sided unequal n",
    );
    let d1 = dunnett_one_sided_crit(0.025, 19.3, 10.0, &[4.0, 7.0, 12.0]).unwrap();
    assert_approx_eq(dunnett_one_sided_p(d1, 19.3, 10.0, &[4.0, 7.0, 12.0]).unwrap(), 0.025, 1e-12, 1e-9, "dunnett one-sided unequal n");

    // Noncentral t and chi^2 CIs (confidence_percent in [0.01, 99.99]) + PValueFormat:
    let (d_lo, d_hi) = noncentral_t_ci(3.2, 18.0, 99.0).unwrap();
    assert!(d_lo < 3.2 && 3.2 < d_hi);
    let (c_lo, c_hi) = noncentral_chi2_ci(14.0, 3.0, 95.0).unwrap();
    assert!(c_lo >= 0.0 && c_hi > c_lo);
    assert_eq!(PValueFormat::FourDecimal.stars(student_t_two_sided_p(8.0, 20.0).unwrap()), "****");
}

#[test]
fn degenerate_and_non_finite_inputs_return_structured_errors_without_panicking() {
    // Empty, singleton, constant, NaN, and +-Inf slices:
    assert!(SummaryMoments::from_slice(&[]).is_err());
    assert!(sample_sd(&[42.0]).is_err());
    assert_eq!(sample_sd(&[7.0, 7.0, 7.0, 7.0]).unwrap(), 0.0);
    assert!(pearson_r(&[7.0, 7.0, 7.0], &[1.0, 2.0, 3.0]).is_err());
    assert!(lag1_autocorrelation(&[7.0, 7.0, 7.0]).is_err());
    assert!(SummaryMoments::from_slice(&[1.0, f64::NAN, 3.0]).is_err());
    assert!(SummaryMoments::from_slice(&[1.0, f64::INFINITY, 3.0]).is_err());
    assert!(SummaryMoments::from_slice(&[1.0, f64::NEG_INFINITY, 3.0]).is_err());
    assert!(covariance(&[1.0, 2.0], &[1.0]).is_err());

    // Invalid distribution parameters & domains:
    assert!(student_t_two_sided_p(1.0, 0.0).is_err());
    assert!(student_t_two_sided_p(1.0, -2.0).is_err());
    assert!(student_t_two_sided_p(f64::NAN, 5.0).is_err());
    assert!(f_sf(-0.1, 2.0, 10.0).is_err());
    assert!(chi2_sf(-1.0, 4.0).is_err());
    assert!(studentized_range_sf(2.0, 1, 10.0).is_err());
    assert!(dunnett_two_sided_p(2.0, 10.0, 5.0, &[]).is_err());

    // Indefinite Cholesky and mismatched buffer lengths:
    let indef = DenseMatrix::from_row_major(2, 2, vec![1.0, 2.0, 2.0, 1.0]).unwrap();
    assert!(cholesky(&indef).is_err());
    assert!(DenseMatrix::from_row_major(2, 2, vec![1.0, 2.0, 3.0]).is_err());
    assert_eq!(DenseMatrix::zeros(0, 3).unwrap().nrows(), 0);

    // Guarded AxisScale on hostile inputs never yields NaN or Inf:
    for scale in [
        AxisScale::Linear,
        AxisScale::Log10,
        AxisScale::Log2,
        AxisScale::Ln,
        AxisScale::Reciprocal,
        AxisScale::Logit(LogitDomain::Percent),
        AxisScale::Logit(LogitDomain::Proportion),
        AxisScale::Probit,
    ] {
        for &bad in &[f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -10.0, 0.0, 1.0, 100.0, 150.0] {
            if let Some(t) = scale.transform(bad) {
                assert!(t.is_finite());
            }
            if let Some(u) = scale.inverse(bad) {
                assert!(u.is_finite());
            }
        }
    }
}

fn require_corpus_dir() -> Option<std::path::PathBuf> {
    let dir = default_nist_strd_dir();
    if corpus_available(&dir) {
        return Some(dir);
    }
    assert!(
        std::env::var_os("CURVEBOOK_REQUIRE_CORPUS").is_none_or(|v| v != "1"),
        "CURVEBOOK_REQUIRE_CORPUS=1 set, but NIST StRD corpus is missing or incomplete at {}",
        dir.display()
    );
    None
}

#[test]
fn nist_strd_real_corpus_univariate_all_9_datasets() {
    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let datasets = load_corpus_univariate_all(&dir).unwrap();
    assert_eq!(datasets.len(), 9);

    for ds in datasets {
        let moments = SummaryMoments::from_slice(&ds.values).unwrap();
        let sd = moments.sample_sd().unwrap();
        let r1 = lag1_autocorrelation(&ds.values).unwrap();

        // Against 106-bit DoubleDouble oracle of the IEEE-754 f64 input vector:
        let (oracle_mean, oracle_sd, oracle_r1) = oracle_univariate_f64(&ds.values);
        assert_lre_at_least(moments.mean, oracle_mean, 14.5, &format!("{} mean (f64 oracle)", ds.name));
        assert_lre_at_least(sd, oracle_sd, 14.5, &format!("{} sd (f64 oracle)", ds.name));
        let oracle_r1_tol = match ds.name.as_str() {
            "NumAcc3" | "NumAcc4" => 1e-10,
            _ => 1e-13,
        };
        assert_approx_eq(r1, oracle_r1, 1e-14, oracle_r1_tol, &format!("{} r1 (f64 oracle)", ds.name));

        // Against decimal NIST certified targets (reflecting IEEE-754 decimal-input rounding on shifted series):
        assert_lre_at_least(moments.mean, ds.certified_mean, 14.5, &format!("{} decimal mean", ds.name));
        let (min_sd_lre, min_r1_lre) = match ds.name.as_str() {
            "NumAcc3" => (9.2, 11.5),   // 1000000.2_f64 decimal input rounding (~9.46 digits sd, ~11.93 r1)
            "NumAcc4" => (8.0, 10.5),   // 10000000.2_f64 decimal input rounding (~8.25 digits sd, ~10.73 r1)
            "Mavro" => (13.0, 13.5),    // 3 constant leading digits around 2.0018 (~13.12 sd, ~13.75 r1)
            "Michelso" => (13.5, 13.2), // 4 constant leading digits around 299.85 (~13.84 sd, ~13.44 r1)
            _ => (15.0, 14.5),
        };
        assert_lre_at_least(sd, ds.certified_sd, min_sd_lre, &format!("{} decimal sd", ds.name));
        assert_lre_at_least(r1, ds.certified_r1, min_r1_lre, &format!("{} decimal r1", ds.name));
    }
}

fn oracle_anova_f64(groups: &[Vec<f64>], all_vals: &[f64], df_b: usize, df_w: usize) -> (f64, f64, f64, f64, f64) {
    let grand =
        all_vals.iter().copied().fold(DoubleDouble::ZERO, |a, v| a + DoubleDouble::from_f64(v)) / DoubleDouble::from_f64(all_vals.len() as f64);
    let mut ss_b = DoubleDouble::ZERO;
    let mut ss_w = DoubleDouble::ZERO;
    for grp in groups {
        let g_n = DoubleDouble::from_f64(grp.len() as f64);
        let g_mean = grp.iter().copied().fold(DoubleDouble::ZERO, |a, v| a + DoubleDouble::from_f64(v)) / g_n;
        ss_b += g_n * (g_mean - grand).sqr();
        for &v in grp {
            ss_w += (DoubleDouble::from_f64(v) - g_mean).sqr();
        }
    }
    let ms_b = ss_b / DoubleDouble::from_f64(df_b as f64);
    let ms_w = ss_w / DoubleDouble::from_f64(df_w as f64);
    let f_stat = ms_b / ms_w;
    let r_sq = ss_b / (ss_b + ss_w);
    let rsd = ms_w.sqrt();
    (ss_b.to_f64(), ss_w.to_f64(), f_stat.to_f64(), r_sq.to_f64(), rsd.to_f64())
}

#[test]
fn nist_strd_real_corpus_anova_all_11_datasets() {
    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let datasets = load_corpus_anova_all(&dir).unwrap();
    assert_eq!(datasets.len(), 11);

    for ds in datasets {
        let mut max_group = 0_usize;
        for &(g, _) in &ds.observations {
            max_group = max_group.max(g);
        }
        let mut groups: Vec<Vec<f64>> = vec![Vec::new(); max_group];
        let mut all_vals = Vec::with_capacity(ds.observations.len());
        for &(g, y) in &ds.observations {
            groups[g - 1].push(y);
            all_vals.push(y);
        }

        let grand = SummaryMoments::from_slice(&all_vals).unwrap();
        let mut ss_between_acc = NeumaierSum::new();
        let mut ss_within_acc = NeumaierSum::new();
        for grp in &groups {
            let gm = SummaryMoments::from_slice(grp).unwrap();
            let diff = gm.mean - grand.mean;
            ss_between_acc.add((gm.count as f64) * diff * diff);
            ss_within_acc.add(gm.css);
        }

        let ss_between = ss_between_acc.total();
        let ss_within = ss_within_acc.total();
        let df_b = ds.certified_df_between as f64;
        let df_w = ds.certified_df_within as f64;
        let f_stat = (ss_between / df_b) / (ss_within / df_w);
        let r_sq = ss_between / (ss_between + ss_within);
        let rsd = (ss_within / df_w).sqrt();

        // Against 106-bit DoubleDouble oracle of the parsed IEEE-754 f64 observations:
        // `SS_within` (`SummaryMoments::css`) hits full 15.95-digit f64 precision on all 11 datasets,
        // while `SS_between` from f64 `SummaryMoments::mean` is bounded by `ulp(mean)` in `gm.mean - grand.mean`.
        let (o_ssb, o_ssw, o_f, o_r2, o_rsd) = oracle_anova_f64(&groups, &all_vals, ds.certified_df_between, ds.certified_df_within);
        let oracle_ssb_floor = match ds.name.as_str() {
            "SmLs07" | "SmLs08" | "SmLs09" => 3.0,
            "AtmWtAg" => 8.5,
            "SmLs04" | "SmLs05" | "SmLs06" => 9.0,
            _ => 13.5,
        };
        assert_lre_at_least(ss_between, o_ssb, oracle_ssb_floor, &format!("{} SS_between (f64 oracle)", ds.name));
        assert_lre_at_least(ss_within, o_ssw, 15.0, &format!("{} SS_within (f64 oracle)", ds.name));
        assert_lre_at_least(f_stat, o_f, oracle_ssb_floor, &format!("{} F (f64 oracle)", ds.name));
        assert_lre_at_least(r_sq, o_r2, oracle_ssb_floor, &format!("{} R^2 (f64 oracle)", ds.name));
        assert_lre_at_least(rsd, o_rsd, 15.0, &format!("{} RSD (f64 oracle)", ds.name));

        // Against decimal NIST certified targets (bounded by IEEE-754 f64 input representation):
        let min_lre = match ds.name.as_str() {
            // SmLs07-09 shift by 1e12 + 0.4, so IEEE-754 f64 input rounding limits decimal LRE to ~3.13-4.57 digits.
            "SmLs07" | "SmLs08" | "SmLs09" => 3.0,
            // AtmWtAg (107.8681..., ~8.79 digits) & SmLs04-06 (1e6 + 0.4, ~9.16-10.59 digits).
            "AtmWtAg" => 8.5,
            "SmLs04" | "SmLs05" | "SmLs06" => 9.0,
            "SiRstv" => 13.0,
            _ => 14.5,
        };
        assert_lre_at_least(ss_between, ds.certified_ss_between, min_lre, &format!("{} SS_between", ds.name));
        assert_lre_at_least(ss_within, ds.certified_ss_within, min_lre, &format!("{} SS_within", ds.name));
        assert_lre_at_least(f_stat, ds.certified_f, min_lre, &format!("{} F", ds.name));
        assert_lre_at_least(r_sq, ds.certified_r_squared, min_lre, &format!("{} R^2", ds.name));
        assert_lre_at_least(rsd, ds.certified_residual_sd, min_lre, &format!("{} RSD", ds.name));
    }
}

#[test]
fn nist_strd_real_corpus_linear_regression_all_11_datasets() {
    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let datasets = load_corpus_linear_all(&dir).unwrap();
    assert_eq!(datasets.len(), 11);

    for ds in datasets {
        let n = ds.design_rows.len();
        let p = ds.certified_betas.len();
        let mut flat = Vec::with_capacity(n * p);
        for row in &ds.design_rows {
            flat.extend_from_slice(row);
        }
        let x = DenseMatrix::from_row_major(n, p, flat).unwrap();
        let (fit, cov_unscaled) = solve_linear_least_squares(&x, &ds.y).unwrap();
        assert_eq!(fit.rank, p, "{} expected full rank {p}, got {}", ds.name, fit.rank);

        let beta_floor = match ds.name.as_str() {
            "Filip" => 7.5,     // degree-10 Vandermonde (kappa(X) ~ 1e10): observed min LRE = 7.63 digits
            "Wampler2" => 13.0, // observed min LRE = 13.20 digits
            _ => 13.5,          // Norris 14.06, Pontius 13.51, Longley 14.66, NoInt1/2 & Wampler1/3/4/5 >= 14.7
        };
        for (j, (&actual_b, &cert_b)) in fit.coefficients.iter().zip(ds.certified_betas.iter()).enumerate() {
            assert_lre_at_least(actual_b, cert_b, beta_floor, &format!("{} B{j}", ds.name));
        }

        if ds.certified_ss_res > 0.0 {
            let rss_floor = match ds.name.as_str() {
                "Filip" => 8.5, // observed RSS LRE = 8.61, RSD = 8.91
                _ => 13.5,      // Norris/Pontius/NoInt1/2/Longley/Wampler3-5 >= 13.57
            };
            assert_lre_at_least(fit.rss, ds.certified_ss_res, rss_floor, &format!("{} RSS", ds.name));
            let sigma2 = fit.rss / (fit.df_residual as f64);
            let rsd = sigma2.sqrt();
            assert_lre_at_least(rsd, ds.certified_residual_sd, rss_floor, &format!("{} RSD", ds.name));
            let se_floor = match ds.name.as_str() {
                "Filip" => 7.2,    // observed min SE LRE = 7.36
                "Longley" => 12.5, // observed min SE LRE = 12.74
                _ => 13.5,         // Norris/Pontius/NoInt1/2/Wampler3-5 >= 13.76
            };
            for (j, &cert_se) in ds.certified_se_betas.iter().enumerate() {
                let se = (sigma2 * cov_unscaled.get(j, j).unwrap()).sqrt();
                assert_lre_at_least(se, cert_se, se_floor, &format!("{} SE(B{j})", ds.name));
            }
        } else {
            assert!(fit.rss < 1e-14, "{} exact polynomial RSS={:.3e}", ds.name, fit.rss);
        }
    }
}
