//! Regression tests covering all confirmed SEC-01 audit findings in `curvebook-math`:
//! `FA-SEC01-01..04`, `FA-SEC01-06..18`, `FA-SEC01-20`, `FA-SEC01-21`, and `FA-SEC01-24`.

use std::fmt;

use curvebook_math::dist::core::{
    chi2_cdf, chi2_sf, f_cdf, f_isf, f_ln_pdf, f_ln_sf, f_ppf, f_sf, norm_cdf, norm_two_sided_isf, student_t_isf, student_t_two_sided_isf,
    student_t_two_sided_p,
};
use curvebook_math::dist::dunnett::{dunnett_loadings_from_sizes, dunnett_two_sided_crit, dunnett_two_sided_p, dunnett_two_sided_p_loadings};
use curvebook_math::dist::noncentral::{ncchi2_cdf, ncchi2_sf, ncf_cdf, nct_cdf, nct_sf};
use curvebook_math::dist::studentized_range::{max_studentized_range_isf, studentized_range_cdf, studentized_range_ln_sf, studentized_range_sf};
use curvebook_math::quad::{QuadOptions, gauss_kronrod_15_panel, integrate_adaptive};
use curvebook_math::roots::{RootOptions, brent_root};
use curvebook_math::special::{erfc_inv, gamma, ln_factorial, ln_reg_beta_xy, reg_beta, reg_gamma_p, reg_gamma_q};
use curvebook_math::sum::{SummaryMoments, pearson_r};
use curvebook_math::{
    AxisScale, DenseMatrix, DeterministicRng, LogitDomain, MathError, PValueFormat, noncentral_chi2_ci, noncentral_f_ci, parameter_dependencies,
    qr_col_pivoted, solve_linear_least_squares, svd, sym_eigen,
};
use curvebook_testkit::assert_approx_eq;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{self, SerializeMap, SerializeTuple};
use serde::{Deserialize, Serialize};

fn assert_rel(actual: f64, expected: f64, rel_tol: f64, label: &str) {
    assert_approx_eq(actual, expected, 0.0, rel_tol, label);
}

#[test]
fn fa_sec01_01_poisson_ptrs_goodness_of_fit_without_stirling_bias() {
    let mut rng = DeterministicRng::from_seed(0x5EC0_0101);
    let n = 150_000_usize;
    let nf = n as f64;

    for &lambda in &[10.0_f64, 25.0, 100.0] {
        let max_k = (lambda + 8.0 * lambda.sqrt()).ceil() as usize;
        let mut counts = vec![0_usize; max_k + 1];
        let mut sum = 0.0_f64;
        let mut sum_sq = 0.0_f64;

        for _ in 0..n {
            let k = rng.poisson(lambda).expect("valid Poisson lambda");
            let kf = k as f64;
            sum += kf;
            sum_sq += kf * kf;
            let idx = (k as usize).min(max_k);
            counts[idx] += 1;
        }

        let emp_mean = sum / nf;
        let emp_var = (sum_sq - sum * emp_mean) / (nf - 1.0);
        let se_mean = (lambda / nf).sqrt();
        assert!((emp_mean - lambda).abs() < 5.0 * se_mean, "lambda={lambda}: emp_mean={emp_mean} vs {lambda}");
        let se_var = (2.0 * lambda * lambda / nf).sqrt();
        assert!((emp_var - lambda).abs() < 6.0 * se_var, "lambda={lambda}: emp_var={emp_var} vs {lambda}");

        let mut pooled_obs = 0.0_f64;
        let mut pooled_exp = 0.0_f64;
        let mut chi2_stat = 0.0_f64;
        let mut bins = 0_usize;
        for (k, &obs) in counts.iter().enumerate() {
            let pmf = (-lambda + (k as f64) * lambda.ln() - ln_factorial(k as u64)).exp();
            pooled_obs += obs as f64;
            pooled_exp += nf * pmf;
            if pooled_exp >= 50.0 {
                let diff = pooled_obs - pooled_exp;
                chi2_stat += (diff * diff) / pooled_exp;
                bins += 1;
                pooled_obs = 0.0;
                pooled_exp = 0.0;
            }
        }
        if pooled_exp > 0.0 {
            let diff = pooled_obs - pooled_exp;
            chi2_stat += (diff * diff) / pooled_exp;
            bins += 1;
        }
        let dof = (bins.saturating_sub(1)) as f64;
        let p_val = chi2_sf(chi2_stat, dof).expect("chi2_sf");
        assert!(p_val > 1e-4, "lambda={lambda}: Poisson PTRS chi2={chi2_stat}, dof={dof}, p={p_val} indicates bias");
    }

    // Large-lambda PTRS regime (lambda = 1e15): standardized z = (k - lambda) / sqrt(lambda) across
    // bins including both normal tails |z| > 1.5 where PTRS us < 0.07 rejection occurs.
    let lambda_big = 1e15_f64;
    let sigma_big = lambda_big.sqrt();
    let n_big = 200_000_usize;
    let n_big_f = n_big as f64;
    let edges = [f64::NEG_INFINITY, -2.5, -2.0, -1.5, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, f64::INFINITY];
    let mut big_counts = [0_usize; 12];
    for _ in 0..n_big {
        let k = rng.poisson(lambda_big).expect("valid Poisson 1e15");
        let z = ((k as f64) - lambda_big) / sigma_big;
        let bin = edges.windows(2).position(|w| z >= w[0] && z < w[1]).unwrap_or(11);
        big_counts[bin] += 1;
    }
    let mut big_chi2 = 0.0_f64;
    for (i, &obs) in big_counts.iter().enumerate() {
        let lo = edges[i];
        let hi = edges[i + 1];
        let prob = (if hi.is_infinite() { 1.0 } else { norm_cdf(hi) }) - (if lo.is_infinite() { 0.0 } else { norm_cdf(lo) });
        let exp_c = n_big_f * prob;
        let diff = (obs as f64) - exp_c;
        big_chi2 += (diff * diff) / exp_c;
    }
    let big_p = chi2_sf(big_chi2, 11.0).expect("chi2_sf 1e15");
    assert!(big_p > 1e-4, "lambda=1e15 PTRS standardized chi2={big_chi2}, p={big_p} indicates tail rejection bias");
}

#[test]
fn fa_sec01_02_pearson_r_avoids_css_product_overflow() {
    let x_pos = [1e200, 2e200, 3e200];
    let y_pos = [-1e200, 0.0, 1e200];
    let r_pos = pearson_r(&x_pos, &y_pos).expect("pearson_r 1e200");
    assert_rel(r_pos, 1.0, 1e-14, "pearson_r +1 1e200");

    let y_neg = [1e200, 0.0, -1e200];
    let r_neg = pearson_r(&x_pos, &y_neg).expect("pearson_r -1e200");
    assert_rel(r_neg, -1.0, 1e-14, "pearson_r -1 1e200");

    let x_orth = [-1e200, 0.0, 1e200];
    let y_orth = [1e200, -2e200, 1e200];
    let r_orth = pearson_r(&x_orth, &y_orth).expect("pearson_r orthogonal 1e200");
    assert!(r_orth.abs() < 1e-14, "expected ~0, got {r_orth}");

    // Unscaled (1e-140, 1e130) band where css itself is normal yet css_x * css_y overflows or flushes:
    assert_rel(pearson_r(&[-1e80, 1e80], &[-1e80, 1e80]).expect("pearson_r 1e80"), 1.0, 1e-14, "pearson_r 1e80");
    assert_rel(
        pearson_r(&[-1e-85, 1e-85, 0.0, 0.0], &[-1e-85, 0.0, 1e-85, 0.0]).expect("pearson_r 1e-85 r=0.5"),
        0.5,
        1e-14,
        "pearson_r 1e-85 r=0.5",
    );
    let r_orth_tiny = pearson_r(&[-1e-85, 0.0, 1e-85, 0.0], &[0.0, -1e-85, 0.0, 1e-85]).expect("pearson_r 1e-85 orth");
    assert_eq!(r_orth_tiny, 0.0);
}

#[test]
fn fa_sec01_03_nct_small_df_huge_delta_and_huge_t_overflow_free() {
    let p_a = nct_cdf(1e10, 1.0, 2e9).expect("nct_cdf large delta 1");
    assert_rel(p_a, 0.841_480_581_121_793_9, 1e-10, "nct_cdf(1e10, 1, 2e9)");
    let sf_a = nct_sf(1e10, 1.0, 2e9).expect("nct_sf large delta 1");
    assert_rel(sf_a, 1.0 - 0.841_480_581_121_793_9, 1e-10, "nct_sf(1e10, 1, 2e9)");

    let p_b = nct_cdf(1.6e9, 1.0, 2e9).expect("nct_cdf large delta 2");
    assert_rel(p_b, 0.211_299_547_333_710_56, 1e-10, "nct_cdf(1.6e9, 1, 2e9)");

    let p_c = nct_cdf(1e9, 10.0, 2e9).expect("nct_cdf df=10 large delta");
    assert_rel(p_c, 1.694_474_393_006_738_5e-5, 1e-9, "nct_cdf(1e9, 10, 2e9)");

    let p_small_df = nct_cdf(1e6, 0.5, 1e6).expect("nct_cdf df=0.5");
    let expected_small = chi2_sf(0.5, 0.5).expect("chi2_sf");
    assert!((p_small_df - expected_small).abs() < 1e-7, "df=0.5 left tail lost mass: {p_small_df} vs {expected_small}");

    let p_ultra = nct_cdf(1e20, 1.0, 2e19).expect("nct_cdf ultra delta");
    assert_rel(p_ultra, 0.841_480_581_121_793_9, 1e-12, "nct_cdf ultra delta");

    assert_eq!(nct_cdf(1e160, 1e7, 1.0).expect("t=1e160 df=1e7"), 1.0);
    assert_eq!(nct_cdf(1e308, 1e7, 1.0).expect("t=1e308 df=1e7"), 1.0);
    assert_rel(nct_cdf(1e160, 1.0, 2e9).expect("t=1e160 df=1"), 1.0, 1e-12, "t=1e160 df=1");
    assert_eq!(nct_sf(1e160, 1e7, 1.0).expect("sf t=1e160"), 0.0);
}

#[test]
fn fa_sec01_04_ncf_and_ncchi2_log_space_far_tails_exact() {
    let f1 = ncf_cdf(1.0, 1.0, 100.0, 500.0).expect("ncf_cdf 1");
    assert_rel(f1, 4.761_815_340_689_547e-101, 1e-10, "ncf_cdf(1,1,100,500)");

    let f2 = ncf_cdf(0.05, 1.0, 10.0, 400.0).expect("ncf_cdf 2");
    assert_rel(f2, 3.559_413_380_012_406_3e-87, 1e-10, "ncf_cdf(0.05,1,10,400)");

    let c1 = ncchi2_cdf(0.5, 1.0, 500.0).expect("ncchi2_cdf");
    assert_rel(c1, 2.812_313_524_956_968e-104, 1e-10, "ncchi2_cdf(0.5,1,500)");

    let f_tiny_400 = ncf_cdf(1.0 / 99.0, 0.02, 0.02, 400.0).expect("tiny ncf 400");
    assert_rel(f_tiny_400, 7.098_863_789_449_53e-88, 1e-10, "tiny ncf 400");

    let f_tiny_1200 = ncf_cdf(1.0 / 99.0, 0.02, 0.02, 1200.0).expect("tiny ncf 1200");
    assert_rel(f_tiny_1200, 3.432_402_798_762_158e-261, 1e-10, "tiny ncf 1200");

    let c_sf = ncchi2_sf(1500.0, 1.0, 200.0).expect("ncchi2_sf far upper");
    assert_rel(c_sf, 8.551_681_058_710_768e-134, 1e-10, "ncchi2_sf(1500,1,200)");
}

#[test]
fn fa_sec01_06_dense_matrix_set_no_overflow_panic() {
    let mut m = DenseMatrix::zeros(2, 2).expect("zeros");
    let err = m.set(usize::MAX, 2, 1.0).expect_err("overflow row index must return DimensionMismatch");
    assert!(matches!(err, MathError::DimensionMismatch { expected: 4, actual: usize::MAX }));
    let half_plus = usize::MAX / 2 + 1;
    let err2 = m.set(half_plus, 2, 1.0).expect_err("wrapping row * ncols + col must return DimensionMismatch");
    assert!(matches!(err2, MathError::DimensionMismatch { expected: 4, actual: usize::MAX }));
}

#[test]
fn fa_sec01_07_chi_scale_tail_min_gap_scales_for_df_below_two() {
    let q = 5000.0;
    let df = 1.99;
    let sf_k2 = studentized_range_sf(q, 2, df).expect("tukey sf k=2");
    let expected_k2 = student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df).expect("t p2");
    assert_rel(sf_k2, expected_k2, 1e-12, "tukey sf k=2 df=1.99");

    let sf_k3 = studentized_range_sf(q, 3, df).expect("tukey sf k=3");
    assert_rel(sf_k3, 1.573_365_325_792e-07, 1e-5, "tukey sf k=3 df=1.99");

    let p_dunnett = dunnett_two_sided_p(q, df, 10.0, &[10.0, 10.0]).expect("dunnett df<2 large d");
    let single = student_t_two_sided_p(q, df).expect("single t");
    assert!(p_dunnett > single && p_dunnett < 2.0 * single, "p_dunnett={p_dunnett} outside ({single}, {})", 2.0 * single);

    let sf_extreme = studentized_range_sf(1e160, 3, 1.5).expect("extreme q finite");
    assert!(sf_extreme.is_finite() && (0.0..=1.0).contains(&sf_extreme));
}

#[test]
fn fa_sec01_08_svd_and_sym_eigen_rescale_extreme_entries() {
    let big_sym = DenseMatrix::from_row_major(2, 2, vec![3e200, 1e200, 1e200, 3e200]).expect("big sym");
    let eig_big = sym_eigen(&big_sym).expect("sym_eigen 1e200");
    assert_rel(eig_big.eigenvalues[0], 4e200, 1e-13, "eig big 0");
    assert_rel(eig_big.eigenvalues[1], 2e200, 1e-13, "eig big 1");

    let tiny_sym = DenseMatrix::from_row_major(2, 2, vec![3e-200, 1e-200, 1e-200, 3e-200]).expect("tiny sym");
    let eig_tiny = sym_eigen(&tiny_sym).expect("sym_eigen 1e-200");
    assert_rel(eig_tiny.eigenvalues[0], 4e-200, 1e-13, "eig tiny 0");
    assert_rel(eig_tiny.eigenvalues[1], 2e-200, 1e-13, "eig tiny 1");

    let skew_big = DenseMatrix::from_row_major(2, 2, vec![0.0, 1e200, -1e200, 0.0]).expect("skew big");
    let err = sym_eigen(&skew_big).expect_err("skew matrix must be rejected");
    assert!(matches!(err, MathError::DomainError { name: "symmetry", .. }), "got {err:?}");

    let a_big = DenseMatrix::from_row_major(3, 2, vec![3e200, 0.0, 0.0, 2e200, 0.0, 0.0]).expect("svd matrix 1e200");
    let s_big = svd(&a_big).expect("svd 1e200");
    assert_rel(s_big.singular_values[0], 3e200, 1e-13, "svd big 0");
    assert_rel(s_big.singular_values[1], 2e200, 1e-13, "svd big 1");
    assert_rel(s_big.u.get(0, 0).expect("u00").abs(), 1.0, 1e-13, "svd u00");

    let a_tiny = DenseMatrix::from_row_major(3, 2, vec![3e-200, 0.0, 0.0, 2e-200, 0.0, 0.0]).expect("svd matrix 1e-200");
    let s_tiny = svd(&a_tiny).expect("svd 1e-200");
    assert_rel(s_tiny.singular_values[0], 3e-200, 1e-13, "svd tiny 0");
    assert_rel(s_tiny.singular_values[1], 2e-200, 1e-13, "svd tiny 1");

    // Matrices whose unscaled `frobenius_norm()` genuinely overflows to +Inf:
    let big4 =
        DenseMatrix::from_row_major(4, 4, vec![9e307, 4e307, 0.0, 0.0, 4e307, 9e307, 0.0, 0.0, 0.0, 0.0, 9e307, 4e307, 0.0, 0.0, 4e307, 9e307])
            .expect("big4");
    assert!(big4.frobenius_norm().is_infinite(), "precondition: frobenius_norm must overflow +Inf");
    let eig4 = sym_eigen(&big4).expect("sym_eigen frobenius Inf");
    assert_rel(eig4.eigenvalues[0], 1.3e308, 1e-13, "eig4 0");
    assert_rel(eig4.eigenvalues[1], 1.3e308, 1e-13, "eig4 1");
    assert_rel(eig4.eigenvalues[2], 5e307, 1e-13, "eig4 2");
    assert_rel(eig4.eigenvalues[3], 5e307, 1e-13, "eig4 3");

    let svd4 = svd(&big4).expect("svd frobenius Inf");
    assert_rel(svd4.singular_values[0], 1.3e308, 1e-13, "svd4 0");
    assert_rel(svd4.singular_values[1], 1.3e308, 1e-13, "svd4 1");
    assert_rel(svd4.singular_values[2], 5e307, 1e-13, "svd4 2");
    assert_rel(svd4.singular_values[3], 5e307, 1e-13, "svd4 3");

    // 6x6 matrix with overflowing frobenius_norm() and large asymmetry must still be rejected cleanly.
    let mut skew6 = DenseMatrix::zeros(6, 6).expect("skew6 zeros");
    for i in 0..6 {
        skew6.set(i, i, 8e307).expect("diag");
    }
    skew6.set(0, 1, 2e307).expect("upper");
    skew6.set(1, 0, -2e307).expect("lower");
    assert!(skew6.frobenius_norm().is_infinite());
    assert!(matches!(sym_eigen(&skew6), Err(MathError::DomainError { name: "symmetry", .. })));
}

#[test]
fn fa_sec01_09_norms_qr_least_squares_and_parameter_deps_extreme_scaling() {
    let sub = DenseMatrix::from_row_major(1, 2, vec![3e-315, 4e-315]).expect("subnormal mat");
    let fnorm = sub.frobenius_norm();
    assert!(fnorm.is_finite() && fnorm > 0.0, "got fnorm={fnorm}");
    assert!((fnorm - 5e-315).abs() <= 1e-318);

    let sub_col = DenseMatrix::from_row_major(2, 1, vec![3e-310, 4e-310]).expect("subnormal col");
    let qr_res = qr_col_pivoted(&sub_col).expect("qr subnormal");
    assert_eq!(qr_res.rank(), 1);

    let big_col = DenseMatrix::from_row_major(2, 1, vec![1.5e308, 1.5e308]).expect("big col");
    let qr_big = qr_col_pivoted(&big_col).expect("qr big col");
    assert_eq!(qr_big.rank(), 1);
    let q_big = qr_big.thin_q().expect("thin_q big");
    assert_rel(q_big.get(0, 0).expect("q00").abs(), std::f64::consts::FRAC_1_SQRT_2, 1e-14, "thin_q big 00");
    assert_rel(q_big.get(1, 0).expect("q10").abs(), std::f64::consts::FRAC_1_SQRT_2, 1e-14, "thin_q big 10");

    // Subnormal 1e-310 * I_2 least squares:
    let sub_eye = DenseMatrix::from_row_major(2, 2, vec![1e-310, 0.0, 0.0, 1e-310]).expect("subnormal I2");
    let (sol_sub, _) = solve_linear_least_squares(&sub_eye, &[1e-310, 2e-310]).expect("lsq subnormal I2");
    assert_rel(sol_sub.coefficients[0], 1.0, 1e-12, "subnormal I2 x0");
    assert_rel(sol_sub.coefficients[1], 2.0, 1e-12, "subnormal I2 x1");

    // Overflowing column norm [1.5e308, 0.0; 1.5e308, 1.0]:
    let big_col_mat = DenseMatrix::from_row_major(2, 2, vec![1.5e308, 0.0, 1.5e308, 1.0]).expect("big col mat");
    let (sol_big, _) = solve_linear_least_squares(&big_col_mat, &[1.5e308, 1.5e308]).expect("lsq 1.5e308");
    assert_eq!(sol_big.rank, 2);
    assert_rel(sol_big.coefficients[0], 1.0, 1e-12, "1.5e308 col x0");
    assert!(sol_big.coefficients[1].abs() < 1e-6, "1.5e308 col x1={}", sol_big.coefficients[1]);
    let (sol_big2, _) = solve_linear_least_squares(&big_col_mat, &[0.0, 1.0]).expect("lsq 1.5e308 rhs=[0,1]");
    assert!(sol_big2.coefficients[0].abs() < 1e-300, "x0={}", sol_big2.coefficients[0]);
    assert_rel(sol_big2.coefficients[1], 1.0, 1e-12, "1.5e308 col2 x1");

    let design = DenseMatrix::from_row_major(3, 2, vec![1e-200, 1e200, 2e-200, -1e200, 1e-200, 3e200]).expect("design");
    let rhs = [2.0, 1.0, 4.0];
    let (sol, _cov) = solve_linear_least_squares(&design, &rhs).expect("lsq extreme column scaling");
    assert_eq!(sol.rank, 2);
    assert_rel(sol.coefficients[0], 1e200, 1e-12, "lsq coeff 0");
    assert_rel(sol.coefficients[1], 1e-200, 1e-12, "lsq coeff 1");
    assert!(sol.rss < 1e-25, "rss={}", sol.rss);

    let design_cov = DenseMatrix::from_row_major(3, 2, vec![1e-100, 1e100, 2e-100, -1e100, 1e-100, 3e100]).expect("design cov");
    let (sol_c, cov) = solve_linear_least_squares(&design_cov, &rhs).expect("lsq 1e100 cov");
    assert_rel(sol_c.coefficients[0], 1e100, 1e-12, "lsq 1e100 coeff 0");
    assert_rel(sol_c.coefficients[1], 1e-100, 1e-12, "lsq 1e-100 coeff 1");
    assert!(cov.get(0, 0).expect("cov00").is_finite() && cov.get(1, 1).expect("cov11").is_finite());

    let a_base = DenseMatrix::from_row_major(4, 2, vec![1.0, 1.0, 1.0, 2.0, 1.0, 3.0, 1.0, 4.0]).expect("base design");
    let a_scaled = DenseMatrix::from_row_major(4, 2, vec![1e-200, 1e200, 1e-200, 2e200, 1e-200, 3e200, 1e-200, 4e200]).expect("scaled design");
    let deps_base = parameter_dependencies(&a_base).expect("deps base");
    let deps_scaled = parameter_dependencies(&a_scaled).expect("deps scaled");
    assert_rel(deps_scaled[0], deps_base[0], 1e-12, "deps 0");
    assert_rel(deps_scaled[1], deps_base[1], 1e-12, "deps 1");
}

#[test]
fn fa_sec01_10_snedecor_f_df1_inf_ratio_split_and_cauchy_student_t_isf() {
    assert_rel(f_cdf(0.01, f64::INFINITY, 10.0).expect("f_cdf Inf"), chi2_sf(1000.0, 10.0).expect("chi2_sf"), 1e-12, "f_cdf df1=Inf");
    assert_rel(f_sf(0.01, f64::INFINITY, 10.0).expect("f_sf Inf"), chi2_cdf(1000.0, 10.0).expect("chi2_cdf"), 1e-12, "f_sf df1=Inf");
    assert_rel(f_ln_sf(10.0, f64::INFINITY, 10.0).expect("f_ln_sf Inf"), chi2_cdf(1.0, 10.0).expect("chi2_cdf").ln(), 1e-12, "f_ln_sf df1=Inf");
    assert_rel(f_ln_pdf(0.5, f64::INFINITY, 10.0).expect("f_ln_pdf Inf").exp(), 0.378_332_748_020_707_15, 1e-12, "f_ln_pdf df1=Inf");

    let q_f = 1e-20;
    let f_crit = f_isf(q_f, f64::INFINITY, 10.0).expect("f_isf df1=Inf");
    assert_rel(f_sf(f_crit, f64::INFINITY, 10.0).expect("roundtrip"), q_f, 1e-11, "f_isf df1=Inf roundtrip");
    let f_lo = f_ppf(1e-20, f64::INFINITY, 10.0).expect("f_ppf df1=Inf");
    assert_rel(f_cdf(f_lo, f64::INFINITY, 10.0).expect("roundtrip ppf"), 1e-20, 1e-11, "f_ppf df1=Inf roundtrip");

    assert_eq!(f_cdf(0.999, f64::INFINITY, f64::INFINITY).expect("Inf Inf cdf <1"), 0.0);
    assert_eq!(f_cdf(1.0, f64::INFINITY, f64::INFINITY).expect("Inf Inf cdf 1"), 1.0);
    assert_eq!(f_sf(1.0, f64::INFINITY, f64::INFINITY).expect("Inf Inf sf 1"), 0.0);
    assert_eq!(f_isf(0.25, f64::INFINITY, f64::INFINITY).expect("Inf Inf isf"), 1.0);
    assert_eq!(f_ppf(0.25, f64::INFINITY, f64::INFINITY).expect("Inf Inf ppf"), 1.0);

    let c_no_under = f_cdf(1e200, 1e-200, 1e200).expect("no ratio underflow");
    assert!(c_no_under > 0.99 && c_no_under <= 1.0, "got {c_no_under}");
    let s_no_over = f_sf(1e-200, 1e200, 1e-200).expect("no premature ratio=Inf");
    assert!(s_no_over > 0.99 && s_no_over <= 1.0, "got {s_no_over}");
    let sym_a = f_cdf(1e300, 2e-160, 2e160).expect("cdf imbalanced");
    let sym_b = f_sf(1e-300, 2e160, 2e-160).expect("sf imbalanced reciprocal");
    assert_rel(sym_a, sym_b, 1e-13, "F reciprocal symmetry");

    let t_cauchy_1 = student_t_isf(1e-200, 1.0).expect("cauchy 1-sided 1e-200");
    assert_rel(t_cauchy_1, std::f64::consts::FRAC_1_PI / 1e-200, 1e-14, "cauchy 1-sided isf");
    let t_cauchy_2 = student_t_two_sided_isf(1e-200, 1.0).expect("cauchy 2-sided 1e-200");
    assert_rel(t_cauchy_2, (2.0 * std::f64::consts::FRAC_1_PI) / 1e-200, 1e-14, "cauchy 2-sided isf");

    let t_half = student_t_two_sided_isf(1e-100, 0.5).expect("t df=0.5 alpha=1e-100");
    assert!(t_half.is_finite() && t_half > 1e190);
    assert_rel(student_t_two_sided_p(t_half, 0.5).expect("roundtrip p2"), 1e-100, 1e-10, "t df=0.5 roundtrip");
}

#[test]
fn fa_sec01_11_reg_beta_huge_shapes_avoid_ab_overflow() {
    assert_eq!(reg_beta(0.5, 1e155, 1e155).expect("reg_beta 1e155 mid"), 0.5);
    assert_eq!(reg_beta(0.5, 1e308, 1e308).expect("reg_beta 1e308 mid"), 0.5);
    assert_eq!(reg_beta(0.5 + 1e-15, 1e155, 1e155).expect("reg_beta 1e155 upper tail"), 1.0);

    // Measure z from the actual binary64 value `x` so binary64 rounding of 0.5 + 2*sd is exact.
    let sd = (0.125_f64 / 1e20_f64).sqrt();
    let x_near = 0.5 + 2.0 * sd;
    let val = reg_beta(x_near, 1e20, 1e20).expect("reg_beta 1e20 +2sigma");
    let expected = curvebook_math::dist::core::norm_cdf((x_near - 0.5) / sd);
    assert_rel(val, expected, 1e-10, "reg_beta 1e20 +2sigma");

    let ln_v = ln_reg_beta_xy(0.5, 0.5, 1e155, 1e155).expect("ln_reg_beta_xy 1e155");
    assert_rel(ln_v, -std::f64::consts::LN_2, 1e-14, "ln_reg_beta_xy 1e155");
}

#[test]
fn fa_sec01_12_gamma_deviance_avoids_a_plus_x_overflow() {
    assert_eq!(reg_gamma_p(1e308, 1.2e308).expect("reg_gamma_p(1e308, 1.2e308)"), 1.0);
    assert_eq!(reg_gamma_q(1e308, 1.2e308).expect("reg_gamma_q(1e308, 1.2e308)"), 0.0);
    assert_eq!(reg_gamma_p(1e308, 0.8e308).expect("reg_gamma_p(1e308, 0.8e308)"), 0.0);
    assert_eq!(reg_gamma_q(1e308, 0.8e308).expect("reg_gamma_q(1e308, 0.8e308)"), 1.0);

    // Measure z from actual binary64 `x_near` (since ulp(1e20) = 16384 shifts `1e20 + 1e10` by ~7e-7).
    let a = 1e20_f64;
    let x_near = a + a.sqrt();
    let p_1sig = reg_gamma_p(a, x_near).expect("1e20 + 1e10");
    assert_rel(p_1sig, curvebook_math::dist::core::norm_cdf((x_near - a) / a.sqrt()), 1e-10, "reg_gamma_p 1e20 +1sigma");
}

#[test]
fn fa_sec01_13_subnormal_erfc_inv_and_norm_two_sided_isf_finite() {
    let z_erfc = erfc_inv(5e-324).expect("erfc_inv subnormal min");
    assert!(z_erfc.is_finite(), "erfc_inv(5e-324) returned {z_erfc}");
    assert_rel(z_erfc, 27.213_293_210_812_946, 1e-13, "erfc_inv(5e-324)");

    let z_norm2 = norm_two_sided_isf(5e-324).expect("norm_two_sided_isf subnormal min");
    assert!(z_norm2.is_finite(), "norm_two_sided_isf(5e-324) returned {z_norm2}");
    assert_rel(z_norm2, 38.485_408_335_567_34, 1e-13, "norm_two_sided_isf(5e-324)");
}

#[test]
fn fa_sec01_14_quad_honors_exact_tolerance_and_extreme_midpoints() {
    let exact = (1.0 - 15.0_f64.cos()) / 15.0;
    let opts = QuadOptions { abs_tol: 1e-12, rel_tol: 1e-12, max_subdivisions: 512 };
    let val = integrate_adaptive(|x| (15.0 * x).sin(), 0.0, 1.0, opts).expect("integrate_adaptive");
    assert!((val - exact).abs() <= 2e-12, "error {} exceeded requested 1e-12 tol", (val - exact).abs());

    let z_int = integrate_adaptive(|_| 0.0, -1e308, 1e308, QuadOptions::default()).expect("extreme bracket quad");
    assert_eq!(z_int, 0.0);

    let root = brent_root(|x| x, -1e308, 1e308, RootOptions::default()).expect("extreme bracket brent_root");
    assert_eq!(root, 0.0);

    // Exhausting max_subdivisions with total_err = 4 * tol (inside old 16x slack) must fail NotConverged.
    let (_est_l, err_l) = gauss_kronrod_15_panel(&mut |x: f64| (x - 0.3).abs(), 0.0, 0.5);
    assert!(err_l > 0.0);
    let err = integrate_adaptive(|x: f64| (x - 0.3).abs(), 0.0, 1.0, QuadOptions { abs_tol: 0.25 * err_l, rel_tol: 0.0, max_subdivisions: 1 })
        .expect_err("total_err = 4 * tol must fail without 16x relaxation");
    assert!(matches!(err, MathError::NotConverged { algorithm: "integrate_adaptive", iterations: 1 }));
}

#[test]
fn fa_sec01_15_nct_opposite_tail_cancellation_free() {
    let sf = nct_sf(2.0, 10.0, -10.0).expect("nct_sf(2, 10, -10)");
    assert_rel(sf, 1.541_674_555_748_44e-29, 1e-12, "nct_sf(2, 10, -10)");

    let cdf_sym = nct_cdf(-2.0, 10.0, 10.0).expect("nct_cdf(-2, 10, 10)");
    assert_rel(cdf_sym, 1.541_674_555_748_44e-29, 1e-12, "nct_cdf(-2, 10, 10)");

    let sf_deep = nct_sf(2.0, 10.0, -30.0).expect("nct_sf(2, 10, -30)");
    assert_rel(sf_deep, 6.181_377_143_351_286e-208, 1e-9, "nct_sf(2, 10, -30)");
}

#[test]
fn fa_sec01_16_noncentral_ci_validates_df_even_when_obs_zero() {
    assert!(matches!(noncentral_f_ci(0.0, -1.0, 5.0, 95.0), Err(MathError::InvalidParameter { name: "df1", .. })));
    assert!(matches!(noncentral_f_ci(0.0, 5.0, f64::NAN, 95.0), Err(MathError::InvalidParameter { name: "df2", .. })));
    assert!(matches!(noncentral_f_ci(0.0, f64::INFINITY, 5.0, 95.0), Err(MathError::InvalidParameter { name: "df1", .. })));

    assert!(matches!(noncentral_chi2_ci(0.0, 0.0, 95.0), Err(MathError::InvalidParameter { name: "df", .. })));
    assert!(matches!(noncentral_chi2_ci(0.0, f64::INFINITY, 95.0), Err(MathError::InvalidParameter { name: "df", .. })));
    assert!(matches!(noncentral_chi2_ci(0.0, f64::NAN, 95.0), Err(MathError::InvalidParameter { name: "df", .. })));
}

#[test]
fn fa_sec01_17_studentized_range_large_k_and_dunnett_imbalanced_sizes() {
    let sf = studentized_range_sf(5.0, 70_000, 10.0).expect("large k sf");
    assert!(sf.is_finite() && sf > 0.0 && sf < 1.0);
    let ln_sf = studentized_range_ln_sf(40.0, 70_000, 10.0).expect("large k ln_sf");
    assert!(ln_sf.is_finite() && ln_sf < 0.0);
    let cdf_inf_tail = studentized_range_cdf(5.0, 70_000, f64::INFINITY).expect("large k gaussian cdf tail");
    assert!(cdf_inf_tail.is_finite() && cdf_inf_tail == 0.0);
    let cdf_inf_mid = studentized_range_cdf(9.5, 70_000, f64::INFINITY).expect("large k gaussian cdf mid");
    assert!(cdf_inf_mid.is_finite() && cdf_inf_mid > 0.0 && cdf_inf_mid < 1.0, "got {cdf_inf_mid}");
    let crit = max_studentized_range_isf(0.05, 1, 70_000, 10.0).expect("large c isf");
    assert!(crit.is_finite() && crit > 5.0);

    let lam = dunnett_loadings_from_sizes(1.0, &[1e20]).expect("loadings 1e20");
    assert_eq!(lam, vec![1.0]);
    let big_lam = dunnett_loadings_from_sizes(1e308, &[1e308]).expect("loadings 1e308");
    assert_rel(big_lam[0], std::f64::consts::FRAC_1_SQRT_2, 1e-14, "dunnett loadings 1e308");

    let p_imbal_20 = dunnett_two_sided_p(2.5, 10.0, 1.0, &[1e20, 1e20]).expect("dunnett 1e20");
    let expected_t = student_t_two_sided_p(2.5, 10.0).expect("single t");
    // For n_i = 1e20, sqrt(1 - lam^2) = 1e-10 produces an exact O(1e-10) lift above single-t.
    assert!(p_imbal_20 > expected_t && p_imbal_20 - expected_t < 1e-10);
    let p_imbal_30 = dunnett_two_sided_p(2.5, 10.0, 1.0, &[1e30, 1e30]).expect("dunnett 1e30");
    assert_rel(p_imbal_30, expected_t, 1e-12, "dunnett 1e30 p");
    assert_rel(dunnett_two_sided_p_loadings(2.5, 10.0, &[1.0, 1.0]).expect("loadings=1.0"), expected_t, 1e-12, "dunnett lam=1 p");
    let d_crit = dunnett_two_sided_crit(0.05, 10.0, 1.0, &[1e30, 1e30]).expect("crit 1e30");
    assert_rel(d_crit, student_t_two_sided_isf(0.05, 10.0).expect("t crit"), 1e-10, "dunnett 1e30 crit");
}

#[derive(Debug, Clone)]
enum MemVal {
    U64(u64),
    F64(f64),
    Opt(Option<Box<MemVal>>),
    Tup(Vec<MemVal>),
    Map(Vec<(String, MemVal)>),
}

#[derive(Debug)]
struct MemErr(String);

impl fmt::Display for MemErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MemErr {}
impl ser::Error for MemErr {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}
impl de::Error for MemErr {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

struct MemSerializer;
struct MapSer(Vec<(String, MemVal)>);
struct TupleSer(Vec<MemVal>);

impl SerializeMap for MapSer {
    type Ok = MemVal;
    type Error = MemErr;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), Self::Error> {
        Err(ser::Error::custom("unused"))
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), Self::Error> {
        Err(ser::Error::custom("unused"))
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(MemVal::Map(self.0))
    }
}

impl ser::SerializeStruct for MapSer {
    type Ok = MemVal;
    type Error = MemErr;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
        self.0.push((key.to_owned(), value.serialize(MemSerializer)?));
        Ok(())
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(MemVal::Map(self.0))
    }
}

impl SerializeTuple for TupleSer {
    type Ok = MemVal;
    type Error = MemErr;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.0.push(value.serialize(MemSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(MemVal::Tup(self.0))
    }
}

impl ser::Serializer for MemSerializer {
    type Ok = MemVal;
    type Error = MemErr;
    type SerializeSeq = ser::Impossible<MemVal, MemErr>;
    type SerializeTuple = TupleSer;
    type SerializeTupleStruct = ser::Impossible<MemVal, MemErr>;
    type SerializeTupleVariant = ser::Impossible<MemVal, MemErr>;
    type SerializeMap = MapSer;
    type SerializeStruct = MapSer;
    type SerializeStructVariant = ser::Impossible<MemVal, MemErr>;

    fn serialize_bool(self, _: bool) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_i8(self, _: i8) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_i16(self, _: i16) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_i32(self, _: i32) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_i64(self, _: i64) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_u8(self, _: u8) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_u16(self, _: u16) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_u32(self, _: u32) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_u64(self, v: u64) -> Result<MemVal, MemErr> {
        Ok(MemVal::U64(v))
    }
    fn serialize_f32(self, _: f32) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_f64(self, v: f64) -> Result<MemVal, MemErr> {
        Ok(MemVal::F64(v))
    }
    fn serialize_char(self, _: char) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_str(self, _: &str) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_none(self) -> Result<MemVal, MemErr> {
        Ok(MemVal::Opt(None))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<MemVal, MemErr> {
        Ok(MemVal::Opt(Some(Box::new(value.serialize(self)?))))
    }
    fn serialize_unit(self) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, _: &T) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(self, _: &'static str, _: u32, _: &'static str, _: &T) -> Result<MemVal, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, MemErr> {
        Ok(TupleSer(Vec::with_capacity(len)))
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeTupleVariant, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> Result<Self::SerializeStruct, MemErr> {
        Ok(MapSer(Vec::with_capacity(len)))
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeStructVariant, MemErr> {
        Err(ser::Error::custom("unsupported"))
    }
}

struct MemDe(MemVal);
struct MapDe(std::vec::IntoIter<(String, MemVal)>, Option<MemVal>);
struct SeqDe(std::vec::IntoIter<MemVal>);
struct KeyDe(String);

impl<'de> MapAccess<'de> for MapDe {
    type Error = MemErr;
    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        match self.0.next() {
            Some((k, v)) => {
                self.1 = Some(v);
                seed.deserialize(KeyDe(k)).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        let val = self.1.take().ok_or_else(|| de::Error::custom("missing value"))?;
        seed.deserialize(MemDe(val))
    }
}

impl<'de> SeqAccess<'de> for SeqDe {
    type Error = MemErr;
    fn next_element_seed<T: de::DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error> {
        match self.0.next() {
            Some(v) => seed.deserialize(MemDe(v)).map(Some),
            None => Ok(None),
        }
    }
}

impl<'de> Deserializer<'de> for KeyDe {
    type Error = MemErr;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_string(self.0)
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf option unit unit_struct
        newtype_struct seq tuple tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de> Deserializer<'de> for MemDe {
    type Error = MemErr;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            MemVal::U64(v) => visitor.visit_u64(v),
            MemVal::F64(v) => visitor.visit_f64(v),
            MemVal::Opt(Some(v)) => visitor.visit_some(MemDe(*v)),
            MemVal::Opt(None) => visitor.visit_none(),
            MemVal::Tup(v) => visitor.visit_seq(SeqDe(v.into_iter())),
            MemVal::Map(m) => visitor.visit_map(MapDe(m.into_iter(), None)),
        }
    }
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            MemVal::Opt(None) => visitor.visit_none(),
            MemVal::Opt(Some(v)) => visitor.visit_some(MemDe(*v)),
            other => visitor.visit_some(MemDe(other)),
        }
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf unit unit_struct
        newtype_struct seq tuple tuple_struct map struct enum identifier ignored_any
    }
}

#[test]
fn fa_sec01_18_summary_moments_scaled_css_serde_roundtrip() {
    let m = SummaryMoments::from_slice(&[-1e200, 1e200]).expect("moments 1e200");
    assert!(m.css.is_infinite(), "css overflows to +Inf for +-1e200");
    let expected_sd = 2.0_f64.sqrt() * 1e200;
    assert_rel(m.sample_sd().expect("finite sd"), expected_sd, 1e-14, "original finite sd");

    let encoded = m.serialize(MemSerializer).expect("serialize SummaryMoments");
    let decoded = SummaryMoments::deserialize(MemDe(encoded.clone())).expect("deserialize SummaryMoments");
    assert_rel(decoded.sample_sd().expect("roundtripped finite sd"), expected_sd, 1e-14, "roundtripped finite sd");

    let legacy = match encoded {
        MemVal::Map(fields) => MemVal::Map(fields.into_iter().filter(|(k, _)| k != "scaled_css").collect()),
        other => other,
    };
    let legacy_decoded = SummaryMoments::deserialize(MemDe(legacy)).expect("legacy deserialize");
    assert_eq!(legacy_decoded.count, 2);
}

#[test]
fn fa_sec01_20_apa_pvalue_format_upper_ceiling() {
    assert_eq!(PValueFormat::Apa.format_p(0.999_4), "> .999");
    assert_eq!(PValueFormat::Apa.format_p(0.999_999_999_9), "> .999");
    assert_eq!(PValueFormat::Apa.format_p(1.0), "1.000");
    assert_eq!(PValueFormat::Apa.format_p(0.998_9), ".999");
}

#[test]
fn fa_sec01_21_axis_scale_logit_percent_subnormals_and_strict_inverse() {
    let logit_pct = AxisScale::Logit(LogitDomain::Percent);
    let t_min = logit_pct.transform(5e-324).expect("subnormal 5e-324 transform must succeed");
    assert!(t_min.is_finite());
    assert_rel(t_min, -749.045_242_107_369_3, 1e-12, "logit_pct subnormal");

    assert!(AxisScale::Log10.inverse(-400.0).is_none());
    assert!(AxisScale::Ln.inverse(-1000.0).is_none());
    assert!(AxisScale::Log2.inverse(-2000.0).is_none());
    assert!(AxisScale::Logit(LogitDomain::Proportion).inverse(-1000.0).is_none());
    assert!(AxisScale::Logit(LogitDomain::Proportion).inverse(100.0).is_none());
    assert!(logit_pct.inverse(-1000.0).is_none());
    assert!(logit_pct.inverse(100.0).is_none());
    assert!(AxisScale::Probit.inverse(-50.0).is_none());
    assert!(AxisScale::Probit.inverse(50.0).is_none());
    assert!(AxisScale::Reciprocal.inverse(1e-320).is_none());

    let scales = [
        AxisScale::Linear,
        AxisScale::Log10,
        AxisScale::Log2,
        AxisScale::Ln,
        AxisScale::Logit(LogitDomain::Percent),
        AxisScale::Logit(LogitDomain::Proportion),
        AxisScale::Probit,
        AxisScale::Reciprocal,
    ];
    for scale in scales {
        for &t in &[-1000.0, -500.0, -100.0, -30.0, -1.0, 0.0, 1.0, 30.0, 100.0, 500.0, 1000.0] {
            if let Some(u) = scale.inverse(t) {
                assert!(scale.transform(u).is_some(), "{scale:?}.inverse({t}) = Some({u}) failed forward transform!");
            }
        }
    }
}

#[test]
fn fa_sec01_24_gamma_reflection_large_negative_half_integers() {
    let g1 = gamma(-173.5).expect("gamma(-173.5)");
    assert_rel(g1, 6.454_083_59e-315, 1e-8, "gamma(-173.5)");
    let g2 = gamma(-172.5).expect("gamma(-172.5)");
    assert_rel(g2, -1.119_783_503_29e-312, 1e-10, "gamma(-172.5)");
}
