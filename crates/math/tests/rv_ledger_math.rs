//! Regression test suite for the 2026-11-16 math review ledger (`RV-43`, `RV-44`, `RV-46`,
//! `RV-47`, `RV-48`, `RV-49`, `RV-70`, `RV-73`, and Section 3 S4 special-function accuracy notes).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::dist::{
    chi2_cdf, chi2_ln_sf, chi2_sf, f_ln_pdf, f_ln_sf, f_pdf, lilliefors_d_from_sorted, norm_isf, norm_ln_sf, norm_sf, studentized_range_ln_sf,
    studentized_range_sf,
};
use curvebook_math::special::{erfcx, gamma, ln_beta, ln_choose, ln_factorial, ln_reg_beta_xy, reg_beta, reg_beta_inv, reg_beta_inv_pair};
use curvebook_math::sum::{SummaryMoments, covariance, lag1_autocorrelation, pearson_r, sample_sd};
use curvebook_math::{DenseMatrix, MathError, qr_col_pivoted, solve_linear_least_squares, svd};

#[test]
fn rv_43_ln_beta_both_large_stirling_branch_and_f_distribution_densities() {
    // Exact factorial identity: ln B(20, 20) = 2 * ln(19!) - ln(39!) = -27.95199188624447105...
    let lb_20 = ln_beta(20.0, 20.0).unwrap();
    let exact_20 = 2.0 * ln_factorial(19) - ln_factorial(39);
    assert!((lb_20 - (-27.951_991_886_244_47)).abs() < 1e-13, "lb_20 = {lb_20}");
    assert!((lb_20 - exact_20).abs() < 1e-13, "lb_20={lb_20} vs exact_20={exact_20}");

    // Continuity across the STIRLING_CUTOFF = 12.0 seam and exact integer identity ln B(12, 15).
    let lb_12_15 = ln_beta(12.0, 15.0).unwrap();
    let exact_12_15 = ln_factorial(11) + ln_factorial(14) - ln_factorial(26);
    assert!((lb_12_15 - exact_12_15).abs() < 1e-13, "lb_12_15={lb_12_15} vs {exact_12_15}");

    let seam_below = ln_beta(11.999_999, 20.0).unwrap();
    let seam_above = ln_beta(12.000_001, 20.0).unwrap();
    assert!((seam_below - seam_above).abs() < 1e-5, "seam jump: {seam_below} vs {seam_above}");

    // Snedecor F density and log-density for df1, df2 >= 24 (a = 12, b = 15):
    // Exact 50-digit value at f = 1.5, df1 = 24, df2 = 30:
    // ln f(1.5; 24, 30) = -0.93778242402656879750..., f(1.5; 24, 30) = 0.39149504345530031816...
    let lpdf = f_ln_pdf(1.5, 24.0, 30.0).unwrap();
    let pdf = f_pdf(1.5, 24.0, 30.0).unwrap();
    assert!((lpdf - (-0.937_782_424_026_568_8)).abs() < 1e-13, "f_ln_pdf = {lpdf}");
    assert!((pdf - 0.391_495_043_455_300_3).abs() < 1e-14, "f_pdf = {pdf}");

    // Extreme-tail f_ln_sf where 1 / ratio underflows to 0.0 (f = 1e200, df1 = 24, df2 = 30):
    // Asymptotic tail: -15 * ln((24/30) * 1e200) - ln(15) - ln B(15, 12) = -6888.54800318113668...
    let lsf_ext = f_ln_sf(1e200, 24.0, 30.0).unwrap();
    assert!((lsf_ext - (-6_888.548_003_181_137)).abs() < 1e-10, "f_ln_sf = {lsf_ext}");

    // reg_beta_inv and reg_beta_inv_pair roundtrip when both a, b >= 12 across body and deep tail
    // (p < 1e-12 exercises the asymptotic ln_beta(a, b) starting guess in initial_beta_w_guess!).
    for &(a, b, p) in &[(20.0, 20.0, 0.25), (20.0, 20.0, 1e-20), (30.0, 45.0, 1e-35), (15.0, 80.0, 1e-15)] {
        let x = reg_beta_inv(a, b, p).unwrap();
        if p >= 1e-12 {
            let p_back = reg_beta(x, a, b).unwrap();
            assert!((p_back - p).abs() <= 1e-13 * p, "a={a}, b={b}, p={p}: p_back={p_back}");
        } else {
            let (xp, yp) = reg_beta_inv_pair(a, b, p, 1.0 - p).unwrap();
            assert!((xp - x).abs() <= 1e-14 * x);
            let ln_back = ln_reg_beta_xy(xp, yp, a, b).unwrap();
            assert!((ln_back - p.ln()).abs() < 1e-12, "a={a}, b={b}, p={p}: ln_back={ln_back} vs {}", p.ln());
        }
    }
}

#[test]
fn rv_44_norm_ln_sf_negative_argument_preserves_relative_accuracy() {
    // At z = -10.0, P(Z > -10) = 1 - P(Z > 10), so ln P(Z > -10) = ln1p(-norm_sf(10))
    // = -7.619853024160593e-24 (must NOT collapse to 0.0!).
    let lsf_neg10 = norm_ln_sf(-10.0);
    let expected = -7.619_853_024_160_527e-24_f64;
    assert!(lsf_neg10 < 0.0, "norm_ln_sf(-10.0) collapsed to {lsf_neg10}");
    assert!((lsf_neg10 - expected).abs() <= 1e-14 * expected.abs(), "got {lsf_neg10:.16e} vs {expected:.16e}");

    // Check smooth relative accuracy across moderate and deep negative z down to subnormal tail.
    for &z in &[-0.5_f64, -2.0, -5.0, -8.5, -15.0, -25.0, -35.0] {
        let s = norm_sf(-z);
        let want = (-s).ln_1p();
        let got = norm_ln_sf(z);
        assert!((got - want).abs() <= 2.0 * f64::EPSILON * want.abs(), "z={z}: got={got:.16e} vs want={want:.16e}");
    }
    assert_eq!(norm_ln_sf(f64::NEG_INFINITY), 0.0);
}

#[test]
fn rv_46_ln_choose_huge_n_avoids_catastrophic_cancellation() {
    // Exact log-product oracle: ln C(n, k) = k * ln(n) - ln(k!) + sum_{i=1}^{k-1} ln1p(-i / n).
    let oracle_ln_choose = |n: u64, k: u64| -> f64 {
        let nf = n as f64;
        let sum_corr: f64 = (1..k).map(|i| (-(i as f64) / nf).ln_1p()).sum();
        (k as f64) * nf.ln() - ln_factorial(k) + sum_corr
    };

    let lc_1e12 = ln_choose(1_000_000_000_000, 40).unwrap();
    let want_1e12 = oracle_ln_choose(1_000_000_000_000, 40);
    assert!((lc_1e12 - 994.920_204_921_604_5).abs() < 1e-11, "lc_1e12 = {lc_1e12}");
    assert!((lc_1e12 - want_1e12).abs() < 1e-11, "lc_1e12={lc_1e12} vs {want_1e12}");

    let lc_1e17 = ln_choose(100_000_000_000_000_000, 40).unwrap();
    let want_1e17 = oracle_ln_choose(100_000_000_000_000_000, 40);
    assert!((lc_1e17 - 1_455.437_223_521_193_7).abs() < 1e-11, "lc_1e17 = {lc_1e17}");
    assert!((lc_1e17 - want_1e17).abs() < 1e-11, "lc_1e17={lc_1e17} vs {want_1e17}");

    // Central binomial identity for n >= 171, k >= 32: ln C(200, 100) = ln(200!) - 2 * ln(100!).
    let lc_200_100 = ln_choose(200, 100).unwrap();
    let exact_200_100 = ln_factorial(200) - 2.0 * ln_factorial(100);
    assert!((lc_200_100 - exact_200_100).abs() < 1e-13, "lc_200_100={lc_200_100} vs {exact_200_100}");
}

#[test]
fn rv_47_chi2_and_inc_gamma_huge_df_outside_six_sigma_converge_accurately() {
    // At df = 1e7 (a = 5e6), SD(chi2) = sqrt(2 * df). Previously points outside ±6 * sqrt(a)
    // (such as df - 7 * sqrt(2 * df)) exceeded MAX_GAMMA_ITERS = 8192 and returned NotConverged.
    let df = 1e7_f64;
    let sd = (2.0 * df).sqrt();
    let x_lo = df - 7.0 * sd;
    let x_hi = df + 7.0 * sd;

    let cdf_lo = chi2_cdf(x_lo, df).unwrap();
    let sf_lo = chi2_sf(x_lo, df).unwrap();
    assert!((cdf_lo - 1.215_944_492_442_266e-12).abs() <= 1e-12 * cdf_lo, "cdf_lo = {cdf_lo:.16e}");
    assert!(((cdf_lo + sf_lo) - 1.0).abs() < 1e-15);

    let cdf_hi = chi2_cdf(x_hi, df).unwrap();
    let sf_hi = chi2_sf(x_hi, df).unwrap();
    let lsf_hi = chi2_ln_sf(x_hi, df).unwrap();
    assert!((sf_hi - 1.346_711_876_127_65e-12).abs() <= 1e-12 * sf_hi, "sf_hi = {sf_hi:.16e}");
    assert!((lsf_hi - sf_hi.ln()).abs() < 1e-13);
    assert!(((cdf_hi + sf_hi) - 1.0).abs() < 1e-15);

    // Also sweep wider offsets (±10, ±30, ±200 SD, and across the 0.25*a Temme boundary) at df = 1e7 and 1e9.
    for &big_df in &[1e7_f64, 1e9_f64] {
        let s = (2.0 * big_df).sqrt();
        for &k_sd in &[-30.0_f64, -10.0, -6.1, 6.1, 10.0, 30.0] {
            let x = big_df + k_sd * s;
            let c = chi2_cdf(x, big_df).unwrap();
            let q = chi2_sf(x, big_df).unwrap();
            assert!(((c + q) - 1.0).abs() < 1e-14, "df={big_df}, k_sd={k_sd}: c={c}, q={q}");
        }
        // Seam continuity across |x - a| = 0.25 * a (chi2 = 0.75 * df and 1.25 * df):
        let lsf_in = chi2_ln_sf(1.249_999 * big_df, big_df).unwrap();
        let lsf_out = chi2_ln_sf(1.250_001 * big_df, big_df).unwrap();
        assert!(lsf_in > lsf_out, "non-monotone across Temme upper seam at df={big_df}: {lsf_in} vs {lsf_out}");
    }
}

#[test]
fn rv_48_rank_deficient_least_squares_agrees_with_svd_rank_and_aliased_columns() {
    // Construct a 6 x 4 matrix where column 3 is zero (forcing the deficient SVD branch) AND
    // column 2 has singular value strictly between pqr.default_tol (eps * 6 * 1.0) and
    // svd.default_tol (eps * 6 * sigma_0) so naive QR rank (3) would disagree with SVD rank (2)!
    let c = (0.5_f64).sqrt();
    let delta = 1.2 * f64::EPSILON * 6.0; // > eps * 6 * 1.0, but < eps * 6 * sqrt(2) after SVD!
    // Columns 0 and 1 are nearly identical unit vectors in span(e_0, e_1) so sigma_0 ~ sqrt(2),
    // column 2 has small orthogonal component `delta` along e_2 so after scaling its pivot is ~1
    // while columns 0, 1, 2 span a 3D subspace with sigma_2 ~ delta < svd.default_tol!
    let x = DenseMatrix::from_row_major(
        6,
        4,
        vec![
            1.0,
            1.0,
            1.0,
            0.0, //
            0.0,
            delta,
            0.0,
            0.0, //
            0.0,
            0.0,
            c * delta,
            0.0, //
            0.0,
            0.0,
            c * delta,
            0.0, //
            0.0,
            0.0,
            0.0,
            0.0, //
            0.0,
            0.0,
            0.0,
            0.0,
        ],
    )
    .unwrap();
    let y = [2.0, 0.0, 0.0, 0.0, 1.0, -1.0];

    // Equilibrate identically to inspect QR rank vs SVD rank:
    let mut x_scaled = DenseMatrix::zeros(6, 4).unwrap();
    for j in 0..4 {
        let nrm = x.col_norm2(j);
        if nrm > 0.0 {
            for i in 0..6 {
                x_scaled.set(i, j, x.get(i, j).unwrap() / nrm).unwrap();
            }
        }
    }
    let pqr = qr_col_pivoted(&x_scaled).unwrap();
    let svd_s = svd(&x_scaled).unwrap();
    let expected_svd_rank = svd_s.rank_with_tol(svd_s.default_tol().max(pqr.default_tol()));

    let (sol, cov) = solve_linear_least_squares(&x, &y).unwrap();
    assert_eq!(sol.rank, expected_svd_rank, "least-squares rank must match SVD truncation rank");
    assert_eq!(sol.df_residual, 6 - sol.rank);
    assert_eq!(sol.aliased_columns.len(), 4 - sol.rank);
    for r in 0..4 {
        for col in 0..4 {
            assert!(cov.get(r, col).unwrap().is_finite());
        }
    }
}

#[test]
fn rv_49_lilliefors_d_from_sorted_rejects_unsorted_and_non_finite() {
    let sorted = [-1.2, -0.4, 0.1, 0.8, 1.7, 2.3];
    let d_ok = lilliefors_d_from_sorted(&sorted).unwrap();
    assert!(d_ok > 0.0 && d_ok < 1.0);

    let unsorted = [-1.2, 0.8, 0.1, -0.4, 1.7, 2.3];
    let err = lilliefors_d_from_sorted(&unsorted).unwrap_err();
    assert!(matches!(err, MathError::DomainError { name: "sorted_values", .. }), "got {err:?}");

    assert!(matches!(lilliefors_d_from_sorted(&[-1.0, 0.0, f64::NAN, 1.0, 2.0]), Err(MathError::DomainError { .. })));
}

#[test]
fn rv_70_studentized_range_ln_sf_deep_tail_accuracy() {
    // Deep-tail check at (q = 60, k = 50, df = 5) and underflowing regimes:
    let lsf = studentized_range_ln_sf(60.0, 50, 5.0).unwrap();
    let sf = studentized_range_sf(60.0, 50, 5.0).unwrap();
    assert!((lsf - sf.ln()).abs() < 1e-12);
    assert!((lsf - (-11.668_195)).abs() < 1e-4, "lsf(60, 50, 5) = {lsf}");

    // Underflowing finite-df tail (q = 1000, k = 50, df = 200 -> sf = 0.0):
    assert_eq!(studentized_range_sf(1000.0, 50, 200.0).unwrap(), 0.0);
    let lsf_deep = studentized_range_ln_sf(1000.0, 50, 200.0).unwrap();
    assert!(lsf_deep.is_finite() && lsf_deep < -700.0, "lsf_deep = {lsf_deep}");

    // Small fractional df < 2 underflow (q = 1e300, k = 10, df = 1.2):
    assert_eq!(studentized_range_sf(1e300, 10, 1.2).unwrap(), 0.0);
    let lsf_small_df = studentized_range_ln_sf(1e300, 10, 1.2).unwrap();
    assert!(lsf_small_df.is_finite() && lsf_small_df < -800.0, "lsf_small_df = {lsf_small_df}");
}

#[test]
fn rv_73_summary_moments_and_correlations_near_1e160_stay_finite_and_compensated() {
    // Wide observations at 1e160 where (x - mean)^2 = 1e320 overflows f64::MAX:
    let wide = [1e160_f64, 2e160, 3e160, 4e160];
    let m_wide = SummaryMoments::from_slice(&wide).unwrap();
    assert!((m_wide.mean - 2.5e160).abs() <= 2.0 * f64::EPSILON * 2.5e160);
    let expected_sd = (5.0_f64 / 3.0).sqrt() * 1e160;
    let sd_wide = m_wide.sample_sd().unwrap();
    let sem_wide = m_wide.sem().unwrap();
    assert!(sd_wide.is_finite() && (sd_wide - expected_sd).abs() <= 4.0 * f64::EPSILON * expected_sd, "sd_wide = {sd_wide:.16e}");
    assert!(sem_wide.is_finite() && (sem_wide - 0.5 * expected_sd).abs() <= 4.0 * f64::EPSILON * expected_sd, "sem_wide = {sem_wide:.16e}");
    assert!((m_wide.population_sd() - (1.25_f64).sqrt() * 1e160).abs() <= 4.0 * f64::EPSILON * 1e160);

    // Pearson r and lag-1 autocorrelation must NOT produce NaN from inf / inf!
    let y_wide = [2e160_f64, 4e160, 6e160, 8e160];
    let r = pearson_r(&wide, &y_wide).unwrap();
    assert!((r - 1.0).abs() < 1e-15, "pearson_r at 1e160 = {r}");
    let r1 = lag1_autocorrelation(&wide).unwrap();
    // For [-1.5, -0.5, 0.5, 1.5], sum_{0..2} d_i d_{i+1} = 0.75 - 0.25 + 0.75 = 1.25, css = 5.0 -> r1 = 0.25!
    assert!((r1 - 0.25).abs() < 1e-15, "lag1_autocorrelation at 1e160 = {r1}");

    // Stiff observations near 1e160 whose deviations |d| = 1e153 have finite css = 2e306 <= f64::MAX:
    let stiff_153 = [1e160 - 1e153, 1e160, 1e160 + 1e153];
    let m_153 = SummaryMoments::from_slice(&stiff_153).unwrap();
    assert!(m_153.css.is_finite());
    assert!((sample_sd(&stiff_153).unwrap() - 1e153).abs() <= 1e-7 * 1e153);
    assert!(covariance(&stiff_153, &stiff_153).unwrap().is_finite());
}

#[test]
fn s4_notes_halley_log_normal_isf_and_integer_gamma_accuracy() {
    // Deep-tail norm_isf roundtrip via erfcx verifying cubic Halley accuracy down to q = 1e-305:
    for &q in &[1e-15_f64, 1e-50, 1e-150, 1e-305] {
        let z = norm_isf(q).unwrap();
        let w = z * std::f64::consts::FRAC_1_SQRT_2;
        let ln_back = -std::f64::consts::LN_2 - 0.5 * z * z + erfcx(w).ln();
        assert!((ln_back - q.ln()).abs() <= 2.0 * f64::EPSILON * q.ln().abs(), "q={q}: z={z}, ln_back={ln_back}");
    }

    // Exact IEEE-754 integer factorials Gamma(n) = (n - 1)! for n = 22 (21!) and n = 23 (22!),
    // and <= 1 ULP accuracy all the way up to Gamma(171) = 170!:
    assert_eq!(gamma(22.0).unwrap(), 51_090_942_171_709_440_000.0_f64);
    assert_eq!(gamma(23.0).unwrap(), 1_124_000_727_777_607_680_000.0_f64);
    let g171 = gamma(171.0).unwrap();
    let expected_170_fact = 7.257_415_615_307_999e306_f64; // round-to-nearest IEEE-754 f64 of 170!
    assert!((g171 - expected_170_fact).abs() <= 2.0 * f64::EPSILON * expected_170_fact, "g171 = {g171:.17e}");
}
