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
    BuiltinEquationSpec::new("plateau_followed_by_one_phase_association", &[], || Box::new(PlateauThenAssoc)),
    BuiltinEquationSpec::new("exponential_growth", &[], || {
        Box::new(ExponentialGrowth { meta: ModelMeta::new("exponential_growth", "Exponential growth", EquationCategory::Exponential, &["Y0", "k"]) })
    }),
    // Growth
    BuiltinEquationSpec::new("exponential_plateau", &[], || Box::new(ExponentialPlateau)),
    BuiltinEquationSpec::new("logistic_growth", &[], || Box::new(LogisticGrowth)),
    BuiltinEquationSpec::new("gompertz_growth", &[], || Box::new(GompertzGrowth)),
    BuiltinEquationSpec::new("beta_growth_then_decay", &[], || Box::new(BetaGrowthDecay)),
    BuiltinEquationSpec::new("malthusian_growth", &[], || {
        Box::new(ExponentialGrowth { meta: ModelMeta::new("malthusian_growth", "Malthusian growth", EquationCategory::Growth, &["Y0", "k"]) })
    }),
    // Radiobiology
    BuiltinEquationSpec::new("linear_quadratic_survival", &[], || Box::new(LinearQuadraticSurvival)),
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
    BuiltinEquationSpec::new("sum_of_two_gaussians", &[], || Box::new(TwoGaussians)),
    BuiltinEquationSpec::new("log_gaussian", &[], || Box::new(LogGaussian)),
    BuiltinEquationSpec::new("cumulative_gaussian", &[], || Box::new(CumulativeGaussian)),
    BuiltinEquationSpec::new("lorentzian", &[], || Box::new(LorentzianPeak)),
    BuiltinEquationSpec::new("weibull_peak", &[], || Box::new(WeibullPeak)),
    // Wave
    BuiltinEquationSpec::new("sine_wave", &[], || {
        Box::new(SineWave {
            meta: ModelMeta::new("sine_wave", "Sine wave", EquationCategory::SineWave, &["Amplitude", "Frequency", "PhaseShift", "Baseline"]),
            damped: false,
        })
    }),
    BuiltinEquationSpec::new("damped_sine_wave", &[], || {
        Box::new(SineWave {
            meta: ModelMeta::new(
                "damped_sine_wave",
                "Damped sine wave",
                EquationCategory::SineWave,
                &["Amplitude", "Frequency", "PhaseShift", "K", "Baseline"],
            ),
            damped: true,
        })
    }),
    BuiltinEquationSpec::new("sinc_wave", &[], || Box::new(SincWave)),
    // Standard Curves & Polynomials
    BuiltinEquationSpec::new("straight_line", &[], || {
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
    BuiltinEquationSpec::new("segmental_linear", &[], || Box::new(SegmentalLinear)),
    BuiltinEquationSpec::new("hinge_smooth_hyperbolic", &[], || Box::new(SmoothHinge)),
    BuiltinEquationSpec::new("polynomial_2", &["quadratic"], || Box::new(Polynomial::new(2))),
    BuiltinEquationSpec::new("polynomial_3", &["cubic"], || Box::new(Polynomial::new(3))),
    BuiltinEquationSpec::new("polynomial_4", &["quartic"], || Box::new(Polynomial::new(4))),
    BuiltinEquationSpec::new("polynomial_5", &[], || Box::new(Polynomial::new(5))),
    BuiltinEquationSpec::new("polynomial_6", &[], || Box::new(Polynomial::new(6))),
    BuiltinEquationSpec::new("power_law", &["power_series"], || Box::new(PowerCurve)),
    BuiltinEquationSpec::new("logarithmic_curve", &[], || Box::new(LogarithmicCurve)),
    BuiltinEquationSpec::new("centered_poly_1", &["centered_linear", "centered_polynomial_1"], || Box::new(CenteredPolynomial::new(1))),
    BuiltinEquationSpec::new("centered_poly_2", &["centered_quadratic", "centered_polynomial_2"], || Box::new(CenteredPolynomial::new(2))),
    BuiltinEquationSpec::new("centered_poly_3", &["centered_cubic", "centered_polynomial_3"], || Box::new(CenteredPolynomial::new(3))),
    BuiltinEquationSpec::new("centered_poly_4", &["centered_quartic", "centered_polynomial_4"], || Box::new(CenteredPolynomial::new(4))),
    BuiltinEquationSpec::new("centered_poly_5", &["centered_polynomial_5"], || Box::new(CenteredPolynomial::new(5))),
    BuiltinEquationSpec::new("centered_poly_6", &["centered_polynomial_6"], || Box::new(CenteredPolynomial::new(6))),
    BuiltinEquationSpec::new("semi_log_line", &["semilog_line"], || Box::new(SemiLogLine)),
    BuiltinEquationSpec::new("log_log_line", &["loglog_line"], || Box::new(LogLogLine)),
    BuiltinEquationSpec::new("probit_line", &[], || Box::new(ProbitLine)),
];

const CURVE_EQUATION_ID_ARRAY: [&str; CURVE_SPECS.len()] = spec_ids::<{ CURVE_SPECS.len() }>(CURVE_SPECS);

/// Canonical IDs exposed by the general curve equation catalog.
pub const CURVE_EQUATION_IDS: &[&str] = &CURVE_EQUATION_ID_ARRAY;

/// Resolve any non-pharmacology, non-NIST built-in equation ID into a [`ParametricModel`].
#[must_use]
pub fn resolve_curve_model(norm_id: &str) -> Option<Box<dyn ParametricModel>> {
    resolve_from_specs(CURVE_SPECS, norm_id)
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
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let e = safe_exp(-k * t)?;
        Some((y0 - plateau) * e + plateau)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = e;
        out[1] = 1.0 - e;
        out[2] = -(y0 - plateau) * t * e;
        out.iter().all(|v| v.is_finite())
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
        let mut out = vec![DerivedScalar { name: "Span".to_owned(), value: params[0] - params[1], units: None }];
        if params[2] > 0.0 {
            out.push(DerivedScalar { name: "HalfLife".to_owned(), value: core::f64::consts::LN_2 / params[2], units: None });
            out.push(DerivedScalar { name: "Tau".to_owned(), value: 1.0 / params[2], units: None });
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
        let (y0, plateau, pct_fast, k_fast, k_slow) = (params[0], params[1], params[2], params[3], params[4]);
        let span = y0 - plateau;
        let f = pct_fast * 0.01;
        let e1 = safe_exp(-k_fast * t)?;
        let e2 = safe_exp(-k_slow * t)?;
        Some(plateau + span * (f * e1 + (1.0 - f) * e2))
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
        let (y0, plateau, pf, ps, kf, ks, kvs) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        let span = y0 - plateau;
        let ff = pf * 0.01;
        let fs = ps * 0.01;
        let fv = 1.0 - ff - fs;
        let e1 = safe_exp(-kf * t)?;
        let e2 = safe_exp(-ks * t)?;
        let e3 = safe_exp(-kvs * t)?;
        Some(plateau + span * (ff * e1 + fs * e2 + fv * e3))
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
        let (y0, x0, plateau, k) = (params[0], params[1], params[2], params[3]);
        if t < x0 {
            Some(y0)
        } else {
            let e = safe_exp(-k * (t - x0))?;
            Some(plateau + (y0 - plateau) * e)
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymax, xmin + 0.2 * span, ymin, 2.0 / span]
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
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let e = safe_exp(-k * t)?;
        Some(y0 + (plateau - y0) * (1.0 - e))
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        let (y0, plateau, k) = (params[0], params[1], params[2]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = e;
        out[1] = 1.0 - e;
        out[2] = (plateau - y0) * t * e;
        out.iter().all(|v| v.is_finite())
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
        let mut out = vec![DerivedScalar { name: "Span".to_owned(), value: params[1] - params[0], units: None }];
        if params[2] > 0.0 {
            out.push(DerivedScalar { name: "HalfTime".to_owned(), value: core::f64::consts::LN_2 / params[2], units: None });
            out.push(DerivedScalar { name: "Tau".to_owned(), value: 1.0 / params[2], units: None });
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
        let (y0, x0, plateau, k) = (params[0], params[1], params[2], params[3]);
        if t < x0 {
            Some(y0)
        } else {
            let e = safe_exp(-k * (t - x0))?;
            Some(y0 + (plateau - y0) * (1.0 - e))
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymin, xmin + 0.2 * span, ymax, 2.0 / span]
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
        let (y0, k) = (params[0], params[1]);
        Some(y0 * safe_exp(k * t)?)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        let Some(e) = safe_exp(params[1] * t) else {
            return false;
        };
        out[0] = e;
        out[1] = params[0] * t * e;
        out.iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let dt = (xmax - xmin).max(1e-6);
        let ratio = (ymax.max(1e-6) / ymin.max(1e-6)).max(1.01);
        vec![ymin.max(1e-3), ratio.ln() / dt]
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
        let (ym, y0, k) = (params[0], params[1], params[2]);
        Some(ym - (ym - y0) * safe_exp(-k * t)?)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&t) = x.first() else {
            return false;
        };
        let (ym, y0, k) = (params[0], params[1], params[2]);
        let Some(e) = safe_exp(-k * t) else {
            return false;
        };
        out[0] = 1.0 - e;
        out[1] = e;
        out[2] = (ym - y0) * t * e;
        out.iter().all(|v| v.is_finite())
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
        let mut out = vec![DerivedScalar { name: "Span".to_owned(), value: params[0] - params[1], units: None }];
        if params[2] > 0.0 {
            out.push(DerivedScalar { name: "HalfTime".to_owned(), value: core::f64::consts::LN_2 / params[2], units: None });
            out.push(DerivedScalar { name: "Tau".to_owned(), value: 1.0 / params[2], units: None });
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
        let (ym, y0, k) = (params[0], params[1], params[2]);
        if y0 == 0.0 {
            return None;
        }
        let e = safe_exp(-k * t)?;
        let denom = y0 + (ym - y0) * e;
        if denom == 0.0 {
            return None;
        }
        Some(ym * y0 / denom)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0), ymin.max(1e-3), 3.0 / (xmax - xmin).max(1.0)]
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
        let (ym, y0, k) = (params[0], params[1], params[2]);
        if ym <= 0.0 || y0 <= 0.0 {
            return None;
        }
        let e = safe_exp(-k * t)?;
        let inner = (ym / y0).ln() * e;
        let outer = safe_exp(-inner)?;
        Some(ym * outer)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0), ymin.max(1e-3), 2.0 / (xmax - xmin).max(1.0)]
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
        let (base, amp, mean, sd) =
            if self.with_baseline { (params[0], params[1], params[2], params[3]) } else { (0.0, params[0], params[1], params[2]) };
        if sd.abs() < 1e-15 {
            return None;
        }
        let z = (x0 - mean) / sd;
        Some(base + amp * safe_exp(-0.5 * z * z)?)
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
        let (a1, m1, s1, a2, m2, s2) = (params[0], params[1], params[2], params[3], params[4], params[5]);
        if s1.abs() < 1e-15 || s2.abs() < 1e-15 {
            return None;
        }
        let z1 = (x0 - m1) / s1;
        let z2 = (x0 - m2) / s2;
        Some(a1 * safe_exp(-0.5 * z1 * z1)? + a2 * safe_exp(-0.5 * z2 * z2)?)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymax, xmin + 0.35 * span, span / 8.0, 0.5 * ymax, xmin + 0.65 * span, span / 8.0]
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
        let (amp, gmean, gsd) = (params[0], params[1], params[2]);
        if x0 <= 0.0 || gmean <= 0.0 || gsd <= 1.0 {
            return None;
        }
        let z = (x0 / gmean).ln() / gsd.ln();
        Some(amp * safe_exp(-0.5 * z * z)?)
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
        let (bottom, top, mean, sd) = (params[0], params[1], params[2], params[3]);
        if sd <= 0.0 {
            return None;
        }
        let z = (x0 - mean) / sd;
        let cdf = curvebook_math::dist::norm_cdf(z);
        Some(bottom + (top - bottom) * cdf)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymin, ymax, 0.5 * (xmin + xmax), ((xmax - xmin) / 6.0).max(1e-3)]
    }
}

/// Lorentzian (Cauchy) peak `Y = Amplitude / (1 + ((X - Center) / (Width / 2))^2)`, where
/// `Width` is the full width at half maximum.
/// Reference: Lorentzian distribution, Bevington & Robinson (2003) ch. 2.
struct LorentzianPeak;

impl ParametricModel for LorentzianPeak {
    impl_static_meta!("lorentzian", "Lorentzian", EquationCategory::Peak, ["Amplitude", "Center", "Width"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (amp, center, width) = (params[0], params[1], params[2]);
        if width.abs() < 1e-15 {
            return None;
        }
        let z = (x0 - center) / (0.5 * width);
        Some(amp / (1.0 + z * z))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, 0.5 * (xmin + xmax), ((xmax - xmin) / 4.0).max(1e-3)]
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

/// Sine wave `Y = Baseline + Amplitude * sin(2 * pi * Frequency * X + PhaseShift)` (phase in
/// radians); the damped variant multiplies the amplitude by `exp(-K * X)`.
/// Reference: sinusoid fitting, Bloomfield (2000). The damped variant is the under-damped
/// linear-oscillator form; no single statistical primary source is cited for it.
struct SineWave {
    meta: ModelMeta,
    damped: bool,
}

impl ParametricModel for SineWave {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if self.damped {
            let (amp, freq, phase, k, base) = (params[0], params[1], params[2], params[3], params[4]);
            let e = safe_exp(-k * t)?;
            Some(base + amp * e * (2.0 * core::f64::consts::PI * freq * t + phase).sin())
        } else {
            let (amp, freq, phase, base) = (params[0], params[1], params[2], params[3]);
            Some(base + amp * (2.0 * core::f64::consts::PI * freq * t + phase).sin())
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let amp = 0.5 * (ymax - ymin);
        let base = 0.5 * (ymax + ymin);
        let freq = 1.0 / (xmax - xmin).max(1.0);
        if self.damped { vec![amp, freq, 0.0, 0.1 * freq, base] } else { vec![amp, freq, 0.0, base] }
    }
}

/// Sinc peak `Y = Baseline + Amplitude * sin(u) / u` with `u = (X - Center) / Width`.
/// Reference: Bracewell (2000), who defines the normalised `sinc x = sin(pi x) / (pi x)`; here the
/// factor `pi` is absorbed into `Width` (unnormalised sinc).
struct SincWave;

impl ParametricModel for SincWave {
    impl_static_meta!("sinc_wave", "Sinc function", EquationCategory::SineWave, ["Amplitude", "Center", "Width", "Baseline"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (amp, center, width, base) = (params[0], params[1], params[2], params[3]);
        if width.abs() < 1e-15 {
            return None;
        }
        let u = (x0 - center) / width;
        let s = if u.abs() < 1e-12 { 1.0 } else { u.sin() / u };
        Some(base + amp * s)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax - ymin, 0.5 * (xmin + xmax), ((xmax - xmin) / 6.0).max(1e-3), ymin]
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
        if self.through_origin { Some(params[0] * x0) } else { Some(params[0] * x0 + params[1]) }
    }
    fn grad(&self, x: &[f64], _params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
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
        let (b1, s1, x_join, s2) = (params[0], params[1], params[2], params[3]);
        if x0 < x_join { Some(b1 + s1 * x0) } else { Some(b1 + s1 * x_join + s2 * (x0 - x_join)) }
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
        let (y0, x_join, s1, s2, delta) = (params[0], params[1], params[2], params[3], params[4]);
        let dx = x0 - x_join;
        let smooth = (dx * dx + delta * delta).sqrt() - delta.abs();
        Some(y0 + s1 * dx + 0.5 * (s2 - s1) * (dx + smooth))
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
        let mut acc = 0.0;
        for &c in params.iter().rev() {
            acc = acc * x0 + c;
        }
        if acc.is_finite() { Some(acc) } else { None }
    }
    fn grad(&self, x: &[f64], _params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        let mut xp = 1.0;
        for slot in out.iter_mut() {
            *slot = xp;
            xp *= x0;
        }
        out.iter().all(|v| v.is_finite())
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
        if x0 < 0.0 {
            return None;
        }
        let val = params[0] * x0.powf(params[1]);
        if val.is_finite() { Some(val) } else { None }
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
        if x0 <= 0.0 {
            return None;
        }
        Some(params[0] * x0.ln() + params[1])
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
        if !x0.is_finite() || !x_center.is_finite() {
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
        if !x0.is_finite() || x0 <= 0.0 || params.len() < 2 {
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
        if !x0.is_finite() || x0 <= 0.0 || params.len() < 2 {
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
        if !x0.is_finite() || params.len() < 2 {
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
