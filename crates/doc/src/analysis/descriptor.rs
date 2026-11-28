//! Declarative per-[`AnalysisKind`] metadata table and exhaustive compile-time dispatch guards.
//!
//! [`ALL_ANALYSIS_DESCRIPTORS`] centralizes routing, domain ownership, table compatibility,
//! engine shorthand aliases, canonical parameter keys, UI categorization, and documentation-site
//! anchors in one pure L1 table. [`AnalysisKindId::from_kind`] and [`AnalysisKindId::descriptor`]
//! match exhaustively with zero wildcards so adding a new [`AnalysisKind`] variant fails to compile
//! until its descriptor record is registered here.

use crate::TableKind;

use super::AnalysisKind;

/// Unit discriminant enum with a 1-to-1 correspondence to every [`AnalysisKind`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum AnalysisKindId {
    /// Derived data-table transform ([`AnalysisKind::Transform`]).
    Transform = 0,
    /// Univariate descriptive statistics & normality battery ([`AnalysisKind::DescriptiveStats`]).
    DescriptiveStats = 1,
    /// Grubbs ESD or ROUT FDR outlier detection ([`AnalysisKind::OutlierDetection`]).
    OutlierDetection = 2,
    /// Parametric or nonparametric two-group / one-sample comparison ([`AnalysisKind::TwoGroupComparison`]).
    TwoGroupComparison = 3,
    /// Bland–Altman method-comparison bias and limits of agreement ([`AnalysisKind::BlandAltman`]).
    BlandAltman = 4,
    /// Row-by-row two-group multiple comparisons ([`AnalysisKind::MultipleRowTTests`]).
    MultipleRowTTests = 5,
    /// Passing–Bablok nonparametric linear method comparison ([`AnalysisKind::PassingBablok`]).
    PassingBablok = 6,
    /// One-way ordinary, Welch/Brown–Forsythe, RM/REML, Kruskal–Wallis, or Friedman ANOVA ([`AnalysisKind::OneWayAnova`]).
    OneWayAnova = 7,
    /// Two-way, three-way, or multifactor `N`-way ANOVA / REML ([`AnalysisKind::FactorialAnova`]).
    FactorialAnova = 8,
    /// Hierarchical nested *t*-test or nested one-way REML ANOVA ([`AnalysisKind::NestedComparison`]).
    NestedComparison = 9,
    /// Contingency, goodness-of-fit, diagnostic accuracy, kappa, or exact binomial test ([`AnalysisKind::Contingency`]).
    Contingency = 10,
    /// Kaplan–Meier survival curves or Cox proportional hazards regression ([`AnalysisKind::Survival`]).
    Survival = 11,
    /// Receiver Operating Characteristic curve and AUC analysis ([`AnalysisKind::RocCurve`]).
    RocCurve = 12,
    /// Multivariable linear, logistic, or Poisson generalized linear model ([`AnalysisKind::GeneralizedLinearModel`]).
    GeneralizedLinearModel = 13,
    /// PCA, correlation matrix, clustering, or Rubin multiple imputation ([`AnalysisKind::Multivariate`]).
    Multivariate = 14,
    /// Simple OLS, ANCOVA line comparison, or Model II Deming regression ([`AnalysisKind::LinearRegression`]).
    LinearRegression = 15,
    /// Nonlinear least-squares, Poisson deviance, Cauchy robust, or ROUT curve fitting ([`AnalysisKind::NonlinearRegression`]).
    NonlinearRegression = 16,
    /// Standalone FWER/FDR adjustment of a pre-computed column of *P* values ([`AnalysisKind::PValueStack`]).
    PValueStack = 17,
    /// Prospective statistical power, sample-size, and minimum-detectable-effect solver ([`AnalysisKind::PowerAnalysis`]).
    PowerAnalysis = 18,
    /// Standalone single-predictor binary logistic regression on `XY` tables ([`AnalysisKind::SimpleLogisticRegression`]).
    SimpleLogisticRegression = 19,
    /// Model-free XY spline (natural cubic, Akima, smoothing), LOWESS, and dense point-to-point curve fitting ([`AnalysisKind::FitSplineLowess`]).
    FitSplineLowess = 20,
    /// Standalone Levene / Brown–Forsythe homogeneity-of-variance test ([`AnalysisKind::LeveneTest`]).
    LeveneTest = 21,
    /// Standalone trapezoidal Area Under the Curve (AUC) analysis ([`AnalysisKind::AreaUnderCurve`]).
    AreaUnderCurve = 22,
    /// Preserved read-only imported result snapshot ([`AnalysisKind::ImportedSnapshot`]).
    ImportedSnapshot = 23,
    /// Per-row replicate summary statistics ([`AnalysisKind::RowStatistics`]).
    RowStatistics = 24,
    /// Row, column, or grand-total proportion/percentage reduction ([`AnalysisKind::FractionOfTotal`]).
    FractionOfTotal = 25,
    /// XY curve differentiation, cumulative integration, and smoothing ([`AnalysisKind::CurveCalculus`]).
    CurveCalculus = 26,
    /// Chained deterministic Monte Carlo simulation ([`AnalysisKind::MonteCarloSimulation`]).
    MonteCarloSimulation = 27,
    /// Standalone empirical frequency distribution & histogram binning ([`AnalysisKind::FrequencyDistribution`]).
    FrequencyDistribution = 28,
    /// General *k*-group one-way Analysis of Covariance ([`AnalysisKind::Ancova`]).
    Ancova = 29,
}

impl AnalysisKindId {
    /// Complete ordered list of every [`AnalysisKindId`] discriminant.
    pub const ALL: [Self; 30] = [
        Self::Transform,
        Self::DescriptiveStats,
        Self::OutlierDetection,
        Self::TwoGroupComparison,
        Self::BlandAltman,
        Self::MultipleRowTTests,
        Self::PassingBablok,
        Self::OneWayAnova,
        Self::FactorialAnova,
        Self::NestedComparison,
        Self::Contingency,
        Self::Survival,
        Self::RocCurve,
        Self::GeneralizedLinearModel,
        Self::Multivariate,
        Self::LinearRegression,
        Self::NonlinearRegression,
        Self::PValueStack,
        Self::PowerAnalysis,
        Self::SimpleLogisticRegression,
        Self::FitSplineLowess,
        Self::LeveneTest,
        Self::AreaUnderCurve,
        Self::ImportedSnapshot,
        Self::RowStatistics,
        Self::FractionOfTotal,
        Self::CurveCalculus,
        Self::MonteCarloSimulation,
        Self::FrequencyDistribution,
        Self::Ancova,
    ];

    /// Total number of [`AnalysisKind`] variants registered in [`ALL_ANALYSIS_DESCRIPTORS`].
    pub const COUNT: usize = Self::ALL.len();

    /// Map any [`AnalysisKind`] value to its canonical [`AnalysisKindId`] discriminant.
    ///
    /// Uses an exhaustive `match` with zero wildcards so adding a new [`AnalysisKind`] variant
    /// fails compilation until wired into [`ALL_ANALYSIS_DESCRIPTORS`].
    #[must_use]
    pub const fn from_kind(kind: &AnalysisKind) -> Self {
        match kind {
            AnalysisKind::Transform(_) => Self::Transform,
            AnalysisKind::DescriptiveStats(_) => Self::DescriptiveStats,
            AnalysisKind::OutlierDetection(_) => Self::OutlierDetection,
            AnalysisKind::TwoGroupComparison(_) => Self::TwoGroupComparison,
            AnalysisKind::BlandAltman(_) => Self::BlandAltman,
            AnalysisKind::MultipleRowTTests(_) => Self::MultipleRowTTests,
            AnalysisKind::PassingBablok(_) => Self::PassingBablok,
            AnalysisKind::OneWayAnova(_) => Self::OneWayAnova,
            AnalysisKind::FactorialAnova(_) => Self::FactorialAnova,
            AnalysisKind::NestedComparison(_) => Self::NestedComparison,
            AnalysisKind::Contingency(_) => Self::Contingency,
            AnalysisKind::Survival(_) => Self::Survival,
            AnalysisKind::RocCurve(_) => Self::RocCurve,
            AnalysisKind::GeneralizedLinearModel(_) => Self::GeneralizedLinearModel,
            AnalysisKind::Multivariate(_) => Self::Multivariate,
            AnalysisKind::LinearRegression(_) => Self::LinearRegression,
            AnalysisKind::NonlinearRegression(_) => Self::NonlinearRegression,
            AnalysisKind::PValueStack(_) => Self::PValueStack,
            AnalysisKind::PowerAnalysis(_) => Self::PowerAnalysis,
            AnalysisKind::SimpleLogisticRegression(_) => Self::SimpleLogisticRegression,
            AnalysisKind::FitSplineLowess(_) => Self::FitSplineLowess,
            AnalysisKind::LeveneTest(_) => Self::LeveneTest,
            AnalysisKind::AreaUnderCurve(_) => Self::AreaUnderCurve,
            AnalysisKind::ImportedSnapshot(_) => Self::ImportedSnapshot,
            AnalysisKind::RowStatistics(_) => Self::RowStatistics,
            AnalysisKind::FractionOfTotal(_) => Self::FractionOfTotal,
            AnalysisKind::CurveCalculus(_) => Self::CurveCalculus,
            AnalysisKind::MonteCarloSimulation(_) => Self::MonteCarloSimulation,
            AnalysisKind::FrequencyDistribution(_) => Self::FrequencyDistribution,
            AnalysisKind::Ancova(_) => Self::Ancova,
        }
    }

    /// Return the static [`AnalysisKindDescriptor`] corresponding to this discriminant.
    ///
    /// Matches exhaustively with zero wildcards so every [`AnalysisKindId`] maps directly to its
    /// descriptor slot.
    #[must_use]
    pub const fn descriptor(self) -> &'static AnalysisKindDescriptor {
        match self {
            Self::Transform => &ALL_ANALYSIS_DESCRIPTORS[0],
            Self::DescriptiveStats => &ALL_ANALYSIS_DESCRIPTORS[1],
            Self::OutlierDetection => &ALL_ANALYSIS_DESCRIPTORS[2],
            Self::TwoGroupComparison => &ALL_ANALYSIS_DESCRIPTORS[3],
            Self::BlandAltman => &ALL_ANALYSIS_DESCRIPTORS[4],
            Self::MultipleRowTTests => &ALL_ANALYSIS_DESCRIPTORS[5],
            Self::PassingBablok => &ALL_ANALYSIS_DESCRIPTORS[6],
            Self::OneWayAnova => &ALL_ANALYSIS_DESCRIPTORS[7],
            Self::FactorialAnova => &ALL_ANALYSIS_DESCRIPTORS[8],
            Self::NestedComparison => &ALL_ANALYSIS_DESCRIPTORS[9],
            Self::Contingency => &ALL_ANALYSIS_DESCRIPTORS[10],
            Self::Survival => &ALL_ANALYSIS_DESCRIPTORS[11],
            Self::RocCurve => &ALL_ANALYSIS_DESCRIPTORS[12],
            Self::GeneralizedLinearModel => &ALL_ANALYSIS_DESCRIPTORS[13],
            Self::Multivariate => &ALL_ANALYSIS_DESCRIPTORS[14],
            Self::LinearRegression => &ALL_ANALYSIS_DESCRIPTORS[15],
            Self::NonlinearRegression => &ALL_ANALYSIS_DESCRIPTORS[16],
            Self::PValueStack => &ALL_ANALYSIS_DESCRIPTORS[17],
            Self::PowerAnalysis => &ALL_ANALYSIS_DESCRIPTORS[18],
            Self::SimpleLogisticRegression => &ALL_ANALYSIS_DESCRIPTORS[19],
            Self::FitSplineLowess => &ALL_ANALYSIS_DESCRIPTORS[20],
            Self::LeveneTest => &ALL_ANALYSIS_DESCRIPTORS[21],
            Self::AreaUnderCurve => &ALL_ANALYSIS_DESCRIPTORS[22],
            Self::ImportedSnapshot => &ALL_ANALYSIS_DESCRIPTORS[23],
            Self::RowStatistics => &ALL_ANALYSIS_DESCRIPTORS[24],
            Self::FractionOfTotal => &ALL_ANALYSIS_DESCRIPTORS[25],
            Self::CurveCalculus => &ALL_ANALYSIS_DESCRIPTORS[26],
            Self::MonteCarloSimulation => &ALL_ANALYSIS_DESCRIPTORS[27],
            Self::FrequencyDistribution => &ALL_ANALYSIS_DESCRIPTORS[28],
            Self::Ancova => &ALL_ANALYSIS_DESCRIPTORS[29],
        }
    }

    /// Canonical kebab-case slug for this analysis kind.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        self.descriptor().slug
    }

    /// Exact Rust / Serde variant name on [`AnalysisKind`] for this discriminant.
    #[must_use]
    pub const fn serde_variant(self) -> &'static str {
        self.descriptor().serde_variant
    }
}

/// High-level computation domain and engine command family owning an [`AnalysisKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnalysisDomainKind {
    /// Derived data-table transforms (`curvebook-transform`, `transform.run`).
    Transform,
    /// Biostatistics, hypothesis testing, survival, multivariate, and power (`curvebook-stats`, `stats.run`).
    Stats,
    /// Linear, Deming, and nonlinear curve regression (`curvebook-fit`, `fit.run`).
    Fit,
}

impl AnalysisDomainKind {
    /// Canonical lowercase domain identifier (`"transform"`, `"stats"`, or `"fit"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transform => "transform",
            Self::Stats => "stats",
            Self::Fit => "fit",
        }
    }

    /// Default auto-generated `ResultSheet` ID prefix when a command omits `sheet_id`.
    #[must_use]
    pub const fn default_sheet_prefix(self) -> &'static str {
        match self {
            Self::Transform => "tr",
            Self::Stats => "stat",
            Self::Fit => "fit",
        }
    }

    /// Workspace L2 crate package name implementing this computation domain.
    #[must_use]
    pub const fn crate_name(self) -> &'static str {
        match self {
            Self::Transform => "curvebook-transform",
            Self::Stats => "curvebook-stats",
            Self::Fit => "curvebook-fit",
        }
    }
}

/// Reactive DAG recomputation routing strategy used by `curvebook-engine::dag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnalysisRecomputeStrategy {
    /// Route through `curvebook_transform::execute_analysis_transform` with a `TransformContext`.
    Transform,
    /// Check [`AnalysisKind::is_compatible_with`] and route through `curvebook_stats::run_analysis`.
    Stats,
    /// Resolve linked `InfoSheet` overrides onto [`crate::PowerAnalysisSpec`] before running `curvebook_stats::run_analysis`.
    PowerWithInfoHooks,
    /// Validate/resolve `InfoSheet` constant hooks into `HookContext` and route through `curvebook_fit::execute_analysis`.
    Fit,
    /// Preserve an imported read-only result snapshot without running a CurveBook solver, transitioning `Imported -> ImportedStale` on upstream edits or missing sources.
    PreserveSnapshot,
    /// Orchestrate chained Monte Carlo iterations inside `curvebook-engine` (`L5`) over any downstream analysis.
    MonteCarloEngine,
}

impl AnalysisRecomputeStrategy {
    /// Canonical lowercase identifier for this DAG recomputation route.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transform => "transform",
            Self::Stats => "stats",
            Self::PowerWithInfoHooks => "power_with_info_hooks",
            Self::Fit => "fit",
            Self::PreserveSnapshot => "preserve_snapshot",
            Self::MonteCarloEngine => "monte_carlo_engine",
        }
    }
}

/// Primary UI catalog grouping for interactive analysis pickers and inspectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnalysisUiCategory {
    /// Derived data transformations (`transform`).
    Transform,
    /// One-way / column / univariate analyses (`column`).
    Column,
    /// Multi-factor / row-wise grouped analyses (`grouped`).
    Grouped,
    /// Continuous XY, linear, nonlinear, and method-comparison regression (`xy`).
    XyAndRegression,
    /// Time-to-event Kaplan–Meier and Cox proportional hazards (`survival`).
    Survival,
    /// Categorical contingency, proportion, and agreement analyses (`contingency`).
    Contingency,
    /// Multivariable GLM, PCA, correlation matrices, clustering, and multiple imputation (`multivariate`).
    Multivariate,
}

impl AnalysisUiCategory {
    /// All seven interactive UI analysis categories in canonical display order.
    pub const ALL: [Self; 7] =
        [Self::Transform, Self::Column, Self::Grouped, Self::XyAndRegression, Self::Survival, Self::Contingency, Self::Multivariate];

    /// Canonical lowercase category slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Transform => "transform",
            Self::Column => "column",
            Self::Grouped => "grouped",
            Self::XyAndRegression => "xy",
            Self::Survival => "survival",
            Self::Contingency => "contingency",
            Self::Multivariate => "multivariate",
        }
    }

    /// Neutral human-readable category header label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transform => "Transform & Derive",
            Self::Column => "Column Analyses",
            Self::Grouped => "Grouped Analyses",
            Self::XyAndRegression => "XY & Regression",
            Self::Survival => "Survival Analyses",
            Self::Contingency => "Contingency",
            Self::Multivariate => "Multivariate & GLM",
        }
    }
}

/// Complete declarative metadata record for one [`AnalysisKind`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisKindDescriptor {
    /// Unit-enum discriminant identifying this [`AnalysisKind`] variant.
    pub id: AnalysisKindId,
    /// Canonical kebab-case slug matching documentation anchors (`#analysis-<slug>`, `#analysis-kind-<slug>`).
    pub slug: &'static str,
    /// Exact Rust / Serde variant name on [`AnalysisKind`].
    pub serde_variant: &'static str,
    /// Canonical human-readable title for documentation headings and registry tables.
    pub title: &'static str,
    /// Concise display label suitable for menus, inspector headers, and dialogs.
    pub short_label: &'static str,
    /// One-sentence scientific scope summary for documentation and catalog listings.
    pub description: &'static str,
    /// Owning engine/command domain (`Transform`, `Stats`, or `Fit`).
    pub domain: AnalysisDomainKind,
    /// Reactive DAG recomputation route used by `curvebook-engine::dag::recompute_result_sheet`.
    pub recompute_strategy: AnalysisRecomputeStrategy,
    /// Primary interactive UI category grouping.
    pub ui_category: AnalysisUiCategory,
    /// Input [`TableKind`] archetypes compatible with at least one configuration of this analysis kind.
    pub compatible_table_kinds: &'static [TableKind],
    /// Whether sub-method parameters restrict [`Self::compatible_table_kinds`] further
    /// (for example [`crate::ContingencyMethod`] rules on `PartsOfWhole` and `Column` tables).
    pub has_method_specific_table_rules: bool,
    /// Default canonical `snake_case` `kind` shorthand accepted by engine analysis commands.
    pub canonical_shorthand: &'static str,
    /// Complete ordered list of `kind` shorthand strings resolving to this variant in command handlers.
    pub shorthands: &'static [&'static str],
    /// Whether built-in `curvebook-fit` equation identifiers also resolve to this analysis kind.
    pub accepts_equation_id_shorthand: bool,
    /// Canonical `snake_case` top-level or nested parameter keys owned by this kind's specification structs.
    pub canonical_param_keys: &'static [&'static str],
    /// Whether this analysis kind carries a [`crate::MultiplicitySpec`] post-hoc / FDR configuration.
    pub supports_multiplicity: bool,
    /// Whether specifications of this kind can reference upstream `InfoSheet` or [`crate::HookSource`] constants.
    pub supports_info_hooks: bool,
    /// Whether `MultipleVariables` sources auto-populate `selected_data_sets` with numeric columns when empty.
    pub auto_selects_mv_numeric_columns: bool,
    /// Interactive UI `ANALYSIS_CATALOG` identifiers associated with this analysis kind.
    pub ui_catalog_ids: &'static [&'static str],
    /// Static manual section anchor ID (`analysis-<slug>`).
    pub docs_anchor: &'static str,
    /// Whether users and commands may create new analyses of this kind directly (`false` for import-only snapshots).
    pub user_creatable: bool,
}

impl AnalysisKindDescriptor {
    /// Return `true` when `table_kind` appears in [`Self::compatible_table_kinds`].
    ///
    /// Note that when [`Self::has_method_specific_table_rules`] is `true` (specifically
    /// [`AnalysisKindId::Contingency`]), callers with a concrete [`AnalysisKind`] instance should
    /// evaluate [`AnalysisKind::is_compatible_with`] to enforce sub-method constraints.
    #[must_use]
    pub const fn supports_table_kind(&self, table_kind: TableKind) -> bool {
        let mut rest = self.compatible_table_kinds;
        while let [head, tail @ ..] = rest {
            if table_kind_eq(*head, table_kind) {
                return true;
            }
            rest = tail;
        }
        false
    }

    /// Return `true` if `shorthand` matches one of [`Self::shorthands`] (trimmed, ASCII case-insensitive).
    #[must_use]
    pub fn accepts_shorthand(&self, shorthand: &str) -> bool {
        let norm = shorthand.trim();
        !norm.is_empty() && self.shorthands.iter().any(|candidate| candidate.eq_ignore_ascii_case(norm))
    }

    /// Return `true` if `key` is listed in [`Self::canonical_param_keys`].
    #[must_use]
    pub fn accepts_canonical_param_key(&self, key: &str) -> bool {
        self.canonical_param_keys.contains(&key)
    }

    /// Return `true` if `catalog_id` is listed in [`Self::ui_catalog_ids`].
    #[must_use]
    pub fn has_ui_catalog_id(&self, catalog_id: &str) -> bool {
        self.ui_catalog_ids.contains(&catalog_id)
    }
}

const XY_ONLY: &[TableKind] = &[TableKind::Xy];
const XY_GROUPED_MV: &[TableKind] = &[TableKind::Xy, TableKind::Grouped, TableKind::MultipleVariables];
const COL_XY_GROUPED_MV: &[TableKind] = &[TableKind::Column, TableKind::Xy, TableKind::Grouped, TableKind::MultipleVariables];
const GROUPED_XY_COL_MV: &[TableKind] = &[TableKind::Grouped, TableKind::Xy, TableKind::Column, TableKind::MultipleVariables];
const COL_XY_MV: &[TableKind] = &[TableKind::Column, TableKind::Xy, TableKind::MultipleVariables];
const MV_XY_COL: &[TableKind] = &[TableKind::MultipleVariables, TableKind::Xy, TableKind::Column];
const MV_XY: &[TableKind] = &[TableKind::MultipleVariables, TableKind::Xy];
const COL_MV: &[TableKind] = &[TableKind::Column, TableKind::MultipleVariables];
const GROUPED_MV: &[TableKind] = &[TableKind::Grouped, TableKind::MultipleVariables];
const SURVIVAL_MV: &[TableKind] = &[TableKind::Survival, TableKind::MultipleVariables];
const CONTINGENCY_TABLES: &[TableKind] = &[TableKind::Contingency, TableKind::PartsOfWhole, TableKind::Column];
const NESTED_ONLY: &[TableKind] = &[TableKind::Nested];
const FRACTION_OF_TOTAL_TABLES: &[TableKind] =
    &[TableKind::Column, TableKind::Xy, TableKind::Grouped, TableKind::Contingency, TableKind::PartsOfWhole, TableKind::MultipleVariables];

/// Exhaustive declarative metadata table covering every [`AnalysisKind`] variant in canonical order.
pub const ALL_ANALYSIS_DESCRIPTORS: [AnalysisKindDescriptor; AnalysisKindId::COUNT] = [
    AnalysisKindDescriptor {
        id: AnalysisKindId::Transform,
        slug: "transform",
        serde_variant: "Transform",
        title: "Transforms & Data Manipulation",
        short_label: "Transform & Derive",
        description: "Mathematical transforms, normalization, baseline subtraction, transpose, row pruning, simulation, and random sampling.",
        domain: AnalysisDomainKind::Transform,
        recompute_strategy: AnalysisRecomputeStrategy::Transform,
        ui_category: AnalysisUiCategory::Transform,
        compatible_table_kinds: &TableKind::ALL,
        has_method_specific_table_rules: false,
        canonical_shorthand: "math",
        shorthands: &[
            "math",
            "transform.math",
            "math_transform",
            "normalize",
            "transform.normalize",
            "remove_baseline",
            "baseline",
            "transform.remove_baseline",
            "transpose",
            "transform.transpose",
            "prune_rows",
            "prune",
            "transform.prune_rows",
            "select_and_transform",
            "select",
            "transform.select_and_transform",
            "simulate",
            "simulate_xy",
            "transform.simulate",
            "random_sample",
            "sample",
            "transform.random_sample",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "x_transform",
            "y_transform",
            "error_propagation",
            "pharmacology_preset",
            "interchange_xy",
            "zero_x_substitute",
            "asymmetric_ci_policy",
            "zero_anchor",
            "hundred_anchor",
            "output_percent",
            "per_data_set",
            "baseline_columns",
            "operation",
            "baseline_kind",
            "retain_unselected_columns",
            "swap_row_and_column_titles",
            "every_k_rows",
            "average_window",
            "filter_formula",
            "target_archetype",
            "derived_columns",
            "complete_rows_only",
            "subject_column",
            "row_title_column",
            "contingency_count_column",
            "include_variable_name_prefixes",
            "seed",
            "row_count",
            "replicates",
            "x_min",
            "x_max",
            "equation_id",
            "parameters",
            "noise",
            "simulation_mode",
            "sample_size",
            "stratify_column",
            "proportional_allocation",
        ],
        supports_multiplicity: false,
        supports_info_hooks: true,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &[
            "transform.math",
            "transform.normalize",
            "transform.baseline",
            "transform.transpose",
            "transform.prune",
            "transform.select",
            "transform.simulate",
            "transform.random_sample",
        ],
        docs_anchor: "analysis-transform",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::DescriptiveStats,
        slug: "descriptive-stats",
        serde_variant: "DescriptiveStats",
        title: "Descriptive Statistics & Normality",
        short_label: "Descriptive Statistics",
        description: "Univariate location, spread, quantiles, normality battery, and one-sample location tests.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "descriptive_stats",
        shorthands: &["descriptive_stats", "descriptive", "column_stats", "normality", "normality_lognormality", "lognormality"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "aggregation",
            "custom_upper_percentile",
            "run_normality_battery",
            "hypothetical_value",
            "normality_only",
            "emit_qq_tabs",
            "visible_metrics",
            "significant_digits",
            "save_as_default",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.descriptive"],
        docs_anchor: "analysis-descriptive-stats",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::OutlierDetection,
        slug: "outlier-detection",
        serde_variant: "OutlierDetection",
        title: "Outlier Detection (Grubbs / ROUT)",
        short_label: "Outlier Detection",
        description: "Single-outlier Grubbs ESD and multi-outlier ROUT false-discovery-rate detection.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "outliers",
        shorthands: &["outliers", "outlier", "rout", "grubbs"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["fdr_q_percent", "alpha", "iterative"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.outliers"],
        docs_anchor: "analysis-outlier-detection",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::TwoGroupComparison,
        slug: "two-group-comparison",
        serde_variant: "TwoGroupComparison",
        title: "Two-Group & One-Sample Comparisons",
        short_label: "Two-Group Tests",
        description: "Parametric t-tests (Student, Welch, paired, ratio, lognormal) and nonparametric rank/sign tests.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "unpaired_t",
        shorthands: &[
            "unpaired_t",
            "t_test",
            "unpaired_t_test",
            "welch_t",
            "welch_t_test",
            "paired_t",
            "paired_t_test",
            "ratio_paired_t",
            "lognormal_t",
            "lognormal_unpaired_t",
            "mann_whitney",
            "mann_whitney_u",
            "wilcoxon",
            "wilcoxon_matched_pairs",
            "wilcoxon_signed_rank",
            "paired_sign_test",
            "paired_sign",
            "sign_test",
            "kolmogorov_smirnov",
            "ks",
            "ks_two_sample",
            "one_sample_t",
            "one_sample_wilcoxon",
            "one_sample_ratio_t",
            "lognormal_welch_t",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["method", "two_tailed", "compute_effect_sizes", "mv_group_column", "mv_response_column", "hypothetical"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.two_group"],
        docs_anchor: "analysis-two-group-comparison",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::BlandAltman,
        slug: "bland-altman",
        serde_variant: "BlandAltman",
        title: "Bland–Altman Method Comparison",
        short_label: "Bland–Altman",
        description: "Method-comparison bias and 95% limits of agreement.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_XY_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "bland_altman",
        shorthands: &["bland_altman", "stats.bland_altman"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["column_a", "column_b", "percent_difference", "xy_compare_x_vs_y"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.bland_altman"],
        docs_anchor: "analysis-bland-altman",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::MultipleRowTTests,
        slug: "multiple-row-t-tests",
        serde_variant: "MultipleRowTTests",
        title: "Multiple Row-by-Row Two-Group Tests",
        short_label: "Multiple Row Tests",
        description: "Row-wise parametric or nonparametric comparisons with FWER/FDR correction.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Grouped,
        compatible_table_kinds: GROUPED_XY_COL_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "multiple_row_t_tests",
        shorthands: &["multiple_row_t_tests", "stats.multiple_row_t_tests", "multiple_row_tests", "multiple_t_tests"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "paired",
            "nonparametric",
            "assume_equal_sd",
            "multiplicity",
            "threshold",
            "report_confidence_intervals",
            "control_column",
            "lognormal_mode",
        ],
        supports_multiplicity: true,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.multiple_row_t_tests"],
        docs_anchor: "analysis-multiple-row-t-tests",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::PassingBablok,
        slug: "passing-bablok",
        serde_variant: "PassingBablok",
        title: "Passing–Bablok Regression",
        short_label: "Passing–Bablok",
        description: "Nonparametric errors-in-variables linear method-comparison regression.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: COL_XY_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "passing_bablok",
        shorthands: &["passing_bablok", "stats.passing_bablok"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["x_column", "y_column"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.passing_bablok"],
        docs_anchor: "analysis-passing-bablok",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::OneWayAnova,
        slug: "one-way-anova",
        serde_variant: "OneWayAnova",
        title: "One-Way ANOVA & Nonparametric Equivalents",
        short_label: "One-Way ANOVA",
        description: "Ordinary, Brown–Forsythe/Welch, repeated-measures/REML, Kruskal–Wallis, and Friedman ANOVA with post-hoc multiple comparisons.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "one_way_anova",
        shorthands: &[
            "one_way_anova",
            "anova",
            "welch_anova",
            "brown_forsythe",
            "rm_one_way_anova",
            "repeated_measures_anova",
            "kruskal_wallis",
            "friedman",
            "lognormal_one_way_anova",
            "lognormal_welch_anova",
            "lognormal_rm_anova",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "method",
            "geisser_greenhouse",
            "mixed_effects_reml",
            "df_method",
            "residual_covariance",
            "compute_effect_sizes",
            "multiplicity",
            "threshold",
            "report_confidence_intervals",
            "control_column",
            "selected_pairs",
        ],
        supports_multiplicity: true,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.one_way_anova"],
        docs_anchor: "analysis-one-way-anova",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::FactorialAnova,
        slug: "factorial-anova",
        serde_variant: "FactorialAnova",
        title: "Two-Way, Three-Way & Factorial ANOVA",
        short_label: "Factorial ANOVA",
        description: "Multi-factor ANOVA / REML mixed models with main effects, interactions, and simple-effects slicing.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Grouped,
        compatible_table_kinds: GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "two_way_anova",
        shorthands: &["two_way_anova", "factorial_anova", "three_way_anova"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "three_way",
            "multifactor_columns",
            "response_column",
            "max_interaction_order",
            "ss_type",
            "repeated_measures",
            "mixed_effects_reml",
            "df_method",
            "residual_covariance",
            "fit_reml_on_missing",
            "compute_effect_sizes",
            "multiplicity",
            "threshold",
            "report_confidence_intervals",
            "control_column",
            "selected_pairs",
            "control_factor",
            "sphericity_correction",
            "reml_zero_variance_policy",
            "comparison_family_mode",
            "preselected_pairs",
            "swap_direction",
            "heteroscedasticity_test",
            "formula",
            "three_way_rm_factors",
            "emit_consolidated_data_tab",
        ],
        supports_multiplicity: true,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.factorial_anova"],
        docs_anchor: "analysis-factorial-anova",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::NestedComparison,
        slug: "nested-comparison",
        serde_variant: "NestedComparison",
        title: "Nested t-Test & Nested One-Way ANOVA",
        short_label: "Nested Comparison",
        description: "Hierarchical nested random-effects / mixed-model comparisons.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: NESTED_ONLY,
        has_method_specific_table_rules: false,
        canonical_shorthand: "nested_anova",
        shorthands: &["nested_anova", "nested_t", "nested"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "two_group_t_test",
            "df_method",
            "multiplicity",
            "threshold",
            "report_confidence_intervals",
            "control_column",
            "selected_pairs",
        ],
        supports_multiplicity: true,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.nested"],
        docs_anchor: "analysis-nested-comparison",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::Contingency,
        slug: "contingency",
        serde_variant: "Contingency",
        title: "Contingency & Proportion Tests",
        short_label: "Contingency & Proportions",
        description: "Chi-square, Fisher's exact, chi-square for trend, McNemar, Cohen's kappa, and exact binomial tests.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Contingency,
        compatible_table_kinds: CONTINGENCY_TABLES,
        has_method_specific_table_rules: true,
        canonical_shorthand: "contingency",
        shorthands: &[
            "contingency",
            "chi_square",
            "fisher_exact",
            "barnard_exact",
            "stats.barnard_exact",
            "cohen_kappa",
            "kappa",
            "fleiss_kappa",
            "exact_binomial",
            "binomial",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "method",
            "yates_continuity",
            "weighting",
            "compute_odds_and_risk_ratios",
            "compute_sensitivity_specificity",
            "compute_phi_and_cramers_v",
            "expected_proportions",
            "null_proportion",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.contingency"],
        docs_anchor: "analysis-contingency",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::Survival,
        slug: "survival",
        serde_variant: "Survival",
        title: "Survival Analysis & Cox Regression",
        short_label: "Survival & Cox PH",
        description: "Kaplan–Meier survival curves, log-rank/Gehan-Breslow-Wilcoxon tests, and Cox proportional hazards regression.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Survival,
        compatible_table_kinds: SURVIVAL_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "kaplan_meier",
        shorthands: &["kaplan_meier", "survival", "log_rank", "cox_ph", "cox"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "log_rank_mantel_cox",
            "gehan_breslow_wilcoxon",
            "trend_test",
            "asymmetrical_greenwood_ci",
            "multiplicity",
            "threshold",
            "report_confidence_intervals",
            "control_column",
            "selected_pairs",
            "mv_time_column",
            "mv_status_column",
            "mv_group_column",
            "time_column",
            "status_column",
            "predictor_columns",
            "reference_levels",
            "tie_method",
            "event_code",
            "censored_code",
            "treat_other_as",
            "ratio_median_ci",
            "mv_status_columns",
            "mv_group_columns",
            "formula",
            "covariate_profiles",
            "nested_comparison",
            "emit_cox_snell_residuals",
            "ci_method",
        ],
        supports_multiplicity: true,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.survival_km", "stats.survival_cox"],
        docs_anchor: "analysis-survival",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::RocCurve,
        slug: "roc-curve",
        serde_variant: "RocCurve",
        title: "ROC Curve Analysis",
        short_label: "ROC Curve",
        description: "Receiver operating characteristic curves, DeLong/Hanley–McNeil AUC confidence intervals, Youden cutoffs, and AUC comparison.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "roc",
        shorthands: &["roc"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["ci_method"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.roc"],
        docs_anchor: "analysis-roc-curve",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::GeneralizedLinearModel,
        slug: "generalized-linear-model",
        serde_variant: "GeneralizedLinearModel",
        title: "Generalized Linear Models (GLM)",
        short_label: "Multiple Regression (GLM)",
        description: "Multiple linear, logistic, and Poisson regression with link diagnostics and likelihood tests.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Multivariate,
        compatible_table_kinds: MV_XY_COL,
        has_method_specific_table_rules: false,
        canonical_shorthand: "glm",
        shorthands: &["glm", "linear_glm", "multiple_linear", "logistic_glm", "multiple_logistic", "poisson_glm", "multiple_poisson"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "family",
            "response_column",
            "predictor_columns",
            "reference_levels",
            "compute_vif",
            "formula",
            "nested_comparison",
            "weights",
            "dispersion_mode",
            "classification_cutoff",
            "ci_method",
            "emit_influence_diagnostics",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.glm"],
        docs_anchor: "analysis-generalized-linear-model",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::Multivariate,
        slug: "multivariate",
        serde_variant: "Multivariate",
        title: "Multivariate Analysis (PCA, Correlation, Clustering, MI)",
        short_label: "Multivariate Analysis",
        description: "Principal component analysis, Pearson/Spearman/Kendall correlation matrices, k-means/hierarchical clustering, and Rubin multiple imputation.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Multivariate,
        compatible_table_kinds: MV_XY_COL,
        has_method_specific_table_rules: false,
        canonical_shorthand: "pca",
        shorthands: &[
            "pca",
            "principal_components",
            "correlation",
            "correlation_matrix",
            "kendall",
            "kendall_tau_b",
            "kendall_correlation",
            "clustering",
            "kmeans",
            "multiple_imputation",
            "stats.multiple_imputation",
            "mice",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "standardize",
            "parallel_analysis_seed",
            "pcr_outcome_column",
            "retention_rule",
            "emit_extra_matrices",
            "spearman_rank",
            "method",
            "k_means_k",
            "distance",
            "linkage",
            "k_sweep_max",
            "consensus_optimal_k",
            "kmeans_algorithm",
            "cluster_direction",
            "cut_height",
            "emit_anova_tab",
            "imputations_m",
            "max_iters",
            "seed",
            "response_column",
            "predictor_columns",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.pca", "stats.pcr", "stats.corr_matrix", "stats.clustering", "stats.multiple_imputation"],
        docs_anchor: "analysis-multivariate",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::LinearRegression,
        slug: "linear-regression",
        serde_variant: "LinearRegression",
        title: "Simple & Deming Linear Regression",
        short_label: "Linear / Deming Regression",
        description: "Model I OLS, ANCOVA slope/intercept comparison, and Model II Deming regression.",
        domain: AnalysisDomainKind::Fit,
        recompute_strategy: AnalysisRecomputeStrategy::Fit,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "linear",
        shorthands: &["linear", "simple_linear", "linear_regression", "deming", "deming_regression"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "deming_model_ii",
            "force_through_origin",
            "fixed_point",
            "standard_addition",
            "extend_line_to_x_intercept",
            "deming_error",
            "confidence_bands",
            "prediction_bands",
            "interpolate_unknowns",
            "x_range_filter",
            "plot_x_min",
            "plot_x_max",
            "plot_segments",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["fit.linear"],
        docs_anchor: "analysis-linear-regression",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::NonlinearRegression,
        slug: "nonlinear-regression",
        serde_variant: "NonlinearRegression",
        title: "Nonlinear Regression & Custom Equations",
        short_label: "Nonlinear Regression",
        description: "Trust-region Levenberg–Marquardt, Poisson deviance, Cauchy robust, and ROUT curve fitting with DSL/ODE support.",
        domain: AnalysisDomainKind::Fit,
        recompute_strategy: AnalysisRecomputeStrategy::Fit,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "nonlinear",
        shorthands: &[
            "nonlinear",
            "nonlinear_regression",
            "four_parameter_logistic",
            "4pl",
            "4pl_agonist",
            "4pl_inhibitor",
            "four_parameter_logistic_inhibitor",
            "ic50",
            "agonist_4pl",
            "inhibitor_4pl",
            "3pl",
            "three_parameter_logistic",
            "3pl_inhibitor",
            "one_site_binding",
            "saturation_binding",
        ],
        accepts_equation_id_shorthand: true,
        canonical_param_keys: &[
            "equation_id",
            "custom_dsl",
            "method",
            "fdr_q_percent",
            "weighting",
            "parameter_rules",
            "profile_likelihood_ci",
            "identify_unstable_parameters",
            "flag_r_squared_below",
            "comparison",
            "alternative_equation_id",
            "confidence_bands",
            "prediction_bands",
            "interpolate_unknowns",
            "replicates_lack_of_fit_test",
            "normality_of_residuals",
            "max_iters",
            "x_range_filter",
            "y_range_filter",
            "plot_function_only",
            "plot_x_min",
            "plot_x_max",
            "homoscedasticity_test",
            "emit_residuals_tab",
            "emit_covariance_tab",
            "mv_group_columns",
            "mv_x_columns",
            "mv_y_column",
            "mv_y_columns",
            "weighting_source",
            "outlier_handling",
            "emit_cleaned_data_tab",
            "replicate_fit_mode",
            "convergence_strictness",
            "identifiability_mode",
            "show_standard_errors",
            "comparison_question",
            "comparison_method",
            "prefer_well_defined_fit",
            "summary_parameter",
            "create_summary_graph",
            "output_significant_digits",
            "fit_flag_rules",
            "relational_constraints",
            "plot_distribution",
            "emit_extra_gof",
        ],
        supports_multiplicity: false,
        supports_info_hooks: true,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["fit.nonlinear"],
        docs_anchor: "analysis-nonlinear-regression",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::PValueStack,
        slug: "p-value-stack",
        serde_variant: "PValueStack",
        title: "P-Value Stack Multiplicity Adjustment",
        short_label: "P-Value Stack",
        description: "Standalone FWER and FDR adjustment over a pre-computed column of P values.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "pvalue_stack",
        shorthands: &["pvalue_stack", "stats.pvalue_stack", "p_value_stack", "pvalues", "pvaluestack", "stats.pvaluestack"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["method", "threshold", "column"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.pvalue_stack"],
        docs_anchor: "analysis-p-value-stack",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::PowerAnalysis,
        slug: "power-analysis",
        serde_variant: "PowerAnalysis",
        title: "Statistical Power & Sample Size",
        short_label: "Power & Sample Size",
        description: "Prospective power, sample-size, and minimum-detectable-effect calculations.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::PowerWithInfoHooks,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: &TableKind::ALL,
        has_method_specific_table_rules: false,
        canonical_shorthand: "power",
        shorthands: &["power", "power_analysis", "stats.power", "power_sample_size"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "family",
            "solve_target",
            "tail_mode",
            "alpha",
            "target_power",
            "sample_size",
            "sample_size_group2",
            "allocation_ratio",
            "effect",
            "num_groups",
            "chi_square_df",
            "two_proportions_mode",
            "continuity_correction",
            "correlation_method",
            "info_sheet_id",
            "survival_method",
            "target_events",
            "event_prob_group1",
            "event_prob_group2",
        ],
        supports_multiplicity: false,
        supports_info_hooks: true,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.power"],
        docs_anchor: "analysis-power-analysis",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::SimpleLogisticRegression,
        slug: "simple-logistic-regression",
        serde_variant: "SimpleLogisticRegression",
        title: "Simple Logistic Regression",
        short_label: "Simple Logistic",
        description: "Single-predictor binary logistic regression on XY tables with profile-likelihood CIs, X at 50%, ROC/AUC, and separation diagnostics.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: XY_ONLY,
        has_method_specific_table_rules: false,
        canonical_shorthand: "simple_logistic",
        shorthands: &["simple_logistic", "simple_logistic_regression", "stats.simple_logistic", "xy_logistic"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["emit_row_classification", "confidence_bands", "classification_cutoff", "plot_x_min", "plot_x_max", "plot_segments"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.simple_logistic"],
        docs_anchor: "analysis-simple-logistic-regression",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::FitSplineLowess,
        slug: "fit-spline-lowess",
        serde_variant: "FitSplineLowess",
        title: "Fit Spline / LOWESS Curve",
        short_label: "Fit Spline / LOWESS",
        description: "Model-free XY natural cubic spline, Akima subspline, smoothing spline, LOWESS, and dense point-to-point curve fitting with optional unknown interpolation.",
        domain: AnalysisDomainKind::Transform,
        recompute_strategy: AnalysisRecomputeStrategy::Transform,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: XY_ONLY,
        has_method_specific_table_rules: false,
        canonical_shorthand: "fit_spline_lowess",
        shorthands: &[
            "fit_spline_lowess",
            "spline_lowess",
            "spline",
            "cubic_spline",
            "natural_cubic_spline",
            "akima",
            "akima_subspline",
            "smoothing_spline",
            "lowess",
            "point_to_point",
            "fit.spline_lowess",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "method",
            "segment_count",
            "smoothing_knots",
            "smoothing_df",
            "lowess_preset",
            "lowess_fraction",
            "lowess_robustness_iters",
            "plot_x_min",
            "plot_x_max",
            "emit_curve_coordinates_tab",
            "interpolate_unknowns",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["fit.spline_lowess"],
        docs_anchor: "analysis-fit-spline-lowess",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::LeveneTest,
        slug: "levene-test",
        serde_variant: "LeveneTest",
        title: "Levene / Brown–Forsythe Variance Test",
        short_label: "Levene / Brown–Forsythe",
        description: "Homogeneity-of-variance F test on absolute deviations from each group's median (Brown–Forsythe, default) or mean (Levene).",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "levene",
        shorthands: &[
            "levene",
            "levene_test",
            "stats.levene",
            "levene_median",
            "levene_mean",
            "brown_forsythe_variance",
            "brown_forsythe_levene",
            "variance_homogeneity",
            "equal_variance_test",
        ],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["center"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.levene"],
        docs_anchor: "analysis-levene-test",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::AreaUnderCurve,
        slug: "area-under-curve",
        serde_variant: "AreaUnderCurve",
        title: "Area Under the Curve (AUC)",
        short_label: "Area Under Curve",
        description: "Trapezoidal integration (Yeh & Kwan 1978) with optional partial-window interpolation, peak detection, and design-gated Bailer (1988) net-area SE.",
        domain: AnalysisDomainKind::Transform,
        recompute_strategy: AnalysisRecomputeStrategy::Transform,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: XY_ONLY,
        has_method_specific_table_rules: false,
        canonical_shorthand: "auc",
        shorthands: &["auc", "area_under_curve", "transform.auc"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["baseline", "x_min", "x_max", "independent_time_points", "min_peak_height", "min_area_fraction"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["transform.auc"],
        docs_anchor: "analysis-area-under-curve",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::ImportedSnapshot,
        slug: "imported-snapshot",
        serde_variant: "ImportedSnapshot",
        title: "Imported Result Snapshot",
        short_label: "Imported Snapshot",
        description: "Preserved read-only result snapshot imported from an external format without native recomputation.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::PreserveSnapshot,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: &[],
        has_method_specific_table_rules: false,
        canonical_shorthand: "",
        shorthands: &[],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["original_kind_label"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &[],
        docs_anchor: "analysis-imported-snapshot",
        user_creatable: false,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::RowStatistics,
        slug: "row-statistics",
        serde_variant: "RowStatistics",
        title: "Row Statistics (Replicate Summaries)",
        short_label: "Row Statistics",
        description: "Per-row totals, means (SD/SEM/%CV/CI/min-max), medians (quartiles/percentiles), and geometric means across replicates.",
        domain: AnalysisDomainKind::Transform,
        recompute_strategy: AnalysisRecomputeStrategy::Transform,
        ui_category: AnalysisUiCategory::Transform,
        compatible_table_kinds: COL_XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "row_statistics",
        shorthands: &["row_statistics", "row_stats", "transform.row_statistics"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["summary", "upper_percentile"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &[],
        docs_anchor: "analysis-row-statistics",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::FractionOfTotal,
        slug: "fraction-of-total",
        serde_variant: "FractionOfTotal",
        title: "Fraction of Total",
        short_label: "Fraction of Total",
        description: "Row, column, or grand-total proportions/percentages with optional Wilson or Clopper–Pearson binomial confidence intervals.",
        domain: AnalysisDomainKind::Transform,
        recompute_strategy: AnalysisRecomputeStrategy::Transform,
        ui_category: AnalysisUiCategory::Transform,
        compatible_table_kinds: FRACTION_OF_TOTAL_TABLES,
        has_method_specific_table_rules: false,
        canonical_shorthand: "fraction_of_total",
        shorthands: &["fraction_of_total", "transform.fraction_of_total"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["denominator", "output_percent", "ci_method"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &[],
        docs_anchor: "analysis-fraction-of-total",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::CurveCalculus,
        slug: "curve-calculus",
        serde_variant: "CurveCalculus",
        title: "Curve Calculus (Differentiate, Integrate, Smooth)",
        short_label: "Curve Calculus",
        description: "Numerical first/second derivatives, cumulative trapezoidal integration, and Savitzky–Golay, moving-average, or LOWESS smoothing.",
        domain: AnalysisDomainKind::Transform,
        recompute_strategy: AnalysisRecomputeStrategy::Transform,
        ui_category: AnalysisUiCategory::XyAndRegression,
        compatible_table_kinds: XY_ONLY,
        has_method_specific_table_rules: false,
        canonical_shorthand: "curve_calculus",
        shorthands: &["curve_calculus", "smooth_differentiate_integrate", "differentiate", "smooth", "transform.curve_calculus"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["operation"],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["transform.curve_calculus"],
        docs_anchor: "analysis-curve-calculus",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::MonteCarloSimulation,
        slug: "monte-carlo-simulation",
        serde_variant: "MonteCarloSimulation",
        title: "Monte Carlo Simulation",
        short_label: "Monte Carlo",
        description: "Deterministic chained Monte Carlo simulation over downstream analyses with tracked scalars and hit-rate tabulations.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::MonteCarloEngine,
        ui_category: AnalysisUiCategory::Transform,
        compatible_table_kinds: &TableKind::ALL,
        has_method_specific_table_rules: false,
        canonical_shorthand: "monte_carlo",
        shorthands: &["monte_carlo", "monte_carlo_simulation", "stats.monte_carlo"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &["seed", "iterations", "downstream_result_sheet_id", "tracked_scalars", "hit_rules", "exclude_flagged_fits"],
        supports_multiplicity: false,
        supports_info_hooks: true,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &[],
        docs_anchor: "analysis-monte-carlo-simulation",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::FrequencyDistribution,
        slug: "frequency-distribution",
        serde_variant: "FrequencyDistribution",
        title: "Frequency Distribution & Histogram",
        short_label: "Frequency Distribution",
        description: "Empirical histogram binning (Sturges, Scott, Freedman–Diaconis, or explicit bin width), counts, relative/cumulative frequencies, and optional Gaussian density overlay.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Column,
        compatible_table_kinds: COL_XY_GROUPED_MV,
        has_method_specific_table_rules: false,
        canonical_shorthand: "frequency_distribution",
        shorthands: &["frequency_distribution", "frequency", "histogram", "freq_dist", "stats.frequency_distribution"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "binning_rule",
            "first_bin_center",
            "last_bin_center",
            "output_mode",
            "overlay_gaussian",
            "aggregation",
            "create_histogram_graph",
        ],
        supports_multiplicity: false,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: true,
        ui_catalog_ids: &["stats.frequency_distribution"],
        docs_anchor: "analysis-frequency-distribution",
        user_creatable: true,
    },
    AnalysisKindDescriptor {
        id: AnalysisKindId::Ancova,
        slug: "ancova",
        serde_variant: "Ancova",
        title: "Analysis of Covariance (ANCOVA)",
        short_label: "ANCOVA",
        description: "General k-group one-way ANCOVA adjusting for continuous covariates with Type I/III SS, homogeneity-of-slopes test, adjusted means, and pairwise contrasts.",
        domain: AnalysisDomainKind::Stats,
        recompute_strategy: AnalysisRecomputeStrategy::Stats,
        ui_category: AnalysisUiCategory::Multivariate,
        compatible_table_kinds: MV_XY,
        has_method_specific_table_rules: false,
        canonical_shorthand: "ancova",
        shorthands: &["ancova", "general_ancova", "one_way_ancova", "analysis_of_covariance", "stats.ancova"],
        accepts_equation_id_shorthand: false,
        canonical_param_keys: &[
            "response_column",
            "group_column",
            "covariate_columns",
            "ss_type",
            "test_homogeneity_of_slopes",
            "multiplicity",
            "compute_effect_sizes",
        ],
        supports_multiplicity: true,
        supports_info_hooks: false,
        auto_selects_mv_numeric_columns: false,
        ui_catalog_ids: &["stats.ancova"],
        docs_anchor: "analysis-ancova",
        user_creatable: true,
    },
];

/// Return the complete slice of all [`AnalysisKindDescriptor`] records in canonical order.
#[must_use]
pub const fn all_analysis_descriptors() -> &'static [AnalysisKindDescriptor] {
    &ALL_ANALYSIS_DESCRIPTORS
}

/// Return the [`AnalysisKindDescriptor`] for a concrete [`AnalysisKind`] instance.
#[must_use]
pub const fn descriptor_for_kind(kind: &AnalysisKind) -> &'static AnalysisKindDescriptor {
    AnalysisKindId::from_kind(kind).descriptor()
}

/// Return the [`AnalysisKindDescriptor`] for a discriminant [`AnalysisKindId`].
#[must_use]
pub const fn descriptor_for_id(id: AnalysisKindId) -> &'static AnalysisKindDescriptor {
    id.descriptor()
}

/// Look up an [`AnalysisKindDescriptor`] by its exact canonical kebab-case [`AnalysisKindDescriptor::slug`].
#[must_use]
pub fn descriptor_by_slug(slug: &str) -> Option<&'static AnalysisKindDescriptor> {
    let trimmed = slug.trim();
    ALL_ANALYSIS_DESCRIPTORS.iter().find(|desc| desc.slug == trimmed)
}

/// Look up an [`AnalysisKindDescriptor`] by its exact Serde/Rust [`AnalysisKind`] variant name.
#[must_use]
pub fn descriptor_by_serde_variant(variant: &str) -> Option<&'static AnalysisKindDescriptor> {
    let trimmed = variant.trim();
    ALL_ANALYSIS_DESCRIPTORS.iter().find(|desc| desc.serde_variant == trimmed)
}

/// Look up an [`AnalysisKindDescriptor`] by any engine `kind` shorthand alias (trimmed, ASCII case-insensitive).
#[must_use]
pub fn descriptor_by_shorthand(shorthand: &str) -> Option<&'static AnalysisKindDescriptor> {
    let norm = shorthand.trim();
    if norm.is_empty() {
        return None;
    }
    ALL_ANALYSIS_DESCRIPTORS.iter().find(|desc| desc.accepts_shorthand(norm))
}

/// Look up an [`AnalysisKindDescriptor`] by an interactive UI `ANALYSIS_CATALOG` identifier.
#[must_use]
pub fn descriptor_by_ui_catalog_id(catalog_id: &str) -> Option<&'static AnalysisKindDescriptor> {
    let trimmed = catalog_id.trim();
    ALL_ANALYSIS_DESCRIPTORS.iter().find(|desc| desc.has_ui_catalog_id(trimmed))
}

/// Resolve an [`AnalysisKindDescriptor`] from a slug, Serde variant name, shorthand alias, or UI catalog ID.
#[must_use]
pub fn resolve_analysis_descriptor(query: &str) -> Option<&'static AnalysisKindDescriptor> {
    descriptor_by_slug(query)
        .or_else(|| descriptor_by_serde_variant(query))
        .or_else(|| descriptor_by_shorthand(query))
        .or_else(|| descriptor_by_ui_catalog_id(query))
}

/// Return the default [`AnalysisKindId`] selected by `analysis.run` when `kind` is omitted for `table_kind`.
#[must_use]
pub const fn default_analysis_kind_id_for_table(table_kind: TableKind) -> AnalysisKindId {
    match table_kind {
        TableKind::Xy => AnalysisKindId::NonlinearRegression,
        TableKind::Column => AnalysisKindId::OneWayAnova,
        TableKind::Grouped => AnalysisKindId::FactorialAnova,
        TableKind::Contingency => AnalysisKindId::Contingency,
        TableKind::Survival => AnalysisKindId::Survival,
        TableKind::MultipleVariables => AnalysisKindId::Multivariate,
        TableKind::Nested => AnalysisKindId::NestedComparison,
        TableKind::PartsOfWhole => AnalysisKindId::Contingency,
    }
}

impl AnalysisKind {
    /// Return the unit discriminant [`AnalysisKindId`] for this variant.
    #[must_use]
    pub const fn id(&self) -> AnalysisKindId {
        AnalysisKindId::from_kind(self)
    }

    /// Return the static [`AnalysisKindDescriptor`] describing this variant.
    #[must_use]
    pub const fn descriptor(&self) -> &'static AnalysisKindDescriptor {
        descriptor_for_kind(self)
    }

    /// Return the canonical kebab-case documentation/registry slug for this variant.
    #[must_use]
    pub const fn slug(&self) -> &'static str {
        self.descriptor().slug
    }

    /// Return the owning computation domain ([`AnalysisDomainKind`]) for this variant.
    #[must_use]
    pub const fn domain(&self) -> AnalysisDomainKind {
        self.descriptor().domain
    }

    /// Return the reactive DAG recomputation routing strategy for this variant.
    #[must_use]
    pub const fn recompute_strategy(&self) -> AnalysisRecomputeStrategy {
        self.descriptor().recompute_strategy
    }
}

const fn table_kind_eq(a: TableKind, b: TableKind) -> bool {
    matches!(
        (a, b),
        (TableKind::Xy, TableKind::Xy)
            | (TableKind::Column, TableKind::Column)
            | (TableKind::Grouped, TableKind::Grouped)
            | (TableKind::Contingency, TableKind::Contingency)
            | (TableKind::Survival, TableKind::Survival)
            | (TableKind::PartsOfWhole, TableKind::PartsOfWhole)
            | (TableKind::MultipleVariables, TableKind::MultipleVariables)
            | (TableKind::Nested, TableKind::Nested)
    )
}

const fn const_str_eq(a: &str, b: &str) -> bool {
    let (mut ab, mut bb) = (a.as_bytes(), b.as_bytes());
    if ab.len() != bb.len() {
        return false;
    }
    while let ([ah, at @ ..], [bh, bt @ ..]) = (ab, bb) {
        if *ah != *bh {
            return false;
        }
        ab = at;
        bb = bt;
    }
    true
}

const fn const_ascii_eq_ignore_case(a: &str, b: &str) -> bool {
    let (mut ab, mut bb) = (a.as_bytes(), b.as_bytes());
    if ab.len() != bb.len() {
        return false;
    }
    while let ([ah, at @ ..], [bh, bt @ ..]) = (ab, bb) {
        if !ah.eq_ignore_ascii_case(bh) {
            return false;
        }
        ab = at;
        bb = bt;
    }
    true
}

const fn const_anchor_matches_slug(anchor: &str, slug: &str) -> bool {
    const PREFIX: &[u8] = b"analysis-";
    let mut ab = anchor.as_bytes();
    let mut sb = slug.as_bytes();
    if ab.len() != PREFIX.len() + sb.len() {
        return false;
    }
    let mut pb = PREFIX;
    while let ([ah, at @ ..], [ph, pt @ ..]) = (ab, pb) {
        if *ah != *ph {
            return false;
        }
        ab = at;
        pb = pt;
    }
    while let ([ah, at @ ..], [sh, st @ ..]) = (ab, sb) {
        if *ah != *sh {
            return false;
        }
        ab = at;
        sb = st;
    }
    true
}

const fn const_slice_contains(haystack: &[&str], needle: &str) -> bool {
    let mut rest = haystack;
    while let [head, tail @ ..] = rest {
        if const_str_eq(head, needle) {
            return true;
        }
        rest = tail;
    }
    false
}

const fn verify_descriptor_entry(idx: usize, expected_id: AnalysisKindId, desc: &AnalysisKindDescriptor) {
    assert!(expected_id as usize == idx);
    assert!(desc.id as usize == idx);
    assert!(desc.id.descriptor().id as usize == idx);
    assert!(!desc.slug.is_empty());
    assert!(!desc.serde_variant.is_empty());
    assert!(!desc.title.is_empty());
    assert!(!desc.short_label.is_empty());
    assert!(!desc.description.is_empty());
    if desc.user_creatable {
        assert!(!desc.canonical_shorthand.is_empty());
        assert!(!desc.shorthands.is_empty());
        assert!(!desc.compatible_table_kinds.is_empty());
        assert!(const_slice_contains(desc.shorthands, desc.canonical_shorthand));
    } else {
        assert!(desc.canonical_shorthand.is_empty());
        assert!(desc.shorthands.is_empty());
        assert!(desc.ui_catalog_ids.is_empty());
    }
    assert!(const_anchor_matches_slug(desc.docs_anchor, desc.slug));
}

const fn verify_pairwise_uniqueness(a: &AnalysisKindDescriptor, b: &AnalysisKindDescriptor) {
    assert!(a.id as usize != b.id as usize);
    assert!(!const_str_eq(a.slug, b.slug));
    assert!(!const_str_eq(a.serde_variant, b.serde_variant));
    assert!(!const_str_eq(a.docs_anchor, b.docs_anchor));

    let mut sa_rest = a.shorthands;
    while let [sa, sa_tail @ ..] = sa_rest {
        assert!(!sa.is_empty());
        let mut sb_rest = b.shorthands;
        while let [sb, sb_tail @ ..] = sb_rest {
            assert!(!const_ascii_eq_ignore_case(sa, sb));
            sb_rest = sb_tail;
        }
        sa_rest = sa_tail;
    }

    let mut ca_rest = a.ui_catalog_ids;
    while let [ca, ca_tail @ ..] = ca_rest {
        assert!(!ca.is_empty());
        let mut cb_rest = b.ui_catalog_ids;
        while let [cb, cb_tail @ ..] = cb_rest {
            assert!(!const_str_eq(ca, cb));
            cb_rest = cb_tail;
        }
        ca_rest = ca_tail;
    }
}

// Compile-time bijection, completeness, and uniqueness invariant verification.
const _: () = {
    let mut i = 0;
    let mut all_ids = AnalysisKindId::ALL.as_slice();
    let mut outer = ALL_ANALYSIS_DESCRIPTORS.as_slice();
    while let ([id, rest_ids @ ..], [a, rest_a @ ..]) = (all_ids, outer) {
        verify_descriptor_entry(i, *id, a);
        let mut inner = rest_a;
        while let [b, rest_b @ ..] = inner {
            verify_pairwise_uniqueness(a, b);
            inner = rest_b;
        }
        all_ids = rest_ids;
        outer = rest_a;
        i += 1;
    }
    assert!(i == AnalysisKindId::COUNT);
    assert!(all_ids.is_empty());
};
