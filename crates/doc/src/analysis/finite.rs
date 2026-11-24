//! Finite-float invariants for analysis specifications.
//!
//! Persisted project files and engine commands must never carry `NaN` or `±Inf` in user-set
//! analysis parameters. [`AnalysisSpec::first_non_finite_float`] finds the first offending field so
//! `curvebook-format` can reject a document with a precise location.

use super::{
    AnalysisKind, AnalysisSpec, FitMethod, MathTransformPreset, NonlinearFitSpec, NormalizeAnchor, OutlierSpec, ParameterScope, SimulationNoise,
    SurvivalAnalysisSpec, TransformAnalysisSpec, TwoGroupMethod, WeightingScheme,
};

/// `Some(path())` when `value` is `NaN` or infinite.
fn non_finite(value: f64, path: impl FnOnce() -> String) -> Option<String> {
    if value.is_finite() { None } else { Some(path()) }
}

/// Like [`non_finite`], for optional values (`None` is always accepted).
fn non_finite_opt(value: Option<f64>, path: impl FnOnce() -> String) -> Option<String> {
    value.and_then(|v| non_finite(v, path))
}

/// Path of `"analysis.<field>"`.
fn at(field: &str) -> impl FnOnce() -> String + '_ {
    move || format!("analysis.{field}")
}

impl AnalysisSpec {
    /// Return the path of the first non-finite (`NaN` or `±Inf`) user-set float in this
    /// specification, or `None` when every checked value is finite.
    ///
    /// The path is relative to the owning result sheet, for example `analysis.y_transform.slope`
    /// or `parameter_rules[0].lower_bound`; `curvebook-format` prefixes the sheet ID when it
    /// rejects a document. Fields are checked in a fixed order, so the reported path is stable.
    /// Computed results are not checked here.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<String> {
        non_finite(self.confidence_level.percent(), at("confidence_level")).or_else(|| kind_first_non_finite_float(&self.kind))
    }
}

fn kind_first_non_finite_float(kind: &AnalysisKind) -> Option<String> {
    match kind {
        AnalysisKind::Transform(transform) => transform_first_non_finite_float(transform),
        AnalysisKind::DescriptiveStats(d) => non_finite_opt(d.custom_upper_percentile, at("custom_upper_percentile"))
            .or_else(|| non_finite_opt(d.hypothetical_value, at("hypothetical_value"))),
        AnalysisKind::OutlierDetection(o) => match o {
            OutlierSpec::Grubbs { alpha, .. } => non_finite(*alpha, at("outlier.alpha")),
            OutlierSpec::Rout { fdr_q_percent } => non_finite(*fdr_q_percent, at("outlier.fdr_q_percent")),
        },
        AnalysisKind::TwoGroupComparison(t) => match t.method {
            TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
                non_finite(hypothetical, at("hypothetical"))
            }
            TwoGroupMethod::UnpairedT
            | TwoGroupMethod::WelchT
            | TwoGroupMethod::PairedT
            | TwoGroupMethod::RatioPairedT
            | TwoGroupMethod::LognormalUnpairedT
            | TwoGroupMethod::MannWhitneyU
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::KolmogorovSmirnov
            | TwoGroupMethod::PairedSignTest => None,
        },
        AnalysisKind::MultipleRowTTests(s) => non_finite(s.multiplicity.threshold, at("multiplicity.threshold")),
        AnalysisKind::OneWayAnova(s) => non_finite(s.multiplicity.threshold, at("multiplicity.threshold")),
        AnalysisKind::FactorialAnova(s) => non_finite(s.multiplicity.threshold, at("multiplicity.threshold")),
        AnalysisKind::NestedComparison(s) => non_finite(s.multiplicity.threshold, at("multiplicity.threshold")),
        AnalysisKind::Contingency(c) => c
            .expected_proportions
            .iter()
            .flatten()
            .enumerate()
            .find_map(|(idx, p)| non_finite(*p, || format!("analysis.expected_proportions[{idx}]")))
            .or_else(|| non_finite_opt(c.null_proportion, at("null_proportion"))),
        AnalysisKind::Survival(s) => match s {
            SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. } => non_finite(multiplicity.threshold, at("multiplicity.threshold")),
            SurvivalAnalysisSpec::CoxProportionalHazards { .. } => None,
        },
        AnalysisKind::NonlinearRegression(fit) => nonlinear_fit_first_non_finite_float(fit),
        AnalysisKind::LinearRegression(fit) => range_first_non_finite_float(fit.x_range_filter, "x_range_filter")
            .or_else(|| non_finite_opt(fit.plot_x_min, at("plot_x_min")))
            .or_else(|| non_finite_opt(fit.plot_x_max, at("plot_x_max"))),
        AnalysisKind::FitSplineLowess(spec) => non_finite_opt(spec.smoothing_df, at("smoothing_df"))
            .or_else(|| non_finite_opt(spec.lowess_fraction, at("lowess_fraction")))
            .or_else(|| non_finite_opt(spec.plot_x_min, at("plot_x_min")))
            .or_else(|| non_finite_opt(spec.plot_x_max, at("plot_x_max"))),
        AnalysisKind::BlandAltman(_)
        | AnalysisKind::PassingBablok(_)
        | AnalysisKind::RocCurve(_)
        | AnalysisKind::GeneralizedLinearModel(_)
        | AnalysisKind::Multivariate(_) => None,
        AnalysisKind::PValueStack(ps) => non_finite(ps.threshold, at("pvalue_stack.threshold")),
        AnalysisKind::PowerAnalysis(p) => p.first_non_finite_float().map(|field| format!("analysis.{field}")),
        AnalysisKind::SimpleLogisticRegression(sl) => non_finite(sl.classification_cutoff, at("classification_cutoff"))
            .or_else(|| non_finite_opt(sl.plot_x_min, at("plot_x_min")))
            .or_else(|| non_finite_opt(sl.plot_x_max, at("plot_x_max"))),
    }
}

/// Check an optional `(min, max)` range filter, reporting `analysis.<name>.min` / `.max`.
fn range_first_non_finite_float(range: Option<(Option<f64>, Option<f64>)>, name: &str) -> Option<String> {
    let (lo, hi) = range?;
    non_finite_opt(lo, || format!("analysis.{name}.min")).or_else(|| non_finite_opt(hi, || format!("analysis.{name}.max")))
}

fn math_preset_first_non_finite_float(preset: &MathTransformPreset, prefix: &str) -> Option<String> {
    match preset {
        MathTransformPreset::LinearScale { slope, intercept } => {
            non_finite(*slope, || format!("{prefix}.slope")).or_else(|| non_finite(*intercept, || format!("{prefix}.intercept")))
        }
        MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor } => {
            non_finite(*molar_unit_factor, || format!("{prefix}.molar_unit_factor"))
        }
        MathTransformPreset::Ln
        | MathTransformPreset::Log10
        | MathTransformPreset::Log2
        | MathTransformPreset::Exp10
        | MathTransformPreset::Exp
        | MathTransformPreset::Reciprocal
        | MathTransformPreset::Sqrt
        | MathTransformPreset::Square
        | MathTransformPreset::Logit
        | MathTransformPreset::Probit
        | MathTransformPreset::ArcsinSqrt => None,
    }
}

fn normalize_anchor_first_non_finite_float(anchor: &NormalizeAnchor, path: &str) -> Option<String> {
    match anchor {
        NormalizeAnchor::FixedValue(v) => non_finite(*v, || path.to_owned()),
        _ => None,
    }
}

fn transform_first_non_finite_float(transform: &TransformAnalysisSpec) -> Option<String> {
    match transform {
        TransformAnalysisSpec::Math { x_transform, y_transform, .. } => x_transform
            .as_ref()
            .and_then(|p| math_preset_first_non_finite_float(p, "analysis.x_transform"))
            .or_else(|| y_transform.as_ref().and_then(|p| math_preset_first_non_finite_float(p, "analysis.y_transform"))),
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. } => {
            normalize_anchor_first_non_finite_float(zero_anchor, "analysis.zero_anchor")
                .or_else(|| normalize_anchor_first_non_finite_float(hundred_anchor, "analysis.hundred_anchor"))
        }
        TransformAnalysisSpec::Simulate { x_min, x_max, parameters, noise, .. } => non_finite(*x_min, at("simulate.x_min"))
            .or_else(|| non_finite(*x_max, at("simulate.x_max")))
            .or_else(|| parameters.iter().enumerate().find_map(|(idx, (_, val))| non_finite(*val, || format!("analysis.simulate.parameters[{idx}]"))))
            .or_else(|| match noise {
                SimulationNoise::Gaussian { sd } => non_finite(*sd, at("simulate.noise.sd")),
                SimulationNoise::RelativePercent { cv_percent } => non_finite(*cv_percent, at("simulate.noise.cv_percent")),
                SimulationNoise::None | SimulationNoise::Poisson => None,
            }),
        TransformAnalysisSpec::RemoveBaseline { .. }
        | TransformAnalysisSpec::Transpose { .. }
        | TransformAnalysisSpec::PruneRows { .. }
        | TransformAnalysisSpec::SelectAndTransform { .. }
        | TransformAnalysisSpec::RandomSample { .. } => None,
    }
}

fn nonlinear_fit_first_non_finite_float(fit: &NonlinearFitSpec) -> Option<String> {
    non_finite_opt(fit.flag_r_squared_below, at("flag_r_squared_below"))
        .or_else(|| match fit.method {
            FitMethod::RoutOutlierElimination { fdr_q_percent } => non_finite(fdr_q_percent, at("fit_method.fdr_q_percent")),
            _ => None,
        })
        .or_else(|| match fit.weighting {
            WeightingScheme::GeneralPowerY(k) => non_finite(k, at("weighting.power")),
            _ => None,
        })
        .or_else(|| {
            fit.parameter_rules.iter().enumerate().find_map(|(idx, rule)| {
                non_finite_opt(rule.initial_override, || format!("parameter_rules[{idx}].initial_override"))
                    .or_else(|| non_finite_opt(rule.lower_bound, || format!("parameter_rules[{idx}].lower_bound")))
                    .or_else(|| non_finite_opt(rule.upper_bound, || format!("parameter_rules[{idx}].upper_bound")))
                    .or_else(|| match &rule.scope {
                        ParameterScope::Constant(v) => non_finite(*v, || format!("parameter_rules[{idx}].constant")),
                        ParameterScope::PerDatasetConstant(entries) => entries
                            .iter()
                            .enumerate()
                            .find_map(|(entry_idx, (_, val))| non_finite(*val, || format!("parameter_rules[{idx}].per_dataset[{entry_idx}]"))),
                        ParameterScope::Local
                        | ParameterScope::SharedGlobal
                        | ParameterScope::HookedConstant(_)
                        | ParameterScope::ColumnTitleConstant => None,
                    })
            })
        })
        .or_else(|| range_first_non_finite_float(fit.x_range_filter, "x_range_filter"))
        .or_else(|| range_first_non_finite_float(fit.y_range_filter, "y_range_filter"))
        .or_else(|| non_finite_opt(fit.plot_x_min, at("plot_x_min")))
        .or_else(|| non_finite_opt(fit.plot_x_max, at("plot_x_max")))
        .or_else(|| match &fit.comparison_question {
            Some(super::ModelComparisonQuestion::ParameterEqualsTheoretical { theoretical_value, .. }) => {
                non_finite(*theoretical_value, at("comparison_question.theoretical_value"))
            }
            _ => None,
        })
        .or_else(|| {
            fit.fit_flag_rules.as_ref().and_then(|rules| {
                non_finite_opt(rules.min_r_squared, at("fit_flag_rules.min_r_squared"))
                    .or_else(|| non_finite_opt(rules.max_dependency, at("fit_flag_rules.max_dependency")))
                    .or_else(|| non_finite_opt(rules.min_normality_p, at("fit_flag_rules.min_normality_p")))
                    .or_else(|| non_finite_opt(rules.min_runs_or_lof_p, at("fit_flag_rules.min_runs_or_lof_p")))
            })
        })
}
