//! Exponential, Growth, Radiobiology, Peak, Wave, and Standard-Curve / Polynomial equations.

use crate::library::{ParametricModel, data_x_min_max, data_y_min_max};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};

/// Canonical IDs exposed by the general curve equation catalog.
pub const CURVE_EQUATION_IDS: &[&str] = &[
    // Exponential
    "one_phase_decay",
    "two_phase_decay",
    "three_phase_decay",
    "plateau_followed_by_one_phase_decay",
    "one_phase_association",
    "plateau_followed_by_one_phase_association",
    "exponential_growth",
    "exponential_plateau",
    // Growth
    "logistic_growth",
    "gompertz_growth",
    "beta_growth_then_decay",
    "malthusian_growth",
    // Radiobiology
    "linear_quadratic_survival",
    // Peak
    "gaussian",
    "gaussian_baseline",
    "sum_of_two_gaussians",
    "log_gaussian",
    "cumulative_gaussian",
    "lorentzian",
    "weibull_peak",
    // Wave
    "sine_wave",
    "damped_sine_wave",
    "sinc_wave",
    // Standard Curves & Polynomials
    "straight_line",
    "line_through_origin",
    "segmental_linear",
    "hinge_smooth_hyperbolic",
    "polynomial_2",
    "polynomial_3",
    "polynomial_4",
    "polynomial_5",
    "polynomial_6",
    "power_series",
    "logarithmic_curve",
];

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
    match norm_id {
        "one_phase_decay" => Some(Box::new(OnePhaseDecay { params: names(&["Y0", "Plateau", "K"]) })),
        "two_phase_decay" => Some(Box::new(TwoPhaseDecay { params: names(&["Y0", "Plateau", "PercentFast", "KFast", "KSlow"]) })),
        "three_phase_decay" => {
            Some(Box::new(ThreePhaseDecay { params: names(&["Y0", "Plateau", "PercentFast", "PercentSlow", "KFast", "KSlow", "KVerySlow"]) }))
        }
        "plateau_followed_by_one_phase_decay" => Some(Box::new(PlateauThenDecay { params: names(&["Y0", "X0", "Plateau", "K"]) })),
        "one_phase_association" => Some(Box::new(OnePhaseAssociation { params: names(&["Y0", "Plateau", "K"]) })),
        "plateau_followed_by_one_phase_association" => Some(Box::new(PlateauThenAssoc { params: names(&["Y0", "X0", "Plateau", "K"]) })),
        "exponential_growth" | "malthusian_growth" => Some(Box::new(ExponentialGrowth {
            id: if norm_id == "malthusian_growth" { "malthusian_growth" } else { "exponential_growth" },
            name: if norm_id == "malthusian_growth" { "Malthusian growth" } else { "Exponential growth" },
            category: if norm_id == "malthusian_growth" { EquationCategory::Growth } else { EquationCategory::Exponential },
            params: names(&["Y0", "k"]),
        })),
        "exponential_plateau" => Some(Box::new(ExponentialPlateau { params: names(&["YM", "Y0", "k"]) })),
        "logistic_growth" => Some(Box::new(LogisticGrowth { params: names(&["YM", "Y0", "k"]) })),
        "gompertz_growth" => Some(Box::new(GompertzGrowth { params: names(&["YM", "Y0", "k"]) })),
        "beta_growth_then_decay" => Some(Box::new(BetaGrowthDecay { params: names(&["Ym", "te", "tm"]) })),
        "linear_quadratic_survival" => Some(Box::new(LinearQuadraticSurvival { params: names(&["Alpha", "Beta"]) })),
        "gaussian" => Some(Box::new(GaussianCurve { with_baseline: false, params: names(&["Amplitude", "Mean", "SD"]) })),
        "gaussian_baseline" => Some(Box::new(GaussianCurve { with_baseline: true, params: names(&["Baseline", "Amplitude", "Mean", "SD"]) })),
        "sum_of_two_gaussians" => Some(Box::new(TwoGaussians { params: names(&["Amp1", "Mean1", "SD1", "Amp2", "Mean2", "SD2"]) })),
        "log_gaussian" => Some(Box::new(LogGaussian { params: names(&["Amplitude", "GeoMean", "GeoSD"]) })),
        "cumulative_gaussian" => Some(Box::new(CumulativeGaussian { params: names(&["Bottom", "Top", "Mean", "SD"]) })),
        "lorentzian" => Some(Box::new(LorentzianPeak { params: names(&["Amplitude", "Center", "Width"]) })),
        "weibull_peak" => Some(Box::new(WeibullPeak { params: names(&["Amplitude", "Scale", "Shape"]) })),
        "sine_wave" => Some(Box::new(SineWave { damped: false, params: names(&["Amplitude", "Frequency", "PhaseShift", "Baseline"]) })),
        "damped_sine_wave" => Some(Box::new(SineWave { damped: true, params: names(&["Amplitude", "Frequency", "PhaseShift", "K", "Baseline"]) })),
        "sinc_wave" => Some(Box::new(SincWave { params: names(&["Amplitude", "Center", "Width", "Baseline"]) })),
        "straight_line" => Some(Box::new(StraightLine { through_origin: false, params: names(&["Slope", "YIntercept"]) })),
        "line_through_origin" => Some(Box::new(StraightLine { through_origin: true, params: names(&["Slope"]) })),
        "segmental_linear" => Some(Box::new(SegmentalLinear { params: names(&["Intercept1", "Slope1", "X0", "Slope2"]) })),
        "hinge_smooth_hyperbolic" => Some(Box::new(SmoothHinge { params: names(&["Y0", "X0", "Slope1", "Slope2", "Delta"]) })),
        "polynomial_2" | "quadratic" => Some(Box::new(Polynomial { degree: 2 })),
        "polynomial_3" | "cubic" => Some(Box::new(Polynomial { degree: 3 })),
        "polynomial_4" | "quartic" => Some(Box::new(Polynomial { degree: 4 })),
        "polynomial_5" => Some(Box::new(Polynomial { degree: 5 })),
        "polynomial_6" => Some(Box::new(Polynomial { degree: 6 })),
        "power_series" => Some(Box::new(PowerCurve { params: names(&["a", "b"]) })),
        "logarithmic_curve" => Some(Box::new(LogarithmicCurve { params: names(&["a", "b"]) })),
        _ => None,
    }
}

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
}

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
}

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
        EquationCategory::Exponential
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (ym, y0, k) = (params[0], params[1], params[2]);
        Some(ym - (ym - y0) * safe_exp(-k * t)?)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, ymin, 2.0 / (xmax - xmin).max(1.0)]
    }
}

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
        let ratio = (ym - y0) / y0;
        let e = safe_exp(-k * t)?;
        let denom = y0 + (ym - y0) * e;
        if denom == 0.0 {
            return None;
        }
        let _ = ratio;
        Some(ym * y0 / denom)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0), ymin.max(1e-3), 3.0 / (xmax - xmin).max(1.0)]
    }
}

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
        if te <= tm || t > te {
            return Some(0.0);
        }
        let denom = te - tm;
        let base = (t / te).max(0.0);
        let exp = te / denom;
        let factor = 1.0 + (te - t) / denom;
        Some(ym * factor * base.powf(exp))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, xmax, 0.5 * (xmin + xmax)]
    }
}

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
        let cdf = lumencraft_math::dist::norm_cdf(z);
        Some(bottom + (top - bottom) * cdf)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymin, ymax, 0.5 * (xmin + xmax), ((xmax - xmin) / 6.0).max(1e-3)]
    }
}

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

struct PowerCurve {
    params: Vec<String>,
}

impl ParametricModel for PowerCurve {
    fn id(&self) -> &str {
        "power_series"
    }
    fn name(&self) -> &str {
        "Power curve (Y = a * X^b)"
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
