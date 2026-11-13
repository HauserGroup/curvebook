//! Parameterized `AnalysisSpec` schemas across transforms (`crates/transform`), biostatistics
//! (`crates/stats`), and linear/nonlinear regression (`crates/fit`), plus `PValueFormat` display
//! rules and `Change Data Analyzed` compatibility checks (`docs/data-tables-and-transforms.md` §5,
//! `docs/statistics-implementation.md` §1–§5, `docs/curve-fitting-implementation.md` §1–§4).

pub use lumencraft_math::PValueFormat;

use crate::TableKind;
use crate::error::{ConfidenceLevel, DocError, validate_confidence_level, validate_title};
use crate::ids::{HookSource, SheetId};

// ============================================================================
// Multiplicity & FDR Specifications (`docs/statistics-implementation.md` §4)
// ============================================================================

/// Familywise error-rate (FWER) or false-discovery-rate (FDR) procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MultiplicityMethod {
    #[default]
    None,
    TukeyHsd,
    Dunnett {
        control_column: usize,
    },
    Sidak,
    Bonferroni,
    HolmSidak,
    DunnettT3,
    GamesHowell,
    TamhaneT2,
    DunnRankSum,
    LinearTrend,
    BenjaminiHochbergFdr,
    BenjaminiYekutieliFdr,
    TwoStageBkyFdr,
}

/// Multiple-comparisons configuration shared across ANOVA, nonparametrics, and survival curves.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MultiplicitySpec {
    pub method: MultiplicityMethod,
    /// Familywise `alpha` (e.g. `0.05`) for FWER methods or desired FDR `Q` (e.g. `0.05` / `0.01`) for FDR methods.
    pub threshold: f64,
    pub report_confidence_intervals: bool,
}

impl Default for MultiplicitySpec {
    fn default() -> Self {
        Self { method: MultiplicityMethod::None, threshold: 0.05, report_confidence_intervals: true }
    }
}

// ============================================================================
// Transform Specifications (`docs/data-tables-and-transforms.md` §5)
// ============================================================================

/// Scalar mathematical transform preset.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MathTransformPreset {
    Ln,
    Log10,
    Log2,
    Exp10,
    Exp,
    Reciprocal,
    Sqrt,
    Square,
    Logit,
    Probit,
    ArcsinSqrt,
    LinearScale {
        slope: f64,
        intercept: f64,
    },
    /// Convert molar concentration (scaled by `molar_unit_factor`, e.g. `1e-9` for nM) to `-log10[M]`.
    ConcentrationToNegativeLogM {
        molar_unit_factor: f64,
    },
}

/// Error-propagation strategy when transforming `Mean + SD/SEM` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ErrorPropagationMode {
    #[default]
    ReplicateEndpoints,
    FirstOrderDeltaMethod,
}

/// Anchor value for `[0%, 100%]` normalization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NormalizeAnchor {
    ColumnMin,
    ColumnMax,
    ColumnSum,
    FirstRow,
    LastRow,
    FixedValue(f64),
    ControlColumnMean { column: usize },
    Hooked(HookSource),
}

/// Baseline calculation method for `Remove Baseline and Column Math`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BaselineOperation {
    Difference,
    Ratio,
    PercentChange,
    FractionalChange,
}

/// Monte Carlo scatter model for `Simulate Data`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SimulationNoise {
    None,
    Gaussian { sd: f64 },
    RelativePercent { cv_percent: f64 },
    Poisson,
}

/// Derived data-table transformation (`crates/transform`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TransformAnalysisSpec {
    Math {
        x_transform: Option<MathTransformPreset>,
        y_transform: Option<MathTransformPreset>,
        error_propagation: ErrorPropagationMode,
    },
    Normalize {
        zero_anchor: NormalizeAnchor,
        hundred_anchor: NormalizeAnchor,
        output_percent: bool,
        per_data_set: bool,
    },
    RemoveBaseline {
        baseline_columns: Vec<usize>,
        operation: BaselineOperation,
    },
    Transpose {
        swap_row_and_column_titles: bool,
    },
    PruneRows {
        every_k_rows: usize,
        average_window: bool,
    },
    SelectAndTransform {
        filter_formula: Option<String>,
        target_archetype: TableKind,
    },
    Simulate {
        seed: u64,
        row_count: usize,
        replicates: usize,
        x_min: f64,
        x_max: f64,
        equation_id: String,
        parameters: Vec<(String, f64)>,
        noise: SimulationNoise,
    },
}

// ============================================================================
// Biostatistics Specifications (`docs/statistics-implementation.md` §2–§5)
// ============================================================================

/// Replicate-subcolumn aggregation rule for Descriptive Statistics (`docs/statistics-implementation.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SubcolumnAggregation {
    #[default]
    RowMeans,
    PerSubcolumn,
    PooledGrandSample,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DescriptiveSpec {
    pub aggregation: SubcolumnAggregation,
    pub custom_upper_percentile: Option<f64>,
    pub run_normality_battery: bool,
    pub hypothetical_value: Option<f64>,
}

impl Default for DescriptiveSpec {
    fn default() -> Self {
        Self { aggregation: SubcolumnAggregation::RowMeans, custom_upper_percentile: None, run_normality_battery: true, hypothetical_value: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum OutlierSpec {
    Grubbs { alpha: f64, iterative: bool },
    Rout { fdr_q_percent: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TwoGroupMethod {
    UnpairedT,
    WelchT,
    PairedT,
    RatioPairedT,
    LognormalUnpairedT,
    MannWhitneyU,
    WilcoxonSignedRank,
    KolmogorovSmirnov,
    OneSampleT { hypothetical: f64 },
    OneSampleWilcoxon { hypothetical: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TwoGroupSpec {
    pub method: TwoGroupMethod,
    pub two_tailed: bool,
    pub compute_effect_sizes: bool,
}

impl Default for TwoGroupSpec {
    fn default() -> Self {
        Self { method: TwoGroupMethod::UnpairedT, two_tailed: true, compute_effect_sizes: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum OneWayMethod {
    #[default]
    OrdinaryAnova,
    BrownForsytheAndWelch,
    RepeatedMeasures {
        geisser_greenhouse: bool,
    },
    KruskalWallis,
    Friedman,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct OneWayAnovaSpec {
    pub method: OneWayMethod,
    pub compute_effect_sizes: bool,
    pub multiplicity: MultiplicitySpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SumOfSquaresType {
    TypeI,
    #[default]
    TypeIII,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RepeatedMeasuresDesign {
    #[default]
    None,
    RowFactorMatched,
    ColumnFactorMatched,
    BothFactorsMatched,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactorialAnovaSpec {
    pub three_way: bool,
    /// Populated when invoked as Multifactor (`N`-way) GLM on a `MultipleVariables` sheet.
    pub multifactor_columns: Vec<usize>,
    pub response_column: Option<usize>,
    pub max_interaction_order: usize,
    pub ss_type: SumOfSquaresType,
    pub repeated_measures: RepeatedMeasuresDesign,
    pub fit_reml_on_missing: bool,
    pub compute_effect_sizes: bool,
    pub multiplicity: MultiplicitySpec,
}

impl Default for FactorialAnovaSpec {
    fn default() -> Self {
        Self {
            three_way: false,
            multifactor_columns: Vec::new(),
            response_column: None,
            max_interaction_order: 2,
            ss_type: SumOfSquaresType::TypeIII,
            repeated_measures: RepeatedMeasuresDesign::None,
            fit_reml_on_missing: true,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NestedAnalysisSpec {
    pub two_group_t_test: bool,
    pub multiplicity: MultiplicitySpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ContingencyMethod {
    PearsonChiSquare { yates_continuity: bool },
    LikelihoodRatioG2,
    FisherExact,
    CochranArmitageTrend,
    McNemarPaired,
}

impl Default for ContingencyMethod {
    fn default() -> Self {
        Self::PearsonChiSquare { yates_continuity: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct ContingencySpec {
    pub method: ContingencyMethod,
    pub compute_odds_and_risk_ratios: bool,
    pub compute_sensitivity_specificity: bool,
    pub compute_phi_and_cramers_v: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CoxTieMethod {
    Breslow,
    #[default]
    Efron,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SurvivalAnalysisSpec {
    KaplanMeier {
        log_rank_mantel_cox: bool,
        gehan_breslow_wilcoxon: bool,
        trend_test: bool,
        asymmetrical_greenwood_ci: bool,
        multiplicity: MultiplicitySpec,
    },
    CoxProportionalHazards {
        time_column: usize,
        status_column: usize,
        predictor_columns: Vec<usize>,
        reference_levels: Vec<(usize, String)>,
        tie_method: CoxTieMethod,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RocCiMethod {
    HanleyMcNeil,
    #[default]
    DeLong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct RocSpec {
    pub ci_method: RocCiMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum GlmFamily {
    #[default]
    Linear,
    Logistic,
    Poisson,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GlmSpec {
    pub family: GlmFamily,
    pub response_column: usize,
    pub predictor_columns: Vec<usize>,
    pub reference_levels: Vec<(usize, String)>,
    pub compute_vif: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MultivariateSpec {
    Pca { standardize: bool, parallel_analysis_seed: Option<u64> },
    CorrelationMatrix { spearman_rank: bool },
    Clustering { k_means_k: Option<usize> },
}

// ============================================================================
// Linear & Nonlinear Regression Specifications (`docs/curve-fitting-implementation.md`)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearRegressionSpec {
    pub deming_model_ii: bool,
    pub force_through_origin: bool,
    pub confidence_bands: bool,
    pub prediction_bands: bool,
    pub interpolate_unknowns: bool,
}

impl Default for LinearRegressionSpec {
    fn default() -> Self {
        Self { deming_model_ii: false, force_through_origin: false, confidence_bands: true, prediction_bands: false, interpolate_unknowns: false }
    }
}

/// Differential weighting scheme for nonlinear regression (`docs/curve-fitting-implementation.md` §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum WeightingScheme {
    #[default]
    None,
    /// `1 / Y^2` (constant `%CV` proportional scatter).
    RelativeSquare,
    /// `1 / |Y|`.
    Relative,
    /// `1 / |Y|^k`.
    GeneralPowerY(f64),
    /// `1 / |X|`.
    InverseX,
    /// `1 / X^2`.
    InverseX2,
    /// `1 / SD^2` across replicates.
    ObservedSd,
}

/// Regression objective function (`docs/curve-fitting-implementation.md` §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum FitMethod {
    #[default]
    LeastSquares,
    PoissonDeviance,
    RobustCauchy,
    RoutOutlierElimination {
        fdr_q_percent: f64,
    },
}

/// Dataset parameter sharing or constant constraint (`docs/curve-fitting-implementation.md` §1.3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ParameterScope {
    Local,
    SharedGlobal,
    Constant(f64),
    HookedConstant(HookSource),
}

/// Constraint and sharing rule for a single named equation parameter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParameterRule {
    pub name: String,
    pub scope: ParameterScope,
    pub initial_override: Option<f64>,
    pub lower_bound: Option<f64>,
    pub upper_bound: Option<f64>,
}

/// Model-comparison configuration (`Extra-sum-of-squares F` vs `AICc`; §2.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonSpec {
    ExtraSumOfSquaresF { alternative_equation_id: String },
    Aicc { alternative_equation_id: String },
}

/// Complete nonlinear regression specification (`crates/fit`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NonlinearFitSpec {
    pub equation_id: String,
    pub custom_dsl: Option<String>,
    pub method: FitMethod,
    pub weighting: WeightingScheme,
    pub parameter_rules: Vec<ParameterRule>,
    pub profile_likelihood_ci: bool,
    pub comparison: Option<ModelComparisonSpec>,
    pub confidence_bands: bool,
    pub prediction_bands: bool,
    pub interpolate_unknowns: bool,
    pub replicates_lack_of_fit_test: bool,
    pub normality_of_residuals: bool,
    pub max_iters: usize,
}

impl NonlinearFitSpec {
    pub const DEFAULT_MAX_ITERS: usize = 1_000;
    pub const HARD_MAX_ITERS: usize = 50_000;

    #[must_use]
    pub fn new(equation_id: impl Into<String>) -> Self {
        Self {
            equation_id: equation_id.into(),
            custom_dsl: None,
            method: FitMethod::LeastSquares,
            weighting: WeightingScheme::None,
            parameter_rules: Vec::new(),
            profile_likelihood_ci: true,
            comparison: None,
            confidence_bands: true,
            prediction_bands: false,
            interpolate_unknowns: false,
            replicates_lack_of_fit_test: false,
            normality_of_residuals: false,
            max_iters: Self::DEFAULT_MAX_ITERS,
        }
    }

    #[must_use]
    pub fn clamped_max_iters(&self) -> usize {
        self.max_iters.clamp(1, Self::HARD_MAX_ITERS)
    }
}

// ============================================================================
// Unified AnalysisKind & AnalysisSpec
// ============================================================================

/// Discriminated statistical, regression, or transformation specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AnalysisKind {
    Transform(TransformAnalysisSpec),
    DescriptiveStats(DescriptiveSpec),
    OutlierDetection(OutlierSpec),
    TwoGroupComparison(TwoGroupSpec),
    OneWayAnova(OneWayAnovaSpec),
    FactorialAnova(FactorialAnovaSpec),
    NestedComparison(NestedAnalysisSpec),
    Contingency(ContingencySpec),
    Survival(SurvivalAnalysisSpec),
    RocCurve(RocSpec),
    GeneralizedLinearModel(GlmSpec),
    Multivariate(MultivariateSpec),
    LinearRegression(LinearRegressionSpec),
    NonlinearRegression(NonlinearFitSpec),
}

impl AnalysisKind {
    /// Dynamically derive the canonical default `ResultSheet` base title (e.g. switching from
    /// `"Ordinary one-way ANOVA"` to `"RM one-way ANOVA"` when parameters change in-place).
    #[must_use]
    pub fn derive_auto_title(&self) -> String {
        match self {
            Self::Transform(t) => match t {
                TransformAnalysisSpec::Math { .. } => String::from("Transform"),
                TransformAnalysisSpec::Normalize { .. } => String::from("Normalize"),
                TransformAnalysisSpec::RemoveBaseline { .. } => String::from("Remove baseline"),
                TransformAnalysisSpec::Transpose { .. } => String::from("Transpose"),
                TransformAnalysisSpec::PruneRows { .. } => String::from("Prune rows"),
                TransformAnalysisSpec::SelectAndTransform { .. } => String::from("Select and transform"),
                TransformAnalysisSpec::Simulate { .. } => String::from("Simulated data"),
            },
            Self::DescriptiveStats(_) => String::from("Descriptive statistics"),
            Self::OutlierDetection(OutlierSpec::Grubbs { .. }) => String::from("Identify outliers (Grubbs)"),
            Self::OutlierDetection(OutlierSpec::Rout { .. }) => String::from("Identify outliers (ROUT)"),
            Self::TwoGroupComparison(s) => match s.method {
                TwoGroupMethod::UnpairedT => String::from("Unpaired t test"),
                TwoGroupMethod::WelchT => String::from("Welch's t test"),
                TwoGroupMethod::PairedT => String::from("Paired t test"),
                TwoGroupMethod::RatioPairedT => String::from("Ratio paired t test"),
                TwoGroupMethod::LognormalUnpairedT => String::from("Unpaired lognormal t test"),
                TwoGroupMethod::MannWhitneyU => String::from("Mann-Whitney test"),
                TwoGroupMethod::WilcoxonSignedRank => String::from("Wilcoxon matched-pairs test"),
                TwoGroupMethod::KolmogorovSmirnov => String::from("Kolmogorov-Smirnov test"),
                TwoGroupMethod::OneSampleT { .. } => String::from("One-sample t test"),
                TwoGroupMethod::OneSampleWilcoxon { .. } => String::from("One-sample Wilcoxon test"),
            },
            Self::OneWayAnova(s) => match s.method {
                OneWayMethod::OrdinaryAnova => String::from("Ordinary one-way ANOVA"),
                OneWayMethod::BrownForsytheAndWelch => String::from("Brown-Forsythe and Welch ANOVA"),
                OneWayMethod::RepeatedMeasures { .. } => String::from("RM one-way ANOVA"),
                OneWayMethod::KruskalWallis => String::from("Kruskal-Wallis test"),
                OneWayMethod::Friedman => String::from("Friedman test"),
            },
            Self::FactorialAnova(s) => {
                if !s.multifactor_columns.is_empty() {
                    String::from("Multifactor ANOVA")
                } else if s.three_way {
                    String::from("Three-way ANOVA")
                } else if s.repeated_measures != RepeatedMeasuresDesign::None {
                    String::from("RM two-way ANOVA")
                } else {
                    String::from("Two-way ANOVA")
                }
            }
            Self::NestedComparison(s) => {
                if s.two_group_t_test {
                    String::from("Nested t test")
                } else {
                    String::from("Nested one-way ANOVA")
                }
            }
            Self::Contingency(s) => match s.method {
                ContingencyMethod::PearsonChiSquare { .. } => String::from("Chi-square test"),
                ContingencyMethod::LikelihoodRatioG2 => String::from("Likelihood-ratio contingency test"),
                ContingencyMethod::FisherExact => String::from("Fisher's exact test"),
                ContingencyMethod::CochranArmitageTrend => String::from("Chi-square test for trend"),
                ContingencyMethod::McNemarPaired => String::from("McNemar's test"),
            },
            Self::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }) => String::from("Survival curve analysis"),
            Self::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }) => String::from("Cox proportional hazards"),
            Self::RocCurve(_) => String::from("ROC curve"),
            Self::GeneralizedLinearModel(g) => match g.family {
                GlmFamily::Linear => String::from("Multiple linear regression"),
                GlmFamily::Logistic => String::from("Multiple logistic regression"),
                GlmFamily::Poisson => String::from("Multiple Poisson regression"),
            },
            Self::Multivariate(MultivariateSpec::Pca { .. }) => String::from("Principal component analysis"),
            Self::Multivariate(MultivariateSpec::CorrelationMatrix { .. }) => String::from("Correlation matrix"),
            Self::Multivariate(MultivariateSpec::Clustering { .. }) => String::from("Cluster analysis"),
            Self::LinearRegression(_) => String::from("Simple linear regression"),
            Self::NonlinearRegression(fit) => format!("Nonlinear regression ({})", fit.equation_id),
        }
    }

    /// Check whether this analysis kind can run against a given [`TableKind`] (used by Analyze
    /// filtering and `Change Data Analyzed` retargeting).
    #[must_use]
    pub fn is_compatible_with(&self, table_kind: TableKind) -> bool {
        match self {
            Self::Transform(_) => true,
            Self::DescriptiveStats(_) | Self::OutlierDetection(_) => {
                matches!(table_kind, TableKind::Column | TableKind::Xy | TableKind::Grouped | TableKind::MultipleVariables)
            }
            Self::TwoGroupComparison(_) | Self::OneWayAnova(_) | Self::RocCurve(_) => {
                matches!(table_kind, TableKind::Column | TableKind::MultipleVariables)
            }
            Self::FactorialAnova(_) => matches!(table_kind, TableKind::Grouped | TableKind::MultipleVariables),
            Self::NestedComparison(_) => matches!(table_kind, TableKind::Nested),
            Self::Contingency(_) => matches!(table_kind, TableKind::Contingency | TableKind::PartsOfWhole),
            Self::Survival(_) => matches!(table_kind, TableKind::Survival | TableKind::MultipleVariables),
            Self::GeneralizedLinearModel(_) | Self::Multivariate(_) => {
                matches!(table_kind, TableKind::MultipleVariables | TableKind::Xy | TableKind::Column)
            }
            Self::LinearRegression(_) | Self::NonlinearRegression(_) => {
                matches!(table_kind, TableKind::Xy | TableKind::Grouped | TableKind::MultipleVariables)
            }
        }
    }

    /// Collect all explicit upstream [`HookSource`] references embedded inside this analysis configuration.
    #[must_use]
    pub fn hooked_sources(&self) -> Vec<HookSource> {
        let mut out = Vec::new();
        match self {
            Self::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
                if let NormalizeAnchor::Hooked(h) = zero_anchor {
                    out.push(h.clone());
                }
                if let NormalizeAnchor::Hooked(h) = hundred_anchor {
                    out.push(h.clone());
                }
            }
            Self::NonlinearRegression(fit) => {
                for rule in &fit.parameter_rules {
                    if let ParameterScope::HookedConstant(h) = &rule.scope {
                        out.push(h.clone());
                    }
                }
            }
            _ => {}
        }
        out
    }
}

/// Stateful, mutable analysis node linking an upstream `DataSheet` (or analyzable `ResultSheet`)
/// to its [`crate::ResultSheet`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisSpec {
    pub id: SheetId,
    pub source_sheet_id: SheetId,
    /// Selected dataset/column indices (`empty` means analyze all columns on `source_sheet_id`).
    pub selected_data_sets: Vec<usize>,
    pub auto_title: String,
    /// Set only when the user manually renamed the sheet; preserved across parameter edits.
    pub custom_title: Option<String>,
    pub confidence_level: ConfidenceLevel,
    pub p_value_format: PValueFormat,
    pub kind: AnalysisKind,
}

impl AnalysisSpec {
    pub fn new(id: impl Into<SheetId>, source_sheet_id: impl Into<SheetId>, kind: AnalysisKind) -> Result<Self, DocError> {
        let id = id.into();
        let source_sheet_id = source_sheet_id.into();
        if id.as_str().trim().is_empty() || source_sheet_id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        let auto_title = kind.derive_auto_title();
        Ok(Self {
            id,
            source_sheet_id,
            selected_data_sets: Vec::new(),
            auto_title,
            custom_title: None,
            confidence_level: ConfidenceLevel::DEFAULT_95,
            p_value_format: PValueFormat::FourDecimal,
            kind,
        })
    }

    /// Displayed sheet title (`custom_title` when pinned by user, else dynamically derived `auto_title`).
    #[must_use]
    pub fn effective_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or(&self.auto_title)
    }

    /// Pin a custom user title (`None` restores automatic parameter-derived naming).
    pub fn set_custom_title(&mut self, custom: Option<String>) -> Result<(), DocError> {
        if let Some(t) = &custom {
            validate_title(t)?;
        }
        self.custom_title = custom.filter(|s| !s.trim().is_empty());
        Ok(())
    }

    /// Update the analysis confidence level (`0.01..=99.99` percent).
    ///
    /// # Errors
    /// Returns [`DocError::InvalidConfidenceLevel`] if `percent` is outside `[0.01, 99.99]` or non-finite.
    pub fn set_confidence_percent(&mut self, percent: f64) -> Result<(), DocError> {
        self.confidence_level = validate_confidence_level(percent)?;
        Ok(())
    }

    /// Mutate analysis parameters in-place (`analysis.updateParams`, `Cmd/Ctrl+T`), refreshing
    /// `auto_title` while preserving any user-pinned `custom_title`.
    pub fn update_kind(&mut self, new_kind: AnalysisKind) {
        self.auto_title = new_kind.derive_auto_title();
        self.kind = new_kind;
    }

    /// Retarget this analysis onto a new compatible table (`analysis.retargetData` / `Change Data Analyzed`).
    pub fn retarget_source(
        &mut self,
        new_source_sheet_id: impl Into<SheetId>,
        target_table_kind: TableKind,
        selected_data_sets: Vec<usize>,
    ) -> Result<(), DocError> {
        if !self.kind.is_compatible_with(target_table_kind) {
            return Err(DocError::IncompatibleRetargetTable { analysis_title: self.effective_title().to_owned(), target_kind: target_table_kind });
        }
        let new_id = new_source_sheet_id.into();
        if new_id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        self.source_sheet_id = new_id;
        self.selected_data_sets = selected_data_sets;
        Ok(())
    }
}
