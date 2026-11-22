//! All 27 canonical NIST StRD nonlinear least-squares benchmark models with exact analytical
//! Jacobians and domain-safe evaluations.
//!
//! Reference: National Institute of Standards and Technology, Statistical Reference Datasets
//! (StRD), Nonlinear Regression collection, <https://www.itl.nist.gov/div898/strd/nls/nls_main.shtml>.
//! Each model function below is transcribed from the certified-model header of the corresponding
//! NIST `.dat` file (parameters `b1..bk` map to indices `0..k-1`). Original data sources, as listed
//! by NIST in each dataset file:
//! - Misra1a, Misra1b, Misra1c, Misra1d: Misra, D., NIST (1978). Dental Research Monomolecular
//!   Adsorption Study.
//! - Chwirut1, Chwirut2: Chwirut, D., NIST (197?). Ultrasonic Reference Block Study.
//! - Lanczos1, Lanczos2, Lanczos3: Lanczos, C. (1956). *Applied Analysis*. Englewood Cliffs, NJ:
//!   Prentice Hall, pp. 272–280.
//! - Gauss1, Gauss2, Gauss3: Rust, B., NIST (1996) (generated data).
//! - DanWood: Daniel, C., & Wood, F. S. (1980). *Fitting Equations to Data*, 2nd ed. New York:
//!   Wiley, pp. 428–431.
//! - Kirby2: Kirby, R., NIST (197?). Scanning electron microscope line width standards.
//! - Hahn1: Hahn, T., NIST (197?). Copper Thermal Expansion Study.
//! - Nelson: Nelson, W. (1981). Analysis of Performance-Degradation Data. *IEEE Transactions on
//!   Reliability* R-30(2):149–155. (The certified response is `log[y]`.)
//! - MGH09: Kowalik, J. S., & Osborne, M. R. (1978). *Methods for Unconstrained Optimization
//!   Problems*. New York: Elsevier North-Holland.
//! - MGH10: Meyer, R. R. (1970). Theoretical and computational aspects of nonlinear regression.
//!   In Rosen, Mangasarian & Ritter (Eds.), *Nonlinear Programming*. New York: Academic Press,
//!   pp. 465–486.
//! - MGH17: Osborne, M. R. (1972). Some aspects of nonlinear least squares calculations. In
//!   Lootsma (Ed.), *Numerical Methods for Nonlinear Optimization*. New York: Academic Press,
//!   pp. 171–189.
//! - Roszman1: Roszman, L., NIST (19??). Quantum Defects for Sulfur I Atom.
//! - ENSO: Kahaner, D., Moler, C., & Nash, S. (1989). *Numerical Methods and Software*.
//!   Englewood Cliffs, NJ: Prentice Hall, pp. 441–445.
//! - Thurber: Thurber, R., NIST (197?). Semiconductor electron mobility modeling.
//! - BoxBOD: Box, G. E. P., Hunter, W. G., & Hunter, J. S. (1978). *Statistics for
//!   Experimenters*. New York: Wiley, pp. 483–487.
//! - Rat42, Rat43: Ratkowsky, D. A. (1983). *Nonlinear Regression Modeling*. New York: Marcel
//!   Dekker (Rat42 pp. 61 and 88; Rat43 pp. 62 and 88).
//! - Eckerle4: Eckerle, K., NIST (197?). Circular Interference Transmittance Study.
//! - Bennett5: Bennett, L., Swartzendruber, L., & Brown, H., NIST (1994). Superconductivity
//!   Magnetization Modeling.

use std::f64::consts::PI;

use super::{BuiltinEquationSpec, ModelMeta, ParametricModel, data_x_min_max, data_y_min_max, resolve_from_specs, safe_exp, spec_ids};
use crate::types::{DataPoint, EquationCategory};

/// Ordered single-source-of-truth registrations for all 27 NIST StRD nonlinear models.
pub(crate) const NIST_SPECS: &[BuiltinEquationSpec] = &[
    BuiltinEquationSpec::new("nist_misra1a", &["misra1a"], || Box::new(NistModel::new("nist_misra1a", "NIST Misra1a", NistKind::Misra1a, 2))),
    BuiltinEquationSpec::new("nist_chwirut2", &["chwirut2"], || Box::new(NistModel::new("nist_chwirut2", "NIST Chwirut2", NistKind::Chwirut, 3))),
    BuiltinEquationSpec::new("nist_chwirut1", &["chwirut1"], || Box::new(NistModel::new("nist_chwirut1", "NIST Chwirut1", NistKind::Chwirut, 3))),
    BuiltinEquationSpec::new("nist_lanczos3", &["lanczos3"], || Box::new(NistModel::new("nist_lanczos3", "NIST Lanczos3", NistKind::Lanczos, 6))),
    BuiltinEquationSpec::new("nist_gauss1", &["gauss1"], || Box::new(NistModel::new("nist_gauss1", "NIST Gauss1", NistKind::Gauss, 8))),
    BuiltinEquationSpec::new("nist_gauss2", &["gauss2"], || Box::new(NistModel::new("nist_gauss2", "NIST Gauss2", NistKind::Gauss, 8))),
    BuiltinEquationSpec::new("nist_danwood", &["danwood"], || Box::new(NistModel::new("nist_danwood", "NIST DanWood", NistKind::DanWood, 2))),
    BuiltinEquationSpec::new("nist_misra1b", &["misra1b"], || Box::new(NistModel::new("nist_misra1b", "NIST Misra1b", NistKind::Misra1b, 2))),
    BuiltinEquationSpec::new("nist_kirby2", &["kirby2"], || Box::new(NistModel::new("nist_kirby2", "NIST Kirby2", NistKind::Kirby2, 5))),
    BuiltinEquationSpec::new("nist_hahn1", &["hahn1"], || Box::new(NistModel::new("nist_hahn1", "NIST Hahn1", NistKind::Hahn1, 7))),
    BuiltinEquationSpec::new("nist_nelson", &["nelson"], || Box::new(NistModel::new("nist_nelson", "NIST Nelson", NistKind::Nelson, 3))),
    BuiltinEquationSpec::new("nist_mgh17", &["mgh17"], || Box::new(NistModel::new("nist_mgh17", "NIST MGH17", NistKind::Mgh17, 5))),
    BuiltinEquationSpec::new("nist_lanczos1", &["lanczos1"], || Box::new(NistModel::new("nist_lanczos1", "NIST Lanczos1", NistKind::Lanczos, 6))),
    BuiltinEquationSpec::new("nist_lanczos2", &["lanczos2"], || Box::new(NistModel::new("nist_lanczos2", "NIST Lanczos2", NistKind::Lanczos, 6))),
    BuiltinEquationSpec::new("nist_gauss3", &["gauss3"], || Box::new(NistModel::new("nist_gauss3", "NIST Gauss3", NistKind::Gauss, 8))),
    BuiltinEquationSpec::new("nist_misra1c", &["misra1c"], || Box::new(NistModel::new("nist_misra1c", "NIST Misra1c", NistKind::Misra1c, 2))),
    BuiltinEquationSpec::new("nist_misra1d", &["misra1d"], || Box::new(NistModel::new("nist_misra1d", "NIST Misra1d", NistKind::Misra1d, 2))),
    BuiltinEquationSpec::new("nist_roszman1", &["roszman1"], || Box::new(NistModel::new("nist_roszman1", "NIST Roszman1", NistKind::Roszman1, 4))),
    BuiltinEquationSpec::new("nist_enso", &["enso"], || Box::new(NistModel::new("nist_enso", "NIST ENSO", NistKind::Enso, 9))),
    BuiltinEquationSpec::new("nist_mgh09", &["mgh09"], || Box::new(NistModel::new("nist_mgh09", "NIST MGH09", NistKind::Mgh09, 4))),
    BuiltinEquationSpec::new("nist_thurber", &["thurber"], || Box::new(NistModel::new("nist_thurber", "NIST Thurber", NistKind::Thurber, 7))),
    BuiltinEquationSpec::new("nist_boxbod", &["boxbod"], || Box::new(NistModel::new("nist_boxbod", "NIST BoxBOD", NistKind::BoxBod, 2))),
    BuiltinEquationSpec::new("nist_rat42", &["rat42"], || Box::new(NistModel::new("nist_rat42", "NIST Rat42", NistKind::Rat42, 3))),
    BuiltinEquationSpec::new("nist_mgh10", &["mgh10"], || Box::new(NistModel::new("nist_mgh10", "NIST MGH10", NistKind::Mgh10, 3))),
    BuiltinEquationSpec::new("nist_eckerle4", &["eckerle4"], || Box::new(NistModel::new("nist_eckerle4", "NIST Eckerle4", NistKind::Eckerle4, 3))),
    BuiltinEquationSpec::new("nist_rat43", &["rat43"], || Box::new(NistModel::new("nist_rat43", "NIST Rat43", NistKind::Rat43, 4))),
    BuiltinEquationSpec::new("nist_bennett5", &["bennett5"], || Box::new(NistModel::new("nist_bennett5", "NIST Bennett5", NistKind::Bennett5, 3))),
];

const NIST_EQUATION_ID_ARRAY: [&str; NIST_SPECS.len()] = spec_ids::<{ NIST_SPECS.len() }>(NIST_SPECS);

/// Canonical identifiers of all 27 NIST Statistical Reference Datasets (StRD) nonlinear models.
pub const NIST_EQUATION_IDS: &[&str] = &NIST_EQUATION_ID_ARRAY;

/// Functional form family implemented by a NIST StRD nonlinear benchmark model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NistKind {
    /// Misra1a `y = b1 * (1 - exp(-b2 * x))` (NIST StRD; Misra 1978); shares its evaluation with `BoxBod`.
    Misra1a,
    /// Chwirut1/2 `y = exp(-b1 * x) / (b2 + b3 * x)` (NIST StRD; Chwirut, NIST).
    Chwirut,
    /// Lanczos1/2/3 `y = b1 e^(-b2 x) + b3 e^(-b4 x) + b5 e^(-b6 x)` (NIST StRD; Lanczos 1956).
    Lanczos,
    /// Gauss1/2/3 exponential baseline plus two Gaussian peaks (NIST StRD; Rust 1996).
    Gauss,
    /// DanWood `y = b1 * x^b2` (NIST StRD; Daniel & Wood 1980).
    DanWood,
    /// Misra1b `y = b1 * (1 - (1 + b2 x / 2)^(-2))` (NIST StRD; Misra 1978).
    Misra1b,
    /// Kirby2 quadratic/quadratic rational (NIST StRD; Kirby, NIST).
    Kirby2,
    /// Hahn1 cubic/cubic rational (NIST StRD; Hahn, NIST).
    Hahn1,
    /// Nelson `log[y] = b1 - b2 * x1 * exp(-b3 * x2)`; predicts `log[y]` (NIST StRD; Nelson 1981).
    Nelson,
    /// MGH17 `y = b1 + b2 e^(-x b4) + b3 e^(-x b5)` (NIST StRD; Osborne 1972).
    Mgh17,
    /// Misra1c `y = b1 * (1 - (1 + 2 b2 x)^(-1/2))` (NIST StRD; Misra 1978).
    Misra1c,
    /// Misra1d `y = b1 b2 x / (1 + b2 x)` (NIST StRD; Misra 1978).
    Misra1d,
    /// Roszman1 `y = b1 - b2 x - arctan(b3 / (x - b4)) / pi` (NIST StRD; Roszman, NIST).
    Roszman1,
    /// ENSO annual plus two free-period harmonics (NIST StRD; Kahaner, Moler & Nash 1989).
    Enso,
    /// MGH09 `y = b1 (x^2 + x b2) / (x^2 + x b3 + b4)` (NIST StRD; Kowalik & Osborne 1978).
    Mgh09,
    /// Thurber cubic/cubic rational (NIST StRD; Thurber, NIST); shares its evaluation with `Hahn1`.
    Thurber,
    /// BoxBOD `y = b1 * (1 - exp(-b2 * x))` (NIST StRD; Box, Hunter & Hunter 1978).
    BoxBod,
    /// Rat42 logistic `y = b1 / (1 + exp(b2 - b3 x))` (NIST StRD; Ratkowsky 1983).
    Rat42,
    /// MGH10 `y = b1 * exp(b2 / (x + b3))` (NIST StRD; Meyer 1970).
    Mgh10,
    /// Eckerle4 `y = (b1 / b2) * exp(-0.5 ((x - b3) / b2)^2)` (NIST StRD; Eckerle, NIST).
    Eckerle4,
    /// Rat43 Richards-type `y = b1 / (1 + exp(b2 - b3 x))^(1/b4)` (NIST StRD; Ratkowsky 1983).
    Rat43,
    /// Bennett5 `y = b1 * (b2 + x)^(-1/b3)` (NIST StRD; Bennett, Swartzendruber & Brown 1994).
    Bennett5,
}

/// Parametric model wrapper for a NIST StRD nonlinear regression benchmark dataset equation.
#[derive(Debug, Clone)]
pub struct NistModel {
    meta: ModelMeta,
    kind: NistKind,
}

impl NistModel {
    fn new(id: &'static str, name: &'static str, kind: NistKind, count: usize) -> Self {
        let params = (1..=count).map(|i| format!("b{i}")).collect();
        Self { meta: ModelMeta { id, name, category: EquationCategory::NistBenchmark, params }, kind }
    }
}

/// Resolve any of the 27 NIST StRD nonlinear model identifiers.
#[must_use]
pub fn resolve_nist_model(norm_id: &str) -> Option<Box<dyn ParametricModel>> {
    resolve_from_specs(NIST_SPECS, norm_id)
}

impl ParametricModel for NistModel {
    impl_meta_methods!();

    fn eval(&self, x_vec: &[f64], b: &[f64], _dataset_index: usize) -> Option<f64> {
        let x = *x_vec.first()?;
        let val = match self.kind {
            NistKind::Misra1a | NistKind::BoxBod => b[0] * (1.0 - safe_exp(-b[1] * x)?),
            NistKind::Chwirut => {
                let d = b[1] + b[2] * x;
                if d == 0.0 {
                    return None;
                }
                safe_exp(-b[0] * x)? / d
            }
            NistKind::Lanczos => b[0] * safe_exp(-b[1] * x)? + b[2] * safe_exp(-b[3] * x)? + b[4] * safe_exp(-b[5] * x)?,
            NistKind::Gauss => {
                if b[4] == 0.0 || b[7] == 0.0 {
                    return None;
                }
                let u1 = (x - b[3]) / b[4];
                let u2 = (x - b[6]) / b[7];
                b[0] * safe_exp(-b[1] * x)? + b[2] * safe_exp(-(u1 * u1))? + b[5] * safe_exp(-(u2 * u2))?
            }
            NistKind::DanWood => {
                if x <= 0.0 {
                    return None;
                }
                b[0] * x.powf(b[1])
            }
            NistKind::Misra1b => {
                let u = 1.0 + 0.5 * b[1] * x;
                if u <= 0.0 {
                    return None;
                }
                b[0] * (1.0 - u.powi(-2))
            }
            NistKind::Kirby2 => {
                let num = b[0] + b[1] * x + b[2] * x * x;
                let den = 1.0 + b[3] * x + b[4] * x * x;
                if den == 0.0 {
                    return None;
                }
                num / den
            }
            NistKind::Hahn1 | NistKind::Thurber => {
                let x2 = x * x;
                let x3 = x2 * x;
                let num = b[0] + b[1] * x + b[2] * x2 + b[3] * x3;
                let den = 1.0 + b[4] * x + b[5] * x2 + b[6] * x3;
                if den == 0.0 {
                    return None;
                }
                num / den
            }
            NistKind::Nelson => {
                let x2 = *x_vec.get(1)?;
                b[0] - b[1] * x * safe_exp(-b[2] * x2)?
            }
            NistKind::Mgh17 => b[0] + b[1] * safe_exp(-x * b[3])? + b[2] * safe_exp(-x * b[4])?,
            NistKind::Misra1c => {
                let u = 1.0 + 2.0 * b[1] * x;
                if u <= 0.0 {
                    return None;
                }
                b[0] * (1.0 - u.powf(-0.5))
            }
            NistKind::Misra1d => {
                let den = 1.0 + b[1] * x;
                if den == 0.0 {
                    return None;
                }
                b[0] * b[1] * x / den
            }
            NistKind::Roszman1 => {
                let dx = x - b[3];
                if dx == 0.0 {
                    return None;
                }
                b[0] - b[1] * x - (b[2] / dx).atan() / PI
            }
            NistKind::Enso => {
                if b[3] == 0.0 || b[6] == 0.0 {
                    return None;
                }
                let t0 = 2.0 * PI * x / 12.0;
                let t1 = 2.0 * PI * x / b[3];
                let t2 = 2.0 * PI * x / b[6];
                b[0] + b[1] * t0.cos() + b[2] * t0.sin() + b[4] * t1.cos() + b[5] * t1.sin() + b[7] * t2.cos() + b[8] * t2.sin()
            }
            NistKind::Mgh09 => {
                let x2 = x * x;
                let den = x2 + x * b[2] + b[3];
                if den == 0.0 {
                    return None;
                }
                b[0] * (x2 + x * b[1]) / den
            }
            NistKind::Rat42 => {
                let e = safe_exp(b[1] - b[2] * x)?;
                b[0] / (1.0 + e)
            }
            NistKind::Mgh10 => {
                let dx = x + b[2];
                if dx == 0.0 {
                    return None;
                }
                b[0] * safe_exp(b[1] / dx)?
            }
            NistKind::Eckerle4 => {
                if b[1] == 0.0 {
                    return None;
                }
                let u = (x - b[2]) / b[1];
                (b[0] / b[1]) * safe_exp(-0.5 * u * u)?
            }
            NistKind::Rat43 => {
                if b[3] == 0.0 {
                    return None;
                }
                let e = safe_exp(b[1] - b[2] * x)?;
                let base = 1.0 + e;
                if base <= 0.0 {
                    return None;
                }
                b[0] * base.powf(-1.0 / b[3])
            }
            NistKind::Bennett5 => {
                if b[2] == 0.0 {
                    return None;
                }
                let base = b[1] + x;
                if base <= 0.0 {
                    return None;
                }
                b[0] * base.powf(-1.0 / b[2])
            }
        };
        val.is_finite().then_some(val)
    }

    fn grad(&self, x_vec: &[f64], b: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x) = x_vec.first() else {
            return false;
        };
        match self.kind {
            NistKind::Misra1a | NistKind::BoxBod => {
                let Some(e) = safe_exp(-b[1] * x) else {
                    return false;
                };
                out[0] = 1.0 - e;
                out[1] = b[0] * x * e;
            }
            NistKind::Chwirut => {
                let d = b[1] + b[2] * x;
                let Some(e) = safe_exp(-b[0] * x) else {
                    return false;
                };
                if d == 0.0 {
                    return false;
                }
                let inv_d = 1.0 / d;
                let f = e * inv_d;
                out[0] = -x * f;
                out[1] = -f * inv_d;
                out[2] = -x * f * inv_d;
            }
            NistKind::Lanczos => {
                let (Some(e1), Some(e2), Some(e3)) = (safe_exp(-b[1] * x), safe_exp(-b[3] * x), safe_exp(-b[5] * x)) else {
                    return false;
                };
                out[0] = e1;
                out[1] = -b[0] * x * e1;
                out[2] = e2;
                out[3] = -b[2] * x * e2;
                out[4] = e3;
                out[5] = -b[4] * x * e3;
            }
            NistKind::Gauss => {
                if b[4] == 0.0 || b[7] == 0.0 {
                    return false;
                }
                let u1 = (x - b[3]) / b[4];
                let u2 = (x - b[6]) / b[7];
                let (Some(e0), Some(e1), Some(e2)) = (safe_exp(-b[1] * x), safe_exp(-u1 * u1), safe_exp(-u2 * u2)) else {
                    return false;
                };
                out[0] = e0;
                out[1] = -b[0] * x * e0;
                out[2] = e1;
                out[3] = b[2] * e1 * (2.0 * u1 / b[4]);
                out[4] = b[2] * e1 * (2.0 * u1 * u1 / b[4]);
                out[5] = e2;
                out[6] = b[5] * e2 * (2.0 * u2 / b[7]);
                out[7] = b[5] * e2 * (2.0 * u2 * u2 / b[7]);
            }
            NistKind::DanWood => {
                if x <= 0.0 {
                    return false;
                }
                let xp = x.powf(b[1]);
                out[0] = xp;
                out[1] = b[0] * xp * x.ln();
            }
            NistKind::Misra1b => {
                let u = 1.0 + 0.5 * b[1] * x;
                if u <= 0.0 {
                    return false;
                }
                out[0] = 1.0 - u.powi(-2);
                out[1] = b[0] * x * u.powi(-3);
            }
            NistKind::Kirby2 => {
                let x2 = x * x;
                let num = b[0] + b[1] * x + b[2] * x2;
                let den = 1.0 + b[3] * x + b[4] * x2;
                if den == 0.0 {
                    return false;
                }
                let inv_d = 1.0 / den;
                let f = num * inv_d;
                out[0] = inv_d;
                out[1] = x * inv_d;
                out[2] = x2 * inv_d;
                out[3] = -f * x * inv_d;
                out[4] = -f * x2 * inv_d;
            }
            NistKind::Hahn1 | NistKind::Thurber => {
                let x2 = x * x;
                let x3 = x2 * x;
                let num = b[0] + b[1] * x + b[2] * x2 + b[3] * x3;
                let den = 1.0 + b[4] * x + b[5] * x2 + b[6] * x3;
                if den == 0.0 {
                    return false;
                }
                let inv_d = 1.0 / den;
                let f = num * inv_d;
                out[0] = inv_d;
                out[1] = x * inv_d;
                out[2] = x2 * inv_d;
                out[3] = x3 * inv_d;
                out[4] = -f * x * inv_d;
                out[5] = -f * x2 * inv_d;
                out[6] = -f * x3 * inv_d;
            }
            NistKind::Nelson => {
                let Some(&x2) = x_vec.get(1) else {
                    return false;
                };
                let Some(e) = safe_exp(-b[2] * x2) else {
                    return false;
                };
                out[0] = 1.0;
                out[1] = -x * e;
                out[2] = b[1] * x * x2 * e;
            }
            NistKind::Mgh17 => {
                let (Some(e1), Some(e2)) = (safe_exp(-x * b[3]), safe_exp(-x * b[4])) else {
                    return false;
                };
                out[0] = 1.0;
                out[1] = e1;
                out[2] = e2;
                out[3] = -b[1] * x * e1;
                out[4] = -b[2] * x * e2;
            }
            NistKind::Misra1c => {
                let u = 1.0 + 2.0 * b[1] * x;
                if u <= 0.0 {
                    return false;
                }
                out[0] = 1.0 - u.powf(-0.5);
                out[1] = b[0] * x * u.powf(-1.5);
            }
            NistKind::Misra1d => {
                let den = 1.0 + b[1] * x;
                if den == 0.0 {
                    return false;
                }
                out[0] = b[1] * x / den;
                out[1] = b[0] * x / (den * den);
            }
            NistKind::Roszman1 => {
                let dx = x - b[3];
                if dx == 0.0 {
                    return false;
                }
                let z = b[2] / dx;
                let denom = PI * (1.0 + z * z);
                out[0] = 1.0;
                out[1] = -x;
                out[2] = -(1.0 / dx) / denom;
                out[3] = -(b[2] / (dx * dx)) / denom;
            }
            NistKind::Enso => {
                if b[3] == 0.0 || b[6] == 0.0 {
                    return false;
                }
                let t0 = 2.0 * PI * x / 12.0;
                let t1 = 2.0 * PI * x / b[3];
                let t2 = 2.0 * PI * x / b[6];
                let (s1, c1) = t1.sin_cos();
                let (s2, c2) = t2.sin_cos();
                out[0] = 1.0;
                out[1] = t0.cos();
                out[2] = t0.sin();
                out[3] = (b[4] * s1 - b[5] * c1) * (t1 / b[3]);
                out[4] = c1;
                out[5] = s1;
                out[6] = (b[7] * s2 - b[8] * c2) * (t2 / b[6]);
                out[7] = c2;
                out[8] = s2;
            }
            NistKind::Mgh09 => {
                let x2 = x * x;
                let num = x2 + x * b[1];
                let den = x2 + x * b[2] + b[3];
                if den == 0.0 {
                    return false;
                }
                let inv_d = 1.0 / den;
                let f = b[0] * num * inv_d;
                out[0] = num * inv_d;
                out[1] = b[0] * x * inv_d;
                out[2] = -f * x * inv_d;
                out[3] = -f * inv_d;
            }
            NistKind::Rat42 => {
                let Some(e) = safe_exp(b[1] - b[2] * x) else {
                    return false;
                };
                let d = 1.0 + e;
                let inv_d = 1.0 / d;
                let f = b[0] * inv_d;
                out[0] = inv_d;
                out[1] = -f * e * inv_d;
                out[2] = f * x * e * inv_d;
            }
            NistKind::Mgh10 => {
                let dx = x + b[2];
                if dx == 0.0 {
                    return false;
                }
                let Some(e) = safe_exp(b[1] / dx) else {
                    return false;
                };
                let f = b[0] * e;
                out[0] = e;
                out[1] = f / dx;
                out[2] = -f * b[1] / (dx * dx);
            }
            NistKind::Eckerle4 => {
                if b[1] == 0.0 {
                    return false;
                }
                let u = (x - b[2]) / b[1];
                let Some(e) = safe_exp(-0.5 * u * u) else {
                    return false;
                };
                let f = (b[0] / b[1]) * e;
                out[0] = e / b[1];
                out[1] = f * (u * u - 1.0) / b[1];
                out[2] = f * u / b[1];
            }
            NistKind::Rat43 => {
                if b[3] == 0.0 {
                    return false;
                }
                let Some(e) = safe_exp(b[1] - b[2] * x) else {
                    return false;
                };
                let base = 1.0 + e;
                if base <= 0.0 {
                    return false;
                }
                let pow_term = base.powf(-1.0 / b[3]);
                let f = b[0] * pow_term;
                out[0] = pow_term;
                out[1] = -f * (e / base) / b[3];
                out[2] = f * x * (e / base) / b[3];
                out[3] = f * base.ln() / (b[3] * b[3]);
            }
            NistKind::Bennett5 => {
                if b[2] == 0.0 {
                    return false;
                }
                let base = b[1] + x;
                if base <= 0.0 {
                    return false;
                }
                let pow_term = base.powf(-1.0 / b[2]);
                let f = b[0] * pow_term;
                out[0] = pow_term;
                out[1] = -f / (b[2] * base);
                out[2] = f * base.ln() / (b[2] * b[2]);
            }
        }
        out.iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (y_min, y_max) = data_y_min_max(points);
        let (x_min, x_max) = data_x_min_max(points);
        let span_x = (x_max - x_min).max(1.0);
        match self.kind {
            NistKind::Misra1a | NistKind::BoxBod | NistKind::Misra1b | NistKind::Misra1c | NistKind::Misra1d => {
                vec![y_max.max(1.0), 1.0 / span_x]
            }
            NistKind::Chwirut => vec![0.1, 0.01, 0.01],
            NistKind::Lanczos => vec![0.1, 1.0, 1.0, 3.0, 1.5, 5.0],
            NistKind::Gauss => {
                vec![y_max * 0.5, 0.01, y_max * 0.5, x_min + 0.3 * span_x, 0.1 * span_x, y_max * 0.5, x_min + 0.6 * span_x, 0.1 * span_x]
            }
            NistKind::DanWood => vec![1.0, 1.0],
            NistKind::Kirby2 => vec![y_min, 0.0, 0.0, 0.0, 0.0],
            NistKind::Hahn1 | NistKind::Thurber => vec![y_min, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            NistKind::Nelson => vec![y_min, 1e-4, -0.01],
            NistKind::Mgh17 => vec![y_min, y_max - y_min, -(y_max - y_min) * 0.5, 0.01, 0.02],
            NistKind::Roszman1 => vec![0.2, 0.0, 1000.0, -100.0],
            NistKind::Enso => vec![0.5 * (y_min + y_max), 1.0, 1.0, 44.0, 1.0, 1.0, 26.0, 1.0, 1.0],
            NistKind::Mgh09 => vec![0.2, 0.2, 0.2, 0.2],
            NistKind::Rat42 => vec![y_max, 2.0, 1.0 / span_x],
            NistKind::Mgh10 => vec![0.01, 5000.0, 300.0],
            NistKind::Eckerle4 => vec![1.5, 5.0, 0.5 * (x_min + x_max)],
            NistKind::Rat43 => vec![y_max, 5.0, 1.0 / span_x, 1.0],
            NistKind::Bennett5 => vec![y_min, -x_min + 50.0, 1.0],
        }
    }
}
