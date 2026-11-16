//! Parameterized `AnalysisSpec` schemas across transforms (`crates/transform`), biostatistics
//! (`crates/stats`), and linear/nonlinear regression (`crates/fit`), plus `PValueFormat` display
//! rules and `Change Data Analyzed` compatibility checks (`docs/data-tables-and-transforms.md` §5,
//! `docs/statistics-implementation.md` §1–§5, `docs/curve-fitting-implementation.md` §1–§4).

pub use lumencraft_math::PValueFormat;

use crate::TableKind;
use crate::error::{ConfidenceLevel, DocError, validate_confidence_level, validate_title};
use crate::ids::{HookSource, SheetId};

const fn default_col_b() -> usize {
    1
}

const fn default_true() -> bool {
    true
}

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
    NewmanKeuls,
    FisherLsd,
}

/// Multiple-comparisons configuration shared across ANOVA, nonparametrics, and survival curves.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
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
    RandomSample {
        seed: u64,
        sample_size: usize,
        #[serde(default)]
        stratify_column: Option<usize>,
        #[serde(default)]
        proportional_allocation: bool,
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
    PairedSignTest,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TwoGroupSpec {
    pub method: TwoGroupMethod,
    pub two_tailed: bool,
    pub compute_effect_sizes: bool,
    #[serde(default)]
    pub mv_group_column: Option<usize>,
    #[serde(default)]
    pub mv_response_column: Option<usize>,
}

impl Default for TwoGroupSpec {
    fn default() -> Self {
        Self { method: TwoGroupMethod::UnpairedT, two_tailed: true, compute_effect_sizes: true, mv_group_column: None, mv_response_column: None }
    }
}

/// Bland-Altman method-comparison specification (`column_a` vs `column_b`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlandAltmanSpec {
    #[serde(default)]
    pub column_a: usize,
    #[serde(default = "default_col_b")]
    pub column_b: usize,
    #[serde(default)]
    pub percent_difference: bool,
}

impl Default for BlandAltmanSpec {
    fn default() -> Self {
        Self { column_a: 0, column_b: 1, percent_difference: false }
    }
}

/// Row-wise multiple two-group t-test or nonparametric comparison specification.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MultipleRowTestsSpec {
    #[serde(default)]
    pub paired: bool,
    #[serde(default)]
    pub nonparametric: bool,
    #[serde(default = "default_true")]
    pub assume_equal_sd: bool,
    #[serde(default)]
    pub multiplicity: MultiplicitySpec,
}

impl Default for MultipleRowTestsSpec {
    fn default() -> Self {
        Self { paired: false, nonparametric: false, assume_equal_sd: true, multiplicity: MultiplicitySpec::default() }
    }
}

/// Passing-Bablok nonparametric method-comparison regression specification (`x_column` vs `y_column`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PassingBablokSpec {
    #[serde(default)]
    pub x_column: usize,
    #[serde(default = "default_col_b")]
    pub y_column: usize,
}

impl Default for PassingBablokSpec {
    fn default() -> Self {
        Self { x_column: 0, y_column: 1 }
    }
}

/// Multiplicity correction procedure for a standalone stack of *P* values (`R10.10`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PValueStackMethod {
    Bonferroni,
    HolmBonferroni,
    Sidak,
    HolmSidak,
    #[default]
    BenjaminiHochberg,
    TwoStageBky,
    BenjaminiYekutieli,
}

impl PValueStackMethod {
    /// All seven multiplicity procedures in canonical order.
    pub const ALL: [Self; 7] =
        [Self::Bonferroni, Self::HolmBonferroni, Self::Sidak, Self::HolmSidak, Self::BenjaminiHochberg, Self::TwoStageBky, Self::BenjaminiYekutieli];

    /// Whether this procedure controls the false discovery rate (FDR, emitting *q*-values)
    /// rather than the family-wise error rate (FWER, emitting adjusted *P* values).
    #[must_use]
    pub const fn is_fdr(self) -> bool {
        matches!(self, Self::BenjaminiHochberg | Self::TwoStageBky | Self::BenjaminiYekutieli)
    }

    /// Canonical short display label for result tables and methods reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bonferroni => "Bonferroni",
            Self::HolmBonferroni => "Holm (1979)",
            Self::Sidak => "Šídák (1967)",
            Self::HolmSidak => "Holm–Šídák",
            Self::BenjaminiHochberg => "Benjamini–Hochberg (1995)",
            Self::TwoStageBky => "Benjamini–Krieger–Yekutieli two-stage (2006)",
            Self::BenjaminiYekutieli => "Benjamini–Yekutieli (2001)",
        }
    }
}

fn default_pvalue_stack_threshold() -> f64 {
    0.05
}

/// Specification for standalone multiplicity adjustment of a stack of raw *P* values (`R10.10`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PValueStackSpec {
    /// Multiplicity correction / FDR procedure.
    #[serde(default)]
    pub method: PValueStackMethod,
    /// Family-wise significance level (`alpha` for FWER) or desired FDR (`Q` for FDR), in `(0.0, 1.0)`.
    #[serde(default = "default_pvalue_stack_threshold")]
    pub threshold: f64,
    /// Optional 0-based column override; `None` resolves `AnalysisSpec::selected_data_sets`.
    #[serde(default)]
    pub column: Option<usize>,
}

impl Default for PValueStackSpec {
    fn default() -> Self {
        Self { method: PValueStackMethod::default(), threshold: default_pvalue_stack_threshold(), column: None }
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

/// Weighting scheme for Cohen's kappa (`Unweighted`, `Linear`, or `Quadratic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum KappaWeighting {
    #[default]
    Unweighted,
    Linear,
    Quadratic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ContingencyMethod {
    PearsonChiSquare {
        yates_continuity: bool,
    },
    LikelihoodRatioG2,
    FisherExact,
    BarnardExact,
    CochranArmitageTrend,
    McNemarPaired,
    CohenKappa {
        #[serde(default)]
        weighting: KappaWeighting,
    },
    FleissKappa,
    ExactBinomial,
}

impl Default for ContingencyMethod {
    fn default() -> Self {
        Self::PearsonChiSquare { yates_continuity: false }
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ContingencySpec {
    pub method: ContingencyMethod,
    pub compute_odds_and_risk_ratios: bool,
    pub compute_sensitivity_specificity: bool,
    pub compute_phi_and_cramers_v: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_proportions: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null_proportion: Option<f64>,
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
        #[serde(default)]
        mv_time_column: Option<usize>,
        #[serde(default)]
        mv_status_column: Option<usize>,
        #[serde(default)]
        mv_group_column: Option<usize>,
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

/// Pairwise distance metric for hierarchical and multivariate clustering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterDistance {
    #[default]
    Euclidean,
    Manhattan,
    Pearson,
    Cosine,
}

/// Agglomerative linkage criterion for hierarchical clustering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterLinkage {
    #[default]
    Ward,
    Complete,
    Single,
    Upgma,
    Wpgma,
    Centroid,
    Median,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MultivariateSpec {
    Pca {
        standardize: bool,
        parallel_analysis_seed: Option<u64>,
    },
    CorrelationMatrix {
        spearman_rank: bool,
    },
    Clustering {
        k_means_k: Option<usize>,
        #[serde(default)]
        distance: ClusterDistance,
        #[serde(default)]
        linkage: ClusterLinkage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        k_sweep_max: Option<usize>,
    },
    MultipleImputation {
        imputations_m: usize,
        max_iters: usize,
        seed: u64,
        #[serde(default)]
        response_column: Option<usize>,
        #[serde(default)]
        predictor_columns: Vec<usize>,
    },
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
    PerDatasetConstant(Vec<(usize, f64)>),
    ColumnTitleConstant,
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

/// Model-comparison configuration (`Extra-sum-of-squares F` vs `AICc` vs `LikelihoodRatioTest`; §2.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonSpec {
    ExtraSumOfSquaresF { alternative_equation_id: String },
    Aicc { alternative_equation_id: String },
    LikelihoodRatioTest { alternative_equation_id: String },
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
    #[serde(default)]
    pub x_range_filter: Option<(Option<f64>, Option<f64>)>,
    #[serde(default)]
    pub y_range_filter: Option<(Option<f64>, Option<f64>)>,
    #[serde(default)]
    pub plot_function_only: bool,
    #[serde(default)]
    pub plot_x_min: Option<f64>,
    #[serde(default)]
    pub plot_x_max: Option<f64>,
    #[serde(default)]
    pub homoscedasticity_test: bool,
    /// Emit the `ResultTabKind::Residuals` tab (`true` by default for new specs; missing legacy JSON
    /// fields deserialize as `false` via `#[serde(default)]`).
    #[serde(default)]
    pub emit_residuals_tab: bool,
    #[serde(default)]
    pub emit_covariance_tab: bool,
    #[serde(default)]
    pub mv_group_columns: Vec<usize>,
    #[serde(default)]
    pub mv_x_columns: Vec<usize>,
    #[serde(default)]
    pub mv_y_column: Option<usize>,
}

impl NonlinearFitSpec {
    pub const DEFAULT_MAX_ITERS: usize = 1_000;
    pub const HARD_MAX_ITERS: usize = 50_000;

    /// Construct a default nonlinear regression specification for `equation_id`.
    ///
    /// Defaults `emit_residuals_tab` to `true` so [`crate::ResultTabKind::Residuals`] diagnostics are
    /// generated out of the box for newly created fits.
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
            x_range_filter: None,
            y_range_filter: None,
            plot_function_only: false,
            plot_x_min: None,
            plot_x_max: None,
            homoscedasticity_test: false,
            emit_residuals_tab: true,
            emit_covariance_tab: false,
            mv_group_columns: Vec::new(),
            mv_x_columns: Vec::new(),
            mv_y_column: None,
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
    BlandAltman(BlandAltmanSpec),
    MultipleRowTTests(MultipleRowTestsSpec),
    PassingBablok(PassingBablokSpec),
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
    PValueStack(PValueStackSpec),
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
                TransformAnalysisSpec::RandomSample { .. } => String::from("Random sample"),
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
                TwoGroupMethod::PairedSignTest => String::from("Sign test"),
            },
            Self::BlandAltman(_) => String::from("Bland-Altman method comparison"),
            Self::MultipleRowTTests(s) => match (s.nonparametric, s.paired) {
                (false, false) => String::from("Multiple unpaired t tests"),
                (false, true) => String::from("Multiple paired t tests"),
                (true, false) => String::from("Multiple Mann-Whitney tests"),
                (true, true) => String::from("Multiple Wilcoxon tests"),
            },
            Self::PassingBablok(_) => String::from("Passing-Bablok regression"),
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
                ContingencyMethod::BarnardExact => String::from("Barnard's exact test"),
                ContingencyMethod::CochranArmitageTrend => String::from("Chi-square test for trend"),
                ContingencyMethod::McNemarPaired => String::from("McNemar's test"),
                ContingencyMethod::CohenKappa { .. } => String::from("Cohen's kappa"),
                ContingencyMethod::FleissKappa => String::from("Fleiss' kappa"),
                ContingencyMethod::ExactBinomial => String::from("Exact binomial test"),
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
            Self::Multivariate(MultivariateSpec::MultipleImputation { .. }) => String::from("Multiple imputation"),
            Self::LinearRegression(_) => String::from("Simple linear regression"),
            Self::NonlinearRegression(fit) => format!("Nonlinear regression ({})", fit.equation_id),
            Self::PValueStack(s) => format!("Analyze stack of P values ({})", s.method.label()),
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
            Self::BlandAltman(_) | Self::PassingBablok(_) => {
                matches!(table_kind, TableKind::Column | TableKind::Xy | TableKind::MultipleVariables)
            }
            Self::MultipleRowTTests(_) => {
                matches!(table_kind, TableKind::Grouped | TableKind::Xy | TableKind::Column | TableKind::MultipleVariables)
            }
            Self::FactorialAnova(_) => matches!(table_kind, TableKind::Grouped | TableKind::MultipleVariables),
            Self::NestedComparison(_) => matches!(table_kind, TableKind::Nested),
            Self::Contingency(s) => {
                matches!(table_kind, TableKind::Contingency | TableKind::PartsOfWhole)
                    || (matches!(s.method, ContingencyMethod::ExactBinomial) && table_kind == TableKind::Column)
            }
            Self::Survival(_) => matches!(table_kind, TableKind::Survival | TableKind::MultipleVariables),
            Self::GeneralizedLinearModel(_) | Self::Multivariate(_) => {
                matches!(table_kind, TableKind::MultipleVariables | TableKind::Xy | TableKind::Column)
            }
            Self::LinearRegression(_) | Self::NonlinearRegression(_) => {
                matches!(table_kind, TableKind::Xy | TableKind::Grouped | TableKind::MultipleVariables)
            }
            Self::PValueStack(_) => {
                matches!(table_kind, TableKind::Column | TableKind::Xy | TableKind::Grouped | TableKind::MultipleVariables)
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

    /// Collect sorted, deduplicated explicit upstream [`SheetId`] references embedded inside this analysis configuration.
    #[must_use]
    pub fn upstream_sheet_ids(&self) -> Vec<SheetId> {
        let mut ids: Vec<SheetId> = self.hooked_sources().into_iter().filter_map(|h| h.upstream_sheet_id().cloned()).collect();
        ids.sort();
        ids.dedup();
        ids
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
