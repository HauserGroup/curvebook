//! D1 reference values for distribution tails, inverses, and noncentral confidence limits.
//!
//! See `reference/distribution_tails.py` for the inputs and independent mpmath 1.3.0 generator.
//! Incomplete beta and gamma use the defining integrals in NIST DLMF §§8.17 and 8.2.
//! Noncentral t uses conditional-normal quadrature. Noncentral F and chi-square use Poisson
//! mixtures of fresh central evaluations with omitted mass below 1e-100. All expected values
//! are rounded once to binary64.

use curvebook_math::dist::noncentral::{ncchi2_cdf, ncchi2_sf, ncf_cdf, ncf_sf, nct_cdf, nct_sf};
use curvebook_math::special::{
    ln_reg_beta_xy, ln_reg_gamma_p, ln_reg_gamma_q, reg_beta_c_inv, reg_beta_inv, reg_beta_inv_pair, reg_beta_xy, reg_gamma_p, reg_gamma_p_inv,
    reg_gamma_q, reg_gamma_q_inv,
};
use curvebook_math::{noncentral_chi2_ci, noncentral_f_ci, noncentral_t_ci};
use curvebook_testkit::assert_approx_eq;

// Require roughly 10 significant digits in the representable special-function tails.
// On log scale an absolute 2e-10 error corresponds to approximately the same relative
// probability error, including when the probability itself underflows.
const SPECIAL_REL_TOL: f64 = 2e-10;
const LOG_ABS_TOL: f64 = 2e-10;

// Mixtures accumulate error across many terms and their confidence limits invert those
// tails. Require nine significant digits without a fixed absolute floor for probabilities.
const MIXTURE_REL_TOL: f64 = 1e-9;

fn assert_tails((lower, upper): (f64, f64), (want_lower, want_upper): (f64, f64), rel_tol: f64, case: &str) {
    assert_approx_eq(lower, want_lower, 0.0, rel_tol, &format!("{case} lower tail"));
    assert_approx_eq(upper, want_upper, 0.0, rel_tol, &format!("{case} upper tail"));
}

fn assert_log_tails((lower, upper): (f64, f64), (want_lower, want_upper): (f64, f64), case: &str) {
    assert_approx_eq(lower, want_lower, LOG_ABS_TOL, 0.0, &format!("{case} log lower tail"));
    assert_approx_eq(upper, want_upper, LOG_ABS_TOL, 0.0, &format!("{case} log upper tail"));
}

#[test]
fn beta_tails_across_fraction_switch_and_underflow() {
    let cases = [
        (
            "beta small shape",
            0.25,
            72.0,
            2.0_f64.powi(-280),
            (2.718_605_587_967_200_7e-21, 1.0),
            (-47.354_167_855_500_4, -2.718_605_587_967_200_7e-21),
        ),
        ("beta underflow", 20.0, 1.25, 2.0_f64.powi(-60), (0.0, 1.0), (-830.921_695_904_796_1, 0.0)),
        // The continued fraction switches to the complementary tail at x = 3.5/12.
        (
            "beta before fraction switch",
            2.5,
            7.5,
            149.0 / 512.0,
            (0.657_340_615_558_338_7, 0.342_659_384_441_661_33),
            (-0.419_552_954_113_424_9, -1.071_018_373_490_348_5),
        ),
        (
            "beta after fraction switch",
            2.5,
            7.5,
            150.0 / 512.0,
            (0.662_105_224_381_769_8, 0.337_894_775_618_230_2),
            (-0.412_330_786_459_042_3, -1.085_020_746_706_575_1),
        ),
    ];
    for (case, a, b, x, expected, expected_logs) in cases {
        let y = 1.0 - x;
        assert_tails((reg_beta_xy(x, y, a, b).unwrap(), reg_beta_xy(y, x, b, a).unwrap()), expected, SPECIAL_REL_TOL, case);
        assert_log_tails((ln_reg_beta_xy(x, y, a, b).unwrap(), ln_reg_beta_xy(y, x, b, a).unwrap()), expected_logs, case);
    }
}

#[test]
fn beta_inverses_match_independent_quantiles() {
    assert_approx_eq(reg_beta_inv(2.5, 7.5, 0.1).unwrap(), 0.093_335_539_842_153_19, 0.0, SPECIAL_REL_TOL, "beta lower quantile");
    assert_approx_eq(reg_beta_c_inv(2.5, 7.5, 0.1).unwrap(), 0.431_066_332_514_247_1, 0.0, SPECIAL_REL_TOL, "beta upper quantile");

    let (x, y) = reg_beta_inv_pair(72.0, 0.25, 1.0, 1e-20).unwrap();
    assert_eq!(x.to_bits(), 1.0_f64.to_bits(), "beta quantile rounds to one");
    assert_approx_eq(y, 9.423_590_227_848_291e-83, 0.0, SPECIAL_REL_TOL, "beta tiny quantile complement");
}

#[test]
fn gamma_tails_across_temme_switch_and_underflow() {
    let cases = [
        ("gamma small shape", 0.25, 2.0_f64.powi(-280), (9.344_998_151_438_823e-22, 1.0), (-48.422_030_802_774_36, -9.344_998_151_438_823e-22)),
        ("gamma underflow", 0.375, 850.0, (1.0, 0.0), (0.0, -855.079_580_862_860_6)),
        // Keep x-a fixed while crossing the shape threshold for Temme's asymptotic expansion.
        (
            "gamma before Temme",
            4095.0,
            4111.0,
            (0.600_604_824_983_859_4, 0.399_395_175_016_140_64),
            (-0.509_818_089_859_016_9, -0.917_803_938_653_912_2),
        ),
        ("gamma at Temme", 4096.0, 4112.0, (0.600_592_837_765_141_9, 0.399_407_162_234_858_06), (-0.509_838_048_636_975_9, -0.917_773_925_675_333_3)),
    ];
    for (case, a, x, expected, expected_logs) in cases {
        assert_tails((reg_gamma_p(a, x).unwrap(), reg_gamma_q(a, x).unwrap()), expected, SPECIAL_REL_TOL, case);
        assert_log_tails((ln_reg_gamma_p(a, x).unwrap(), ln_reg_gamma_q(a, x).unwrap()), expected_logs, case);
    }
}

#[test]
fn gamma_inverses_match_independent_quantiles() {
    assert_approx_eq(reg_gamma_p_inv(0.25, 1e-20).unwrap(), 6.749_697_893_111_73e-81, 0.0, SPECIAL_REL_TOL, "gamma lower quantile");
    assert_approx_eq(reg_gamma_q_inv(0.375, 1e-60).unwrap(), 134.225_220_463_804_02, 0.0, SPECIAL_REL_TOL, "gamma upper quantile");
}

#[test]
fn noncentral_t_tails_match_quadrature() {
    let cases = [
        ("t small lower tail", 0.5, 1.5, 12.0, (2.793_834_875_783_376_3e-28, 1.0)),
        ("t small upper tail", -0.75, 7.5, -11.0, (1.0, 4.001_058_151_721_363e-24)),
        ("t ordinary", 2.75, 6.0, 1.25, (0.864_203_775_638_669_6, 0.135_796_224_361_330_43)),
    ];
    for (case, t, df, delta, expected) in cases {
        assert_tails((nct_cdf(t, df, delta).unwrap(), nct_sf(t, df, delta).unwrap()), expected, MIXTURE_REL_TOL, case);
    }
}

#[test]
fn noncentral_f_tails_match_poisson_mixture() {
    let cases = [
        ("F tiny lower tail", 0.125, 1.5, 40.0, 100.0, (2.642_792_656_786_328_4e-22, 1.0)),
        ("F large noncentrality", 80.0, 5.0, 120.0, 320.0, (0.885_639_743_517_957_8, 0.114_360_256_482_042_22)),
    ];
    for (case, f, df1, df2, lambda, expected) in cases {
        assert_tails((ncf_cdf(f, df1, df2, lambda).unwrap(), ncf_sf(f, df1, df2, lambda).unwrap()), expected, MIXTURE_REL_TOL, case);
    }
}

#[test]
fn noncentral_chi_square_tails_match_poisson_mixture() {
    let cases = [
        ("chi-square small df", 3.0, 0.75, 120.0, (1.836_848_589_427_992_3e-20, 1.0)),
        ("chi-square tiny upper tail", 720.0, 8.0, 320.0, (1.0, 7.778_440_106_814_559e-19)),
        ("chi-square ordinary", 48.0, 8.0, 40.0, (0.529_240_824_197_922_6, 0.470_759_175_802_077_43)),
    ];
    for (case, x, df, lambda, expected) in cases {
        assert_tails((ncchi2_cdf(x, df, lambda).unwrap(), ncchi2_sf(x, df, lambda).unwrap()), expected, MIXTURE_REL_TOL, case);
    }
}

#[test]
fn noncentral_intervals_match_independent_inversions() {
    let cases = [
        ("t interval", noncentral_t_ci(2.75, 6.0, 95.0), (0.202_519_197_430_068_35, 5.162_631_886_324_342)),
        ("F interval", noncentral_f_ci(80.0, 5.0, 120.0, 95.0), (277.866_120_215_331_7, 533.269_072_807_556_4)),
        ("chi-square interval", noncentral_chi2_ci(48.0, 8.0, 95.0), (18.779_868_567_845_266, 71.055_376_418_909_18)),
    ];
    for (case, actual, (expected_lower, expected_upper)) in cases {
        let (lower, upper) = actual.unwrap();
        assert_approx_eq(lower, expected_lower, 0.0, MIXTURE_REL_TOL, &format!("{case} lower limit"));
        assert_approx_eq(upper, expected_upper, 0.0, MIXTURE_REL_TOL, &format!("{case} upper limit"));
    }
}
