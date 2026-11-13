//! Canonical built-in equation library organized by [`EquationCategory`]: Dose–Response,
//! Receptor Binding, Enzyme Kinetics, Exponential, Growth, Peaks, Waves, Standard Curves /
//! Polynomials, Radiobiology, and all 27 NIST StRD nonlinear regression models.

pub mod curves;
pub mod nist;
pub mod pharma;

use crate::types::{DataPoint, DerivedScalar, EquationCategory};

/// Trait implemented by every built-in or user-defined parametric curve model.
pub trait ParametricModel: Send + Sync {
    /// Immutable canonical identifier (e.g., `"log_agonist_vs_response_4pl"`, `"michaelis_menten"`).
    fn id(&self) -> &str;
    /// Human-readable equation title.
    fn name(&self) -> &str;
    /// Library namespace category.
    fn category(&self) -> EquationCategory;
    /// Ordered parameter names matching indices `0..param_count()`.
    fn param_names(&self) -> &[String];
    /// Number of parameters `P`.
    fn param_count(&self) -> usize {
        self.param_names().len()
    }
    /// Evaluate `f(x; theta)` for observation predictor slice `x` (`x[0]` for 1D) and dataset index.
    /// Returns `None` if `x` or `params` fall outside the mathematical domain or overflow `f64`.
    fn eval(&self, x: &[f64], params: &[f64], dataset_index: usize) -> Option<f64>;
    /// Optional analytical gradient `df/dtheta_k` written into `out` (`len == param_count()`).
    /// Returns `true` if analytical derivatives were written, or `false` to fall back to central differences.
    fn grad(&self, _x: &[f64], _params: &[f64], _dataset_index: usize, _out: &mut [f64]) -> bool {
        false
    }
    /// Deterministic data-driven initial parameter estimates from `(x, y)` observations.
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64>;
    /// Optional default `(lower_bound, upper_bound)` per parameter.
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None); self.param_count()]
    }
    /// Derived scalar transforms (e.g., `EC50`, `t_half`, `Span`, inflection point).
    fn derived_transforms(&self, _params: &[f64]) -> Vec<DerivedScalar> {
        Vec::new()
    }
}

/// Metadata summary for cataloging available built-in equations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquationCatalogEntry {
    pub id: String,
    pub name: String,
    pub category: EquationCategory,
    pub param_names: Vec<String>,
}

/// Look up any built-in equation by canonical `equation_id` (case-insensitive, `-` / `_` normalized).
#[must_use]
pub fn get_builtin_model(equation_id: &str) -> Option<Box<dyn ParametricModel>> {
    let norm = normalize_id(equation_id);
    pharma::resolve_pharma_model(&norm).or_else(|| curves::resolve_curve_model(&norm)).or_else(|| nist::resolve_nist_model(&norm))
}

/// List all built-in equations across every [`EquationCategory`].
#[must_use]
pub fn list_builtin_equations() -> Vec<EquationCatalogEntry> {
    let mut ids = Vec::new();
    ids.extend(pharma::PHARMA_EQUATION_IDS);
    ids.extend(curves::CURVE_EQUATION_IDS);
    ids.extend(nist::NIST_EQUATION_IDS);
    ids.into_iter()
        .filter_map(|id| {
            let m = get_builtin_model(id)?;
            Some(EquationCatalogEntry {
                id: m.id().to_owned(),
                name: m.name().to_owned(),
                category: m.category(),
                param_names: m.param_names().to_vec(),
            })
        })
        .collect()
}

pub(crate) fn normalize_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

pub(crate) fn data_y_min_max(points: &[DataPoint]) -> (f64, f64) {
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    for p in points {
        if p.y.is_finite() {
            y_min = y_min.min(p.y);
            y_max = y_max.max(p.y);
        }
    }
    if !y_min.is_finite() || !y_max.is_finite() { (0.0, 1.0) } else { (y_min, y_max) }
}

pub(crate) fn data_x_min_max(points: &[DataPoint]) -> (f64, f64) {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    for p in points {
        let x = p.x0();
        if x.is_finite() {
            x_min = x_min.min(x);
            x_max = x_max.max(x);
        }
    }
    if !x_min.is_finite() || !x_max.is_finite() { (0.0, 1.0) } else { (x_min, x_max) }
}

/// Estimate X at which Y crosses the midpoint `(y_min + y_max) / 2` via sorted linear scan.
pub(crate) fn estimate_x_at_ymid(points: &[DataPoint]) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    let (y_min, y_max) = data_y_min_max(points);
    let y_mid = 0.5 * (y_min + y_max);
    let mut sorted: Vec<(f64, f64)> = points.iter().map(|p| (p.x0(), p.y)).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for w in sorted.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if (y0 <= y_mid && y1 >= y_mid) || (y0 >= y_mid && y1 <= y_mid) {
            let dy = y1 - y0;
            if dy.abs() > 1e-15 {
                let frac = ((y_mid - y0) / dy).clamp(0.0, 1.0);
                return x0 + frac * (x1 - x0);
            }
            return 0.5 * (x0 + x1);
        }
    }
    0.5 * (sorted.first().map_or(0.0, |v| v.0) + sorted.last().map_or(0.0, |v| v.0))
}
