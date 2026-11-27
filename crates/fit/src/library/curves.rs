//! Exponential, Growth, Radiobiology, Peak, Wave, and Standard-Curve / Polynomial equations.
//!
//! Every equation below cites the published source of its functional form in short
//! `Author (Year)` form on its model struct; the full references are collected here. Where the
//! library's parameterisation differs from the original paper (e.g. `Y0` / `Plateau` / `K` instead
//! of the original constants) the struct comment says so and names the textbook that uses the
//! library's parameterisation.
//!
//! References:
//! - Motulsky, H. J., & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear
//!   and Nonlinear Regression: A Practical Guide to Curve Fitting*. New York: Oxford University Press.
//! - Seber, G. A. F., & Wild, C. J. (1989). *Nonlinear Regression*. New York: Wiley.
//!   (Ch. 7 growth models; ch. 8 compartmental / sum-of-exponentials models; ch. 9 multiphase and
//!   spline regressions.)
//! - Malthus, T. R. (1798). *An Essay on the Principle of Population*. London: J. Johnson.
//! - Verhulst, P.-F. (1838). Notice sur la loi que la population suit dans son accroissement.
//!   *Correspondance mathématique et physique* 10:113–121.
//! - Gompertz, B. (1825). On the nature of the function expressive of the law of human mortality,
//!   and on a new mode of determining the value of life contingencies. *Philosophical Transactions
//!   of the Royal Society of London* 115:513–583.
//! - Yin, X., Goudriaan, J., Lantinga, E. A., Vos, J., & Spiertz, H. J. (2003). A flexible sigmoid
//!   function of determinate growth. *Annals of Botany* 91:361–371. doi:10.1093/aob/mcg029.
//! - Fowler, J. F. (1989). The linear-quadratic formula and progress in fractionated radiotherapy.
//!   *British Journal of Radiology* 62:679–694.
//! - Chadwick, K. H., & Leenhouts, H. P. (1973). A molecular theory of cell survival. *Physics in
//!   Medicine and Biology* 18:78–87. doi:10.1088/0031-9155/18/1/007.
//! - Brenner, D. J. (2008). The linear-quadratic model is an appropriate methodology for
//!   determining isoeffective doses at large doses per fraction. *Seminars in Radiation Oncology*
//!   18:234–239. doi:10.1016/j.semradonc.2008.04.004.
//! - Hodgkin, A. L., & Huxley, A. F. (1952). A quantitative description of membrane current and its
//!   application to conduction and excitation in nerve. *Journal of Physiology* 117:500–544.
//!   doi:10.1113/jphysiol.1952.sp004764.
//! - Bevington, P. R., & Robinson, D. K. (2003). *Data Reduction and Error Analysis for the
//!   Physical Sciences*, 3rd ed. New York: McGraw-Hill. (Ch. 2: Gaussian and Lorentzian distributions.)
//! - Limpert, E., Stahel, W. A., & Abbt, M. (2001). Log-normal distributions across the sciences:
//!   keys and clues. *BioScience* 51:341–352.
//! - Weibull, W. (1951). A statistical distribution function of wide applicability. *Journal of
//!   Applied Mechanics* 18:293–297.
//! - Finney, D. J. (1971). *Probit Analysis*, 3rd ed. Cambridge: Cambridge University Press.
//! - Bloomfield, P. (2000). *Fourier Analysis of Time Series: An Introduction*, 2nd ed. New York: Wiley.
//! - Bracewell, R. N. (2000). *The Fourier Transform and Its Applications*, 3rd ed. Boston: McGraw-Hill.
//! - Draper, N. R., & Smith, H. (1998). *Applied Regression Analysis*, 3rd ed. New York: Wiley.
//! - Hudson, D. J. (1966). Fitting segmented curves whose join points have to be estimated.
//!   *Journal of the American Statistical Association* 61:1097–1129.
//! - Watts, D. G., & Bacon, D. W. (1974). Using an hyperbola as a transition model to fit
//!   two-regime straight-line data. *Technometrics* 16.
//! - Huxley, J. S. (1932). *Problems of Relative Growth*. London: Methuen.
//! - Baker, G. A., & Graves-Morris, P. (1996). *Padé Approximants*, 2nd ed. Cambridge: Cambridge
//!   University Press.

use crate::library::{BuiltinEquationSpec, ModelMeta, ParametricModel, data_x_min_max, data_y_min_max, resolve_from_specs, safe_exp, spec_ids};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};
use curvebook_doc::{ParameterRule, ParameterScope};

/// Ordered single-source-of-truth registrations for the general curve equation family.
pub(crate) const CURVE_SPECS: &[BuiltinEquationSpec] = &[
    // Exponential
    BuiltinEquationSpec::new("one_phase_decay", &[], || Box::new(OnePhaseDecay)),
    BuiltinEquationSpec::new("two_phase_decay", &[], || Box::new(TwoPhaseDecay)),
    BuiltinEquationSpec::new("three_phase_decay", &[], || Box::new(ThreePhaseDecay)),
    BuiltinEquationSpec::new("plateau_followed_by_one_phase_decay", &[], || Box::new(PlateauThenDecay)),
    BuiltinEquationSpec::new("one_phase_association", &[], || Box::new(OnePhaseAssociation)),
    BuiltinEquationSpec::new("two_phase_association", &[], || Box::new(TwoPhaseAssociation)),
    BuiltinEquationSpec::new("plateau_followed_by_one_phase_association", &[], || Box::new(PlateauThenAssoc)),
    BuiltinEquationSpec::new("exponential_growth", &[], || {
        Box::new(ExponentialGrowth { meta: ModelMeta::new("exponential_growth", "Exponential growth", EquationCategory::Exponential, &["Y0", "k"]) })
    }),
    BuiltinEquationSpec::new("log_exponential_growth", &[], || Box::new(LogExponentialGrowth)),
    // Growth
    BuiltinEquationSpec::new("exponential_plateau", &[], || Box::new(ExponentialPlateau)),
    BuiltinEquationSpec::new("logistic_growth", &[], || Box::new(LogisticGrowth)),
    BuiltinEquationSpec::new("boltzmann_sigmoid", &[], || Box::new(BoltzmannSigmoid)),
    BuiltinEquationSpec::new("gompertz_growth", &[], || Box::new(GompertzGrowth)),
    BuiltinEquationSpec::new("beta_growth_then_decay", &[], || Box::new(BetaGrowthDecay)),
    BuiltinEquationSpec::new("malthusian_growth", &[], || {
        Box::new(ExponentialGrowth { meta: ModelMeta::new("malthusian_growth", "Malthusian growth", EquationCategory::Growth, &["Y0", "k"]) })
    }),
    // Radiobiology
    BuiltinEquationSpec::new("linear_quadratic_survival", &[], || Box::new(LinearQuadraticSurvival)),
    BuiltinEquationSpec::new("linear_quadratic_percent_surviving", &[], || {
        Box::new(LinearQuadraticVariant {
            meta: ModelMeta::new(
                "linear_quadratic_percent_surviving",
                "Linear-quadratic radiation survival (percent surviving)",
                EquationCategory::Radiobiology,
                &["Alpha", "Beta"],
            ),
            mode: LinearQuadraticMode::PercentSurviving,
        })
    }),
    BuiltinEquationSpec::new("linear_quadratic_number_surviving", &[], || {
        Box::new(LinearQuadraticVariant {
            meta: ModelMeta::new(
                "linear_quadratic_number_surviving",
                "Linear-quadratic radiation survival (number surviving)",
                EquationCategory::Radiobiology,
                &["Y0", "Alpha", "Beta"],
            ),
            mode: LinearQuadraticMode::NumberSurviving,
        })
    }),
    BuiltinEquationSpec::new("linear_quadratic_fraction_dead", &[], || {
        Box::new(LinearQuadraticVariant {
            meta: ModelMeta::new(
                "linear_quadratic_fraction_dead",
                "Linear-quadratic radiation death (fraction dead)",
                EquationCategory::Radiobiology,
                &["Alpha", "Beta"],
            ),
            mode: LinearQuadraticMode::FractionDead,
        })
    }),
    BuiltinEquationSpec::new("linear_quadratic_percent_dead", &[], || {
        Box::new(LinearQuadraticVariant {
            meta: ModelMeta::new(
                "linear_quadratic_percent_dead",
                "Linear-quadratic radiation death (percent dead)",
                EquationCategory::Radiobiology,
                &["Alpha", "Beta"],
            ),
            mode: LinearQuadraticMode::PercentDead,
        })
    }),
    BuiltinEquationSpec::new("linear_quadratic_number_dead", &[], || {
        Box::new(LinearQuadraticVariant {
            meta: ModelMeta::new(
                "linear_quadratic_number_dead",
                "Linear-quadratic radiation death (number dead)",
                EquationCategory::Radiobiology,
                &["Y0", "Alpha", "Beta"],
            ),
            mode: LinearQuadraticMode::NumberDead,
        })
    }),
    // Peak
    BuiltinEquationSpec::new("gaussian", &[], || {
        Box::new(GaussianCurve {
            meta: ModelMeta::new("gaussian", "Gaussian", EquationCategory::Gaussian, &["Amplitude", "Mean", "SD"]),
            with_baseline: false,
        })
    }),
    BuiltinEquationSpec::new("gaussian_baseline", &[], || {
        Box::new(GaussianCurve {
            meta: ModelMeta::new("gaussian_baseline", "Gaussian with baseline", EquationCategory::Gaussian, &["Baseline", "Amplitude", "Mean", "SD"]),
            with_baseline: true,
        })
    }),
    BuiltinEquationSpec::new("gaussian_area", &[], || Box::new(GaussianArea)),
    BuiltinEquationSpec::new("sum_of_two_gaussians", &[], || Box::new(TwoGaussians)),
    BuiltinEquationSpec::new("log_gaussian", &[], || Box::new(LogGaussian)),
    BuiltinEquationSpec::new("lognormal_area", &[], || Box::new(LognormalArea)),
    BuiltinEquationSpec::new("cumulative_gaussian", &[], || Box::new(CumulativeGaussian)),
    BuiltinEquationSpec::new("cumulative_gaussian_percent", &[], || Box::new(CumulativeGaussianScaled::percent())),
    BuiltinEquationSpec::new("cumulative_gaussian_fraction", &[], || Box::new(CumulativeGaussianScaled::fraction())),
    BuiltinEquationSpec::new("cumulative_gaussian_counts", &[], || Box::new(CumulativeGaussianCounts)),
    BuiltinEquationSpec::new("lorentzian", &[], || Box::new(LorentzianPeak)),
    BuiltinEquationSpec::new("sum_of_two_lorentzians", &[], || Box::new(TwoLorentzians)),
    BuiltinEquationSpec::new("weibull_peak", &[], || Box::new(WeibullPeak)),
    // Wave
    BuiltinEquationSpec::new("sine_wave", &[], || Box::new(SineWave)),
    BuiltinEquationSpec::new("standard_sine_wave", &[], || Box::new(StandardSineWave)),
    BuiltinEquationSpec::new("damped_sine_wave", &[], || Box::new(DampedSineWave)),
    BuiltinEquationSpec::new("sinc_wave", &[], || Box::new(SincWave)),
    // Standard Curves & Polynomials
    BuiltinEquationSpec::new("horizontal_line", &[], || Box::new(HorizontalLine)),
    BuiltinEquationSpec::new("straight_line", &["polynomial_1"], || {
        Box::new(StraightLine {
            meta: ModelMeta::new("straight_line", "Straight line", EquationCategory::StandardCurve, &["Slope", "YIntercept"]),
            through_origin: false,
        })
    }),
    BuiltinEquationSpec::new("line_through_origin", &[], || {
        Box::new(StraightLine {
            meta: ModelMeta::new("line_through_origin", "Line through origin", EquationCategory::StandardCurve, &["Slope"]),
            through_origin: true,
        })
    }),
    BuiltinEquationSpec::new("pade_1_1", &[], || Box::new(Pade11)),
    BuiltinEquationSpec::new("two_lines_intersection", &[], || Box::new(TwoLinesIntersection)),
    BuiltinEquationSpec::new("segmental_linear", &[], || Box::new(SegmentalLinear)),
    BuiltinEquationSpec::new("hinge_smooth_hyperbolic", &[], || Box::new(SmoothHinge)),
    BuiltinEquationSpec::new("polynomial_2", &["quadratic"], || Box::new(Polynomial::new(2))),
    BuiltinEquationSpec::new("polynomial_3", &["cubic"], || Box::new(Polynomial::new(3))),
    BuiltinEquationSpec::new("polynomial_4", &["quartic"], || Box::new(Polynomial::new(4))),
    BuiltinEquationSpec::new("polynomial_5", &[], || Box::new(Polynomial::new(5))),
    BuiltinEquationSpec::new("polynomial_6", &[], || Box::new(Polynomial::new(6))),
    BuiltinEquationSpec::new("power_law", &["power_series"], || Box::new(PowerCurve)),
    BuiltinEquationSpec::new("power_series_two_term", &[], || Box::new(PowerSeriesTwoTerm)),
    BuiltinEquationSpec::new("logarithmic_curve", &[], || Box::new(LogarithmicCurve)),
    BuiltinEquationSpec::new("centered_poly_1", &["centered_linear", "centered_polynomial_1"], || Box::new(CenteredPolynomial::new(1))),
    BuiltinEquationSpec::new("centered_poly_2", &["centered_quadratic", "centered_polynomial_2"], || Box::new(CenteredPolynomial::new(2))),
    BuiltinEquationSpec::new("centered_poly_3", &["centered_cubic", "centered_polynomial_3"], || Box::new(CenteredPolynomial::new(3))),
    BuiltinEquationSpec::new("centered_poly_4", &["centered_quartic", "centered_polynomial_4"], || Box::new(CenteredPolynomial::new(4))),
    BuiltinEquationSpec::new("centered_poly_5", &["centered_polynomial_5"], || Box::new(CenteredPolynomial::new(5))),
    BuiltinEquationSpec::new("centered_poly_6", &["centered_polynomial_6"], || Box::new(CenteredPolynomial::new(6))),
    BuiltinEquationSpec::new("semi_log_line", &["semilog_line"], || Box::new(SemiLogLine)),
    BuiltinEquationSpec::new("semilog_x_lin_y_log", &[], || Box::new(SemilogXLinYLog)),
    BuiltinEquationSpec::new("log_log_line", &["loglog_line"], || Box::new(LogLogLine)),
    BuiltinEquationSpec::new("probit_line", &[], || Box::new(ProbitLine)),
    // Classic Legacy Reparameterizations
    BuiltinEquationSpec::new("classic_one_phase_decay", &[], || Box::new(ClassicOnePhaseDecay)),
    BuiltinEquationSpec::new("classic_two_phase_decay", &[], || Box::new(ClassicTwoPhaseDecay)),
    BuiltinEquationSpec::new("classic_one_phase_association", &[], || Box::new(ClassicOnePhaseAssociation)),
    BuiltinEquationSpec::new("classic_two_phase_association", &[], || Box::new(ClassicTwoPhaseAssociation)),
    BuiltinEquationSpec::new("classic_exponential_growth", &[], || Box::new(ClassicExponentialGrowth)),
    BuiltinEquationSpec::new("classic_sine_wave", &[], || Box::new(ClassicSineWave)),
];

const CURVE_EQUATION_ID_ARRAY: [&str; CURVE_SPECS.len()] = spec_ids::<{ CURVE_SPECS.len() }>(CURVE_SPECS);

/// Canonical IDs exposed by the general curve equation catalog.
pub const CURVE_EQUATION_IDS: &[&str] = &CURVE_EQUATION_ID_ARRAY;

/// Resolve any non-pharmacology, non-NIST built-in equation ID into a [`ParametricModel`].
#[must_use]
pub fn resolve_curve_model(norm_id: &str) -> Option<Box<dyn ParametricModel>> {
    resolve_from_specs(CURVE_SPECS, norm_id)
}

#[inline]
fn all_finite_params(params: &[f64], count: usize) -> bool {
    params.len() >= count && params[..count].iter().all(|v| v.is_finite())
}

#[inline]
fn push_finite_scalar(out: &mut Vec<DerivedScalar>, name: &str, value: f64) {
    if value.is_finite() {
        out.push(DerivedScalar::new(name.to_owned(), value, None));
    }
}

fn estimate_exp_growth_seed(points: &[DataPoint]) -> Vec<f64> {
    let (xmin, xmax) = data_x_min_max(points);
    let max_abs_x = xmin.abs().max(xmax.abs());
    let k_cap = if max_abs_x > 1e-300 && max_abs_x.is_finite() { 600.0 / max_abs_x } else { 10.0 };
    let pos_pairs: Vec<(f64, f64)> =
        points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite() && p.y > 0.0).map(|p| (p.x0(), p.y.ln())).collect();
    let (sign, pairs) = if pos_pairs.len() >= 2 {
        (1.0_f64, pos_pairs)
    } else {
        let neg_pairs: Vec<(f64, f64)> =
            points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite() && p.y < 0.0).map(|p| (p.x0(), (-p.y).ln())).collect();
        if neg_pairs.len() >= 2 { (-1.0_f64, neg_pairs) } else { (1.0_f64, pos_pairs) }
    };
    if pairs.len() >= 2 {
        let n = pairs.len() as f64;
        let mean_x = pairs.iter().map(|&(x, _)| x).sum::<f64>() / n;
        let mean_lny = pairs.iter().map(|&(_, lny)| lny).sum::<f64>() / n;
        let (k_ols, _) = estimate_ols_from_pairs(&pairs);
        let mut k = if k_ols.is_finite() { k_ols.clamp(-k_cap, k_cap) } else { 0.0 };
        let mut ln_y0 = mean_lny - k * mean_x;
        if !ln_y0.is_finite() || !(-650.0..=650.0).contains(&ln_y0) {
            let target_ln_y0 = if ln_y0.is_finite() { ln_y0.clamp(-650.0, 650.0) } else { 0.0 };
            if mean_x.abs() > 1e-12 {
                k = ((mean_lny - target_ln_y0) / mean_x).clamp(-k_cap, k_cap);
            }
            ln_y0 = (mean_lny - k * mean_x).clamp(-650.0, 650.0);
        }
        let y0 = sign * ln_y0.exp();
        if y0.is_finite() && y0 != 0.0 && k.is_finite() {
            return vec![y0, k];
        }
    }
    let (ymin, ymax) = data_y_min_max(points);
    let dt = (xmax - xmin).max(1e-6);
    let ratio = (ymax.max(1e-6) / ymin.max(1e-6)).max(1.01);
    let k = (ratio.ln() / dt).clamp(-k_cap, k_cap);
    let y0 = if ymin > 0.0 && ymin.is_finite() { (ymin.ln() - k * xmin).clamp(-650.0, 650.0).exp() } else { 1e-3 };
    vec![y0, k]
}

/// One-phase exponential decay `Y = (Y0 - Plateau) * exp(-K * X) + Plateau`.
/// `HalfLife = ln 2 / K`, `Tau = 1 / K`.
/// Reference: first-order (single-compartment) kinetics, Seber & Wild (1989) ch. 8; this
/// `Y0` / `Plateau` / `K` parameterisation follows Motulsky & Christopoulos (2004).
struct OnePhaseDecay;

impl ParametricModel for OnePhaseDecay {
    impl_static_meta!("one_phase_decay", "One phase decay", EquationCategory::Exponential, ["Y0", "Plateau", "K"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let e = safe_exp(-k * t)?;
        let y = (y0 - plateau) * e + plateau;
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = e;
        out[1] = 1.0 - e;
        out[2] = -(y0 - plateau) * t * e;
        out[..3].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, ymin, 2.0 / (xmax - xmin).max(1.0)]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 {
            push_finite_scalar(&mut out, "Span", params[0] - params[1]);
            if params[2].is_finite() && params[2] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife", core::f64::consts::LN_2 / params[2]);
                push_finite_scalar(&mut out, "Tau", 1.0 / params[2]);
            }
        }
        out
    }
}

/// Two-phase (bi-exponential) decay with `PercentFast` of the span decaying at `KFast`.
/// Reference: sum-of-exponentials (two-compartment) model, Seber & Wild (1989) ch. 8; the
/// `Plateau` / `PercentFast` parameterisation follows Motulsky & Christopoulos (2004).
struct TwoPhaseDecay;

impl ParametricModel for TwoPhaseDecay {
    impl_static_meta!("two_phase_decay", "Two phase decay", EquationCategory::Exponential, ["Y0", "Plateau", "PercentFast", "KFast", "KSlow"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 5) {
            return None;
        }
        let (y0, plateau, pct_fast, k_fast, k_slow) = (params[0], params[1], params[2], params[3], params[4]);
        let span = y0 - plateau;
        let f = pct_fast * 0.01;
        let e1 = safe_exp(-k_fast * t)?;
        let e2 = safe_exp(-k_slow * t)?;
        let y = plateau + span * (f * e1 + (1.0 - f) * e2);
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span_x = (xmax - xmin).max(1.0);
        vec![ymax, ymin, 50.0, 4.0 / span_x, 0.5 / span_x]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), Some(100.0)), (Some(0.0), None), (Some(0.0), None)]
    }
}

/// Three-phase (tri-exponential) decay; the very-slow fraction is `1 - PercentFast - PercentSlow`.
/// Reference: sum-of-exponentials (compartmental) model, Seber & Wild (1989) ch. 8.
struct ThreePhaseDecay;

impl ParametricModel for ThreePhaseDecay {
    impl_static_meta!(
        "three_phase_decay",
        "Three phase decay",
        EquationCategory::Exponential,
        ["Y0", "Plateau", "PercentFast", "PercentSlow", "KFast", "KSlow", "KVerySlow"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 7) {
            return None;
        }
        let (y0, plateau, pf, ps, kf, ks, kvs) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        let span = y0 - plateau;
        let ff = pf * 0.01;
        let fs = ps * 0.01;
        let fv = 1.0 - ff - fs;
        let e1 = safe_exp(-kf * t)?;
        let e2 = safe_exp(-ks * t)?;
        let e3 = safe_exp(-kvs * t)?;
        let y = plateau + span * (ff * e1 + fs * e2 + fv * e3);
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span_x = (xmax - xmin).max(1.0);
        vec![ymax, ymin, 33.0, 33.0, 5.0 / span_x, 1.0 / span_x, 0.2 / span_x]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), Some(100.0)), (Some(0.0), Some(100.0)), (Some(0.0), None), (Some(0.0), None), (Some(0.0), None)]
    }
}

/// Constant `Y0` until `X0`, then one-phase decay toward `Plateau` (piecewise composite).
/// Reference: the decay segment is the first-order form of Seber & Wild (1989) ch. 8; the hold
/// segment is a piecewise join (Seber & Wild 1989, ch. 9). No single primary source for the composite.
struct PlateauThenDecay;

impl ParametricModel for PlateauThenDecay {
    impl_static_meta!(
        "plateau_followed_by_one_phase_decay",
        "Plateau followed by one phase decay",
        EquationCategory::Exponential,
        ["Y0", "X0", "Plateau", "K"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (y0, x0, plateau, k) = (params[0], params[1], params[2], params[3]);
        if t < x0 {
            Some(y0)
        } else {
            let e = safe_exp(-k * (t - x0))?;
            let y = plateau + (y0 - plateau) * e;
            y.is_finite().then_some(y)
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let raw_span = (xmax - xmin).max(0.0);
        vec![ymax, xmin + 0.2 * raw_span, ymin, 2.0 / raw_span.max(1e-12)]
    }
}

/// One-phase exponential association `Y = Y0 + (Plateau - Y0) * (1 - exp(-K * X))`.
/// Reference: monomolecular (first-order approach to an asymptote) form, Seber & Wild (1989)
/// ch. 7; this kinetics parameterisation `(Y0, Plateau, K)` follows Motulsky & Christopoulos
/// (2004). See [`ExponentialPlateau`] for the `Growth`-category `(YM, Y0, k)` parameterisation.
struct OnePhaseAssociation;

impl ParametricModel for OnePhaseAssociation {
    impl_static_meta!("one_phase_association", "One phase association", EquationCategory::Exponential, ["Y0", "Plateau", "K"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let e = safe_exp(-k * t)?;
        let y = y0 + (plateau - y0) * (1.0 - e);
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = e;
        out[1] = 1.0 - e;
        out[2] = (plateau - y0) * t * e;
        out[..3].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymin, ymax, 2.0 / (xmax - xmin).max(1.0)]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 {
            push_finite_scalar(&mut out, "Span", params[1] - params[0]);
            if params[2].is_finite() && params[2] > 0.0 {
                push_finite_scalar(&mut out, "HalfTime", core::f64::consts::LN_2 / params[2]);
                push_finite_scalar(&mut out, "Tau", 1.0 / params[2]);
            }
        }
        out
    }
}

/// Constant `Y0` until `X0`, then one-phase association toward `Plateau` (piecewise composite).
/// Reference: monomolecular segment per Seber & Wild (1989) ch. 7, joined piecewise (ch. 9).
/// No single primary source for the composite.
struct PlateauThenAssoc;

impl ParametricModel for PlateauThenAssoc {
    impl_static_meta!(
        "plateau_followed_by_one_phase_association",
        "Plateau followed by one phase association",
        EquationCategory::Exponential,
        ["Y0", "X0", "Plateau", "K"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (y0, x0, plateau, k) = (params[0], params[1], params[2], params[3]);
        if t < x0 {
            Some(y0)
        } else {
            let e = safe_exp(-k * (t - x0))?;
            let y = y0 + (plateau - y0) * (1.0 - e);
            y.is_finite().then_some(y)
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let raw_span = (xmax - xmin).max(0.0);
        vec![ymin, xmin + 0.2 * raw_span, ymax, 2.0 / raw_span.max(1e-12)]
    }
}

/// Unbounded exponential (Malthusian) growth `Y = Y0 * exp(k * X)`.
/// Reference: Malthus (1798); see also Seber & Wild (1989) ch. 7.
struct ExponentialGrowth {
    meta: ModelMeta,
}

impl ParametricModel for ExponentialGrowth {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let (y0, k) = (params[0], params[1]);
        let y = y0 * safe_exp(k * t)?;
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 2) || out.len() < 2 {
            return false;
        }
        let Some(e) = safe_exp(params[1] * t) else {
            return false;
        };
        out[0] = e;
        out[1] = params[0] * t * e;
        out[..2].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        estimate_exp_growth_seed(points)
    }
}

/// Exponential plateau (monomolecular / Mitscherlich) growth `Y = YM - (YM - Y0) * exp(-k * X)`.
/// Categorised under `Growth` with the ceiling parameterisation `(YM, Y0, k)` matching
/// `logistic_growth` and `gompertz_growth` (Seber & Wild 1989 ch. 7), distinct from
/// `one_phase_association` (`Exponential` kinetics parameterisation `(Y0, Plateau, K)`).
/// Reference: Seber & Wild (1989) ch. 7.
struct ExponentialPlateau;

impl ParametricModel for ExponentialPlateau {
    impl_static_meta!("exponential_plateau", "Exponential plateau", EquationCategory::Growth, ["YM", "Y0", "k"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (ym, y0, k) = (params[0], params[1], params[2]);
        let y = ym - (ym - y0) * safe_exp(-k * t)?;
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (ym, y0, k) = (params[0], params[1], params[2]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = 1.0 - e;
        out[1] = e;
        out[2] = (ym - y0) * t * e;
        out[..3].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, ymin, 2.0 / (xmax - xmin).max(1.0)]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 {
            push_finite_scalar(&mut out, "Span", params[0] - params[1]);
            if params[2].is_finite() && params[2] > 0.0 {
                push_finite_scalar(&mut out, "HalfTime", core::f64::consts::LN_2 / params[2]);
                push_finite_scalar(&mut out, "Tau", 1.0 / params[2]);
            }
        }
        out
    }
}

/// Logistic growth `Y = YM * Y0 / (Y0 + (YM - Y0) * exp(-k * X))` (so `Y(0) = Y0`).
/// Reference: Verhulst (1838); the `YM` / `Y0` / `k` parameterisation is the logistic of
/// Seber & Wild (1989) ch. 7 written in terms of the initial value.
struct LogisticGrowth;

impl ParametricModel for LogisticGrowth {
    impl_static_meta!("logistic_growth", "Logistic growth", EquationCategory::Growth, ["YM", "Y0", "k"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (ym, y0, k) = (params[0], params[1], params[2]);
        if y0 == 0.0 {
            return None;
        }
        let e = safe_exp(-k * t)?;
        let denom = y0 + (ym - y0) * e;
        if denom == 0.0 || !denom.is_finite() {
            return None;
        }
        let y = ym * (y0 / denom);
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0), ymin.max(1e-3), 3.0 / (xmax - xmin).max(1.0)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 && all_finite_params(params, 3) {
            let (ym, y0, k) = (params[0], params[1], params[2]);
            if y0 > 0.0 && ym > y0 && k.abs() > 1e-15 {
                let x_inflect = ((ym - y0) / y0).ln() / k;
                push_finite_scalar(&mut out, "InflectionPoint", x_inflect);
            }
            push_finite_scalar(&mut out, "YInflection", 0.5 * ym);
            if k.abs() > 1e-15 {
                push_finite_scalar(&mut out, "1/k", 1.0 / k);
            }
        }
        out
    }
}

/// Gompertz growth `Y = YM * exp(-ln(YM / Y0) * exp(-k * X))` (so `Y(0) = Y0`).
/// Reference: Gompertz (1825); the initial-value parameterisation is one of the equivalent
/// Gompertz forms in Seber & Wild (1989) ch. 7.
struct GompertzGrowth;

impl ParametricModel for GompertzGrowth {
    impl_static_meta!("gompertz_growth", "Gompertz growth", EquationCategory::Growth, ["YM", "Y0", "k"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (ym, y0, k) = (params[0], params[1], params[2]);
        if ym <= 0.0 || y0 <= 0.0 {
            return None;
        }
        let e = safe_exp(-k * t)?;
        let inner = (ym / y0).ln() * e;
        let outer = safe_exp(-inner)?;
        let y = ym * outer;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0), ymin.max(1e-3), 2.0 / (xmax - xmin).max(1.0)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 && all_finite_params(params, 3) {
            let (ym, y0, k) = (params[0], params[1], params[2]);
            if ym > 0.0 && y0 > 0.0 && ym > y0 && k.abs() > 1e-15 {
                let ln_ratio = (ym / y0).ln();
                if ln_ratio > 0.0 {
                    let x_inflect = ln_ratio.ln() / k;
                    push_finite_scalar(&mut out, "InflectionPoint", x_inflect);
                }
            }
            push_finite_scalar(&mut out, "YInflection", ym / core::f64::consts::E);
            if k.abs() > 1e-15 {
                push_finite_scalar(&mut out, "1/k", 1.0 / k);
            }
        }
        out
    }
}

/// Beta growth function `Y = Ym * (1 + (te - X) / (te - tm)) * (X / te)^(te / (te - tm))`
/// for `0 <= X <= te` (with `0 <= tm < te`), holding `Y = Ym` for `X > te` (determinate
/// growth ceiling) and `Y = 0` for `X <= 0`.
/// Reference: Yin, Goudriaan, Lantinga, Vos & Spiertz (2003), Eqs. (5)-(6) (`tb = 0`, `wb = 0`).
struct BetaGrowthDecay;

impl ParametricModel for BetaGrowthDecay {
    impl_static_meta!("beta_growth_then_decay", "Beta growth then decay", EquationCategory::Growth, ["Ym", "te", "tm"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (ym, te, tm) = (params[0], params[1], params[2]);
        if !t.is_finite() || !ym.is_finite() || !te.is_finite() || !tm.is_finite() || te <= 0.0 || tm < 0.0 || te <= tm {
            return None;
        }
        if t <= 0.0 {
            return Some(0.0);
        }
        if t >= te {
            return Some(ym);
        }
        let denom = te - tm;
        let base = t / te;
        let exp = te / denom;
        let factor = 1.0 + (te - t) / denom;
        let val = ym * factor * base.powf(exp);
        val.is_finite().then_some(val)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let te = if xmax.is_finite() && xmax > 1e-6 { xmax } else { 1.0 };
        let x0 = if xmin.is_finite() { xmin.clamp(0.0, 0.8 * te) } else { 0.0 };
        let tm = (0.5 * (x0 + te)).clamp(0.0, 0.9 * te);
        vec![ymax, te, tm]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(1e-12), None), (Some(0.0), None)]
    }
}

/// Linear-quadratic cell survival `S = exp(-(alpha * D + beta * D^2))` (surviving fraction).
/// Reference: Fowler (1989).
struct LinearQuadraticSurvival;

impl ParametricModel for LinearQuadraticSurvival {
    impl_static_meta!("linear_quadratic_survival", "Linear-quadratic radiation survival", EquationCategory::Radiobiology, ["Alpha", "Beta"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let d = x.first().copied()?;
        if !d.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let (alpha, beta) = (params[0], params[1]);
        safe_exp(-(alpha * d + beta * d * d))
    }
    fn initial_guess(&self, _points: &[DataPoint]) -> Vec<f64> {
        vec![0.2, 0.02]
    }
}

/// Gaussian peak `Y = Baseline + Amplitude * exp(-0.5 * ((X - Mean) / SD)^2)` (unnormalised:
/// `Amplitude` is the peak height, not an area).
/// Reference: Gaussian line shape, Bevington & Robinson (2003) ch. 2.
struct GaussianCurve {
    meta: ModelMeta,
    with_baseline: bool,
}

impl ParametricModel for GaussianCurve {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let p_len = if self.with_baseline { 4 } else { 3 };
        if !x0.is_finite() || !all_finite_params(params, p_len) {
            return None;
        }
        let (base, amp, mean, sd) =
            if self.with_baseline { (params[0], params[1], params[2], params[3]) } else { (0.0, params[0], params[1], params[2]) };
        if sd.abs() < 1e-15 {
            return None;
        }
        let z = (x0 - mean) / sd;
        let y = base + amp * safe_exp(-0.5 * z * z)?;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let peak_x = points
            .iter()
            .filter(|p| p.x0().is_finite() && p.y.is_finite())
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .map_or(0.5 * (xmin + xmax), |p| p.x0());
        let sd = ((xmax - xmin) / 6.0).max(1e-3);
        if self.with_baseline { vec![ymin, ymax - ymin, peak_x, sd] } else { vec![ymax, peak_x, sd] }
    }
}

/// Sum of two unnormalised Gaussian peaks (no baseline).
/// Reference: Gaussian line shape, Bevington & Robinson (2003) ch. 2.
struct TwoGaussians;

impl ParametricModel for TwoGaussians {
    impl_static_meta!("sum_of_two_gaussians", "Sum of two Gaussians", EquationCategory::Peak, ["Amp1", "Mean1", "SD1", "Amp2", "Mean2", "SD2"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 6) {
            return None;
        }
        let (a1, m1, s1, a2, m2, s2) = (params[0], params[1], params[2], params[3], params[4], params[5]);
        if s1.abs() < 1e-15 || s2.abs() < 1e-15 {
            return None;
        }
        let z1 = (x0 - m1) / s1;
        let z2 = (x0 - m2) / s2;
        let y = a1 * safe_exp(-0.5 * z1 * z1)? + a2 * safe_exp(-0.5 * z2 * z2)?;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let raw_span = (xmax - xmin).max(0.0);
        let width = raw_span.max(1e-12) / 8.0;
        vec![ymax, xmin + 0.35 * raw_span, width, 0.5 * ymax, xmin + 0.65 * raw_span, width]
    }
}

/// Log-normal-shaped peak `Y = Amplitude * exp(-0.5 * (ln(X / GeoMean) / ln(GeoSD))^2)`,
/// parameterised by the geometric mean and the multiplicative (geometric) standard deviation.
/// Reference: Limpert, Stahel & Abbt (2001) (median `mu*` and multiplicative SD `sigma*` form).
struct LogGaussian;

impl ParametricModel for LogGaussian {
    impl_static_meta!("log_gaussian", "Log Gaussian", EquationCategory::Peak, ["Amplitude", "GeoMean", "GeoSD"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (amp, gmean, gsd) = (params[0], params[1], params[2]);
        if x0 <= 0.0 || gmean <= 0.0 || gsd <= 1.0 {
            return None;
        }
        let z = (x0 / gmean).ln() / gsd.ln();
        let y = amp * safe_exp(-0.5 * z * z)?;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, (xmin.max(1e-3) * xmax.max(1e-3)).sqrt(), 2.0]
    }
}

/// Cumulative Gaussian `Y = Bottom + (Top - Bottom) * Phi((X - Mean) / SD)`.
/// Reference: normal-sigmoid (probit) response curve, Finney (1971).
struct CumulativeGaussian;

impl ParametricModel for CumulativeGaussian {
    impl_static_meta!("cumulative_gaussian", "Cumulative Gaussian", EquationCategory::Gaussian, ["Bottom", "Top", "Mean", "SD"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (bottom, top, mean, sd) = (params[0], params[1], params[2], params[3]);
        if sd <= 0.0 {
            return None;
        }
        let z = (x0 - mean) / sd;
        if !z.is_finite() {
            return None;
        }
        let cdf = curvebook_math::dist::norm_cdf(z);
        let y = bottom + (top - bottom) * cdf;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymin, ymax, 0.5 * (xmin + xmax), ((xmax - xmin) / 6.0).max(1e-3)]
    }
}

/// Lorentzian (Cauchy) peak `Y = Amplitude / (1 + ((X - Center) / Width)^2)`, where
/// `Width` is the half-width at half maximum (HWHM) and derived `FWHM = 2 * Width`.
/// Reference: Lorentzian distribution, Bevington & Robinson (2003) ch. 2.
struct LorentzianPeak;

impl ParametricModel for LorentzianPeak {
    impl_static_meta!("lorentzian", "Lorentzian", EquationCategory::Peak, ["Amplitude", "Center", "Width"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (amp, center, width) = (params[0], params[1], params[2]);
        if width <= 0.0 {
            return None;
        }
        let z = (x0 - center) / width;
        let y = amp / (1.0 + z * z);
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (amp, center, width) = (params[0], params[1], params[2]);
        if width <= 0.0 {
            return false;
        }
        let z = (x0 - center) / width;
        let denom = 1.0 + z * z;
        let inv_d = 1.0 / denom;
        let inv_d2 = inv_d * inv_d;
        out[0] = inv_d;
        out[1] = 2.0 * amp * z * inv_d2 / width;
        out[2] = 2.0 * amp * z * z * inv_d2 / width;
        out[..3].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let peak_x = points
            .iter()
            .filter(|p| p.x0().is_finite() && p.y.is_finite())
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .map_or(0.5 * (xmin + xmax), |p| p.x0());
        vec![ymax, peak_x, ((xmax - xmin) / 8.0).max(1e-3)]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(1e-15), None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 && params[2].is_finite() && params[2] > 0.0 {
            push_finite_scalar(&mut out, "FWHM", 2.0 * params[2]);
        }
        out
    }
}

/// Scaled Weibull density `Y = Amplitude * (k / lambda) * (X / lambda)^(k - 1) * exp(-(X / lambda)^k)`
/// with `Scale = lambda`, `Shape = k` (`Amplitude` is the area under the curve).
/// Reference: Weibull (1951).
struct WeibullPeak;

impl ParametricModel for WeibullPeak {
    impl_static_meta!("weibull_peak", "Weibull density curve", EquationCategory::Peak, ["Amplitude", "Scale", "Shape"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (amp, scale, shape) = (params[0], params[1], params[2]);
        if !x0.is_finite() || x0 < 0.0 || !amp.is_finite() || !scale.is_finite() || scale <= 0.0 || !shape.is_finite() || shape <= 0.0 {
            return None;
        }
        if x0 == 0.0 {
            if (shape - 1.0).abs() <= 1e-15 {
                let y = amp / scale;
                return y.is_finite().then_some(y);
            }
            return (shape > 1.0).then_some(0.0);
        }
        let u = x0 / scale;
        if !u.is_finite() {
            return None;
        }
        let e = safe_exp(-u.powf(shape))?;
        let y = amp * (shape / scale) * u.powf(shape - 1.0) * e;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax * (xmax - xmin).max(1.0), xmax.abs().max(1.0) * 0.5, 2.0]
    }
}

/// Sine wave with nonzero baseline parameterized by `Wavelength`:
/// `Y = Amplitude * sin(2 * pi * X / Wavelength + PhaseShift) + Baseline` (`Wavelength > 0`,
/// `PhaseShift` in radians), with derived `Frequency = 1 / Wavelength`.
/// Reference: sinusoid fitting, Bloomfield (2000).
struct SineWave;

impl ParametricModel for SineWave {
    impl_static_meta!("sine_wave", "Sine wave", EquationCategory::SineWave, ["Amplitude", "Wavelength", "PhaseShift", "Baseline"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (amp, wavelength, phase, base) = (params[0], params[1], params[2], params[3]);
        if wavelength <= 0.0 {
            return None;
        }
        let y = base + amp * (2.0 * core::f64::consts::PI * t / wavelength + phase).sin();
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 4) || out.len() < 4 {
            return false;
        }
        let (amp, wavelength, phase) = (params[0], params[1], params[2]);
        if wavelength <= 0.0 {
            return false;
        }
        let angle = 2.0 * core::f64::consts::PI * t / wavelength + phase;
        let (s, c) = angle.sin_cos();
        out[0] = s;
        out[1] = -amp * c * (2.0 * core::f64::consts::PI * t) / (wavelength * wavelength);
        out[2] = amp * c;
        out[3] = 1.0;
        out[..4].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let amp = 0.5 * (ymax - ymin);
        let base = 0.5 * (ymax + ymin);
        let wavelength = (xmax - xmin).max(1.0);
        vec![amp, wavelength, 0.0, base]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(1e-15), None), (None, None), (None, None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[1].is_finite() && params[1] > 0.0 {
            push_finite_scalar(&mut out, "Frequency", 1.0 / params[1]);
        }
        out
    }
}

/// Damped sine wave parameterized by `Wavelength` (canonical zero-baseline form):
/// `Y = Amplitude * exp(-K * X) * sin(2 * pi * X / Wavelength + PhaseShift)` (`Wavelength > 0`,
/// `PhaseShift` in radians), with derived `Frequency = 1 / Wavelength` and `HalfLife = ln(2) / K`.
/// Reference: Bloomfield (2000); under-damped linear oscillator decay.
struct DampedSineWave;

impl ParametricModel for DampedSineWave {
    impl_static_meta!("damped_sine_wave", "Damped sine wave", EquationCategory::SineWave, ["Amplitude", "K", "Wavelength", "PhaseShift"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (amp, k, wavelength, phase) = (params[0], params[1], params[2], params[3]);
        if wavelength <= 0.0 {
            return None;
        }
        let e = safe_exp(-k * t)?;
        let y = amp * e * (2.0 * core::f64::consts::PI * t / wavelength + phase).sin();
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 4) || out.len() < 4 {
            return false;
        }
        let (amp, k, wavelength, phase) = (params[0], params[1], params[2], params[3]);
        if wavelength <= 0.0 {
            return false;
        }
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        let angle = 2.0 * core::f64::consts::PI * t / wavelength + phase;
        let (s, c) = angle.sin_cos();
        out[0] = e * s;
        out[1] = -t * amp * e * s;
        out[2] = -amp * e * c * (2.0 * core::f64::consts::PI * t) / (wavelength * wavelength);
        out[3] = amp * e * c;
        out[..4].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let amp = ymax.abs().max(ymin.abs()).max(1.0);
        let span = (xmax - xmin).max(1.0);
        vec![amp, 0.1 / span, span, 0.0]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(1e-15), None), (None, None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 {
            if params[2].is_finite() && params[2] > 0.0 {
                push_finite_scalar(&mut out, "Frequency", 1.0 / params[2]);
            }
            if params[1].is_finite() && params[1] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife", core::f64::consts::LN_2 / params[1]);
            }
        }
        out
    }
}

/// Sinc wave parameterized by `Wavelength`: `Y = Amplitude * sin(u) / u` with
/// `u = 2 * pi * X / Wavelength` (`Y = Amplitude` at `u = 0`), with derived `Frequency = 1 / Wavelength`.
/// Reference: Bracewell (2000).
struct SincWave;

impl ParametricModel for SincWave {
    impl_static_meta!("sinc_wave", "Sinc function", EquationCategory::SineWave, ["Amplitude", "Wavelength"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let (amp, wavelength) = (params[0], params[1]);
        if wavelength <= 0.0 {
            return None;
        }
        let u = 2.0 * core::f64::consts::PI * x0 / wavelength;
        if !u.is_finite() {
            return None;
        }
        let s = if u.abs() < 1e-12 { 1.0 } else { u.sin() / u };
        let y = amp * s;
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 2) || out.len() < 2 {
            return false;
        }
        let (amp, wavelength) = (params[0], params[1]);
        if wavelength <= 0.0 {
            return false;
        }
        let u = 2.0 * core::f64::consts::PI * x0 / wavelength;
        if !u.is_finite() {
            return false;
        }
        let s = if u.abs() < 1e-12 { 1.0 } else { u.sin() / u };
        let ds_du = if u.abs() < 1e-6 { -u / 3.0 + (u * u * u) / 30.0 } else { (u * u.cos() - u.sin()) / (u * u) };
        out[0] = s;
        out[1] = amp * ds_du * (-u / wavelength);
        out[..2].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let amp = ymax.abs().max(ymin.abs()).max(1.0);
        vec![amp, ((xmax - xmin) / 2.0).max(1e-3)]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(1e-15), None)]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[1].is_finite() && params[1] > 0.0 {
            push_finite_scalar(&mut out, "Frequency", 1.0 / params[1]);
        }
        out
    }
}

/// Straight line `Y = Slope * X + YIntercept` (or through the origin).
/// Reference: Draper & Smith (1998).
struct StraightLine {
    meta: ModelMeta,
    through_origin: bool,
}

impl ParametricModel for StraightLine {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let p_len = if self.through_origin { 1 } else { 2 };
        if !x0.is_finite() || !all_finite_params(params, p_len) {
            return None;
        }
        let y = if self.through_origin { params[0] * x0 } else { params[0] * x0 + params[1] };
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], _params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        let p_len = if self.through_origin { 1 } else { 2 };
        if !x0.is_finite() || out.len() < p_len {
            return false;
        }
        out[0] = x0;
        if !self.through_origin {
            out[1] = 1.0;
        }
        true
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let slope = if (xmax - xmin).abs() > 1e-12 { (ymax - ymin) / (xmax - xmin) } else { 1.0 };
        if self.through_origin { vec![slope] } else { vec![slope, ymin - slope * xmin] }
    }
}

/// Two-segment continuous linear regression with join point `X0`.
/// Reference: Hudson (1966); Seber & Wild (1989) ch. 9.
struct SegmentalLinear;

impl ParametricModel for SegmentalLinear {
    impl_static_meta!("segmental_linear", "Segmental linear regression", EquationCategory::StandardCurve, ["Intercept1", "Slope1", "X0", "Slope2"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (b1, s1, x_join, s2) = (params[0], params[1], params[2], params[3]);
        let y = if x0 < x_join { b1 + s1 * x0 } else { b1 + s1 * x_join + s2 * (x0 - x_join) };
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, _) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymin, 1.0, 0.5 * (xmin + xmax), 0.1]
    }
}

/// Two-regime straight lines joined by a hyperbolic transition:
/// `Y = Y0 + Slope1 * dx + (Slope2 - Slope1) / 2 * (dx + sqrt(dx^2 + Delta^2) - |Delta|)`,
/// `dx = X - X0`, so `Y(X0) = Y0` and `Delta -> 0` recovers the sharp hinge.
/// Reference: Watts & Bacon (1974) hyperbola transition model, reparameterised so that `Y0` is the
/// value at the join; see also Seber & Wild (1989) ch. 9.
struct SmoothHinge;

impl ParametricModel for SmoothHinge {
    impl_static_meta!(
        "hinge_smooth_hyperbolic",
        "Hinge (smooth hyperbolic transition)",
        EquationCategory::StandardCurve,
        ["Y0", "X0", "Slope1", "Slope2", "Delta"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 5) {
            return None;
        }
        let (y0, x_join, s1, s2, delta) = (params[0], params[1], params[2], params[3], params[4]);
        let dx = x0 - x_join;
        let smooth = dx.hypot(delta) - delta.abs();
        let y = y0 + s1 * dx + 0.5 * (s2 - s1) * (dx + smooth);
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, _) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymin, 0.5 * (xmin + xmax), 1.0, 0.1, ((xmax - xmin) * 0.05).max(1e-3)]
    }
}

/// Polynomial `Y = B0 + B1 * X + ... + Bd * X^d` (degree 2–6), linear in its parameters.
/// Reference: polynomial regression, Draper & Smith (1998).
struct Polynomial {
    meta: ModelMeta,
    degree: usize,
}

impl Polynomial {
    fn new(degree: usize) -> Self {
        let (id, name) = match degree {
            2 => ("polynomial_2", "Second order polynomial (quadratic)"),
            3 => ("polynomial_3", "Third order polynomial (cubic)"),
            4 => ("polynomial_4", "Fourth order polynomial"),
            5 => ("polynomial_5", "Fifth order polynomial"),
            _ => ("polynomial_6", "Sixth order polynomial"),
        };
        let params = (0..=degree).map(|k| format!("B{k}")).collect();
        Self { meta: ModelMeta { id, name, category: EquationCategory::Polynomial, params }, degree }
    }
}

impl ParametricModel for Polynomial {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, self.degree + 1) {
            return None;
        }
        let mut acc = 0.0;
        for &c in params[..=self.degree].iter().rev() {
            acc = acc * x0 + c;
        }
        acc.is_finite().then_some(acc)
    }
    fn grad(&self, x: &[f64], _params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || out.len() < self.degree + 1 {
            return false;
        }
        let mut xp = 1.0;
        for slot in &mut out[..=self.degree] {
            *slot = xp;
            xp *= x0;
        }
        out[..=self.degree].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, _) = data_y_min_max(points);
        let mut v = vec![0.0; self.degree + 1];
        v[0] = ymin;
        v
    }
}

/// Power law `Y = a * X^b` (allometric form; `"power_series"` is retained as a backwards-compatible
/// lookup alias for persisted workspaces).
/// Reference: Huxley (1932).
struct PowerCurve;

impl ParametricModel for PowerCurve {
    impl_static_meta!("power_law", "Power law (Y = a * X^b)", EquationCategory::StandardCurve, ["a", "b"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 < 0.0 || !all_finite_params(params, 2) {
            return None;
        }
        let val = params[0] * x0.powf(params[1]);
        val.is_finite().then_some(val)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax.max(1.0), 1.0]
    }
}

/// Logarithmic curve `Y = a * ln(X) + b` (linear regression on transformed `X`).
/// Reference: Draper & Smith (1998).
struct LogarithmicCurve;

impl ParametricModel for LogarithmicCurve {
    impl_static_meta!("logarithmic_curve", "Logarithmic curve (Y = a * ln(X) + b)", EquationCategory::StandardCurve, ["a", "b"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 <= 0.0 || !all_finite_params(params, 2) {
            return None;
        }
        let y = params[0] * x0.ln() + params[1];
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, _) = data_y_min_max(points);
        vec![1.0, ymin]
    }
}

fn estimate_ols_from_pairs(pairs: &[(f64, f64)]) -> (f64, f64) {
    if pairs.is_empty() {
        return (1.0, 0.0);
    }
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|&(x, _)| x).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|&(_, y)| y).sum::<f64>() / n;
    let mut sxx = 0.0_f64;
    let mut sxy = 0.0_f64;
    for &(x, y) in pairs {
        let dx = x - mean_x;
        sxx += dx * dx;
        sxy += dx * (y - mean_y);
    }
    let slope = if sxx > 1e-30 { sxy / sxx } else { 0.0 };
    let intercept = mean_y - slope * mean_x;
    (slope, intercept)
}

fn estimate_centered_linear_seed(points: &[DataPoint]) -> (f64, f64, f64) {
    let pairs: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
    if pairs.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|&(x, _)| x).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|&(_, y)| y).sum::<f64>() / n;
    let (slope, _) = estimate_ols_from_pairs(&pairs);
    (mean_x, mean_y, slope)
}

/// Centered polynomial model: `Y = sum_{k=0..degree} B_k * (X - XCenter)^k`.
/// Reference: centred polynomial regression, Draper & Smith (1998). `XCenter` is meant to be held
/// fixed: with `XCenter` free the model is over-parameterised (same function space as the
/// uncentred polynomial of the same degree).
struct CenteredPolynomial {
    meta: ModelMeta,
    degree: usize,
}

impl CenteredPolynomial {
    fn new(degree: usize) -> Self {
        let clamped = degree.clamp(1, 6);
        let (id, name) = match clamped {
            1 => ("centered_poly_1", "Centered polynomial (order 1)"),
            2 => ("centered_poly_2", "Centered polynomial (order 2)"),
            3 => ("centered_poly_3", "Centered polynomial (order 3)"),
            4 => ("centered_poly_4", "Centered polynomial (order 4)"),
            5 => ("centered_poly_5", "Centered polynomial (order 5)"),
            _ => ("centered_poly_6", "Centered polynomial (order 6)"),
        };
        let mut params: Vec<String> = (0..=clamped).map(|k| format!("B{k}")).collect();
        params.push("XCenter".to_owned());
        Self { meta: ModelMeta { id, name, category: EquationCategory::StandardCurve, params }, degree: clamped }
    }
}

impl ParametricModel for CenteredPolynomial {
    impl_meta_methods!();

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let x_center = *params.get(self.degree + 1)?;
        let coeffs = params.get(..=self.degree)?;
        if !x0.is_finite() || !x_center.is_finite() || !coeffs.iter().all(|v| v.is_finite()) {
            return None;
        }
        let dx = x0 - x_center;
        let mut acc = 0.0_f64;
        for &coeff in coeffs.iter().rev() {
            acc = acc * dx + coeff;
        }
        acc.is_finite().then_some(acc)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        let Some(&x_center) = params.get(self.degree + 1) else {
            return false;
        };
        if !x0.is_finite() || !x_center.is_finite() || params.len() < self.degree + 2 || out.len() < self.degree + 2 {
            return false;
        }
        let dx = x0 - x_center;
        let mut pow = 1.0_f64;
        for slot in &mut out[..=self.degree] {
            *slot = pow;
            pow *= dx;
        }
        let mut d_dx = 0.0_f64;
        for (k, &coeff) in params[1..=self.degree].iter().enumerate().rev() {
            d_dx = d_dx * dx + (k + 1) as f64 * coeff;
        }
        out[self.degree + 1] = -d_dx;
        out[..self.degree + 2].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (mean_x, mean_y, slope) = estimate_centered_linear_seed(points);
        let mut guess = vec![0.0_f64; self.degree + 2];
        guess[0] = mean_y;
        guess[1] = slope;
        guess[self.degree + 1] = mean_x;
        guess
    }

    fn default_parameter_rules(&self, points: &[DataPoint]) -> Vec<ParameterRule> {
        let (mean_x, _, _) = estimate_centered_linear_seed(points);
        vec![ParameterRule {
            name: "XCenter".to_owned(),
            scope: ParameterScope::Constant(mean_x),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        }]
    }
}

/// Semi-logarithmic standard curve: `Y = Slope * log10(X) + YIntercept`.
/// Reference: linear regression on transformed `X`, Draper & Smith (1998).
struct SemiLogLine;

impl ParametricModel for SemiLogLine {
    impl_static_meta!("semi_log_line", "Semi-log line (Y vs log10(X))", EquationCategory::StandardCurve, ["Slope", "YIntercept"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 <= 0.0 || !all_finite_params(params, 2) {
            return None;
        }
        let y = params[0] * x0.log10() + params[1];
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || x0 <= 0.0 || params.len() < 2 || out.len() < 2 {
            return false;
        }
        let lx = x0.log10();
        out[0] = lx;
        out[1] = 1.0;
        lx.is_finite()
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let pairs: Vec<(f64, f64)> =
            points.iter().filter(|p| p.x0() > 0.0 && p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0().log10(), p.y)).collect();
        let (slope, intercept) = estimate_ols_from_pairs(&pairs);
        vec![slope, intercept]
    }
}

/// Power-law standard curve in log10-log10 form: `Y = 10^(Slope * log10(X) + LogYIntercept)`.
/// Reference: the allometric power law of Huxley (1932) written in log-log form.
struct LogLogLine;

impl ParametricModel for LogLogLine {
    impl_static_meta!("log_log_line", "Log-log regression line", EquationCategory::StandardCurve, ["Slope", "LogYIntercept"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 <= 0.0 || !all_finite_params(params, 2) {
            return None;
        }
        let exponent = params[0] * x0.log10() + params[1];
        let y = 10.0_f64.powf(exponent);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || x0 <= 0.0 || params.len() < 2 || out.len() < 2 {
            return false;
        }
        let Some(y) = self.eval(x, params, 0) else {
            return false;
        };
        out[0] = y * x0.ln();
        out[1] = y * core::f64::consts::LN_10;
        out[0].is_finite() && out[1].is_finite()
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let pairs: Vec<(f64, f64)> = points
            .iter()
            .filter(|p| p.x0() > 0.0 && p.y > 0.0 && p.x0().is_finite() && p.y.is_finite())
            .map(|p| (p.x0().log10(), p.y.log10()))
            .collect();
        let (slope, log_intercept) = estimate_ols_from_pairs(&pairs);
        vec![slope, log_intercept]
    }
}

/// Probit percentage curve: `Y = NormalCdf(Slope * X + Intercept) * 100`.
/// Reference: Finney (1971). Uses the modern normal-equivalent-deviate scale (no classical `+5`
/// probit offset).
struct ProbitLine;

impl ParametricModel for ProbitLine {
    impl_static_meta!("probit_line", "Probit curve", EquationCategory::StandardCurve, ["Slope", "Intercept"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let z = params[0] * x0 + params[1];
        if !z.is_finite() {
            return None;
        }
        let y = curvebook_math::dist::norm_cdf(z) * 100.0;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || params.len() < 2 || out.len() < 2 {
            return false;
        }
        let z = params[0] * x0 + params[1];
        if !z.is_finite() {
            return false;
        }
        let scale = 100.0 * curvebook_math::dist::norm_pdf(z);
        out[0] = scale * x0;
        out[1] = scale;
        out[0].is_finite() && out[1].is_finite()
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let pairs: Vec<(f64, f64)> = points
            .iter()
            .filter(|p| p.x0().is_finite() && p.y.is_finite())
            .filter_map(|p| {
                let prob = (p.y / 100.0).clamp(0.01, 0.99);
                curvebook_math::dist::norm_ppf(prob).ok().map(|z| (p.x0(), z))
            })
            .collect();
        let (slope, intercept) = estimate_ols_from_pairs(&pairs);
        vec![slope, intercept]
    }
}

/// Two-phase (bi-exponential) association:
/// `Y = Y0 + (Plateau - Y0) * [p_fast * (1 - exp(-KFast * X)) + (1 - p_fast) * (1 - exp(-KSlow * X))]`,
/// where `p_fast = 0.01 * PercentFast`.
///
/// Reference: two-compartment sum-of-exponentials model, Seber & Wild (1989) ch. 8;
/// `PercentFast` parameterisation per Motulsky & Christopoulos (2004).
struct TwoPhaseAssociation;

impl ParametricModel for TwoPhaseAssociation {
    impl_static_meta!(
        "two_phase_association",
        "Two phase association",
        EquationCategory::Exponential,
        ["Y0", "Plateau", "PercentFast", "KFast", "KSlow"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || params.len() < 5 || !params[..5].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [y0, plateau, pct_fast, k_fast, k_slow] = [params[0], params[1], params[2], params[3], params[4]];
        let span = plateau - y0;
        let f = pct_fast * 0.01;
        let e1 = safe_exp(-k_fast * t)?;
        let e2 = safe_exp(-k_slow * t)?;
        let y = y0 + span * (f * (1.0 - e1) + (1.0 - f) * (1.0 - e2));
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || params.len() < 5 || out.len() < 5 || !params[..5].iter().all(|v| v.is_finite()) {
            return false;
        }
        let [y0, plateau, pct_fast, k_fast, k_slow] = [params[0], params[1], params[2], params[3], params[4]];
        let (Some(e1), Some(e2)) = (safe_exp(-k_fast * t), safe_exp(-k_slow * t)) else {
            return false;
        };
        let span = plateau - y0;
        let f = pct_fast * 0.01;
        let assoc = f * (1.0 - e1) + (1.0 - f) * (1.0 - e2);
        out[0] = 1.0 - assoc;
        out[1] = assoc;
        out[2] = span * 0.01 * (e2 - e1);
        out[3] = span * f * t * e1;
        out[4] = span * (1.0 - f) * t * e2;
        out[..5].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span_x = (xmax - xmin).max(1.0);
        vec![ymin, ymax, 50.0, 4.0 / span_x, 0.5 / span_x]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), Some(100.0)), (Some(0.0), None), (Some(0.0), None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 5 {
            push_finite_scalar(&mut out, "Span", params[1] - params[0]);
            if params[3] > 0.0 && params[3].is_finite() {
                push_finite_scalar(&mut out, "TauFast", 1.0 / params[3]);
                push_finite_scalar(&mut out, "HalfTimeFast", core::f64::consts::LN_2 / params[3]);
            }
            if params[4] > 0.0 && params[4].is_finite() {
                push_finite_scalar(&mut out, "TauSlow", 1.0 / params[4]);
                push_finite_scalar(&mut out, "HalfTimeSlow", core::f64::consts::LN_2 / params[4]);
            }
        }
        out
    }
}

/// Natural log of exponential growth: `Y = LnY0 + k * X`, appropriate for natural-log-transformed
/// population counts with homoscedastic log-space scatter.
///
/// Reference: Malthus (1798); log-linear growth regression per Draper & Smith (1998).
struct LogExponentialGrowth;

impl ParametricModel for LogExponentialGrowth {
    impl_static_meta!("log_exponential_growth", "Log of exponential growth", EquationCategory::Exponential, ["LnY0", "k"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || params.len() < 2 || !params[0].is_finite() || !params[1].is_finite() {
            return None;
        }
        let y = params[0] + params[1] * t;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], _params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || out.len() < 2 {
            return false;
        }
        out[0] = 1.0;
        out[1] = t;
        true
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let pairs: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
        let (slope, intercept) = estimate_ols_from_pairs(&pairs);
        vec![intercept, slope]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 {
            if let Some(y0) = safe_exp(params[0]) {
                push_finite_scalar(&mut out, "Y0", y0);
            }
            if params[1].is_finite() && params[1] > 0.0 {
                push_finite_scalar(&mut out, "DoublingTime", core::f64::consts::LN_2 / params[1]);
            }
        }
        out
    }
}

/// Boltzmann voltage-gated conductance sigmoid:
/// `Y = Bottom + (Top - Bottom) / (1 + exp((V50 - X) / Slope))`,
/// where `V50` is the half-activation membrane potential and `Slope` is the slope factor (`RT / (z F)`).
///
/// Reference: Hodgkin & Huxley (1952).
struct BoltzmannSigmoid;

impl ParametricModel for BoltzmannSigmoid {
    impl_static_meta!("boltzmann_sigmoid", "Boltzmann sigmoidal", EquationCategory::Growth, ["Bottom", "Top", "V50", "Slope"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let v = x.first().copied()?;
        if !v.is_finite() || params.len() < 4 || !params[..4].iter().all(|p| p.is_finite()) {
            return None;
        }
        let [bottom, top, v50, slope] = [params[0], params[1], params[2], params[3]];
        if slope.abs() < 1e-15 {
            return None;
        }
        let arg = (v50 - v) / slope;
        if !arg.is_finite() {
            return None;
        }
        let sig = if arg > 700.0 {
            0.0
        } else if arg < -700.0 {
            1.0
        } else {
            1.0 / (1.0 + arg.exp())
        };
        let y = bottom + (top - bottom) * sig;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&v) = x.first() else {
            return false;
        };
        if !v.is_finite() || params.len() < 4 || out.len() < 4 || !params[..4].iter().all(|p| p.is_finite()) {
            return false;
        }
        let [bottom, top, v50, slope] = [params[0], params[1], params[2], params[3]];
        if slope.abs() < 1e-15 {
            return false;
        }
        let arg = (v50 - v) / slope;
        if !arg.is_finite() {
            return false;
        }
        let (sig, dsig) = if arg > 700.0 {
            (0.0, 0.0)
        } else if arg < -700.0 {
            (1.0, 0.0)
        } else {
            let e = arg.exp();
            let s = 1.0 / (1.0 + e);
            let ds = if e <= 1.0 {
                e * s * s
            } else {
                let inv = 1.0 / e;
                (inv / (1.0 + inv)) / (1.0 + inv)
            };
            (s, ds)
        };
        let span = top - bottom;
        out[0] = 1.0 - sig;
        out[1] = sig;
        out[2] = -span * dsig / slope;
        out[3] = span * dsig * arg / slope;
        out[..4].iter().all(|g| g.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let v50 = crate::library::estimate_x_at_ymid(points);
        let slope = ((xmax - xmin) / 8.0).max(1e-3);
        vec![ymin, ymax, v50, slope]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[0].is_finite() && params[1].is_finite() {
            push_finite_scalar(&mut out, "Span", params[1] - params[0]);
        }
        out
    }
}

#[derive(Clone, Copy)]
enum LinearQuadraticMode {
    PercentSurviving,
    NumberSurviving,
    FractionDead,
    PercentDead,
    NumberDead,
}

/// Linear–quadratic radiation cell-survival and cell-death response family with
/// `S(D) = exp(-(Alpha * D + Beta * D^2))`:
/// - `linear_quadratic_percent_surviving`: `100 * S(D)`
/// - `linear_quadratic_number_surviving`: `Y0 * S(D)`
/// - `linear_quadratic_fraction_dead`: `1 - S(D)`
/// - `linear_quadratic_percent_dead`: `100 * (1 - S(D))`
/// - `linear_quadratic_number_dead`: `Y0 * (1 - S(D))`
///
/// Reference: Chadwick & Leenhouts (1973); Fowler (1989); Brenner (2008).
struct LinearQuadraticVariant {
    meta: ModelMeta,
    mode: LinearQuadraticMode,
}

impl ParametricModel for LinearQuadraticVariant {
    impl_meta_methods!();

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let d = x.first().copied()?;
        if !d.is_finite() || params.len() < self.meta.params.len() || !params[..self.meta.params.len()].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (scale, alpha, beta, is_dead) = match self.mode {
            LinearQuadraticMode::PercentSurviving => (100.0, params[0], params[1], false),
            LinearQuadraticMode::NumberSurviving => (params[0], params[1], params[2], false),
            LinearQuadraticMode::FractionDead => (1.0, params[0], params[1], true),
            LinearQuadraticMode::PercentDead => (100.0, params[0], params[1], true),
            LinearQuadraticMode::NumberDead => (params[0], params[1], params[2], true),
        };
        let exponent = -(alpha * d + beta * d * d);
        if !exponent.is_finite() || exponent > 700.0 {
            return None;
        }
        let y = if is_dead { scale * (-exponent.exp_m1()) } else { scale * safe_exp(exponent)? };
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&d) = x.first() else {
            return false;
        };
        let p_len = self.meta.params.len();
        if !d.is_finite() || params.len() < p_len || out.len() < p_len || !params[..p_len].iter().all(|v| v.is_finite()) {
            return false;
        }
        let (scale, alpha, beta, has_y0, is_dead) = match self.mode {
            LinearQuadraticMode::PercentSurviving => (100.0, params[0], params[1], false, false),
            LinearQuadraticMode::NumberSurviving => (params[0], params[1], params[2], true, false),
            LinearQuadraticMode::FractionDead => (1.0, params[0], params[1], false, true),
            LinearQuadraticMode::PercentDead => (100.0, params[0], params[1], false, true),
            LinearQuadraticMode::NumberDead => (params[0], params[1], params[2], true, true),
        };
        let exponent = -(alpha * d + beta * d * d);
        let Some(s) = safe_exp(exponent) else {
            return false;
        };
        let sign = if is_dead { 1.0 } else { -1.0 };
        if has_y0 {
            out[0] = if is_dead { -exponent.exp_m1() } else { s };
            out[1] = sign * scale * d * s;
            out[2] = sign * scale * d * d * s;
        } else {
            out[0] = sign * scale * d * s;
            out[1] = sign * scale * d * d * s;
        }
        out[..p_len].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        match self.mode {
            LinearQuadraticMode::NumberSurviving | LinearQuadraticMode::NumberDead => vec![ymax.max(1.0), 0.2, 0.02],
            LinearQuadraticMode::PercentSurviving | LinearQuadraticMode::FractionDead | LinearQuadraticMode::PercentDead => vec![0.2, 0.02],
        }
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        match self.mode {
            LinearQuadraticMode::NumberSurviving | LinearQuadraticMode::NumberDead => vec![(Some(0.0), None), (None, None), (None, None)],
            LinearQuadraticMode::PercentSurviving | LinearQuadraticMode::FractionDead | LinearQuadraticMode::PercentDead => {
                vec![(None, None), (None, None)]
            }
        }
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let (alpha, beta) = match self.mode {
            LinearQuadraticMode::NumberSurviving | LinearQuadraticMode::NumberDead => {
                if params.len() < 3 {
                    return Vec::new();
                }
                (params[1], params[2])
            }
            _ => {
                if params.len() < 2 {
                    return Vec::new();
                }
                (params[0], params[1])
            }
        };
        let mut out = Vec::new();
        if alpha.is_finite() && beta.is_finite() && beta.abs() > 1e-15 {
            push_finite_scalar(&mut out, "AlphaBetaRatio", alpha / beta);
        }
        out
    }
}

/// Area-parameterised Gaussian peak:
/// `Y = (Area / (SD * sqrt(2 * pi))) * exp(-0.5 * ((X - Mean) / SD)^2)`.
///
/// Reference: Bevington & Robinson (2003) ch. 2; Motulsky & Christopoulos (2004).
struct GaussianArea;

impl ParametricModel for GaussianArea {
    impl_static_meta!("gaussian_area", "Gaussian (area)", EquationCategory::Gaussian, ["Area", "Mean", "SD"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || params.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [area, mean, sd] = [params[0], params[1], params[2]];
        if sd <= 0.0 {
            return None;
        }
        let z = (x0 - mean) / sd;
        let e = safe_exp(-0.5 * z * z)?;
        let inv_sqrt_2pi = 1.0 / (2.0 * core::f64::consts::PI).sqrt();
        let y = (area * inv_sqrt_2pi / sd) * e;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || params.len() < 3 || out.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return false;
        }
        let [area, mean, sd] = [params[0], params[1], params[2]];
        if sd <= 0.0 {
            return false;
        }
        let z = (x0 - mean) / sd;
        let Some(e) = safe_exp(-0.5 * z * z) else {
            return false;
        };
        let inv_sqrt_2pi = 1.0 / (2.0 * core::f64::consts::PI).sqrt();
        let phi = (inv_sqrt_2pi / sd) * e;
        let y = area * phi;
        out[0] = phi;
        out[1] = y * z / sd;
        out[2] = y * (z * z - 1.0) / sd;
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let peak_x = points
            .iter()
            .filter(|p| p.x0().is_finite() && p.y.is_finite())
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .map_or(0.5 * (xmin + xmax), |p| p.x0());
        let sd = ((xmax - xmin) / 6.0).max(1e-3);
        let area = ymax * sd * (2.0 * core::f64::consts::PI).sqrt();
        vec![area, peak_x, sd]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(1e-15), None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 && params[0].is_finite() && params[2].is_finite() && params[2] > 0.0 {
            let amp = params[0] / (params[2] * (2.0 * core::f64::consts::PI).sqrt());
            let fwhm = 2.0 * (2.0 * core::f64::consts::LN_2).sqrt() * params[2];
            push_finite_scalar(&mut out, "Amplitude", amp);
            push_finite_scalar(&mut out, "Area", params[0]);
            push_finite_scalar(&mut out, "FWHM", fwhm);
        }
        out
    }
}

/// Area-scaled lognormal peak:
/// `Y = (A / X) * exp(-0.5 * (ln(X / GeoMean) / ln(GeoSD))^2)` (`X > 0`, `GeoMean > 0`, `GeoSD > 1`),
/// with integrated area `Area = A * sqrt(2 * pi) * ln(GeoSD)` and peak amplitude at
/// `Mode = GeoMean * exp(-ln(GeoSD)^2)`.
///
/// Reference: Limpert, Stahel & Abbt (2001).
struct LognormalArea;

impl ParametricModel for LognormalArea {
    impl_static_meta!("lognormal_area", "Lognormal (area)", EquationCategory::Peak, ["A", "GeoMean", "GeoSD"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 <= 0.0 || params.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [a, gmean, gsd] = [params[0], params[1], params[2]];
        if gmean <= 0.0 || gsd <= 1.0 {
            return None;
        }
        let sigma = gsd.ln();
        let z = (x0 / gmean).ln() / sigma;
        let e = safe_exp(-0.5 * z * z)?;
        let y = (a / x0) * e;
        y.is_finite().then_some(y)
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let gmean = (xmin.max(1e-3) * xmax.max(1e-3)).sqrt();
        vec![ymax * gmean, gmean, 2.0]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(1e-15), None), (Some(1.0 + 1e-9), None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 && params[0].is_finite() && params[1].is_finite() && params[2].is_finite() && params[1] > 0.0 && params[2] > 1.0 {
            let sigma = params[2].ln();
            let area = params[0] * (2.0 * core::f64::consts::PI).sqrt() * sigma;
            let mode = params[1] * (-sigma * sigma).exp();
            let amp = (params[0] / params[1]) * (0.5 * sigma * sigma).exp();
            if amp.is_finite() {
                out.push(DerivedScalar::new("Amplitude", amp, None));
            }
            if area.is_finite() {
                out.push(DerivedScalar::new("Area", area, None));
            }
            if mode.is_finite() {
                out.push(DerivedScalar::new("Mode", mode, None));
            }
        }
        out
    }
}

/// Zero-baseline three-parameter standard sine wave:
/// `Y = Amplitude * sin(2 * pi * X / Wavelength + PhaseShift)` (`Wavelength > 0`, `PhaseShift` in radians).
///
/// Reference: Bloomfield (2000).
struct StandardSineWave;

impl ParametricModel for StandardSineWave {
    impl_static_meta!(
        "standard_sine_wave",
        "Standard sine wave (zero baseline)",
        EquationCategory::SineWave,
        ["Amplitude", "Wavelength", "PhaseShift"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || params.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [amp, wavelength, phase] = [params[0], params[1], params[2]];
        if wavelength <= 0.0 {
            return None;
        }
        let y = amp * (2.0 * core::f64::consts::PI * t / wavelength + phase).sin();
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || params.len() < 3 || out.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return false;
        }
        let [amp, wavelength, phase] = [params[0], params[1], params[2]];
        if wavelength <= 0.0 {
            return false;
        }
        let angle = 2.0 * core::f64::consts::PI * t / wavelength + phase;
        let (s, c) = angle.sin_cos();
        out[0] = s;
        out[1] = -amp * c * (2.0 * core::f64::consts::PI * t) / (wavelength * wavelength);
        out[2] = amp * c;
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let amp = ymax.abs().max(ymin.abs()).max(1.0);
        let wavelength = (xmax - xmin).max(1.0);
        vec![amp, wavelength, 0.0]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(1e-15), None), (None, None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[1].is_finite() && params[1] > 0.0 {
            push_finite_scalar(&mut out, "Frequency", 1.0 / params[1]);
        }
        out
    }
}

/// Global intersection point of two (or more) straight lines:
/// `Y = YCross + Slope_d * (X - XCross)`, with globally shared `(XCross, YCross)` and local `Slope`
/// per dataset, yielding direct SEs and profile-likelihood CIs for the crossing coordinates.
///
/// Reference: Draper & Smith (1998); Seber & Wild (1989) ch. 9.
struct TwoLinesIntersection;

impl ParametricModel for TwoLinesIntersection {
    impl_static_meta!("two_lines_intersection", "Two lines intersection", EquationCategory::StandardCurve, ["XCross", "YCross", "Slope"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || params.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [x_cross, y_cross, slope] = [params[0], params[1], params[2]];
        let y = y_cross + slope * (x0 - x_cross);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || params.len() < 3 || out.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return false;
        }
        out[0] = -params[2];
        out[1] = 1.0;
        out[2] = x0 - params[0];
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let first_ds = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| p.data_set_index).min();
        if let Some(ds_a) = first_ds {
            let ds0: Vec<(f64, f64)> =
                points.iter().filter(|p| p.data_set_index == ds_a && p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
            let second_ds =
                points.iter().filter(|p| p.data_set_index > ds_a && p.x0().is_finite() && p.y.is_finite()).map(|p| p.data_set_index).min();
            let ds1: Vec<(f64, f64)> = second_ds.map_or_else(Vec::new, |ds_b| {
                points.iter().filter(|p| p.data_set_index == ds_b && p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect()
            });
            if ds0.len() >= 2 && ds1.len() >= 2 {
                let (m0, b0) = estimate_ols_from_pairs(&ds0);
                let (m1, b1) = estimate_ols_from_pairs(&ds1);
                let dm = m0 - m1;
                if dm.abs() > 1e-12 && dm.is_finite() {
                    let xc = (b1 - b0) / dm;
                    let yc = m0 * xc + b0;
                    if xc.is_finite() && yc.is_finite() {
                        return vec![xc, yc, m0];
                    }
                }
            }
        }
        let pairs: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
        let (slope, intercept) = estimate_ols_from_pairs(&pairs);
        let (xmin, xmax) = data_x_min_max(points);
        let x_mid = 0.5 * (xmin + xmax);
        vec![x_mid, intercept + slope * x_mid, slope]
    }

    fn default_parameter_rules(&self, _points: &[DataPoint]) -> Vec<ParameterRule> {
        vec![
            ParameterRule {
                name: "XCross".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "YCross".to_owned(),
                scope: ParameterScope::SharedGlobal,
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ]
    }
}

#[inline]
fn safe_real_pow(base: f64, exp: f64) -> Option<f64> {
    if !base.is_finite() || !exp.is_finite() {
        return None;
    }
    if base == 0.0 {
        if exp < 0.0 {
            return None;
        }
        return Some(if exp == 0.0 { 1.0 } else { 0.0 });
    }
    if base < 0.0 {
        let rounded = exp.round();
        if (exp - rounded).abs() > 1e-12 || !rounded.is_finite() || rounded.abs() > i32::MAX as f64 {
            return None;
        }
        let v = base.powi(rounded as i32);
        return v.is_finite().then_some(v);
    }
    let v = base.powf(exp);
    v.is_finite().then_some(v)
}

/// Two-term power series `Y = A * X^B + C * X^D`, guarded against `X = 0` with negative exponents
/// and `X < 0` with non-integer exponents.
///
/// Reference: Huxley (1932); Seber & Wild (1989).
struct PowerSeriesTwoTerm;

impl ParametricModel for PowerSeriesTwoTerm {
    impl_static_meta!(
        "power_series_two_term",
        "Two-term power series (Y = A * X^B + C * X^D)",
        EquationCategory::StandardCurve,
        ["A", "B", "C", "D"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || params.len() < 4 || !params[..4].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [a, b, c, d] = [params[0], params[1], params[2], params[3]];
        let t1 = a * safe_real_pow(x0, b)?;
        let t2 = c * safe_real_pow(x0, d)?;
        let y = t1 + t2;
        y.is_finite().then_some(y)
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let scale = if ymax.is_finite() && ymax.abs() > 1e-6 { ymax } else { 1.0 };
        vec![0.5 * scale, 1.0, 0.5 * scale, 2.0]
    }
}

/// Rational Padé (1,1) approximant `Y = (A0 + A1 * X) / (1 + B1 * X)`.
/// Reference: Baker & Graves-Morris (1996).
struct Pade11;

impl ParametricModel for Pade11 {
    impl_static_meta!("pade_1_1", "Padé (1,1) approximant", EquationCategory::StandardCurve, ["A0", "A1", "B1"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (a0, a1, b1) = (params[0], params[1], params[2]);
        let denom = 1.0 + b1 * x0;
        if !denom.is_finite() || denom.abs() < 1e-15 {
            return None;
        }
        let y = (a0 + a1 * x0) / denom;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (a0, a1, b1) = (params[0], params[1], params[2]);
        let denom = 1.0 + b1 * x0;
        if !denom.is_finite() || denom.abs() < 1e-15 {
            return false;
        }
        let inv_d = 1.0 / denom;
        let y = (a0 + a1 * x0) * inv_d;
        out[0] = inv_d;
        out[1] = x0 * inv_d;
        out[2] = -y * x0 * inv_d;
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let pairs: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
        let (slope, intercept) = estimate_ols_from_pairs(&pairs);
        vec![intercept, slope, 0.0]
    }
}

/// Horizontal line (constant null model) `Y = Mean`.
/// Reference: Draper & Smith (1998).
struct HorizontalLine;

impl ParametricModel for HorizontalLine {
    impl_static_meta!("horizontal_line", "Horizontal line", EquationCategory::StandardCurve, ["Mean"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 1) {
            return None;
        }
        Some(params[0])
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 1) || out.is_empty() {
            return false;
        }
        out[0] = 1.0;
        true
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let mut sum = 0.0_f64;
        let mut n = 0_usize;
        for p in points {
            if p.y.is_finite() {
                sum += p.y;
                n += 1;
            }
        }
        vec![if n > 0 { sum / (n as f64) } else { 0.0 }]
    }
}

/// Semilog line with linear `X` and base-10 log `Y`: `Y = 10^(Slope * X + YIntercept)`.
/// Reference: Motulsky & Christopoulos (2004).
struct SemilogXLinYLog;

impl ParametricModel for SemilogXLinYLog {
    impl_static_meta!("semilog_x_lin_y_log", "Semilog line (X linear, Y log)", EquationCategory::StandardCurve, ["Slope", "YIntercept"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let exp = params[0] * x0 + params[1];
        if !exp.is_finite() || !(-300.0..=300.0).contains(&exp) {
            return None;
        }
        let y = 10.0_f64.powf(exp);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 2) || out.len() < 2 {
            return false;
        }
        let exp = params[0] * x0 + params[1];
        if !exp.is_finite() || !(-300.0..=300.0).contains(&exp) {
            return false;
        }
        let y = 10.0_f64.powf(exp);
        let factor = y * core::f64::consts::LN_10;
        out[0] = factor * x0;
        out[1] = factor;
        out[..2].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let pairs: Vec<(f64, f64)> =
            points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite() && p.y > 0.0).map(|p| (p.x0(), p.y.log10())).collect();
        if pairs.len() >= 2 {
            let (slope, intercept) = estimate_ols_from_pairs(&pairs);
            if slope.is_finite() && intercept.is_finite() {
                return vec![slope, intercept];
            }
        }
        vec![0.1, 0.0]
    }
}

/// Sum of two HWHM Lorentzian (Cauchy) peaks:
/// `Y = Amplitude1 / (1 + ((X - Center1) / Width1)^2) + Amplitude2 / (1 + ((X - Center2) / Width2)^2)`.
/// Reference: Bevington & Robinson (2003) ch. 2.
struct TwoLorentzians;

impl ParametricModel for TwoLorentzians {
    impl_static_meta!(
        "sum_of_two_lorentzians",
        "Sum of two Lorentzians",
        EquationCategory::Peak,
        ["Amplitude1", "Center1", "Width1", "Amplitude2", "Center2", "Width2"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 6) {
            return None;
        }
        let (a1, c1, w1, a2, c2, w2) = (params[0], params[1], params[2], params[3], params[4], params[5]);
        if w1 <= 0.0 || w2 <= 0.0 {
            return None;
        }
        let z1 = (x0 - c1) / w1;
        let z2 = (x0 - c2) / w2;
        let y = a1 / (1.0 + z1 * z1) + a2 / (1.0 + z2 * z2);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 6) || out.len() < 6 {
            return false;
        }
        let (a1, c1, w1, a2, c2, w2) = (params[0], params[1], params[2], params[3], params[4], params[5]);
        if w1 <= 0.0 || w2 <= 0.0 {
            return false;
        }
        let z1 = (x0 - c1) / w1;
        let d1 = 1.0 / (1.0 + z1 * z1);
        out[0] = d1;
        out[1] = 2.0 * a1 * z1 * d1 * d1 / w1;
        out[2] = 2.0 * a1 * z1 * z1 * d1 * d1 / w1;

        let z2 = (x0 - c2) / w2;
        let d2 = 1.0 / (1.0 + z2 * z2);
        out[3] = d2;
        out[4] = 2.0 * a2 * z2 * d2 * d2 / w2;
        out[5] = 2.0 * a2 * z2 * z2 * d2 * d2 / w2;
        out[..6].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(0.0);
        let width = (span / 12.0).max(1e-3);
        vec![ymax, xmin + 0.35 * span, width, 0.5 * ymax, xmin + 0.65 * span, width]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(1e-15), None), (None, None), (None, None), (Some(1e-15), None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 6 {
            if params[2].is_finite() && params[2] > 0.0 {
                push_finite_scalar(&mut out, "FWHM1", 2.0 * params[2]);
            }
            if params[5].is_finite() && params[5] > 0.0 {
                push_finite_scalar(&mut out, "FWHM2", 2.0 * params[5]);
            }
        }
        out
    }
}

/// Fixed-scale Cumulative Gaussian (`100 * Phi((X - Mean) / SD)` or `Phi((X - Mean) / SD)`).
/// Reference: Finney (1971).
struct CumulativeGaussianScaled {
    meta: ModelMeta,
    scale: f64,
}

impl CumulativeGaussianScaled {
    fn percent() -> Self {
        Self {
            meta: ModelMeta::new("cumulative_gaussian_percent", "Cumulative Gaussian (percentages)", EquationCategory::Gaussian, &["Mean", "SD"]),
            scale: 100.0,
        }
    }

    fn fraction() -> Self {
        Self {
            meta: ModelMeta::new("cumulative_gaussian_fraction", "Cumulative Gaussian (fractions)", EquationCategory::Gaussian, &["Mean", "SD"]),
            scale: 1.0,
        }
    }
}

impl ParametricModel for CumulativeGaussianScaled {
    impl_meta_methods!();

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let (mean, sd) = (params[0], params[1]);
        if sd <= 0.0 {
            return None;
        }
        let z = (x0 - mean) / sd;
        if !z.is_finite() {
            return None;
        }
        let y = self.scale * curvebook_math::dist::norm_cdf(z);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 2) || out.len() < 2 {
            return false;
        }
        let (mean, sd) = (params[0], params[1]);
        if sd <= 0.0 {
            return false;
        }
        let z = (x0 - mean) / sd;
        if !z.is_finite() {
            return false;
        }
        let pdf = curvebook_math::dist::norm_pdf(z);
        out[0] = -self.scale * pdf / sd;
        out[1] = -self.scale * pdf * z / sd;
        out[..2].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (xmin, xmax) = data_x_min_max(points);
        vec![0.5 * (xmin + xmax), ((xmax - xmin) / 6.0).max(1e-3)]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(1e-15), None)]
    }
}

/// Cumulative Gaussian scaled to total count `N`: `Y = N * Phi((X - Mean) / SD)`.
/// Reference: Finney (1971).
struct CumulativeGaussianCounts;

impl ParametricModel for CumulativeGaussianCounts {
    impl_static_meta!("cumulative_gaussian_counts", "Cumulative Gaussian (counts)", EquationCategory::Gaussian, ["N", "Mean", "SD"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (n, mean, sd) = (params[0], params[1], params[2]);
        if sd <= 0.0 {
            return None;
        }
        let z = (x0 - mean) / sd;
        if !z.is_finite() {
            return None;
        }
        let y = n * curvebook_math::dist::norm_cdf(z);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (n, mean, sd) = (params[0], params[1], params[2]);
        if sd <= 0.0 {
            return false;
        }
        let z = (x0 - mean) / sd;
        if !z.is_finite() {
            return false;
        }
        let cdf = curvebook_math::dist::norm_cdf(z);
        let pdf = curvebook_math::dist::norm_pdf(z);
        out[0] = cdf;
        out[1] = -n * pdf / sd;
        out[2] = -n * pdf * z / sd;
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0), 0.5 * (xmin + xmax), ((xmax - xmin) / 6.0).max(1e-3)]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (None, None), (Some(1e-15), None)]
    }
}

/// Classic one-phase exponential decay `Y = Span * exp(-K * X) + Plateau`.
/// Reference: Seber & Wild (1989) ch. 8; Motulsky & Christopoulos (2004).
struct ClassicOnePhaseDecay;

impl ParametricModel for ClassicOnePhaseDecay {
    impl_static_meta!("classic_one_phase_decay", "One-phase exponential decay (classic)", EquationCategory::Exponential, ["Span", "K", "Plateau"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 3) {
            return None;
        }
        let (span, k, plateau) = (params[0], params[1], params[2]);
        let y = span * safe_exp(-k * t)? + plateau;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 3) || out.len() < 3 {
            return false;
        }
        let (span, k) = (params[0], params[1]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = e;
        out[1] = -span * t * e;
        out[2] = 1.0;
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax - ymin, 2.0 / (xmax - xmin).max(1.0), ymin]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(0.0), None), (None, None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 3 {
            push_finite_scalar(&mut out, "Y0", params[0] + params[2]);
            if params[1].is_finite() && params[1] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife", core::f64::consts::LN_2 / params[1]);
                push_finite_scalar(&mut out, "Tau", 1.0 / params[1]);
            }
        }
        out
    }
}

/// Classic two-phase exponential decay `Y = Span1 * exp(-K1 * X) + Span2 * exp(-K2 * X) + Plateau`.
/// Reference: Seber & Wild (1989) ch. 8; Motulsky & Christopoulos (2004).
struct ClassicTwoPhaseDecay;

impl ParametricModel for ClassicTwoPhaseDecay {
    impl_static_meta!(
        "classic_two_phase_decay",
        "Two-phase exponential decay (classic)",
        EquationCategory::Exponential,
        ["Span1", "K1", "Span2", "K2", "Plateau"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 5) {
            return None;
        }
        let (s1, k1, s2, k2, plateau) = (params[0], params[1], params[2], params[3], params[4]);
        let y = s1 * safe_exp(-k1 * t)? + s2 * safe_exp(-k2 * t)? + plateau;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 5) || out.len() < 5 {
            return false;
        }
        let (s1, k1, s2, k2) = (params[0], params[1], params[2], params[3]);
        let (Some(e1), Some(e2)) = (safe_exp(-k1 * t), safe_exp(-k2 * t)) else {
            return false;
        };
        out[0] = e1;
        out[1] = -s1 * t * e1;
        out[2] = e2;
        out[3] = -s2 * t * e2;
        out[4] = 1.0;
        out[..5].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span_x = (xmax - xmin).max(1.0);
        let half_y = 0.5 * (ymax - ymin);
        vec![half_y, 4.0 / span_x, half_y, 0.5 / span_x, ymin]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(0.0), None), (None, None), (Some(0.0), None), (None, None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 5 {
            if params[1].is_finite() && params[1] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife1", core::f64::consts::LN_2 / params[1]);
            }
            if params[3].is_finite() && params[3] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife2", core::f64::consts::LN_2 / params[3]);
            }
        }
        out
    }
}

/// Classic one-phase exponential association `Y = Ymax * (1 - exp(-K * X))`.
/// Reference: Seber & Wild (1989) ch. 8; Motulsky & Christopoulos (2004).
struct ClassicOnePhaseAssociation;

impl ParametricModel for ClassicOnePhaseAssociation {
    impl_static_meta!("classic_one_phase_association", "One-phase exponential association (classic)", EquationCategory::Exponential, ["Ymax", "K"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let (ymax, k) = (params[0], params[1]);
        let y = ymax * (1.0 - safe_exp(-k * t)?);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 2) || out.len() < 2 {
            return false;
        }
        let (ymax, k) = (params[0], params[1]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = 1.0 - e;
        out[1] = ymax * t * e;
        out[..2].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, 2.0 / (xmax - xmin).max(1.0)]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(0.0), None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[1].is_finite() && params[1] > 0.0 {
            push_finite_scalar(&mut out, "HalfLife", core::f64::consts::LN_2 / params[1]);
            push_finite_scalar(&mut out, "Tau", 1.0 / params[1]);
        }
        out
    }
}

/// Classic two-phase exponential association
/// `Y = Ymax1 * (1 - exp(-K1 * X)) + Ymax2 * (1 - exp(-K2 * X))`.
/// Reference: Seber & Wild (1989) ch. 8; Motulsky & Christopoulos (2004).
struct ClassicTwoPhaseAssociation;

impl ParametricModel for ClassicTwoPhaseAssociation {
    impl_static_meta!(
        "classic_two_phase_association",
        "Two-phase exponential association (classic)",
        EquationCategory::Exponential,
        ["Ymax1", "K1", "Ymax2", "K2"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (y1, k1, y2, k2) = (params[0], params[1], params[2], params[3]);
        let y = y1 * (1.0 - safe_exp(-k1 * t)?) + y2 * (1.0 - safe_exp(-k2 * t)?);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 4) || out.len() < 4 {
            return false;
        }
        let (y1, k1, y2, k2) = (params[0], params[1], params[2], params[3]);
        let (Some(e1), Some(e2)) = (safe_exp(-k1 * t), safe_exp(-k2 * t)) else {
            return false;
        };
        out[0] = 1.0 - e1;
        out[1] = y1 * t * e1;
        out[2] = 1.0 - e2;
        out[3] = y2 * t * e2;
        out[..4].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span_x = (xmax - xmin).max(1.0);
        vec![0.5 * ymax, 4.0 / span_x, 0.5 * ymax, 0.5 / span_x]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (Some(0.0), None), (None, None), (Some(0.0), None)]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 4 {
            if params[1].is_finite() && params[1] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife1", core::f64::consts::LN_2 / params[1]);
            }
            if params[3].is_finite() && params[3] > 0.0 {
                push_finite_scalar(&mut out, "HalfLife2", core::f64::consts::LN_2 / params[3]);
            }
        }
        out
    }
}

/// Classic exponential growth `Y = Start * exp(K * X)`.
/// Reference: Malthus (1798); Seber & Wild (1989) ch. 7.
struct ClassicExponentialGrowth;

impl ParametricModel for ClassicExponentialGrowth {
    impl_static_meta!("classic_exponential_growth", "Exponential growth (classic)", EquationCategory::Exponential, ["Start", "K"]);

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 2) {
            return None;
        }
        let (start, k) = (params[0], params[1]);
        let y = start * safe_exp(k * t)?;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 2) || out.len() < 2 {
            return false;
        }
        let (start, k) = (params[0], params[1]);
        let Some(e) = safe_exp(k * t) else {
            return false;
        };
        out[0] = e;
        out[1] = start * t * e;
        out[..2].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        estimate_exp_growth_seed(points)
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[1].is_finite() && params[1] > 0.0 {
            push_finite_scalar(&mut out, "DoublingTime", core::f64::consts::LN_2 / params[1]);
        }
        out
    }
}

/// Classic frequency-parameterized sine wave
/// `Y = Baseline + Amplitude * sin(2 * pi * Frequency * X + PhaseShift)`.
/// Reference: Bloomfield (2000).
struct ClassicSineWave;

impl ParametricModel for ClassicSineWave {
    impl_static_meta!(
        "classic_sine_wave",
        "Sine wave (frequency, classic)",
        EquationCategory::SineWave,
        ["Amplitude", "Frequency", "PhaseShift", "Baseline"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || !all_finite_params(params, 4) {
            return None;
        }
        let (amp, freq, phase, base) = (params[0], params[1], params[2], params[3]);
        let y = base + amp * (2.0 * core::f64::consts::PI * freq * t + phase).sin();
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        if !t.is_finite() || !all_finite_params(params, 4) || out.len() < 4 {
            return false;
        }
        let (amp, freq, phase) = (params[0], params[1], params[2]);
        let angle = 2.0 * core::f64::consts::PI * freq * t + phase;
        let (s, c) = angle.sin_cos();
        out[0] = s;
        out[1] = amp * c * (2.0 * core::f64::consts::PI * t);
        out[2] = amp * c;
        out[3] = 1.0;
        out[..4].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let amp = 0.5 * (ymax - ymin);
        let base = 0.5 * (ymax + ymin);
        let freq = 1.0 / (xmax - xmin).max(1.0);
        vec![amp, freq, 0.0, base]
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        if params.len() >= 2 && params[1].is_finite() && params[1].abs() > 1e-15 {
            push_finite_scalar(&mut out, "Wavelength", 1.0 / params[1]);
        }
        out
    }
}
