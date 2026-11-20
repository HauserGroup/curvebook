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

use crate::library::{BuiltinEquationSpec, ParametricModel, data_x_min_max, data_y_min_max, resolve_from_specs, spec_ids};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};
use curvebook_doc::{ParameterRule, ParameterScope};

/// Ordered single-source-of-truth registrations for the general curve equation family.
pub(crate) const CURVE_SPECS: &[BuiltinEquationSpec] = &[
    // Exponential
    BuiltinEquationSpec { id: "one_phase_decay", aliases: &[], build: || Box::new(OnePhaseDecay { params: names(&["Y0", "Plateau", "K"]) }) },
    BuiltinEquationSpec {
        id: "two_phase_decay",
        aliases: &[],
        build: || Box::new(TwoPhaseDecay { params: names(&["Y0", "Plateau", "PercentFast", "KFast", "KSlow"]) }),
    },
    BuiltinEquationSpec {
        id: "three_phase_decay",
        aliases: &[],
        build: || Box::new(ThreePhaseDecay { params: names(&["Y0", "Plateau", "PercentFast", "PercentSlow", "KFast", "KSlow", "KVerySlow"]) }),
    },
    BuiltinEquationSpec {
        id: "plateau_followed_by_one_phase_decay",
        aliases: &[],
        build: || Box::new(PlateauThenDecay { params: names(&["Y0", "X0", "Plateau", "K"]) }),
    },
    BuiltinEquationSpec {
        id: "one_phase_association",
        aliases: &[],
        build: || Box::new(OnePhaseAssociation { params: names(&["Y0", "Plateau", "K"]) }),
    },
    BuiltinEquationSpec {
        id: "plateau_followed_by_one_phase_association",
        aliases: &[],
        build: || Box::new(PlateauThenAssoc { params: names(&["Y0", "X0", "Plateau", "K"]) }),
    },
    BuiltinEquationSpec {
        id: "exponential_growth",
        aliases: &[],
        build: || {
            Box::new(ExponentialGrowth {
                id: "exponential_growth",
                name: "Exponential growth",
                category: EquationCategory::Exponential,
                params: names(&["Y0", "k"]),
            })
        },
    },
    // Growth
    BuiltinEquationSpec { id: "exponential_plateau", aliases: &[], build: || Box::new(ExponentialPlateau { params: names(&["YM", "Y0", "k"]) }) },
    BuiltinEquationSpec { id: "logistic_growth", aliases: &[], build: || Box::new(LogisticGrowth { params: names(&["YM", "Y0", "k"]) }) },
    BuiltinEquationSpec { id: "gompertz_growth", aliases: &[], build: || Box::new(GompertzGrowth { params: names(&["YM", "Y0", "k"]) }) },
    BuiltinEquationSpec { id: "beta_growth_then_decay", aliases: &[], build: || Box::new(BetaGrowthDecay { params: names(&["Ym", "te", "tm"]) }) },
    BuiltinEquationSpec {
        id: "malthusian_growth",
        aliases: &[],
        build: || {
            Box::new(ExponentialGrowth {
                id: "malthusian_growth",
                name: "Malthusian growth",
                category: EquationCategory::Growth,
                params: names(&["Y0", "k"]),
            })
        },
    },
    // Radiobiology
    BuiltinEquationSpec {
        id: "linear_quadratic_survival",
        aliases: &[],
        build: || Box::new(LinearQuadraticSurvival { params: names(&["Alpha", "Beta"]) }),
    },
    // Peak
    BuiltinEquationSpec {
        id: "gaussian",
        aliases: &[],
        build: || Box::new(GaussianCurve { with_baseline: false, params: names(&["Amplitude", "Mean", "SD"]) }),
    },
    BuiltinEquationSpec {
        id: "gaussian_baseline",
        aliases: &[],
        build: || Box::new(GaussianCurve { with_baseline: true, params: names(&["Baseline", "Amplitude", "Mean", "SD"]) }),
    },
    BuiltinEquationSpec {
        id: "sum_of_two_gaussians",
        aliases: &[],
        build: || Box::new(TwoGaussians { params: names(&["Amp1", "Mean1", "SD1", "Amp2", "Mean2", "SD2"]) }),
    },
    BuiltinEquationSpec { id: "log_gaussian", aliases: &[], build: || Box::new(LogGaussian { params: names(&["Amplitude", "GeoMean", "GeoSD"]) }) },
    BuiltinEquationSpec {
        id: "cumulative_gaussian",
        aliases: &[],
        build: || Box::new(CumulativeGaussian { params: names(&["Bottom", "Top", "Mean", "SD"]) }),
    },
    BuiltinEquationSpec { id: "lorentzian", aliases: &[], build: || Box::new(LorentzianPeak { params: names(&["Amplitude", "Center", "Width"]) }) },
    BuiltinEquationSpec { id: "weibull_peak", aliases: &[], build: || Box::new(WeibullPeak { params: names(&["Amplitude", "Scale", "Shape"]) }) },
    // Wave
    BuiltinEquationSpec {
        id: "sine_wave",
        aliases: &[],
        build: || Box::new(SineWave { damped: false, params: names(&["Amplitude", "Frequency", "PhaseShift", "Baseline"]) }),
    },
    BuiltinEquationSpec {
        id: "damped_sine_wave",
        aliases: &[],
        build: || Box::new(SineWave { damped: true, params: names(&["Amplitude", "Frequency", "PhaseShift", "K", "Baseline"]) }),
    },
    BuiltinEquationSpec {
        id: "sinc_wave",
        aliases: &[],
        build: || Box::new(SincWave { params: names(&["Amplitude", "Center", "Width", "Baseline"]) }),
    },
    // Standard Curves & Polynomials
    BuiltinEquationSpec {
        id: "straight_line",
        aliases: &[],
        build: || Box::new(StraightLine { through_origin: false, params: names(&["Slope", "YIntercept"]) }),
    },
    BuiltinEquationSpec {
        id: "line_through_origin",
        aliases: &[],
        build: || Box::new(StraightLine { through_origin: true, params: names(&["Slope"]) }),
    },
    BuiltinEquationSpec {
        id: "segmental_linear",
        aliases: &[],
        build: || Box::new(SegmentalLinear { params: names(&["Intercept1", "Slope1", "X0", "Slope2"]) }),
    },
    BuiltinEquationSpec {
        id: "hinge_smooth_hyperbolic",
        aliases: &[],
        build: || Box::new(SmoothHinge { params: names(&["Y0", "X0", "Slope1", "Slope2", "Delta"]) }),
    },
    BuiltinEquationSpec { id: "polynomial_2", aliases: &["quadratic"], build: || Box::new(Polynomial { degree: 2 }) },
    BuiltinEquationSpec { id: "polynomial_3", aliases: &["cubic"], build: || Box::new(Polynomial { degree: 3 }) },
    BuiltinEquationSpec { id: "polynomial_4", aliases: &["quartic"], build: || Box::new(Polynomial { degree: 4 }) },
    BuiltinEquationSpec { id: "polynomial_5", aliases: &[], build: || Box::new(Polynomial { degree: 5 }) },
    BuiltinEquationSpec { id: "polynomial_6", aliases: &[], build: || Box::new(Polynomial { degree: 6 }) },
    BuiltinEquationSpec { id: "power_law", aliases: &["power_series"], build: || Box::new(PowerCurve { params: names(&["a", "b"]) }) },
    BuiltinEquationSpec { id: "logarithmic_curve", aliases: &[], build: || Box::new(LogarithmicCurve { params: names(&["a", "b"]) }) },
    BuiltinEquationSpec {
        id: "centered_poly_1",
        aliases: &["centered_linear", "centered_polynomial_1"],
        build: || Box::new(CenteredPolynomial::new(1)),
    },
    BuiltinEquationSpec {
        id: "centered_poly_2",
        aliases: &["centered_quadratic", "centered_polynomial_2"],
        build: || Box::new(CenteredPolynomial::new(2)),
    },
    BuiltinEquationSpec {
        id: "centered_poly_3",
        aliases: &["centered_cubic", "centered_polynomial_3"],
        build: || Box::new(CenteredPolynomial::new(3)),
    },
    BuiltinEquationSpec {
        id: "centered_poly_4",
        aliases: &["centered_quartic", "centered_polynomial_4"],
        build: || Box::new(CenteredPolynomial::new(4)),
    },
    BuiltinEquationSpec { id: "centered_poly_5", aliases: &["centered_polynomial_5"], build: || Box::new(CenteredPolynomial::new(5)) },
    BuiltinEquationSpec { id: "centered_poly_6", aliases: &["centered_polynomial_6"], build: || Box::new(CenteredPolynomial::new(6)) },
    BuiltinEquationSpec {
        id: "semi_log_line",
        aliases: &["semilog_line"],
        build: || Box::new(SemiLogLine { params: names(&["Slope", "YIntercept"]) }),
    },
    BuiltinEquationSpec {
        id: "log_log_line",
        aliases: &["loglog_line"],
        build: || Box::new(LogLogLine { params: names(&["Slope", "LogYIntercept"]) }),
    },
    BuiltinEquationSpec { id: "probit_line", aliases: &[], build: || Box::new(ProbitLine { params: names(&["Slope", "Intercept"]) }) },
];

const CURVE_EQUATION_ID_ARRAY: [&str; CURVE_SPECS.len()] = spec_ids::<{ CURVE_SPECS.len() }>(CURVE_SPECS);

/// Canonical IDs exposed by the general curve equation catalog.
pub const CURVE_EQUATION_IDS: &[&str] = &CURVE_EQUATION_ID_ARRAY;

fn names(slice: &[&str]) -> Vec<String> {
    slice.iter().map(|s| (*s).to_owned()).collect()
}

#[inline]
fn safe_exp(arg: f64) -> Option<f64> {
    if !arg.is_finite() || arg > 700.0 {
        return None;
    }
    let v = arg.exp();
    if v.is_finite() { Some(v) } else { None }
}

/// Resolve any non-pharmacology, non-NIST built-in equation ID into a [`ParametricModel`].
#[must_use]
pub fn resolve_curve_model(norm_id: &str) -> Option<Box<dyn ParametricModel>> {
    resolve_from_specs(CURVE_SPECS, norm_id)
}

/// One-phase exponential decay `Y = (Y0 - Plateau) * exp(-K * X) + Plateau`.
/// `HalfLife = ln 2 / K`, `Tau = 1 / K`.
/// Reference: first-order (single-compartment) kinetics, Seber & Wild (1989) ch. 8; this
/// `Y0` / `Plateau` / `K` parameterisation follows Motulsky & Christopoulos (2004).
struct OnePhaseDecay {
    params: Vec<String>,
}

impl ParametricModel for OnePhaseDecay {
    fn id(&self) -> &str {
        "one_phase_decay"
    }
    fn name(&self) -> &str {
        "One phase decay"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct TwoPhaseDecay {
    params: Vec<String>,
}

impl ParametricModel for TwoPhaseDecay {
    fn id(&self) -> &str {
        "two_phase_decay"
    }
    fn name(&self) -> &str {
        "Two phase decay"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct ThreePhaseDecay {
    params: Vec<String>,
}

impl ParametricModel for ThreePhaseDecay {
    fn id(&self) -> &str {
        "three_phase_decay"
    }
    fn name(&self) -> &str {
        "Three phase decay"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct PlateauThenDecay {
    params: Vec<String>,
}

impl ParametricModel for PlateauThenDecay {
    fn id(&self) -> &str {
        "plateau_followed_by_one_phase_decay"
    }
    fn name(&self) -> &str {
        "Plateau followed by one phase decay"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct OnePhaseAssociation {
    params: Vec<String>,
}

impl ParametricModel for OnePhaseAssociation {
    fn id(&self) -> &str {
        "one_phase_association"
    }
    fn name(&self) -> &str {
        "One phase association"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct PlateauThenAssoc {
    params: Vec<String>,
}

impl ParametricModel for PlateauThenAssoc {
    fn id(&self) -> &str {
        "plateau_followed_by_one_phase_association"
    }
    fn name(&self) -> &str {
        "Plateau followed by one phase association"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
    id: &'static str,
    name: &'static str,
    category: EquationCategory,
    params: Vec<String>,
}

impl ParametricModel for ExponentialGrowth {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> EquationCategory {
        self.category
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct ExponentialPlateau {
    params: Vec<String>,
}

impl ParametricModel for ExponentialPlateau {
    fn id(&self) -> &str {
        "exponential_plateau"
    }
    fn name(&self) -> &str {
        "Exponential plateau"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Growth
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct LogisticGrowth {
    params: Vec<String>,
}

impl ParametricModel for LogisticGrowth {
    fn id(&self) -> &str {
        "logistic_growth"
    }
    fn name(&self) -> &str {
        "Logistic growth"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Growth
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct GompertzGrowth {
    params: Vec<String>,
}

impl ParametricModel for GompertzGrowth {
    fn id(&self) -> &str {
        "gompertz_growth"
    }
    fn name(&self) -> &str {
        "Gompertz growth"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Growth
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct BetaGrowthDecay {
    params: Vec<String>,
}

impl ParametricModel for BetaGrowthDecay {
    fn id(&self) -> &str {
        "beta_growth_then_decay"
    }
    fn name(&self) -> &str {
        "Beta growth then decay"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Growth
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct LinearQuadraticSurvival {
    params: Vec<String>,
}

impl ParametricModel for LinearQuadraticSurvival {
    fn id(&self) -> &str {
        "linear_quadratic_survival"
    }
    fn name(&self) -> &str {
        "Linear-quadratic radiation survival"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Radiobiology
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
    with_baseline: bool,
    params: Vec<String>,
}

impl ParametricModel for GaussianCurve {
    fn id(&self) -> &str {
        if self.with_baseline { "gaussian_baseline" } else { "gaussian" }
    }
    fn name(&self) -> &str {
        if self.with_baseline { "Gaussian with baseline" } else { "Gaussian" }
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Gaussian
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
        let peak_x = points.iter().max_by(|a, b| a.y.total_cmp(&b.y)).map_or(0.5 * (xmin + xmax), |p| p.x0());
        let sd = ((xmax - xmin) / 6.0).max(1e-3);
        if self.with_baseline { vec![ymin, ymax - ymin, peak_x, sd] } else { vec![ymax, peak_x, sd] }
    }
}

/// Sum of two unnormalised Gaussian peaks (no baseline).
/// Reference: Gaussian line shape, Bevington & Robinson (2003) ch. 2.
struct TwoGaussians {
    params: Vec<String>,
}

impl ParametricModel for TwoGaussians {
    fn id(&self) -> &str {
        "sum_of_two_gaussians"
    }
    fn name(&self) -> &str {
        "Sum of two Gaussians"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Peak
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct LogGaussian {
    params: Vec<String>,
}

impl ParametricModel for LogGaussian {
    fn id(&self) -> &str {
        "log_gaussian"
    }
    fn name(&self) -> &str {
        "Log Gaussian"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Peak
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct CumulativeGaussian {
    params: Vec<String>,
}

impl ParametricModel for CumulativeGaussian {
    fn id(&self) -> &str {
        "cumulative_gaussian"
    }
    fn name(&self) -> &str {
        "Cumulative Gaussian"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Gaussian
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct LorentzianPeak {
    params: Vec<String>,
}

impl ParametricModel for LorentzianPeak {
    fn id(&self) -> &str {
        "lorentzian"
    }
    fn name(&self) -> &str {
        "Lorentzian"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Peak
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct WeibullPeak {
    params: Vec<String>,
}

impl ParametricModel for WeibullPeak {
    fn id(&self) -> &str {
        "weibull_peak"
    }
    fn name(&self) -> &str {
        "Weibull density curve"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Peak
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (amp, scale, shape) = (params[0], params[1], params[2]);
        if x0 < 0.0 || scale <= 0.0 || shape <= 0.0 {
            return None;
        }
        if x0 == 0.0 {
            return Some(0.0);
        }
        let u = x0 / scale;
        let e = safe_exp(-u.powf(shape))?;
        Some(amp * (shape / scale) * u.powf(shape - 1.0) * e)
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
    damped: bool,
    params: Vec<String>,
}

impl ParametricModel for SineWave {
    fn id(&self) -> &str {
        if self.damped { "damped_sine_wave" } else { "sine_wave" }
    }
    fn name(&self) -> &str {
        if self.damped { "Damped sine wave" } else { "Sine wave" }
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::SineWave
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct SincWave {
    params: Vec<String>,
}

impl ParametricModel for SincWave {
    fn id(&self) -> &str {
        "sinc_wave"
    }
    fn name(&self) -> &str {
        "Sinc function"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::SineWave
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
    through_origin: bool,
    params: Vec<String>,
}

impl ParametricModel for StraightLine {
    fn id(&self) -> &str {
        if self.through_origin { "line_through_origin" } else { "straight_line" }
    }
    fn name(&self) -> &str {
        if self.through_origin { "Line through origin" } else { "Straight line" }
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct SegmentalLinear {
    params: Vec<String>,
}

impl ParametricModel for SegmentalLinear {
    fn id(&self) -> &str {
        "segmental_linear"
    }
    fn name(&self) -> &str {
        "Segmental linear regression"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct SmoothHinge {
    params: Vec<String>,
}

impl ParametricModel for SmoothHinge {
    fn id(&self) -> &str {
        "hinge_smooth_hyperbolic"
    }
    fn name(&self) -> &str {
        "Hinge (smooth hyperbolic transition)"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
    degree: usize,
}

impl ParametricModel for Polynomial {
    fn id(&self) -> &str {
        match self.degree {
            2 => "polynomial_2",
            3 => "polynomial_3",
            4 => "polynomial_4",
            5 => "polynomial_5",
            _ => "polynomial_6",
        }
    }
    fn name(&self) -> &str {
        match self.degree {
            2 => "Second order polynomial (quadratic)",
            3 => "Third order polynomial (cubic)",
            4 => "Fourth order polynomial",
            5 => "Fifth order polynomial",
            _ => "Sixth order polynomial",
        }
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::Polynomial
    }
    fn param_names(&self) -> &[String] {
        static P2: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| names(&["B0", "B1", "B2"]));
        static P3: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| names(&["B0", "B1", "B2", "B3"]));
        static P4: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| names(&["B0", "B1", "B2", "B3", "B4"]));
        static P5: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| names(&["B0", "B1", "B2", "B3", "B4", "B5"]));
        static P6: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| names(&["B0", "B1", "B2", "B3", "B4", "B5", "B6"]));
        match self.degree {
            2 => &P2,
            3 => &P3,
            4 => &P4,
            5 => &P5,
            _ => &P6,
        }
    }
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
struct PowerCurve {
    params: Vec<String>,
}

impl ParametricModel for PowerCurve {
    fn id(&self) -> &str {
        "power_law"
    }
    fn name(&self) -> &str {
        "Power law (Y = a * X^b)"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
struct LogarithmicCurve {
    params: Vec<String>,
}

impl ParametricModel for LogarithmicCurve {
    fn id(&self) -> &str {
        "logarithmic_curve"
    }
    fn name(&self) -> &str {
        "Logarithmic curve (Y = a * ln(X) + b)"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
    degree: usize,
    id: &'static str,
    name: &'static str,
    params: Vec<String>,
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
        Self { degree: clamped, id, name, params }
    }
}

impl ParametricModel for CenteredPolynomial {
    fn id(&self) -> &str {
        self.id
    }

    fn name(&self) -> &str {
        self.name
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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
struct SemiLogLine {
    params: Vec<String>,
}

impl ParametricModel for SemiLogLine {
    fn id(&self) -> &str {
        "semi_log_line"
    }

    fn name(&self) -> &str {
        "Semi-log line (Y vs log10(X))"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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
struct LogLogLine {
    params: Vec<String>,
}

impl ParametricModel for LogLogLine {
    fn id(&self) -> &str {
        "log_log_line"
    }

    fn name(&self) -> &str {
        "Log-log regression line"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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
struct ProbitLine {
    params: Vec<String>,
}

impl ParametricModel for ProbitLine {
    fn id(&self) -> &str {
        "probit_line"
    }

    fn name(&self) -> &str {
        "Probit curve"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::StandardCurve
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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
