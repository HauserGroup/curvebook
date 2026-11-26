//! Canonical built-in equation library organized by [`EquationCategory`]: Dose–Response,
//! Receptor Binding, Enzyme Kinetics, Exponential, Growth, Peaks, Waves, Standard Curves /
//! Polynomials, Radiobiology, and all 27 NIST StRD nonlinear regression models.
//!
//! Per D1 (AGENTS.md), every built-in equation cites the published source of its functional form:
//! full references sit in the `//!` header of each submodule ([`pharma`], [`curves`], [`nist`]) and
//! short `Author (Year)` citations on each model. General references for the nonlinear least-squares
//! models in this library:
//! - Motulsky, H. J., & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear
//!   and Nonlinear Regression: A Practical Guide to Curve Fitting*. New York: Oxford University Press.
//! - Seber, G. A. F., & Wild, C. J. (1989). *Nonlinear Regression*. New York: Wiley.
//! - Ratkowsky, D. A. (1990). *Handbook of Nonlinear Regression Models*. New York: Marcel Dekker.
//! - NIST Statistical Reference Datasets, Nonlinear Regression:
//!   <https://www.itl.nist.gov/div898/strd/nls/nls_main.shtml>.

/// Implement [`ParametricModel`] metadata methods (`id`, `name`, `category`, `param_names`)
/// from compile-time literals using a lazily initialized static parameter slice.
macro_rules! impl_static_meta {
    ($id:literal, $name:literal, $cat:expr, [$($param:literal),+ $(,)?]) => {
        fn id(&self) -> &str {
            $id
        }
        fn name(&self) -> &str {
            $name
        }
        fn category(&self) -> EquationCategory {
            $cat
        }
        fn param_names(&self) -> &[String] {
            static PARAMS: std::sync::LazyLock<Vec<String>> =
                std::sync::LazyLock::new(|| crate::library::param_names_vec(&[$($param),+]));
            &PARAMS
        }
    };
}

/// Delegate [`ParametricModel`] metadata methods (`id`, `name`, `category`, `param_names`)
/// to a `self.meta: ModelMeta` field.
macro_rules! impl_meta_methods {
    () => {
        fn id(&self) -> &str {
            self.meta.id
        }
        fn name(&self) -> &str {
            self.meta.name
        }
        fn category(&self) -> EquationCategory {
            self.meta.category
        }
        fn param_names(&self) -> &[String] {
            &self.meta.params
        }
    };
}

pub mod curves;
pub mod nist;
pub mod pharma;

use crate::types::{DataPoint, DerivedScalar, EquationCategory};
use curvebook_doc::ParameterRule;

/// Shared metadata descriptor for multi-variant [`ParametricModel`] implementations.
#[derive(Debug, Clone)]
pub(crate) struct ModelMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub category: EquationCategory,
    pub params: Vec<String>,
}

impl ModelMeta {
    pub(crate) fn new(id: &'static str, name: &'static str, category: EquationCategory, params: &[&str]) -> Self {
        Self { id, name, category, params: param_names_vec(params) }
    }
}

pub(crate) fn param_names_vec(slice: &[&str]) -> Vec<String> {
    slice.iter().map(|s| (*s).to_owned()).collect()
}

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
    /// Deterministic data-driven initial parameter estimates using optional numeric dataset
    /// column-title values (e.g., for custom DSL `MeanColumnTitle` / `LogMeanColumnTitle`
    /// initial-value anchors). Defaults to [`ParametricModel::initial_guess`].
    fn initial_guess_with_column_titles(&self, points: &[DataPoint], column_titles: &[f64]) -> Vec<f64> {
        let _ = column_titles;
        self.initial_guess(points)
    }
    /// Whether `eval` and `grad` interpret `dataset_index` as the raw sheet column index
    /// (e.g., `<A>`, `<B>` dataset selectors in [`crate::CompiledDslModel`]) rather than the
    /// 0-based ordinal index among selected datasets. Built-in multi-dataset models use
    /// ordinal indices (`false`) so non-column-A selections route correctly.
    fn uses_sheet_dataset_index(&self) -> bool {
        false
    }
    /// Optional default `(lower_bound, upper_bound)` per parameter.
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None); self.param_count()]
    }
    /// Optional default parameter rules (such as fixing `XCenter` to the sample mean in centered
    /// polynomials so the Jacobian stays full rank) when the caller provides no explicit rule for
    /// that parameter name.
    fn default_parameter_rules(&self, _points: &[DataPoint]) -> Vec<ParameterRule> {
        Vec::new()
    }
    /// Derived scalar transforms (e.g., `EC50`, `t_half`, `Span`, inflection point).
    fn derived_transforms(&self, _params: &[f64]) -> Vec<DerivedScalar> {
        Vec::new()
    }
}

/// Metadata summary for cataloging available built-in equations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquationCatalogEntry {
    /// Canonical snake_case identifier accepted by `get_builtin_model` (case-insensitive, `-` / `_` normalized).
    pub id: String,
    /// Human-readable display name shown in equation pickers and report headers.
    pub name: String,
    /// Functional equation family category (`EquationCategory`).
    pub category: EquationCategory,
    /// Ordered parameter names matching the model's parameter vector layout.
    pub param_names: Vec<String>,
}

/// Single-source-of-truth registration entry for a built-in parametric model.
pub(crate) struct BuiltinEquationSpec {
    /// Canonical snake_case identifier exposed in the catalogue.
    pub id: &'static str,
    /// Additional normalized aliases accepted by [`get_builtin_model`].
    pub aliases: &'static [&'static str],
    /// Factory constructing the model instance.
    pub build: fn() -> Box<dyn ParametricModel>,
}

impl BuiltinEquationSpec {
    #[must_use]
    pub(crate) const fn new(id: &'static str, aliases: &'static [&'static str], build: fn() -> Box<dyn ParametricModel>) -> Self {
        Self { id, aliases, build }
    }

    #[must_use]
    pub fn matches(&self, norm_id: &str) -> bool {
        self.id == norm_id || self.aliases.contains(&norm_id)
    }

    #[must_use]
    pub fn catalog_entry(&self) -> EquationCatalogEntry {
        let m = (self.build)();
        EquationCatalogEntry { id: m.id().to_owned(), name: m.name().to_owned(), category: m.category(), param_names: m.param_names().to_vec() }
    }
}

/// Resolve a normalized equation identifier against an ordered family registration slice.
pub(crate) fn resolve_from_specs(specs: &[BuiltinEquationSpec], norm_id: &str) -> Option<Box<dyn ParametricModel>> {
    specs.iter().find(|spec| spec.matches(norm_id)).map(|spec| (spec.build)())
}

/// Extract canonical IDs from an ordered family registration slice at compile time.
pub(crate) const fn spec_ids<const N: usize>(specs: &[BuiltinEquationSpec]) -> [&'static str; N] {
    let mut out = [""; N];
    let mut i = 0;
    while i < N && i < specs.len() {
        out[i] = specs[i].id;
        i += 1;
    }
    out
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
    pharma::PHARMA_SPECS.iter().chain(curves::CURVE_SPECS.iter()).chain(nist::NIST_SPECS.iter()).map(BuiltinEquationSpec::catalog_entry).collect()
}

pub(crate) fn normalize_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

#[inline]
pub(crate) fn safe_exp(arg: f64) -> Option<f64> {
    if !arg.is_finite() || arg > 700.0 {
        return None;
    }
    let v = arg.exp();
    v.is_finite().then_some(v)
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
    let mut sorted: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
    if sorted.is_empty() {
        return 0.0;
    }
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    for &(_, y) in &sorted {
        y_min = y_min.min(y);
        y_max = y_max.max(y);
    }
    let y_mid = 0.5 * y_min + 0.5 * y_max;
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for w in sorted.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if (y0 <= y_mid && y1 >= y_mid) || (y0 >= y_mid && y1 <= y_mid) {
            let dy = y1 - y0;
            if dy.abs() > 1e-15 && dy.is_finite() {
                let frac = ((y_mid - y0) / dy).clamp(0.0, 1.0);
                let est = x0 + frac * (x1 - x0);
                return if est.is_finite() { est } else { 0.5 * x0 + 0.5 * x1 };
            }
            return 0.5 * x0 + 0.5 * x1;
        }
    }
    0.5 * sorted[0].0 + 0.5 * sorted[sorted.len() - 1].0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{curves, get_builtin_model, list_builtin_equations, nist, pharma};

    #[test]
    fn family_registrations_drive_catalog_and_resolver_without_drift() {
        let mut seen_tokens = BTreeSet::new();
        let all_specs = pharma::PHARMA_SPECS.iter().chain(curves::CURVE_SPECS.iter()).chain(nist::NIST_SPECS.iter());

        let mut expected_ids = Vec::new();
        for spec in all_specs {
            assert!(seen_tokens.insert(spec.id), "duplicate canonical equation id `{}`", spec.id);
            let built = (spec.build)();
            assert_eq!(built.id(), spec.id, "factory id mismatch for `{}`", spec.id);
            let resolved = get_builtin_model(spec.id).unwrap_or_else(|| panic!("canonical id `{}` failed to resolve", spec.id));
            assert_eq!(resolved.id(), spec.id);
            for &alias in spec.aliases {
                assert!(seen_tokens.insert(alias), "duplicate equation alias `{alias}`");
                let alias_model = get_builtin_model(alias).unwrap_or_else(|| panic!("alias `{alias}` failed to resolve"));
                assert_eq!(alias_model.id(), spec.id, "alias `{alias}` resolved to wrong canonical id");
            }
            expected_ids.push(spec.id);
        }

        let pharma_spec_ids: Vec<&str> = pharma::PHARMA_SPECS.iter().map(|s| s.id).collect();
        let curve_spec_ids: Vec<&str> = curves::CURVE_SPECS.iter().map(|s| s.id).collect();
        let nist_spec_ids: Vec<&str> = nist::NIST_SPECS.iter().map(|s| s.id).collect();
        assert_eq!(pharma::PHARMA_EQUATION_IDS, pharma_spec_ids.as_slice());
        assert_eq!(curves::CURVE_EQUATION_IDS, curve_spec_ids.as_slice());
        assert_eq!(nist::NIST_EQUATION_IDS, nist_spec_ids.as_slice());

        let catalog = list_builtin_equations();
        let catalog_ids: Vec<&str> = catalog.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(catalog_ids, expected_ids);
        assert_eq!(catalog.len(), 68 + 56 + 27);

        // Spot-check representative aliases across all three families (including hyphen/case normalization).
        assert_eq!(get_builtin_model("4PL").unwrap().id(), "log_agonist_vs_response_4pl");
        assert_eq!(get_builtin_model("gaddum-schild").unwrap().id(), "ec50_shift_gaddum_schild");
        assert_eq!(get_builtin_model("centered-quadratic").unwrap().id(), "centered_poly_2");
        assert_eq!(get_builtin_model("semilog_line").unwrap().id(), "semi_log_line");
        assert_eq!(get_builtin_model("misra1a").unwrap().id(), "nist_misra1a");
        assert_eq!(get_builtin_model("nist_bennett5").unwrap().id(), "nist_bennett5");
        assert!(get_builtin_model("nist_nist_misra1a").is_none());
    }
}
