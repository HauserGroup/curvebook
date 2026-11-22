//! Parameterized `AnalysisSpec` schemas across transforms (`crates/transform`), biostatistics
//! (`crates/stats`), and linear/nonlinear regression (`crates/fit`), plus `PValueFormat` display
//! rules and `Change Data Analyzed` compatibility checks (`docs/implementation/data-tables-and-transforms.md` §5,
//! `docs/implementation/statistics-implementation.md` §1–§5, `docs/implementation/curve-fitting-implementation.md` §1–§4).

pub use curvebook_math::PValueFormat;

use crate::TableKind;
use crate::error::{ConfidenceLevel, DocError, validate_confidence_level, validate_title};
use crate::ids::{HookSource, SheetId};

pub mod descriptor;
mod finite;

pub use descriptor::*;

const fn default_col_b() -> usize {
    1
}

const fn default_true() -> bool {
    true
}

// ============================================================================
// Multiplicity & FDR Specifications (`docs/implementation/statistics-implementation.md` §4)
// ============================================================================

/// Familywise error-rate (FWER) or false-discovery-rate (FDR) procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MultiplicityMethod {
    /// No post-hoc multiple-comparisons adjustment (unadjusted pairwise comparisons or omnibus test only).
    #[default]
    None,
    /// Tukey's Honestly Significant Difference all-pairwise Studentized range test under pooled MSE (Tukey 1953; Kramer 1956 for unequal sample sizes).
    TukeyHsd,
    /// Dunnett's many-to-one test comparing every treatment group against a designated control column under pooled MSE (Dunnett 1955).
    Dunnett {
        /// Zero-based index of the reference control group column within the analyzed groups.
        control_column: usize,
    },
    /// Single-step Šídák FWER adjustment `p_adj = 1 - (1 - p)^m` assuming positive orthant dependence / independence (Šídák 1967).
    Sidak,
    /// Classical single-step Bonferroni FWER bound `p_adj = min(1, m * p)` valid under arbitrary dependence (Dunn 1961).
    Bonferroni,
    /// Sequential step-down Holm–Šídák FWER procedure (`1 - (1 - p_(i))^(m - i + 1)` with monotonicity enforcement; Holm 1979; Šídák 1967).
    HolmSidak,
    /// Dunnett's T3 all-pairwise Studentized maximum modulus test for unequal variances and small sample sizes using Welch–Satterthwaite *df* (Dunnett 1980).
    DunnettT3,
    /// Games–Howell all-pairwise Studentized range test for heteroscedastic groups with Welch–Satterthwaite *df* (Games & Howell 1976).
    GamesHowell,
    /// Tamhane's T2 conservative all-pairwise Šídák-*t* test for unequal variances using Welch–Satterthwaite *df* (Tamhane 1979).
    TamhaneT2,
    /// Dunn's nonparametric rank-sum post-hoc comparisons following Kruskal–Wallis or Friedman tests with Bonferroni adjustment (Dunn 1964).
    DunnRankSum,
    /// Orthogonal linear polynomial trend contrast across ordered columns (`1 df` linear slope test; Zar 2010).
    LinearTrend,
    /// Benjamini–Hochberg linear step-up False Discovery Rate procedure emitting *q*-values (Benjamini & Hochberg 1995).
    BenjaminiHochbergFdr,
    /// Benjamini–Yekutieli conservative harmonic-sum FDR procedure valid under arbitrary test dependence (Benjamini & Yekutieli 2001).
    BenjaminiYekutieliFdr,
    /// Two-stage adaptive linear step-up Benjamini–Krieger–Yekutieli FDR procedure estimating `pi_0` (Benjamini, Krieger, & Yekutieli 2006).
    TwoStageBkyFdr,
    /// Student–Newman–Keuls stepwise range test (Newman 1939; Keuls 1952; does not strongly control FWER for `> 3` groups and omits simultaneous CIs).
    NewmanKeuls,
    /// Unadjusted Fisher's Least Significant Difference pairwise *t* comparisons using the omnibus residual MSE and *df* (Fisher 1935).
    FisherLsd,
}

/// Multiple-comparisons configuration shared across ANOVA, nonparametrics, and survival curves.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MultiplicitySpec {
    /// Selected FWER, FDR, or unadjusted post-hoc comparison procedure.
    pub method: MultiplicityMethod,
    /// Familywise significance threshold `alpha` in `(0, 1)` (for example `0.05`) for FWER/unadjusted methods, or target false discovery rate `Q` in `(0, 1)` (for example `0.05` or `0.01`) for FDR methods (`BenjaminiHochbergFdr`, `BenjaminiYekutieliFdr`, `TwoStageBkyFdr`).
    pub threshold: f64,
    /// When `true`, computes simultaneous `(1 - threshold)` confidence intervals for pairwise mean differences where supported by `method`.
    pub report_confidence_intervals: bool,
}

impl Default for MultiplicitySpec {
    fn default() -> Self {
        Self { method: MultiplicityMethod::None, threshold: 0.05, report_confidence_intervals: true }
    }
}

// ============================================================================
// Transform Specifications (`docs/implementation/data-tables-and-transforms.md` §5)
// ============================================================================

/// Scalar mathematical transform preset.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MathTransformPreset {
    /// Natural logarithm `ln(u)` (maps `u <= 0` to empty).
    Ln,
    /// Base-10 logarithm `log10(u)` (maps `u <= 0` to empty).
    Log10,
    /// Base-2 logarithm `log2(u)` (maps `u <= 0` to empty).
    Log2,
    /// Base-10 antilogarithm `10^u`.
    Exp10,
    /// Natural exponential `exp(u)`.
    Exp,
    /// Multiplicative inverse `1 / u` (maps `u == 0` to empty).
    Reciprocal,
    /// Principal square root `sqrt(u)` (maps `u < 0` to empty).
    Sqrt,
    /// Quadratic square `u^2`.
    Square,
    /// Unit-proportion log-odds `ln(u / (1 - u))` for `u in (0, 1)`.
    Logit,
    /// Finney's +5 probit `5 + Phi^{-1}(u)` (inverse standard normal CDF plus 5) for unit proportions `u in (0, 1)`.
    Probit,
    /// Angular variance-stabilizing Freeman–Tukey / Bliss transform `arcsin(sqrt(u))` in radians for proportions `u in [0, 1]` (Zar 2010).
    ArcsinSqrt,
    /// Affine rescaling `slope * u + intercept`.
    LinearScale {
        /// Multiplicative scale factor.
        slope: f64,
        /// Additive offset applied after multiplying by `slope`.
        intercept: f64,
    },
    /// Convert molar concentration (scaled by `molar_unit_factor`, e.g. `1e-9` for nM) to `-log10[M]` (`pEC50` / `pIC50` scale).
    ConcentrationToNegativeLogM {
        /// Molar conversion multiplier `[M] = u * molar_unit_factor` (`> 0`, for example `1.0` for M, `1e-6` for µM, `1e-9` for nM).
        molar_unit_factor: f64,
    },
}

/// Error-propagation strategy when transforming `Mean + SD/SEM` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ErrorPropagationMode {
    /// Transform `[mean - spread, mean + spread]` endpoints directly through monotone `g` to produce asymmetric `[Lower, Upper]` limits.
    #[default]
    ReplicateEndpoints,
    /// First-order Taylor delta-method approximation `SD(g(Y)) ~= |g'(mean)| * SD(Y)`.
    FirstOrderDeltaMethod,
}

/// Anchor value for `[0%, 100%]` normalization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NormalizeAnchor {
    /// Minimum active value observed in the series (or across the whole table when `per_data_set == false`).
    ColumnMin,
    /// Maximum active value observed in the series (or across the whole table when `per_data_set == false`).
    ColumnMax,
    /// Sum of active values in the series (for fraction-of-total scaling when paired with `FixedValue(0.0)`).
    ColumnSum,
    /// Active value (or replicate mean) in the first non-empty row (`row == 0`).
    FirstRow,
    /// Active value (or replicate mean) in the last non-empty row.
    LastRow,
    /// Explicit user-entered constant anchor in data units.
    FixedValue(f64),
    /// Arithmetic mean of all active observations in a designated control column.
    ControlColumnMean {
        /// Zero-based data-set column index providing the reference mean.
        column: usize,
    },
    /// Dynamic scalar resolved via [`HookSource`] from an `InfoSheet` constant or upstream `ResultSheet` (for example fitted `Bottom` or `Top` plateau).
    Hooked(HookSource),
}

/// Baseline calculation method for `Remove Baseline and Column Math`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BaselineOperation {
    /// Additive subtraction `Y - Baseline`.
    Difference,
    /// Multiplicative fold ratio `Y / Baseline` (maps `Baseline == 0` to empty).
    Ratio,
    /// Percentage change `100 * (Y - Baseline) / Baseline`.
    PercentChange,
    /// Unitless fractional change `(Y - Baseline) / Baseline`.
    FractionalChange,
}

/// Monte Carlo scatter model for `Simulate Data`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SimulationNoise {
    /// Deterministic noise-free curve evaluation `Y = f(X)`.
    None,
    /// Additive homoscedastic normal error `Y = f(X) + Normal(0, sd^2)`.
    Gaussian {
        /// Standard deviation `sd >= 0` in response *Y* units.
        sd: f64,
    },
    /// Multiplicative proportional normal error `Y = f(X) * (1 + Normal(0, (cv_percent / 100)^2))`.
    RelativePercent {
        /// Coefficient of variation `%CV >= 0` expressed in percentage points (for example `10.0` for 10% CV).
        cv_percent: f64,
    },
    /// Discrete Poisson count draw `Y ~ Poisson(max(0, f(X)))`.
    Poisson,
}

/// Derived column definition evaluated by [`TransformAnalysisSpec::SelectAndTransform`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DerivedColumnFormula {
    /// Output column header name on the transformed table.
    #[serde(alias = "name")]
    pub title: String,
    /// Row-wise formula expression evaluated via `curvebook-transform::formula`.
    #[serde(alias = "expr")]
    pub formula: String,
}

impl DerivedColumnFormula {
    /// Construct a named derived column formula.
    #[must_use]
    pub fn new(title: impl Into<String>, formula: impl Into<String>) -> Self {
        Self { title: title.into(), formula: formula.into() }
    }
}

/// Derived data-table transformation (`crates/transform`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TransformAnalysisSpec {
    /// Element-wise nonlinear or affine mathematical transform of *X* and/or *Y* values.
    Math {
        /// Optional scalar transform applied to independent *X* coordinates; `None` leaves *X* unchanged.
        x_transform: Option<MathTransformPreset>,
        /// Optional scalar transform applied to response *Y* cells; `None` leaves *Y* unchanged.
        y_transform: Option<MathTransformPreset>,
        /// Strategy for transforming pre-summarized `Mean + SD/SEM` spread subcolumns.
        error_propagation: ErrorPropagationMode,
    },
    /// Linear rescaling mapping `[zero_anchor, hundred_anchor]` onto `[0, 100]` (when `output_percent == true`) or `[0, 1]` (unit fraction).
    Normalize {
        /// Value mapped to `0.0` (`0%`).
        zero_anchor: NormalizeAnchor,
        /// Value mapped to `100.0` (`100%`) or `1.0` (unit fraction).
        hundred_anchor: NormalizeAnchor,
        /// When `true`, multiplies normalized fractions by `100.0` (percentage scale); when `false`, outputs unit fractions `[0, 1]`.
        output_percent: bool,
        /// When `true`, computes anchors (`ColumnMin`, `ColumnMax`, etc.) independently within each data-set column; when `false`, pools across all selected columns.
        per_data_set: bool,
    },
    /// Row-matched baseline subtraction, ratio, or percent/fractional change relative to one or more baseline columns.
    RemoveBaseline {
        /// Zero-based indices of designated baseline columns (averaged row-by-row when multiple baseline columns are supplied).
        baseline_columns: Vec<usize>,
        /// Algebraic baseline combination rule (`Difference`, `Ratio`, `PercentChange`, or `FractionalChange`).
        operation: BaselineOperation,
    },
    /// Matrix transposition swapping rows and data-set columns.
    Transpose {
        /// When `true`, exchanges upstream `row_titles` with data-set column headers on the transposed output sheet.
        swap_row_and_column_titles: bool,
    },
    /// Row decimation or non-overlapping block averaging across every `k` consecutive rows.
    PruneRows {
        /// Positive block stride `k >= 1`.
        every_k_rows: usize,
        /// When `true`, averages active *X* and *Y* observations within each `k`-row window; when `false`, samples every `k`-th row directly.
        average_window: bool,
    },
    /// Row-predicate filtering, archetype conversion, and derived-column projection.
    SelectAndTransform {
        /// Optional boolean row-filter expression (for example `"[Age] >= 18 && [Dose] > 0"`); `None` retains all rows.
        filter_formula: Option<String>,
        /// Output [`TableKind`] archetype produced by the projection.
        target_archetype: TableKind,
        /// Additional calculated columns appended during projection.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        derived_columns: Vec<DerivedColumnFormula>,
    },
    /// Deterministic Monte Carlo XY curve simulation using `curvebook-math::DeterministicRng`.
    Simulate {
        /// 64-bit SplitMix64/Xoshiro256** PRNG seed ensuring bit-reproducible simulated replicates across platforms.
        seed: u64,
        /// Number of *X* grid rows (`>= 1`) spaced linearly across `[x_min, x_max]`.
        row_count: usize,
        /// Number of raw replicate *Y* subcolumns simulated per row (`1..=MAX_SUBCOLUMNS`).
        replicates: usize,
        /// Inclusive lower *X* coordinate in data units.
        x_min: f64,
        /// Inclusive upper *X* coordinate in data units.
        x_max: f64,
        /// Built-in curve equation ID (for example `"log_agonist_vs_response_4pl"`) or custom formula expression.
        equation_id: String,
        /// Named `(parameter_name, true_value)` bindings evaluated across `[x_min, x_max]`.
        parameters: Vec<(String, f64)>,
        /// Random error distribution superimposed on the true curve `f(X)`.
        noise: SimulationNoise,
    },
    /// Deterministic simple or stratified without-replacement random row sub-sampling.
    RandomSample {
        /// 64-bit deterministic PRNG seed.
        seed: u64,
        /// Target total number of rows to draw without replacement (`>= 1`).
        sample_size: usize,
        /// Optional zero-based categorical column index used for stratified sampling; `None` performs simple random sampling across all rows.
        #[serde(default)]
        stratify_column: Option<usize>,
        /// When `true` and `stratify_column` is `Some(_)`, allocates stratum draws proportionally to stratum sizes (largest-remainder Hamilton rounding); when `false`, splits draws equally across strata.
        #[serde(default = "default_true")]
        proportional_allocation: bool,
    },
}

// ============================================================================
// Biostatistics Specifications (`docs/implementation/statistics-implementation.md` §2–§5)
// ============================================================================

/// Replicate-subcolumn aggregation rule for Descriptive Statistics (`docs/implementation/statistics-implementation.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SubcolumnAggregation {
    /// Average active replicates within each row first, then compute univariate descriptive moments across row means.
    #[default]
    RowMeans,
    /// Report separate descriptive summaries for each individual subcolumn (`Y1`, `Y2`, ...).
    PerSubcolumn,
    /// Pool all non-excluded cells across every row and subcolumn of a data set into one grand sample.
    PooledGrandSample,
}

/// Column descriptive statistics, normality/lognormality battery, and optional one-sample location test specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DescriptiveSpec {
    /// How multi-subcolumn replicates are collapsed prior to computing moments and quantiles.
    pub aggregation: SubcolumnAggregation,
    /// Optional upper percentile `p_hi` in `[50.0, 100.0)` (percentage scale, e.g. `Some(90.0)` for the 10th and 90th Hyndman–Fan Type-7 percentiles); `None` reports standard quartiles/extrema only.
    pub custom_upper_percentile: Option<f64>,
    /// When `true`, runs Shapiro–Wilk (`3 <= n <= 5000`; Royston 1995), D'Agostino–Pearson omnibus `K^2` (`n >= 8`), Anderson–Darling `A^2*`, and Lilliefors KS normality & lognormality tests.
    pub run_normality_battery: bool,
    /// Optional null location `mu_0` in data units triggering one-sample Student *t* and Wilcoxon signed-rank tests alongside descriptive moments; `None` omits one-sample hypothesis tests.
    pub hypothetical_value: Option<f64>,
}

impl Default for DescriptiveSpec {
    fn default() -> Self {
        Self { aggregation: SubcolumnAggregation::RowMeans, custom_upper_percentile: None, run_normality_battery: true, hypothetical_value: None }
    }
}

/// Univariate column outlier detection specification (`Grubbs` ESD vs `ROUT` FDR).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum OutlierSpec {
    /// Two-sided Grubbs extreme studentized deviate (ESD) test under normality (Grubbs 1969).
    Grubbs {
        /// Two-sided significance level `alpha` in `(0, 1)` (unit probability, for example `0.05` or `0.01`).
        alpha: f64,
        /// When `true`, applies sequential ESD removal one extreme observation at a time until no candidate crosses `alpha`; when `false`, tests at most one outlier per column.
        iterative: bool,
    },
    /// Robust regression and Outlier removal adapted to univariate location (`68.27th` percentile RSDR + Benjamini–Hochberg step-up over the furthest 30% of observations; Motulsky & Brown 2006).
    Rout {
        /// Maximum desired false discovery rate `Q` expressed in **percentage points** in `(0.0, 100.0]` (for example `1.0` for `Q = 1%`).
        fdr_q_percent: f64,
    },
}

/// Parametric or nonparametric two-group / one-sample comparison method (`crates/stats::two_group`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TwoGroupMethod {
    /// Unpaired two-sample Student *t*-test assuming equal population variances (`df = n_a + n_b - 2`; Student 1908; Zar 2010).
    UnpairedT,
    /// Unpaired two-sample Welch *t*-test allowing unequal variances with Welch–Satterthwaite fractional *df* (Welch 1947; Satterthwaite 1946).
    WelchT,
    /// Paired-differences Student *t*-test across complete matched rows (`d_r = y_{r,b} - y_{r,a}` or `a - b`, `df = n_pairs - 1`).
    PairedT,
    /// Ratio paired *t*-test on `ln(A / B)` across strictly positive matched pairs, reporting the geometric mean of ratios and back-transformed CI.
    RatioPairedT,
    /// Unpaired *t*-test on natural-log-transformed strictly positive observations (`ln(Y)`), reporting the geometric mean ratio `exp(mean_ln_a - mean_ln_b)` and back-transformed CI.
    LognormalUnpairedT,
    /// Unpaired nonparametric Mann–Whitney *U* / Wilcoxon rank-sum test with exact permutation *p*-values for small samples, tie-corrected normal approximation otherwise, and Hodges–Lehmann shift CI (Mann & Whitney 1947; Hodges & Lehmann 1963).
    MannWhitneyU,
    /// Paired nonparametric Wilcoxon signed-rank test with Pratt zero handling, exact small-sample *p*-values, and Walsh-average pseudo-median CI (Wilcoxon 1945; Pratt 1959).
    WilcoxonSignedRank,
    /// Two-sample Kolmogorov–Smirnov supremum EDF distance test `D = sup_x |F_a(x) - F_b(x)|` with exact two-sample lattice *p*-values (Smirnov 1948; Hodges 1958).
    KolmogorovSmirnov,
    /// One-sample Student *t*-test comparing each selected column mean against `hypothetical` (`df = n - 1`).
    OneSampleT {
        /// Null population mean `mu_0` in data units.
        hypothetical: f64,
    },
    /// One-sample Wilcoxon signed-rank test comparing each selected column median against `hypothetical`.
    OneSampleWilcoxon {
        /// Null population median `theta_0` in data units.
        hypothetical: f64,
    },
    /// Exact binomial sign test on the signs of non-zero paired differences `(A_r - B_r)` (Dixon & Mood 1946).
    PairedSignTest,
}

/// Two-group (or one-sample) hypothesis test specification (`Column` or `MultipleVariables` sheets).
///
/// Confidence intervals for mean differences, Hodges–Lehmann shifts, geometric mean ratios, and
/// noncentral-*t* standardized effect sizes (`d`, `g`, Glass's `Delta`) are evaluated at the parent
/// [`AnalysisSpec::confidence_level`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TwoGroupSpec {
    /// Parametric or nonparametric test kernel.
    pub method: TwoGroupMethod,
    /// `true` for a two-sided *p*-value; `false` for a directional one-sided *p*-value in the direction of the observed contrast.
    pub two_tailed: bool,
    /// When `true`, computes standardized effect sizes (Cohen's `d`, Hedges' bias-corrected `g`, Glass's `Delta` with Steiger 2004 noncentral-*t* CIs on pooled `df = n_a + n_b - 2`, or rank-biserial / Cliff's `delta` for rank tests, plus `r^2`).
    pub compute_effect_sizes: bool,
    /// Zero-based two-level categorical grouping column index when running on a [`TableKind::MultipleVariables`] sheet; `None` on `Column` tables (or auto-selects the first binary categorical variable on `MultipleVariables`).
    #[serde(default)]
    pub mv_group_column: Option<usize>,
    /// Zero-based continuous outcome column index when running on a [`TableKind::MultipleVariables`] sheet; `None` on `Column` tables (or auto-selects the first continuous variable on `MultipleVariables`).
    #[serde(default)]
    pub mv_response_column: Option<usize>,
}

impl Default for TwoGroupSpec {
    fn default() -> Self {
        Self { method: TwoGroupMethod::UnpairedT, two_tailed: true, compute_effect_sizes: true, mv_group_column: None, mv_response_column: None }
    }
}

/// Bland-Altman method-comparison specification (`column_a` vs `column_b`; Bland & Altman 1986, 1999).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlandAltmanSpec {
    /// Zero-based index of measurement method *A*.
    #[serde(default)]
    pub column_a: usize,
    /// Zero-based index of reference/comparison measurement method *B*.
    #[serde(default = "default_col_b")]
    pub column_b: usize,
    /// When `false`, evaluates absolute differences `A - B` vs pair averages `(A + B) / 2`; when `true`, evaluates percentage differences `100 * (A - B) / ((A + B) / 2)`. Limits of agreement are `bias ± z_{1 - alpha/2} * SD_diff` at [`AnalysisSpec::confidence_level`].
    #[serde(default)]
    pub percent_difference: bool,
    /// Explicit comparison mode on [`TableKind::Xy`] tables: `Some(true)` locks comparison to `X` versus `Y[0]` regardless of whether additional `Y` data sets are added later, `Some(false)` compares `Y[column_a]` versus `Y[column_b]`, and `None` preserves legacy auto-selection (`X` vs `Y[0]` when one `Y` data set exists, `Y[column_a]` vs `Y[column_b]` otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xy_compare_x_vs_y: Option<bool>,
}

impl Default for BlandAltmanSpec {
    fn default() -> Self {
        Self { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: None }
    }
}

/// Row-wise multiple two-group t-test or nonparametric comparison specification.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MultipleRowTestsSpec {
    /// When `true`, pairs replicates by subcolumn index (`PairedT` or `WilcoxonSignedRank`); when `false`, treats columns as independent samples (`UnpairedT`, `WelchT`, or `MannWhitneyU`).
    #[serde(default)]
    pub paired: bool,
    /// When `true`, runs nonparametric rank tests (`MannWhitneyU` or `WilcoxonSignedRank`) per row instead of parametric *t*-tests.
    #[serde(default)]
    pub nonparametric: bool,
    /// When `true` (and `!paired && !nonparametric`), pools within-row variance across the two groups (`Student` *t*); when `false`, uses row-wise Welch–Satterthwaite *df*.
    #[serde(default = "default_true")]
    pub assume_equal_sd: bool,
    /// Multiplicity or FDR procedure applied across the family of tested rows (commonly `BenjaminiHochbergFdr`, `TwoStageBkyFdr`, or `HolmSidak` for Volcano plots).
    #[serde(default)]
    pub multiplicity: MultiplicitySpec,
}

impl Default for MultipleRowTestsSpec {
    fn default() -> Self {
        Self { paired: false, nonparametric: false, assume_equal_sd: true, multiplicity: MultiplicitySpec::default() }
    }
}

/// Passing-Bablok nonparametric method-comparison regression specification (`x_column` vs `y_column`; Passing & Bablok 1983).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PassingBablokSpec {
    /// Zero-based column index of reference method *X*.
    #[serde(default)]
    pub x_column: usize,
    /// Zero-based column index of candidate method *Y*.
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
    /// Single-step Bonferroni FWER bound `min(1, m * p)` (Dunn 1961).
    Bonferroni,
    /// Step-down Holm–Bonferroni FWER procedure `(m - i + 1) * p_(i)` (Holm 1979).
    HolmBonferroni,
    /// Single-step Šídák FWER bound `1 - (1 - p)^m` (Šídák 1967).
    Sidak,
    /// Step-down Holm–Šídák FWER procedure `1 - (1 - p_(i))^(m - i + 1)` (Holm 1979; Šídák 1967).
    HolmSidak,
    /// Benjamini–Hochberg linear step-up FDR *q*-values (Benjamini & Hochberg 1995).
    #[default]
    BenjaminiHochberg,
    /// Benjamini–Krieger–Yekutieli two-stage adaptive FDR *q*-values (Benjamini, Krieger, & Yekutieli 2006).
    TwoStageBky,
    /// Benjamini–Yekutieli harmonic-factor FDR *q*-values valid under arbitrary dependence (Benjamini & Yekutieli 2001).
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

/// Omnibus one-way comparison method across `k >= 2` groups (`crates/stats::anova`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum OneWayMethod {
    /// Classical Fisher homoscedastic one-way ANOVA *F*-test (`df_between = k - 1`, `df_within = N - k`; Fisher 1925; Zar 2010).
    #[default]
    OrdinaryAnova,
    /// Heteroscedastic Brown–Forsythe `F*` and Welch `W` one-way ANOVA for groups with unequal variances (Brown & Forsythe 1974; Welch 1951).
    BrownForsytheAndWelch,
    /// One-way repeated-measures ANOVA across matched subject rows (`ColumnDesignHint::PairedRepeatedMeasures`).
    RepeatedMeasures {
        /// When `true`, multiplies omnibus `(df_treatment, df_residual)` by the Greenhouse–Geisser sphericity estimate `epsilon_hat` (Greenhouse & Geisser 1959; Huynh–Feldt `epsilon_tilde` is also reported in diagnostics); when `false`, assumes compound symmetry (`epsilon = 1`).
        geisser_greenhouse: bool,
    },
    /// Nonparametric Kruskal–Wallis one-way analysis of variance by ranks with tie correction (Kruskal & Wallis 1952).
    KruskalWallis,
    /// Nonparametric Friedman rank test across complete matched subject blocks (Friedman 1937).
    Friedman,
}

/// Denominator degrees-of-freedom approximation for REML mixed-effects ANOVA and nested models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MixedModelDfMethod {
    /// Small-sample adjusted `Phi_A` covariance and scaled `F*` statistic (Kenward & Roger, 1997).
    #[default]
    KenwardRoger,
    /// Satterthwaite moment-matched denominator *df* using unadjusted Wald covariance `Phi` (Fai & Cornelius, 1996).
    Satterthwaite,
}

/// One-way ANOVA, heteroscedastic ANOVA, repeated-measures/REML mixed model, or nonparametric `k`-group comparison specification.
///
/// Effect-size confidence intervals (`eta^2`, `omega^2` via Steiger 2004 noncentral-*F* inversion at
/// [`AnalysisSpec::confidence_level`]) and post-hoc CIs follow `compute_effect_sizes` and `multiplicity`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct OneWayAnovaSpec {
    /// Omnibus parametric or nonparametric one-way model family.
    pub method: OneWayMethod,
    /// Fit linear mixed-effects REML instead of classical sphericity RM ANOVA even on complete tables (also engaged automatically when repeated-measures rows contain missing values).
    #[serde(default)]
    pub mixed_effects_reml: bool,
    /// Denominator degrees-of-freedom method (`KenwardRoger` or `Satterthwaite`) used when REML mixed-effects estimation runs.
    #[serde(default)]
    pub df_method: MixedModelDfMethod,
    /// Within-subject covariance structure (`CompoundSymmetry` vs `Unstructured`) fitted when REML mixed-effects estimation runs.
    #[serde(default)]
    pub residual_covariance: ResidualCovariance,
    /// When `true`, computes omnibus effect sizes (`eta^2`, `omega^2`, Cohen's `f`, with exact noncentral-*F* CIs for parametric ANOVA; `epsilon^2` / Kendall's `W` for rank tests).
    pub compute_effect_sizes: bool,
    /// Post-hoc pairwise, vs-control, or trend multiple-comparisons configuration.
    pub multiplicity: MultiplicitySpec,
}

/// Sequential vs marginal sum-of-squares decomposition for unbalanced factorial linear models (Maxwell, Delaney, & Kelley 2017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SumOfSquaresType {
    /// Sequential (hierarchical) Type I sums of squares conditioned on preceding terms in model order.
    TypeI,
    /// Marginal (orthogonal contrast) Type III sums of squares testing each main effect and interaction adjusted for all other terms.
    #[default]
    TypeIII,
}

/// Within-subject matching topology for two-way `GroupedTable` repeated-measures ANOVA and REML mixed models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RepeatedMeasuresDesign {
    /// Completely randomized between-subjects factorial design (all cells hold independent observations).
    #[default]
    None,
    /// Subjects correspond to subcolumns within each column group and are measured repeatedly across row-factor levels.
    RowFactorMatched,
    /// Subjects correspond to `(row, subcol)` slots and are measured repeatedly across column-factor data sets.
    ColumnFactorMatched,
    /// Doubly repeated-measures design where each subcolumn `s` is the same subject measured across every `(row, column)` combination.
    BothFactorsMatched,
}

/// Residual covariance structure fitted by REML for repeated-measures mixed models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ResidualCovariance {
    /// Random-intercept / compound-symmetry covariance with equal variance and correlation across timepoints.
    #[default]
    CompoundSymmetry,
    /// Unstructured positive-definite `t x t` covariance matrix (`t(t + 1) / 2` parameters via Cholesky REML).
    Unstructured,
}

/// Two-way, three-way (`2 × 2 × K`), or general `N`-way multifactor ANOVA / REML mixed-model specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactorialAnovaSpec {
    /// When `true` on a `GroupedTable` with [`crate::ThreeWayColumnGrouping`], fits a `Row × Factor1 × Factor2` three-way ANOVA; when `false`, fits a two-way `Row × Column` ANOVA.
    pub three_way: bool,
    /// Zero-based categorical factor column indices when invoked as an `N`-way multifactor GLM on a [`TableKind::MultipleVariables`] sheet (`empty` on `GroupedTable` sources).
    pub multifactor_columns: Vec<usize>,
    /// Zero-based continuous outcome column index on a [`TableKind::MultipleVariables`] sheet; `None` on `GroupedTable` sources.
    pub response_column: Option<usize>,
    /// Highest factorial interaction order included in the model (`1` = main effects only, `2` = two-way interactions, `3` = up to three-way interactions).
    pub max_interaction_order: usize,
    /// Type I (sequential) vs Type III (marginal) sum-of-squares decomposition.
    pub ss_type: SumOfSquaresType,
    /// Within-subject matching structure on `GroupedTable` repeated-measures designs.
    pub repeated_measures: RepeatedMeasuresDesign,
    /// Fit linear mixed-effects REML instead of classical sphericity RM ANOVA even on complete tables.
    #[serde(default)]
    pub mixed_effects_reml: bool,
    /// Denominator degrees-of-freedom method (`KenwardRoger` or `Satterthwaite`) used for REML mixed-effects *F*-tests.
    #[serde(default)]
    pub df_method: MixedModelDfMethod,
    /// Within-subject residual covariance structure (`CompoundSymmetry` or `Unstructured`) for REML mixed models.
    #[serde(default)]
    pub residual_covariance: ResidualCovariance,
    /// When `true` (default), automatically upgrades repeated-measures designs with missing replicate cells (`has_missing_replicates()`) to REML mixed-effects estimation rather than dropping incomplete subjects.
    pub fit_reml_on_missing: bool,
    /// When `true`, reports partial `eta_p^2` and (for classical OLS/RM factorial ANOVA) total `eta^2` per term (`eta^2` is omitted on REML mixed-effects fits).
    pub compute_effect_sizes: bool,
    /// Post-hoc cell/simple-effect/main-effect multiple-comparisons configuration.
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
            mixed_effects_reml: false,
            df_method: MixedModelDfMethod::KenwardRoger,
            residual_covariance: ResidualCovariance::CompoundSymmetry,
            fit_reml_on_missing: true,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec::default(),
        }
    }
}

/// Hierarchical two-level nested *t*-test or nested one-way REML ANOVA specification for [`TableKind::Nested`] tables.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct NestedAnalysisSpec {
    /// When `true`, requires exactly `2` selected treatment groups and reports a two-sided nested *t*-test (`t = sqrt(F)` with sign `mean_A - mean_B`) and mean-difference CI; when `false`, runs `k`-group nested one-way ANOVA.
    pub two_group_t_test: bool,
    /// Denominator degrees-of-freedom approximation (`KenwardRoger` or `Satterthwaite`) accounting for biological-unit variance components under unequal sub-unit sizes.
    #[serde(default)]
    pub df_method: MixedModelDfMethod,
    /// Post-hoc group-level multiple-comparisons configuration when `k >= 3` groups are analyzed.
    pub multiplicity: MultiplicitySpec,
}

/// Weighting scheme for Cohen's kappa (`Unweighted`, `Linear`, or `Quadratic`; Cohen 1960, 1968; Fleiss, Cohen, & Everitt 1969).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum KappaWeighting {
    /// Nominal identity weights (`w_{ij} = 1` iff `i == j`, else `0`; Cohen 1960).
    #[default]
    Unweighted,
    /// Cicchetti–Allison linear ordinal disagreement weights `w_{ij} = 1 - |i - j| / (k - 1)` (Cohen 1968).
    Linear,
    /// Fleiss–Cohen quadratic ordinal disagreement weights `w_{ij} = 1 - ((i - j) / (k - 1))^2` (Fleiss, Cohen, & Everitt 1969).
    Quadratic,
}

/// Categorical association, exact table, paired discordance, or inter-rater agreement test (`crates/stats::categorical`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ContingencyMethod {
    /// Pearson's `X^2 = sum (O - E)^2 / E` test of independence (`R × C`) or goodness-of-fit (`PartsOfWhole`).
    PearsonChiSquare {
        /// When `true` on `2 × 2` tables, applies Yates's `0.5` continuity correction `(|O - E| - 0.5)_+^2 / E` (Yates 1934).
        yates_continuity: bool,
    },
    /// Wilks log-likelihood ratio deviance test `G^2 = 2 * sum O * ln(O / E)` on `(r - 1)(c - 1)` *df*.
    LikelihoodRatioG2,
    /// Fisher's exact conditional hypergeometric test (two-sided minimum-likelihood sum on `2 × 2` and Freeman–Halton network generalization on `R × C`; Fisher 1935; Freeman & Halton 1951).
    FisherExact,
    /// Barnard's unconditional exact CSM/Boschloo-style supremum test over nuisance proportion `pi in (0, 1)` on `2 × 2` tables (Barnard 1945).
    BarnardExact,
    /// Cochran–Armitage `1 df` linear trend test in proportions across ordered columns of a `2 × C` or `R × 2` table (Cochran 1954; Armitage 1955).
    CochranArmitageTrend,
    /// McNemar's paired discordance test on matched `2 × 2` tables with exact binomial and mid-*p* values plus discordant odds ratio `(b / c)` (McNemar 1947).
    McNemarPaired,
    /// Two-rater Cohen's kappa inter-rater agreement on square `k × k` tables with asymptotic Fleiss–Cohen–Everitt standard error (Cohen 1960, 1968).
    CohenKappa {
        /// Nominal (`Unweighted`) vs ordinal (`Linear` or `Quadratic`) disagreement weights.
        #[serde(default)]
        weighting: KappaWeighting,
    },
    /// Multi-rater Fleiss' kappa across `N` subjects (rows) and `k` categories (columns) with constant or row-sum rater counts (Fleiss 1971).
    FleissKappa,
    /// Exact two-sided Clopper–Pearson / Sterne one-sample binomial test against [`ContingencySpec::null_proportion`] (Clopper & Pearson 1934).
    ExactBinomial,
}

impl Default for ContingencyMethod {
    fn default() -> Self {
        Self::PearsonChiSquare { yates_continuity: false }
    }
}

/// Categorical contingency, goodness-of-fit, diagnostic accuracy, or kappa specification.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ContingencySpec {
    /// Primary contingency, exact, trend, paired, or agreement test method.
    pub method: ContingencyMethod,
    /// When `true` on `2 × 2` tables, reports Odds Ratio (Baptista–Pike exact conditional mid-*p* / Fisher CI; Baptista & Pike 1977), Relative Risk (Koopman asymptotic score CI; Koopman 1984), Difference in Proportions (Newcombe Wilson score CI; Newcombe 1998), and Number Needed to Treat (`NNT = 1 / |RD|`).
    pub compute_odds_and_risk_ratios: bool,
    /// When `true` on `2 × 2` tables, reports diagnostic Sensitivity, Specificity, and Positive/Negative Predictive Values (`PPV`/`NPV`) with Wilson score CIs (Wilson 1927).
    pub compute_sensitivity_specificity: bool,
    /// When `true`, reports Pearson's `phi` (`2 × 2`) and Cramér's `V = sqrt(X^2 / (N * min(r - 1, c - 1)))` association coefficients (Cramér 1946).
    pub compute_phi_and_cramers_v: bool,
    /// Optional non-negative expected category weights for one-way goodness-of-fit on [`TableKind::PartsOfWhole`] tables (normalized internally to sum to `1.0`); `None` tests discrete uniform `1 / k` proportions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_proportions: Option<Vec<f64>>,
    /// Hypothesized success probability `p_0 in (0, 1)` for [`ContingencyMethod::ExactBinomial`]; `None` defaults to `0.5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null_proportion: Option<f64>,
}

/// Partial-likelihood tied-event approximation for Cox proportional hazards regression (`crates/stats::survival`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CoxTieMethod {
    /// Breslow (1974) tied-event partial likelihood approximation (fastest; slightly conservative when tie fractions are heavy).
    Breslow,
    /// Efron (1977) tied-event partial likelihood approximation (default; closely matches exact discrete partial likelihood under moderate ties).
    #[default]
    Efron,
    /// Exact discrete partial likelihood summing over all `C(|R_i|, d_i)` subsets of tied failures (Cox 1972; Kalbfleisch & Prentice 2002).
    Exact,
}

/// Time-to-event Kaplan–Meier or multivariable Cox proportional hazards specification (`crates/stats::survival`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SurvivalAnalysisSpec {
    /// Nonparametric product-limit Kaplan–Meier survival estimator with cohort comparison tests (Kaplan & Meier 1958).
    KaplanMeier {
        /// When `true`, computes the Mantel–Cox log-rank chi-square test weighting all follow-up times equally (Mantel 1966).
        log_rank_mantel_cox: bool,
        /// When `true`, computes the Gehan–Breslow–Wilcoxon test weighting early event times by the number at risk (Gehan 1965; Breslow 1970).
        gehan_breslow_wilcoxon: bool,
        /// When `true` and `>= 3` cohorts are analyzed, computes the `1 df` log-rank test for trend across ordered columns (Tarone 1975).
        trend_test: bool,
        /// When `true`, computes asymmetrical complementary log-log `ln(-ln S(t))` Greenwood confidence bounds guaranteed to stay within `[0, 1]` (Kalbfleisch & Prentice 2002); when `false`, uses symmetric linear Greenwood bounds clamped to `[0, 1]`.
        asymmetrical_greenwood_ci: bool,
        /// Pairwise log-rank multiple-comparisons configuration across `>= 3` cohorts.
        multiplicity: MultiplicitySpec,
        /// Optional zero-based elapsed-time column index when running on a [`TableKind::MultipleVariables`] sheet; `None` on `SurvivalTable`.
        #[serde(default)]
        mv_time_column: Option<usize>,
        /// Optional zero-based event/censoring indicator column index when running on a [`TableKind::MultipleVariables`] sheet; `None` on `SurvivalTable`.
        #[serde(default)]
        mv_status_column: Option<usize>,
        /// Optional zero-based categorical cohort/arm column index when running on a [`TableKind::MultipleVariables`] sheet; `None` analyzes a single pooled cohort or uses `SurvivalTable::cohorts`.
        #[serde(default)]
        mv_group_column: Option<usize>,
    },
    /// Semi-parametric multivariable Cox proportional hazards model `h(t | X) = h_0(t) exp(X beta)` on a [`TableKind::MultipleVariables`] sheet (Cox 1972).
    CoxProportionalHazards {
        /// Zero-based variable column index holding non-negative follow-up times `t >= 0`.
        time_column: usize,
        /// Zero-based variable column index holding binary event indicators (`1` = event, `0` = censored).
        status_column: usize,
        /// Ordered zero-based continuous or categorical covariate column indices included in `X`.
        predictor_columns: Vec<usize>,
        /// Optional `(categorical_col_idx, reference_level_label)` overrides fixing the baseline hazard reference category (`HR = 1.0`).
        reference_levels: Vec<(usize, String)>,
        /// Tied-failure partial-likelihood handling (`Breslow`, `Efron`, or `Exact`).
        tie_method: CoxTieMethod,
    },
}

/// Standard-error and confidence-interval method for Area Under the ROC Curve (`crates/stats::roc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RocCiMethod {
    /// Exponential-distribution variance approximation of Hanley & McNeil (1982).
    HanleyMcNeil,
    /// Nonparametric structural-components U-statistic variance of DeLong, DeLong, & Clarke-Pearson (1988).
    #[default]
    DeLong,
}

/// Receiver Operating Characteristic (ROC) curve analysis specification (`crates/stats::roc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct RocSpec {
    /// Variance estimator (`DeLong` or `HanleyMcNeil`) used for the AUC standard error, *z*-test against `0.5`, and confidence interval at [`AnalysisSpec::confidence_level`].
    pub ci_method: RocCiMethod,
}

/// Exponential-dispersion family and canonical link for Generalized Linear Models (`crates/stats::regression`; McCullagh & Nelder 1989).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum GlmFamily {
    /// Gaussian error with identity link `E[Y | X] = X beta` (ordinary multivariable linear regression via QR/SVD).
    #[default]
    Linear,
    /// Binomial Bernoulli error (`Y in {0, 1}`) with canonical logit link `ln(p / (1 - p)) = X beta` fitted by IRLS, reporting odds ratios `exp(beta_j)` and Hosmer–Lemeshow / Tjur `R^2` diagnostics.
    Logistic,
    /// Poisson count error (`Y in {0, 1, 2, ...}`) with canonical log link `ln(E[Y | X]) = X beta` fitted by IRLS, reporting rate ratios `exp(beta_j)` and Pearson/deviance dispersion diagnostics.
    Poisson,
}

/// Multivariable Generalized Linear Model specification (`MultipleVariables`, `Xy`, or `Column` sheets).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GlmSpec {
    /// Error distribution and link function (`Linear`, `Logistic`, or `Poisson`).
    pub family: GlmFamily,
    /// Zero-based variable column index of the dependent outcome variable *Y*.
    pub response_column: usize,
    /// Ordered zero-based predictor column indices (continuous covariates and categorical factors).
    pub predictor_columns: Vec<usize>,
    /// Optional `(categorical_col_idx, reference_level_label)` overrides selecting the reference dummy-coding level; columns absent here use the first level of [`crate::VariableColumn::ordered_levels`].
    pub reference_levels: Vec<(usize, String)>,
    /// When `true`, computes Variance Inflation Factors (`VIF_j = 1 / (1 - R_j^2)`) across predictors to diagnose multicollinearity.
    pub compute_vif: bool,
}

/// Pairwise distance metric for hierarchical and multivariate clustering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterDistance {
    /// L2 Euclidean distance `sqrt(sum_k (x_{ik} - x_{jk})^2)`.
    #[default]
    Euclidean,
    /// L1 city-block Manhattan distance `sum_k |x_{ik} - x_{jk}|`.
    Manhattan,
    /// Correlation dissimilarity `1 - r_{ij}` based on Pearson's correlation across features.
    Pearson,
    /// Angular cosine dissimilarity `1 - (x_i . x_j) / (||x_i||_2 ||x_j||_2)`.
    Cosine,
}

/// Agglomerative linkage criterion for hierarchical clustering (Lance & Williams 1967).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterLinkage {
    /// Ward's minimum within-cluster variance increase (`Delta ESS`; Ward 1963).
    #[default]
    Ward,
    /// Farthest-neighbor maximum pairwise inter-cluster distance.
    Complete,
    /// Nearest-neighbor minimum pairwise inter-cluster distance.
    Single,
    /// Unweighted Pair Group Method with Arithmetic Mean (`|A| * |B|`-weighted average pairwise distance; Sokal & Michener 1958).
    Upgma,
    /// Weighted Pair Group Method with Arithmetic Mean (equal `0.5` weight to each merged branch).
    Wpgma,
    /// Squared/Euclidean distance between cluster centroids (UPGMC).
    Centroid,
    /// Gower's unweighted midpoint centroid linkage (WPGMC; Gower 1967).
    Median,
}

/// Exploratory multivariate analysis or Rubin multiple-imputation specification (`crates/stats::multivariate`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MultivariateSpec {
    /// Principal Component Analysis via SVD of the centered (and optionally standardized) data matrix (Jolliffe 2002),
    /// or supervised Principal Component Regression (PCR) when `pcr_outcome_column` is specified.
    Pca {
        /// When `true`, centers and scales each continuous variable to unit variance (correlation-matrix PCA); when `false`, centers only (covariance-matrix PCA).
        standardize: bool,
        /// Optional deterministic PRNG seed enabling Horn (1965) Monte Carlo parallel analysis eigenvalue retention thresholds; `None` uses Kaiser (`lambda > 1`) / cumulative-variance criteria only.
        parallel_analysis_seed: Option<u64>,
        /// Optional zero-based continuous outcome variable column index enabling supervised Principal Component Regression (PCR); `None` runs unsupervised PCA.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pcr_outcome_column: Option<usize>,
    },
    /// Pairwise correlation matrix across continuous columns with two-sided *p*-values and Fisher-*z* confidence intervals.
    CorrelationMatrix {
        /// When `true`, computes Spearman rank-order correlations `r_s` (Spearman 1904); when `false`, computes Pearson product-moment correlations `r`.
        spearman_rank: bool,
    },
    /// Agglomerative hierarchical clustering and optional *k*-means++ partitioning with silhouette diagnostics (Arthur & Vassilvitskii 2007; Rousseeuw 1987).
    Clustering {
        /// Optional cluster count `k >= 2` for *k*-means++ partitioning; `None` runs agglomerative hierarchical clustering only.
        k_means_k: Option<usize>,
        /// Observation-to-observation dissimilarity metric for hierarchical clustering.
        #[serde(default)]
        distance: ClusterDistance,
        /// Inter-cluster agglomerative merge criterion.
        #[serde(default)]
        linkage: ClusterLinkage,
        /// Optional upper cluster count `K_max >= 2` for a `k = 2..=K_max` elbow/silhouette sweep; `None` uses the default bounded sweep when applicable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        k_sweep_max: Option<usize>,
    },
    /// Chained-equations / predictive-mean-matching Multiple Imputation with Rubin's (1987) pooling rules across `m` completed datasets.
    MultipleImputation {
        /// Number of imputed completed datasets `m >= 2` (commonly `5`–`20`).
        imputations_m: usize,
        /// Number of Gibbs / chained-equations sweep iterations per imputation.
        max_iters: usize,
        /// Deterministic PRNG seed ensuring reproducible imputations.
        seed: u64,
        /// Optional zero-based outcome variable column index for pooled linear regression across the `m` imputations; `None` reports pooled column means/SDs only.
        #[serde(default)]
        response_column: Option<usize>,
        /// Zero-based predictor variable column indices when `response_column` is `Some(_)`.
        #[serde(default)]
        predictor_columns: Vec<usize>,
    },
}

// ============================================================================
// Linear & Nonlinear Regression Specifications (`docs/implementation/curve-fitting-implementation.md`)
// ============================================================================

/// Simple ordinary least-squares (Model I) or Deming orthogonal/errors-in-variables (Model II) linear regression specification (`crates/fit::linear`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearRegressionSpec {
    /// When `true`, fits Deming (1943) Model II regression accounting for measurement error in both *X* and *Y* (equal-variance ratio `delta = 1` by default) with jackknife SEs; when `false`, fits ordinary least-squares `Y = intercept + slope * X` (plus ANCOVA slope/intercept comparison across `>= 2` datasets).
    pub deming_model_ii: bool,
    /// When `true`, constrains `intercept = 0.0` (`Y = slope * X`, `df = n - 1`).
    pub force_through_origin: bool,
    /// When `true`, samples two-sided Working–Hotelling / Student-*t* confidence bands around the fitted line at [`AnalysisSpec::confidence_level`].
    pub confidence_bands: bool,
    /// When `true`, samples two-sided single-observation prediction bands around the fitted line at [`AnalysisSpec::confidence_level`].
    pub prediction_bands: bool,
    /// When `true`, interpolates unpaired `(X, empty)` rows (`Y` from `X`) and `(empty, Y)` rows (`X` from `Y` via Fieller/inverse calibration) on the source sheet.
    pub interpolate_unknowns: bool,
    /// Inclusive X bounds for observations entering the fit. A missing bound leaves that side unrestricted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_range_filter: Option<(Option<f64>, Option<f64>)>,
    /// Fitted-curve starting X, overriding the filter bound or fitted data minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_x_min: Option<f64>,
    /// Fitted-curve ending X, overriding the filter bound or fitted data maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_x_max: Option<f64>,
    /// Number of curve segments, or the default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_segments: Option<usize>,
}

impl LinearRegressionSpec {
    /// Number of segments when no count is specified.
    pub const DEFAULT_PLOT_SEGMENTS: usize = 100;
    /// Maximum number of segments for a fitted line.
    pub const MAX_PLOT_SEGMENTS: usize = 10_000;
}

impl Default for LinearRegressionSpec {
    fn default() -> Self {
        Self {
            deming_model_ii: false,
            force_through_origin: false,
            confidence_bands: true,
            prediction_bands: false,
            interpolate_unknowns: false,
            x_range_filter: None,
            plot_x_min: None,
            plot_x_max: None,
            plot_segments: None,
        }
    }
}

/// Differential weighting scheme for nonlinear regression (`docs/implementation/curve-fitting-implementation.md` §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum WeightingScheme {
    /// Uniform weights `w_i = 1` (homoscedastic constant-SD error assumption).
    #[default]
    None,
    /// `1 / Y^2` (constant `%CV` proportional scatter).
    RelativeSquare,
    /// `1 / |Y|` (Poisson-like variance proportional to mean response).
    Relative,
    /// General power-of-*Y* weight `1 / |Y|^k` with caller-supplied exponent `k`.
    GeneralPowerY(f64),
    /// `1 / |X|` weighting inversely proportional to independent variable magnitude.
    InverseX,
    /// `1 / X^2` weighting inversely proportional to squared *X*.
    InverseX2,
    /// Empirical replicate variance weighting `1 / SD_i^2` across replicates at each row.
    ObservedSd,
}

/// Regression objective function (`docs/implementation/curve-fitting-implementation.md` §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum FitMethod {
    /// Ordinary or weighted sum-of-squares minimization via trust-region Levenberg–Marquardt (Moré 1978).
    #[default]
    LeastSquares,
    /// Exact Poisson log-likelihood deviance minimization `2 * sum (y_i ln(y_i / mu_i) - (y_i - mu_i))` for non-negative count responses `y_i >= 0` and `mu_i > 0` (McCullagh & Nelder 1989).
    PoissonDeviance,
    /// Outlier-resistant Lorentzian/Cauchy M-estimator minimizing `sum ln(1 + (r_i / RSDR)^2)` without deleting points (Motulsky & Brown 2006; asymptotic SEs/CIs are intentionally omitted under robust loss).
    RobustCauchy,
    /// Two-stage ROUT outlier detection (Cauchy robust fit -> RSDR + Benjamini–Hochberg FDR over the furthest 30% of residuals) followed by refitting ordinary/weighted least squares on retained inliers (Motulsky & Brown 2006).
    RoutOutlierElimination {
        /// Target false discovery rate `Q` expressed in **percentage points** in `(0.0, 100.0]` (default `1.0` for `Q = 1%`).
        fdr_q_percent: f64,
    },
}

/// Dataset parameter sharing or constant constraint (`docs/implementation/curve-fitting-implementation.md` §1.3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ParameterScope {
    /// Estimate an independent parameter value separately for each fitted dataset.
    Local,
    /// Share and jointly optimize a single global parameter across all fitted datasets simultaneously.
    SharedGlobal,
    /// Fix the parameter to an explicit constant value across all datasets (`0` free *df*).
    Constant(f64),
    /// Fix the parameter dynamically to an upstream [`HookSource`] constant (`InfoSheet` or `ResultSheet` scalar).
    HookedConstant(HookSource),
    /// Fix the parameter to dataset-specific `(data_set_index, value)` constants (datasets absent from the list remain free `Local` parameters).
    PerDatasetConstant(Vec<(usize, f64)>),
    /// Fix the parameter for each dataset to the numeric value parsed from that dataset's column header title (for example Schild `[B]` or competitive inhibitor `[I]`).
    ColumnTitleConstant,
}

/// Constraint and sharing rule for a single named equation parameter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParameterRule {
    /// Exact equation parameter name matched against `ParametricModel::param_names` (for example `"Bottom"`, `"Top"`, `"LogEC50"`, or `"Km"`).
    pub name: String,
    /// Dataset sharing or constant-fixing scope for `name`.
    pub scope: ParameterScope,
    /// Optional explicit starting guess overriding the model's automatic initial-value heuristic; `None` uses the model's heuristic initializer.
    pub initial_override: Option<f64>,
    /// Optional inclusive box-constraint lower bound (`theta >= lower_bound`); `None` leaves the parameter unbounded below (unless the model defines intrinsic domain bounds).
    pub lower_bound: Option<f64>,
    /// Optional inclusive box-constraint upper bound (`theta <= upper_bound`); `None` leaves the parameter unbounded above.
    pub upper_bound: Option<f64>,
}

/// Model-comparison configuration (`Extra-sum-of-squares F` vs `AICc` vs `LikelihoodRatioTest`; §2.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonSpec {
    /// Nested extra-sum-of-squares *F*-test comparing the simpler null model (`alternative_equation_id`) against the fuller model (`F = ((SS_1 - SS_2)/(df_1 - df_2)) / (SS_2 / df_2)`; Motulsky & Christopoulos 2004).
    ExtraSumOfSquaresF {
        /// Built-in equation ID of the competing/null model.
        alternative_equation_id: String,
    },
    /// Small-sample corrected Akaike Information Criterion (`AICc`) evidence ratio and Akaike weights valid for nested or non-nested models (Burnham & Anderson 2002).
    Aicc {
        /// Built-in equation ID of the competing model.
        alternative_equation_id: String,
    },
    /// Nested deviance likelihood-ratio chi-square test (`Delta D ~ chi^2_{df_1 - df_2}`, used with [`FitMethod::PoissonDeviance`]).
    LikelihoodRatioTest {
        /// Built-in equation ID of the competing nested model.
        alternative_equation_id: String,
    },
}

/// Complete nonlinear regression specification (`crates/fit`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NonlinearFitSpec {
    /// Canonical built-in equation identifier (for example `"log_agonist_vs_response_4pl"` or `"michaelis_menten"`), or user label when `custom_dsl` is `Some(_)`.
    pub equation_id: String,
    /// Optional custom algebraic, piecewise `<A>`/`<B>`, implicit, or first-order RK45 ODE equation source compiled by `curvebook-fit::dsl`; `None` resolves `equation_id` from the built-in equation library.
    pub custom_dsl: Option<String>,
    /// Estimation objective (`LeastSquares`, `PoissonDeviance`, `RobustCauchy`, or `RoutOutlierElimination`).
    pub method: FitMethod,
    /// Observation weighting scheme applied to residuals (`None` for unweighted; ignored by `PoissonDeviance`).
    pub weighting: WeightingScheme,
    /// Per-parameter sharing (`SharedGlobal`), constant fixing, initial guesses, and box bounds.
    pub parameter_rules: Vec<ParameterRule>,
    /// When `true`, computes Venzon–Moolgavkar (1988) / Bates–Watts (1988) asymmetrical profile-likelihood confidence intervals (falling back to [`crate::CiBound::Open`] on flat plateaus) at [`AnalysisSpec::confidence_level`]; when `false`, reports symmetric Wald-*t* intervals.
    pub profile_likelihood_ci: bool,
    /// Assess whether the fitted data support stable estimation of the free parameters.
    #[serde(default)]
    pub identify_unstable_parameters: bool,
    /// Warn when the reported unweighted R squared is below this threshold in [0, 1].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag_r_squared_below: Option<f64>,
    /// Optional competing model comparison (`ExtraSumOfSquaresF`, `Aicc`, or `LikelihoodRatioTest`); `None` fits a single model.
    pub comparison: Option<ModelComparisonSpec>,
    /// When `true`, evaluates two-sided delta-method confidence bands along each [`crate::FittedCurveOverlay`] at [`AnalysisSpec::confidence_level`].
    pub confidence_bands: bool,
    /// When `true`, evaluates two-sided single-observation prediction bands along each [`crate::FittedCurveOverlay`] at [`AnalysisSpec::confidence_level`].
    pub prediction_bands: bool,
    /// When `true`, solves unpaired unknown rows (`X` from `Y` via root-finding or `Y` from `X` via forward evaluation) with confidence intervals.
    pub interpolate_unknowns: bool,
    /// When `true` and replicate subcolumns exist, runs the pure-error vs lack-of-fit *F*-test (Draper & Smith 1998).
    pub replicates_lack_of_fit_test: bool,
    /// When `true`, runs Shapiro–Wilk, D'Agostino–Pearson, Anderson–Darling, and Lilliefors KS normality tests plus Wald–Wolfowitz runs test on residuals.
    pub normality_of_residuals: bool,
    /// Maximum Levenberg–Marquardt iterations (clamped into `1..=Self::HARD_MAX_ITERS` via [`Self::clamped_max_iters`]).
    pub max_iters: usize,
    /// Optional inclusive `(min_x, max_x)` domain window restricting which observations enter the fit; `None` (or `None` bound) leaves that side unrestricted.
    #[serde(default)]
    pub x_range_filter: Option<(Option<f64>, Option<f64>)>,
    /// Optional inclusive `(min_y, max_y)` response window restricting which observations enter the fit; `None` leaves *Y* unrestricted.
    #[serde(default)]
    pub y_range_filter: Option<(Option<f64>, Option<f64>)>,
    /// When `true`, evaluates and plots the theoretical curve over `[plot_x_min, plot_x_max]` using fixed/initial parameter values without fitting data.
    #[serde(default)]
    pub plot_function_only: bool,
    /// Explicit lower *X* coordinate for curve grid sampling; `None` uses the observed data minimum.
    #[serde(default)]
    pub plot_x_min: Option<f64>,
    /// Explicit upper *X* coordinate for curve grid sampling; `None` uses the observed data maximum.
    #[serde(default)]
    pub plot_x_max: Option<f64>,
    /// When `true`, runs Spearman rank correlation between `|residual|` and predicted `|Y_hat|` to test homoscedasticity.
    #[serde(default)]
    pub homoscedasticity_test: bool,
    /// Emit the `ResultTabKind::Residuals` tab (`true` by default for new specs; missing legacy JSON
    /// fields deserialize as `false` via `#[serde(default)]`).
    #[serde(default)]
    pub emit_residuals_tab: bool,
    /// When `true`, emits the [`crate::ResultTabKind::CovarianceMatrix`] parameter correlation/covariance tab and parameter dependency indices.
    #[serde(default)]
    pub emit_covariance_tab: bool,
    /// Zero-based categorical grouping column indices partitioning rows into separate fitted datasets on [`TableKind::MultipleVariables`] sources.
    #[serde(default)]
    pub mv_group_columns: Vec<usize>,
    /// Zero-based independent predictor column indices (`X`, or multiple predictors `(X, Z, ...)` for multivariable custom DSL equations) on [`TableKind::MultipleVariables`] sources.
    #[serde(default)]
    pub mv_x_columns: Vec<usize>,
    /// Zero-based continuous dependent response column index on [`TableKind::MultipleVariables`] sources; `None` on `Xy` / `Grouped` tables.
    #[serde(default)]
    pub mv_y_column: Option<usize>,
}

impl NonlinearFitSpec {
    /// Default Levenberg–Marquardt iteration budget (`1_000`) assigned by [`Self::new`].
    pub const DEFAULT_MAX_ITERS: usize = 1_000;
    /// Hard safety ceiling (`50_000`) enforced by [`Self::clamped_max_iters`].
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
            identify_unstable_parameters: false,
            flag_r_squared_below: None,
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

    /// Return `max_iters` clamped into `1..=Self::HARD_MAX_ITERS`.
    #[must_use]
    pub fn clamped_max_iters(&self) -> usize {
        self.max_iters.clamp(1, Self::HARD_MAX_ITERS)
    }
}

// ============================================================================
// Power & Sample Size Specifications (`R8.7` / `R2-S07-10`)
// ============================================================================

/// Supported hypothesis test families for power and sample-size planning (Cohen 1988).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PowerTestFamily {
    /// Two-sample unpaired Student/Welch *t*-test via exact noncentral-*t* distribution (`df = n_1 + n_2 - 2`, Cohen's `d`).
    #[default]
    TwoSampleUnpairedT,
    /// Paired-differences *t*-test via exact noncentral-*t* distribution (`df = n - 1`, Cohen's `d_z`).
    TwoSamplePairedT,
    /// One-sample *t*-test via exact noncentral-*t* distribution (`df = n - 1`, Cohen's `d`).
    OneSampleT,
    /// Balanced `k`-group one-way fixed-effects ANOVA via exact noncentral-*F* distribution (`df_1 = k - 1`, `df_2 = k(n - 1)`, Cohen's `f`).
    OneWayAnova,
    /// Two independent binomial proportions test (`p_1` vs `p_2`; Fleiss, Levin, & Paik 2003 / Cohen's `h`).
    TwoProportions,
    /// Bivariate Pearson correlation test `H_0: rho = 0` (`r` in `(-1, 1)`).
    Correlation,
    /// Contingency or goodness-of-fit chi-square test via exact noncentral `chi^2` distribution (`df > 0`, Cohen's `w`).
    ChiSquare,
    /// Simple linear regression slope test via exact noncentral-*F* / *t* (`df_1 = 1`, `df_2 = n - 2`, Cohen's `f^2 = R^2 / (1 - R^2)`).
    SimpleLinearRegression,
}

impl PowerTestFamily {
    /// All eight supported power-analysis test families.
    pub const ALL: [Self; 8] = [
        Self::TwoSampleUnpairedT,
        Self::TwoSamplePairedT,
        Self::OneSampleT,
        Self::OneWayAnova,
        Self::TwoProportions,
        Self::Correlation,
        Self::ChiSquare,
        Self::SimpleLinearRegression,
    ];

    /// Neutral display label for titles, inspectors, and methods reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TwoSampleUnpairedT => "Unpaired t test",
            Self::TwoSamplePairedT => "Paired t test",
            Self::OneSampleT => "One-sample t test",
            Self::OneWayAnova => "One-way ANOVA",
            Self::TwoProportions => "Two proportions",
            Self::Correlation => "Pearson correlation",
            Self::ChiSquare => "Chi-square test",
            Self::SimpleLinearRegression => "Simple linear regression",
        }
    }

    /// Canonical standardized effect-size metric label for this family.
    #[must_use]
    pub const fn default_effect_label(self) -> &'static str {
        match self {
            Self::TwoSampleUnpairedT | Self::OneSampleT => "Cohen's d",
            Self::TwoSamplePairedT => "Cohen's dz",
            Self::OneWayAnova => "Cohen's f",
            Self::TwoProportions => "Cohen's h",
            Self::Correlation => "Pearson r",
            Self::ChiSquare => "Cohen's w",
            Self::SimpleLinearRegression => "Cohen's f^2",
        }
    }

    /// Return `true` when one-sided alternatives are meaningful for this family.
    #[must_use]
    pub const fn supports_one_sided(self) -> bool {
        !matches!(self, Self::OneWayAnova | Self::ChiSquare)
    }
}

/// Direction for one-sided power hypothesis alternatives (`H1: theta > theta_0` vs `<`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PowerOneSidedDirection {
    /// Upper-tail alternative `H_1: theta > theta_0`.
    #[default]
    Greater,
    /// Lower-tail alternative `H_1: theta < theta_0`.
    Less,
}

/// Hypothesis tail mode for power and sample-size calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PowerTailMode {
    /// Two-sided rejection region splitting `alpha / 2` in each tail.
    #[default]
    TwoSided,
    /// Directional one-sided rejection region at full `alpha` (rejected for omnibus `F` / `chi^2` families).
    OneSided(PowerOneSidedDirection),
}

/// Target unknown quantity solved by a power analysis specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PowerSolveTarget {
    /// Solve required integer sample size (`n_1` / per-group `n` / total `N`) to achieve `target_power` at `alpha` and `effect`.
    #[default]
    SampleSize,
    /// Evaluate achieved statistical power `1 - beta` in `(0, 1)` given `sample_size`, `alpha`, and `effect`.
    Power,
    /// Solve the minimum detectable effect size given `sample_size`, `alpha`, and `target_power`.
    DetectableEffect,
}

/// Variance / standard-error approximation convention for [`PowerTestFamily::TwoProportions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum TwoProportionsMethod {
    /// Pooled proportion SE under `H_0` paired with unpooled binomial SE under `H_1` (Fleiss, Levin, & Paik 2003).
    #[default]
    PooledNullUnpooledAlt,
    /// Unpooled Wald SE `sqrt(p_1(1 - p_1)/n_1 + p_2(1 - p_2)/n_2)` under both `H_0` and `H_1`.
    Unpooled,
    /// Arcsine-transformed standardized effect `h = 2 arcsin(sqrt(p_1)) - 2 arcsin(sqrt(p_2))` normal approximation (Cohen 1988).
    CohensH,
}

/// Power-curve convention for [`PowerTestFamily::Correlation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CorrelationPowerMethod {
    /// Classical Fisher `arctanh(r)` normal variance `1 / (n - 3)` with normal critical values (Fisher 1921; Cohen 1988).
    #[default]
    FisherZ,
    /// Exact `t_{n-2}` critical value transformed into sample-`r` cutoff, evaluated via Fisher `z` distribution under `H_1`.
    FisherZWithTCrit,
    /// Exact correlation distribution integration under bivariate normality for `H_0: rho = 0` vs `H_1: rho != 0`.
    ExactTTransform,
    /// Noncentral-*t* approximation treating standardized predictor *X* as fixed (`ncp = sqrt(n - 2) * rho / sqrt(1 - rho^2)`).
    ConditionalSlopeT,
}

/// Effect-size parameterization for [`PowerAnalysisSpec`].
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum PowerEffectSpec {
    /// Placeholder used solely when `solve_target == PowerSolveTarget::DetectableEffect` (`validate` rejects `Unspecified` for `SampleSize` and `Power` solves).
    #[default]
    Unspecified,
    /// Direct standardized effect size (`d` for unpaired/one-sample *t*, `d_z` for paired *t*, `f` for ANOVA, `h` for two proportions, `r` in `(-1, 1)` for correlation, `w` for chi-square, or `f^2` for simple linear regression).
    Standardized(f64),
    /// Raw mean difference and within-group standard deviation(s) for one-sample, unpaired, or correlated paired *t*-tests.
    MeanDiffAndSd {
        /// Hypothesized difference in means `mu_1 - mu_2` (or `mu - mu_0`) in data units.
        mean_diff: f64,
        /// Common (or group-1) population standard deviation `sigma > 0` in data units.
        sd: f64,
        /// Optional group-2 standard deviation `sigma_2 > 0` for unequal-SD two-sample planning; `None` assumes `sigma_2 == sd`.
        #[serde(default)]
        sd_group2: Option<f64>,
        /// Optional within-pair Pearson correlation `rho in [-1, 1)` converting marginal `(sd, sd_group2)` into `sd_diff = sqrt(sd_1^2 + sd_2^2 - 2 rho sd_1 sd_2)` for [`PowerTestFamily::TwoSamplePairedT`]; `None` treats `sd` directly.
        #[serde(default)]
        within_pair_correlation: Option<f64>,
    },
    /// Raw mean and standard deviation of paired differences `D = Y_1 - Y_2` (`d_z = mean_diff / sd_diff`).
    PairedDiff {
        /// Expected mean of paired differences in data units.
        mean_diff: f64,
        /// Standard deviation of paired differences (`> 0`) in data units.
        sd_diff: f64,
    },
    /// Explicit group means `mu_1, ..., mu_k` and common within-group SD `sigma > 0` for [`PowerTestFamily::OneWayAnova`] (`f = sigma_means / within_sd`).
    GroupMeansAndSd {
        /// Hypothesized population means across `k >= 2` groups in data units.
        means: Vec<f64>,
        /// Common within-group standard deviation (`> 0`) in data units.
        within_sd: f64,
    },
    /// Explicit group event proportions `(p_1, p_2)` in `[0, 1]` for [`PowerTestFamily::TwoProportions`].
    TwoProportions {
        /// Group-1 event proportion `p_1 in [0, 1]`.
        p1: f64,
        /// Group-2 event proportion `p_2 in [0, 1]`.
        p2: f64,
    },
    /// Reference baseline proportion `p_1 in [0, 1]` used when solving [`PowerSolveTarget::DetectableEffect`] for `p_2` in [`PowerTestFamily::TwoProportions`].
    BaselineProportion {
        /// Known control/baseline event proportion `p_1 in [0, 1]`.
        p1: f64,
    },
    /// Coefficient of determination `R^2 in [0, 1)` for [`PowerTestFamily::SimpleLinearRegression`] (`f^2 = R^2 / (1 - R^2)`).
    RSquared(f64),
    /// Raw slope `beta_1`, predictor SD `sigma_x > 0`, and residual SD `sigma_eps > 0` for [`PowerTestFamily::SimpleLinearRegression`].
    RegressionSlope {
        /// Hypothesized linear slope `beta_1` (`Y` units per `X` unit).
        beta1: f64,
        /// Standard deviation of predictor *X* (`> 0`).
        sigma_x: f64,
        /// Residual error standard deviation `sigma_epsilon > 0`.
        sigma_eps: f64,
    },
}

/// First-class L1 document specification for statistical power, sample-size, and
/// minimum-detectable-effect calculations.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PowerAnalysisSpec {
    /// Target statistical hypothesis test family.
    pub family: PowerTestFamily,
    /// Which quantity (`SampleSize`, `Power`, or `DetectableEffect`) is solved for.
    pub solve_target: PowerSolveTarget,
    /// Two-sided vs directional one-sided rejection region.
    pub tail_mode: PowerTailMode,
    /// Two-sided or one-sided significance level `alpha` in `(0, 1)`.
    pub alpha: f64,
    /// Target statistical power `1 - beta` in `(0, 1)`; required (`Some`) when solving `SampleSize` or `DetectableEffect`, `None` (unused) when solving `Power`.
    #[serde(default)]
    pub target_power: Option<f64>,
    /// Primary sample size (`n1` for two-sample tests, per-group `n` for `OneWayAnova`, or total `N` for one-sample/correlation/chi-square/regression); required (`Some`) when solving `Power` or `DetectableEffect`, `None` when solving `SampleSize`.
    #[serde(default)]
    pub sample_size: Option<f64>,
    /// Optional explicit second-group sample size `n2 > 0` for two-sample tests; `None` derives `n2 = allocation_ratio * n1`.
    #[serde(default)]
    pub sample_size_group2: Option<f64>,
    /// Allocation ratio `k = n2 / n1 > 0` when `sample_size_group2` is `None`.
    pub allocation_ratio: f64,
    /// Effect-size specification (`Unspecified` allowed only when solving `DetectableEffect`).
    pub effect: PowerEffectSpec,
    /// Group count `k >= 2` for `OneWayAnova` (`None` defaults to `3` or `means.len()` when `GroupMeansAndSd` is supplied).
    #[serde(default)]
    pub num_groups: Option<usize>,
    /// Degrees of freedom `df > 0` for `ChiSquare`.
    pub chi_square_df: f64,
    /// Variance / SE convention for `TwoProportions`.
    pub two_proportions_mode: TwoProportionsMethod,
    /// Yates / Fleiss continuity correction for `TwoProportions`.
    pub continuity_correction: bool,
    /// Power-curve convention for `Correlation`.
    pub correlation_method: CorrelationPowerMethod,
    /// Optional linked `InfoSheet` supplying parameter hooks; `None` when standalone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_sheet_id: Option<SheetId>,
}

impl Default for PowerAnalysisSpec {
    fn default() -> Self {
        Self {
            family: PowerTestFamily::TwoSampleUnpairedT,
            solve_target: PowerSolveTarget::SampleSize,
            tail_mode: PowerTailMode::TwoSided,
            alpha: 0.05,
            target_power: Some(0.80),
            sample_size: None,
            sample_size_group2: None,
            allocation_ratio: 1.0,
            effect: PowerEffectSpec::Unspecified,
            num_groups: None,
            chi_square_df: 1.0,
            two_proportions_mode: TwoProportionsMethod::PooledNullUnpooledAlt,
            continuity_correction: false,
            correlation_method: CorrelationPowerMethod::FisherZ,
            info_sheet_id: None,
        }
    }
}

impl PowerAnalysisSpec {
    /// Derive the default result-sheet title for this power specification.
    #[must_use]
    pub fn derive_auto_title(&self) -> String {
        format!("Power & sample size ({})", self.family.label())
    }

    /// Return the first non-finite float field path in this power specification, if any.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<&'static str> {
        if !self.alpha.is_finite() {
            return Some("power.alpha");
        }
        if self.target_power.is_some_and(|v| !v.is_finite()) {
            return Some("power.target_power");
        }
        if self.sample_size.is_some_and(|v| !v.is_finite()) {
            return Some("power.sample_size");
        }
        if self.sample_size_group2.is_some_and(|v| !v.is_finite()) {
            return Some("power.sample_size_group2");
        }
        if !self.allocation_ratio.is_finite() {
            return Some("power.allocation_ratio");
        }
        if !self.chi_square_df.is_finite() {
            return Some("power.chi_square_df");
        }
        match &self.effect {
            PowerEffectSpec::Unspecified => None,
            PowerEffectSpec::Standardized(v) => (!v.is_finite()).then_some("power.effect.standardized"),
            PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation } => {
                if !mean_diff.is_finite()
                    || !sd.is_finite()
                    || sd_group2.is_some_and(|v| !v.is_finite())
                    || within_pair_correlation.is_some_and(|v| !v.is_finite())
                {
                    Some("power.effect.mean_diff_and_sd")
                } else {
                    None
                }
            }
            PowerEffectSpec::PairedDiff { mean_diff, sd_diff } => {
                (!mean_diff.is_finite() || !sd_diff.is_finite()).then_some("power.effect.paired_diff")
            }
            PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
                (!within_sd.is_finite() || means.iter().any(|m| !m.is_finite())).then_some("power.effect.group_means_and_sd")
            }
            PowerEffectSpec::TwoProportions { p1, p2 } => (!p1.is_finite() || !p2.is_finite()).then_some("power.effect.two_proportions"),
            PowerEffectSpec::BaselineProportion { p1 } => (!p1.is_finite()).then_some("power.effect.baseline_proportion"),
            PowerEffectSpec::RSquared(v) => (!v.is_finite()).then_some("power.effect.r_squared"),
            PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps } => {
                (!beta1.is_finite() || !sigma_x.is_finite() || !sigma_eps.is_finite()).then_some("power.effect.regression_slope")
            }
        }
    }

    /// Validate domain constraints that apply to every persisted [`PowerAnalysisSpec`],
    /// including unconfigured draft specifications (`PowerAnalysisSpec::default()`).
    pub fn validate_domain(&self) -> Result<(), DocError> {
        let invalid = |reason: &'static str| DocError::InvalidCellValue { archetype: TableKind::Column, row: 0, col: 0, reason };
        if let Some(reason) = self.first_non_finite_float() {
            return Err(invalid(reason));
        }
        if self.alpha <= 0.0 || self.alpha >= 1.0 {
            return Err(invalid("power.alpha must lie strictly in (0, 1)"));
        }
        if matches!(self.tail_mode, PowerTailMode::OneSided(_)) && !self.family.supports_one_sided() {
            return Err(invalid("one-sided tail mode is not supported for F/chi-square power tests"));
        }
        if let Some(pw) = self.target_power
            && (pw <= 0.0 || pw >= 1.0)
        {
            return Err(invalid("power.target_power must lie strictly in (0, 1)"));
        }
        if let Some(n) = self.sample_size
            && n <= 0.0
        {
            return Err(invalid("power.sample_size must be > 0"));
        }
        if let Some(n2) = self.sample_size_group2
            && n2 <= 0.0
        {
            return Err(invalid("power.sample_size_group2 must be > 0"));
        }
        if self.allocation_ratio <= 0.0 {
            return Err(invalid("power.allocation_ratio must be > 0"));
        }
        if self.chi_square_df <= 0.0 {
            return Err(invalid("power.chi_square_df must be > 0"));
        }
        if let Some(k) = self.num_groups
            && k < 2
        {
            return Err(invalid("power.num_groups must be >= 2"));
        }
        match &self.effect {
            PowerEffectSpec::MeanDiffAndSd { sd, sd_group2, within_pair_correlation, .. } => {
                if *sd <= 0.0 || sd_group2.is_some_and(|s| s <= 0.0) {
                    return Err(invalid("power.effect standard deviation must be > 0"));
                }
                if within_pair_correlation.is_some_and(|r| !(-1.0..1.0).contains(&r)) {
                    return Err(invalid("power.effect within_pair_correlation must lie in [-1, 1)"));
                }
            }
            PowerEffectSpec::PairedDiff { sd_diff, .. } if *sd_diff <= 0.0 => {
                return Err(invalid("power.effect sd_diff must be > 0"));
            }
            PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
                if means.len() < 2 {
                    return Err(invalid("power.effect GroupMeansAndSd requires at least 2 means"));
                }
                if *within_sd <= 0.0 {
                    return Err(invalid("power.effect within_sd must be > 0"));
                }
            }
            PowerEffectSpec::TwoProportions { p1, p2 } if !(0.0..=1.0).contains(p1) || !(0.0..=1.0).contains(p2) => {
                return Err(invalid("power.effect proportions must lie in [0, 1]"));
            }
            PowerEffectSpec::BaselineProportion { p1 } if !(0.0..=1.0).contains(p1) => {
                return Err(invalid("power.effect baseline proportion must lie in [0, 1]"));
            }
            PowerEffectSpec::RSquared(r2) if *r2 < 0.0 || *r2 >= 1.0 => {
                return Err(invalid("power.effect R^2 must lie in [0, 1)"));
            }
            PowerEffectSpec::RegressionSlope { sigma_x, sigma_eps, .. } if *sigma_x <= 0.0 || *sigma_eps <= 0.0 => {
                return Err(invalid("power.effect regression SDs must be > 0"));
            }
            _ => {}
        }
        if let Some(ref info_id) = self.info_sheet_id {
            info_id.validate_non_empty_bounded()?;
        }
        Ok(())
    }

    /// Validate domain constraints on this power specification (`alpha`, `target_power`,
    /// sample sizes, allocation ratio, tail compatibility, and solver readiness).
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_domain()?;
        let invalid = |reason: &'static str| DocError::InvalidCellValue { archetype: TableKind::Column, row: 0, col: 0, reason };
        if self.solve_target != PowerSolveTarget::Power && self.target_power.is_none() {
            return Err(invalid("power.target_power is required when solving SampleSize or DetectableEffect"));
        }
        if self.solve_target != PowerSolveTarget::SampleSize && self.sample_size.is_none() {
            return Err(invalid("power.sample_size is required when solving Power or DetectableEffect"));
        }
        if self.solve_target != PowerSolveTarget::DetectableEffect && matches!(self.effect, PowerEffectSpec::Unspecified) {
            return Err(invalid("power.effect must be specified unless solving DetectableEffect"));
        }
        Ok(())
    }
}

impl<'de> serde::Deserialize<'de> for PowerAnalysisSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(default)]
        struct RawPowerAnalysisSpec {
            family: PowerTestFamily,
            solve_target: PowerSolveTarget,
            tail_mode: PowerTailMode,
            alpha: f64,
            target_power: Option<f64>,
            sample_size: Option<f64>,
            sample_size_group2: Option<f64>,
            allocation_ratio: f64,
            chi_square_df: f64,
            num_groups: Option<usize>,
            two_proportions_mode: TwoProportionsMethod,
            continuity_correction: bool,
            correlation_method: CorrelationPowerMethod,
            effect: PowerEffectSpec,
            info_sheet_id: Option<SheetId>,
        }

        impl Default for RawPowerAnalysisSpec {
            fn default() -> Self {
                let d = PowerAnalysisSpec::default();
                Self {
                    family: d.family,
                    solve_target: d.solve_target,
                    tail_mode: d.tail_mode,
                    alpha: d.alpha,
                    target_power: d.target_power,
                    sample_size: d.sample_size,
                    sample_size_group2: d.sample_size_group2,
                    allocation_ratio: d.allocation_ratio,
                    chi_square_df: d.chi_square_df,
                    num_groups: d.num_groups,
                    two_proportions_mode: d.two_proportions_mode,
                    continuity_correction: d.continuity_correction,
                    correlation_method: d.correlation_method,
                    effect: d.effect,
                    info_sheet_id: d.info_sheet_id,
                }
            }
        }

        let raw = RawPowerAnalysisSpec::deserialize(deserializer)?;
        let spec = Self {
            family: raw.family,
            solve_target: raw.solve_target,
            tail_mode: raw.tail_mode,
            alpha: raw.alpha,
            target_power: raw.target_power,
            sample_size: raw.sample_size,
            sample_size_group2: raw.sample_size_group2,
            allocation_ratio: raw.allocation_ratio,
            chi_square_df: raw.chi_square_df,
            num_groups: raw.num_groups,
            two_proportions_mode: raw.two_proportions_mode,
            continuity_correction: raw.continuity_correction,
            correlation_method: raw.correlation_method,
            effect: raw.effect,
            info_sheet_id: raw.info_sheet_id,
        };
        spec.validate_domain().map_err(serde::de::Error::custom)?;
        Ok(spec)
    }
}

impl crate::schema::JsonSchema for PowerTestFamily {
    fn schema_name() -> &'static str {
        "PowerTestFamily"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        crate::schema::enum_schema(&[
            "TwoSampleUnpairedT",
            "TwoSamplePairedT",
            "OneSampleT",
            "OneWayAnova",
            "TwoProportions",
            "Correlation",
            "ChiSquare",
            "SimpleLinearRegression",
        ])
        .with_title("PowerTestFamily")
    }
}

impl crate::schema::JsonSchema for PowerSolveTarget {
    fn schema_name() -> &'static str {
        "PowerSolveTarget"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        crate::schema::enum_schema(&["SampleSize", "Power", "DetectableEffect"]).with_title("PowerSolveTarget")
    }
}

impl crate::schema::JsonSchema for PowerTailMode {
    fn schema_name() -> &'static str {
        "PowerTailMode"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        let dir = crate::schema::enum_schema(&["Greater", "Less"]);
        let one_sided = crate::schema::object_schema(vec![("OneSided", dir)], &["OneSided"], false);
        crate::schema::one_of_schema(vec![crate::schema::const_string_schema("TwoSided"), one_sided]).with_title("PowerTailMode")
    }
}

impl crate::schema::JsonSchema for TwoProportionsMethod {
    fn schema_name() -> &'static str {
        "TwoProportionsMethod"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        crate::schema::enum_schema(&["PooledNullUnpooledAlt", "Unpooled", "CohensH"]).with_title("TwoProportionsMethod")
    }
}

impl crate::schema::JsonSchema for CorrelationPowerMethod {
    fn schema_name() -> &'static str {
        "CorrelationPowerMethod"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        crate::schema::enum_schema(&["FisherZ", "FisherZWithTCrit", "ExactTTransform", "ConditionalSlopeT"]).with_title("CorrelationPowerMethod")
    }
}

impl crate::schema::JsonSchema for PowerEffectSpec {
    fn schema_name() -> &'static str {
        "PowerEffectSpec"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        use crate::schema::{array_schema, nullable_schema, number_schema, object_schema, one_of_schema};
        one_of_schema(vec![
            crate::schema::const_string_schema("Unspecified"),
            object_schema(vec![("Standardized", number_schema())], &["Standardized"], false),
            object_schema(
                vec![(
                    "MeanDiffAndSd",
                    object_schema(
                        vec![
                            ("mean_diff", number_schema()),
                            ("sd", number_schema().with_minimum_f64(f64::MIN_POSITIVE)),
                            ("sd_group2", nullable_schema(number_schema())),
                            ("within_pair_correlation", nullable_schema(number_schema().with_minimum_f64(-1.0).with_maximum_f64(1.0))),
                        ],
                        &["mean_diff", "sd"],
                        false,
                    ),
                )],
                &["MeanDiffAndSd"],
                false,
            ),
            object_schema(
                vec![(
                    "PairedDiff",
                    object_schema(vec![("mean_diff", number_schema()), ("sd_diff", number_schema())], &["mean_diff", "sd_diff"], false),
                )],
                &["PairedDiff"],
                false,
            ),
            object_schema(
                vec![(
                    "GroupMeansAndSd",
                    object_schema(vec![("means", array_schema(number_schema())), ("within_sd", number_schema())], &["means", "within_sd"], false),
                )],
                &["GroupMeansAndSd"],
                false,
            ),
            object_schema(
                vec![(
                    "TwoProportions",
                    object_schema(
                        vec![
                            ("p1", number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0)),
                            ("p2", number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0)),
                        ],
                        &["p1", "p2"],
                        false,
                    ),
                )],
                &["TwoProportions"],
                false,
            ),
            object_schema(
                vec![(
                    "BaselineProportion",
                    object_schema(vec![("p1", number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))], &["p1"], false),
                )],
                &["BaselineProportion"],
                false,
            ),
            object_schema(vec![("RSquared", number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))], &["RSquared"], false),
            object_schema(
                vec![(
                    "RegressionSlope",
                    object_schema(
                        vec![("beta1", number_schema()), ("sigma_x", number_schema()), ("sigma_eps", number_schema())],
                        &["beta1", "sigma_x", "sigma_eps"],
                        false,
                    ),
                )],
                &["RegressionSlope"],
                false,
            ),
        ])
        .with_title("PowerEffectSpec")
    }
}

impl crate::schema::JsonSchema for PowerAnalysisSpec {
    fn schema_name() -> &'static str {
        "PowerAnalysisSpec"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        use crate::schema::{bool_schema, integer_schema, nullable_schema, number_schema, object_schema, ref_schema};
        object_schema(
            vec![
                ("family", PowerTestFamily::json_schema()),
                ("solve_target", PowerSolveTarget::json_schema()),
                ("tail_mode", PowerTailMode::json_schema()),
                ("alpha", number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0)),
                ("target_power", nullable_schema(number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))),
                ("sample_size", nullable_schema(number_schema().with_minimum_f64(0.0))),
                ("sample_size_group2", nullable_schema(number_schema().with_minimum_f64(0.0))),
                ("allocation_ratio", number_schema().with_minimum_f64(f64::MIN_POSITIVE)),
                ("effect", PowerEffectSpec::json_schema()),
                ("num_groups", nullable_schema(integer_schema().with_minimum_i64(2))),
                ("chi_square_df", number_schema().with_minimum_f64(f64::MIN_POSITIVE)),
                ("two_proportions_mode", TwoProportionsMethod::json_schema()),
                ("continuity_correction", bool_schema()),
                ("correlation_method", CorrelationPowerMethod::json_schema()),
                ("info_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ],
            &[
                "family",
                "solve_target",
                "tail_mode",
                "alpha",
                "allocation_ratio",
                "effect",
                "chi_square_df",
                "two_proportions_mode",
                "continuity_correction",
                "correlation_method",
            ],
            false,
        )
        .with_title("PowerAnalysisSpec")
    }
}

// ============================================================================
// Unified AnalysisKind & AnalysisSpec
// ============================================================================

/// Discriminated statistical, regression, or transformation specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AnalysisKind {
    /// Derived data-table transformation, normalization, baseline removal, transpose, pruning, or Monte Carlo simulation ([`TransformAnalysisSpec`]).
    Transform(TransformAnalysisSpec),
    /// Univariate column descriptive moments, percentiles, normality battery, and optional one-sample location tests ([`DescriptiveSpec`]).
    DescriptiveStats(DescriptiveSpec),
    /// Univariate Grubbs ESD or ROUT FDR outlier identification ([`OutlierSpec`]).
    OutlierDetection(OutlierSpec),
    /// Parametric or nonparametric two-group / one-sample comparison ([`TwoGroupSpec`]).
    TwoGroupComparison(TwoGroupSpec),
    /// Bland–Altman method-comparison bias and limits of agreement ([`BlandAltmanSpec`]).
    BlandAltman(BlandAltmanSpec),
    /// Row-by-row multiple two-group comparisons with FWER/FDR control ([`MultipleRowTestsSpec`]).
    MultipleRowTTests(MultipleRowTestsSpec),
    /// Passing–Bablok nonparametric errors-in-variables linear regression ([`PassingBablokSpec`]).
    PassingBablok(PassingBablokSpec),
    /// One-way ordinary, Welch/Brown–Forsythe, repeated-measures/REML, Kruskal–Wallis, or Friedman ANOVA ([`OneWayAnovaSpec`]).
    OneWayAnova(OneWayAnovaSpec),
    /// Two-way, three-way (`2 × 2 × K`), or multifactor `N`-way ANOVA / REML mixed model ([`FactorialAnovaSpec`]).
    FactorialAnova(FactorialAnovaSpec),
    /// Hierarchical nested *t*-test or nested one-way REML ANOVA ([`NestedAnalysisSpec`]).
    NestedComparison(NestedAnalysisSpec),
    /// Contingency chi-square, exact, trend, McNemar, kappa, or exact binomial analysis ([`ContingencySpec`]).
    Contingency(ContingencySpec),
    /// Kaplan–Meier survival curves or multivariable Cox proportional hazards regression ([`SurvivalAnalysisSpec`]).
    Survival(SurvivalAnalysisSpec),
    /// Receiver Operating Characteristic (ROC) curve and DeLong/Hanley–McNeil AUC analysis ([`RocSpec`]).
    RocCurve(RocSpec),
    /// Multivariable linear, logistic, or Poisson generalized linear model ([`GlmSpec`]).
    GeneralizedLinearModel(GlmSpec),
    /// Principal component analysis, correlation matrix, clustering, or Rubin multiple imputation ([`MultivariateSpec`]).
    Multivariate(MultivariateSpec),
    /// Simple OLS (Model I), ANCOVA line comparison, or Deming (Model II) linear regression ([`LinearRegressionSpec`]).
    LinearRegression(LinearRegressionSpec),
    /// Nonlinear least-squares, Poisson deviance, Cauchy robust, or ROUT curve fitting and custom DSL/ODE modeling ([`NonlinearFitSpec`]).
    NonlinearRegression(NonlinearFitSpec),
    /// Standalone FWER/FDR adjustment of a column of pre-computed *P* values ([`PValueStackSpec`]).
    PValueStack(PValueStackSpec),
    /// Statistical power, sample-size, and minimum-detectable-effect solver ([`PowerAnalysisSpec`]).
    PowerAnalysis(PowerAnalysisSpec),
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
            Self::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) => {
                String::from(if pcr_outcome_column.is_some() { "Principal component regression" } else { "Principal component analysis" })
            }
            Self::Multivariate(MultivariateSpec::CorrelationMatrix { .. }) => String::from("Correlation matrix"),
            Self::Multivariate(MultivariateSpec::Clustering { .. }) => String::from("Cluster analysis"),
            Self::Multivariate(MultivariateSpec::MultipleImputation { .. }) => String::from("Multiple imputation"),
            Self::LinearRegression(_) => String::from("Simple linear regression"),
            Self::NonlinearRegression(fit) => format!("Nonlinear regression ({})", fit.equation_id),
            Self::PValueStack(s) => format!("Analyze stack of P values ({})", s.method.label()),
            Self::PowerAnalysis(s) => s.derive_auto_title(),
        }
    }

    /// Check whether this analysis kind can run against a given [`TableKind`] (used by Analyze
    /// filtering and `Change Data Analyzed` retargeting).
    #[must_use]
    pub fn is_compatible_with(&self, table_kind: TableKind) -> bool {
        match self {
            Self::Contingency(s) => match table_kind {
                TableKind::Contingency => true,
                TableKind::PartsOfWhole => {
                    matches!(s.method, ContingencyMethod::PearsonChiSquare { .. } | ContingencyMethod::ExactBinomial)
                }
                TableKind::Column => matches!(s.method, ContingencyMethod::ExactBinomial),
                _ => false,
            },
            other => other.descriptor().supports_table_kind(table_kind),
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
        if let Self::PowerAnalysis(p) = self
            && let Some(info_id) = &p.info_sheet_id
        {
            ids.push(info_id.clone());
        }
        ids.sort();
        ids.dedup();
        ids
    }
}

/// Stateful, mutable analysis node linking an upstream `DataSheet` (or analyzable `ResultSheet`)
/// to its [`crate::ResultSheet`].
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnalysisSpec {
    /// Project-unique identifier shared with the owning [`crate::ResultSheet`].
    pub id: SheetId,
    /// Upstream [`crate::DataSheet`] (or analyzable transform [`crate::ResultSheet`]) supplying input data.
    pub source_sheet_id: SheetId,
    /// Selected zero-based dataset/column indices (`empty` means analyze all columns on `source_sheet_id`).
    pub selected_data_sets: Vec<usize>,
    /// Dynamically derived default navigator title (`kind.derive_auto_title()`), updated whenever parameters change via [`Self::update_kind`].
    pub auto_title: String,
    /// Set only when the user manually renamed the sheet; preserved across parameter edits (`None` displays `auto_title`).
    pub custom_title: Option<String>,
    /// Validated two-sided confidence level in `[0.01%, 99.99%]` (default `95.0%`, `alpha = 0.05`) governing parameter CIs, effect-size CIs, limits of agreement, and curve bands.
    pub confidence_level: ConfidenceLevel,
    /// Publication formatting and asterisk-threshold preset applied to `ResultCell::PValue` entries.
    pub p_value_format: PValueFormat,
    /// Concrete statistical, regression, or transformation payload.
    pub kind: AnalysisKind,
}

impl AnalysisSpec {
    /// Construct a validated `AnalysisSpec` at 95% confidence and `FourDecimal` *p*-value formatting, rejecting empty/oversized sheet IDs or invalid analysis specs.
    pub fn new(id: impl Into<SheetId>, source_sheet_id: impl Into<SheetId>, kind: AnalysisKind) -> Result<Self, DocError> {
        let id = id.into();
        let source_sheet_id = source_sheet_id.into();
        let auto_title = kind.derive_auto_title();
        let spec = Self {
            id,
            source_sheet_id,
            selected_data_sets: Vec::new(),
            auto_title,
            custom_title: None,
            confidence_level: ConfidenceLevel::DEFAULT_95,
            p_value_format: PValueFormat::FourDecimal,
            kind,
        };
        spec.validate_structural()?;
        Ok(spec)
    }

    /// Validate structural invariants (sheet identifier bounds, optional custom title, finite floats,
    /// and [`PowerAnalysisSpec::validate_domain`] when configured for power analysis).
    pub fn validate_structural(&self) -> Result<(), DocError> {
        self.id.validate_non_empty_bounded()?;
        self.source_sheet_id.validate_non_empty_bounded()?;
        if let Some(custom) = &self.custom_title {
            validate_title(custom)?;
        }
        if self.first_non_finite_float().is_some() {
            return Err(DocError::InvalidCellValue {
                archetype: TableKind::Column,
                row: 0,
                col: 0,
                reason: "analysis specification contains a non-finite float",
            });
        }
        if let AnalysisKind::PowerAnalysis(ref p) = self.kind {
            p.validate_domain()?;
        }
        Ok(())
    }

    /// Perform full analysis validation, including strict [`PowerAnalysisSpec::validate`].
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_structural()?;
        if let AnalysisKind::PowerAnalysis(ref p) = self.kind {
            p.validate()?;
        }
        Ok(())
    }

    /// Mutate analysis parameters in-place while validating domain invariants (`try` counterpart to [`Self::update_kind`]).
    pub fn try_update_kind(&mut self, new_kind: AnalysisKind) -> Result<(), DocError> {
        let mut probe = self.clone();
        probe.auto_title = new_kind.derive_auto_title();
        probe.kind = new_kind;
        probe.validate_structural()?;
        *self = probe;
        Ok(())
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
        new_id.validate_non_empty_bounded()?;
        self.source_sheet_id = new_id;
        self.selected_data_sets = selected_data_sets;
        Ok(())
    }
}

impl<'de> serde::Deserialize<'de> for AnalysisSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawAnalysisSpec {
            id: SheetId,
            source_sheet_id: SheetId,
            selected_data_sets: Vec<usize>,
            auto_title: String,
            custom_title: Option<String>,
            confidence_level: ConfidenceLevel,
            p_value_format: PValueFormat,
            kind: AnalysisKind,
        }

        let raw = RawAnalysisSpec::deserialize(deserializer)?;
        let spec = Self {
            id: raw.id,
            source_sheet_id: raw.source_sheet_id,
            selected_data_sets: raw.selected_data_sets,
            auto_title: raw.auto_title,
            custom_title: raw.custom_title,
            confidence_level: raw.confidence_level,
            p_value_format: raw.p_value_format,
            kind: raw.kind,
        };
        spec.validate_structural().map_err(serde::de::Error::custom)?;
        Ok(spec)
    }
}

// ============================================================================
// Column-Title Numeric Hook Parser
// ============================================================================

/// Failure returned by [`parse_column_title_numeric_hook`] when a data-set column
/// title does not specify an unambiguous finite numeric scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnTitleHookError {
    /// Column title was empty or whitespace-only.
    EmptyTitle,
    /// Explicit `lhs = rhs` assignment syntax was malformed or missing a valid identifier on the left-hand side.
    InvalidAssignment {
        /// Original column title.
        title: String,
    },
    /// Column title did not begin with an unambiguous numeric literal (for example, embedded digits or hyphens inside identifiers such as `"Inhibitor-10 nM"`, `"IL-6 10 ng/mL"`, or `"CO2 5%"`).
    MissingNumericValue {
        /// Original column title.
        title: String,
    },
    /// Parsed numeric literal was `NaN`, `±Inf`, or overflowed `f64`.
    NonFiniteValue {
        /// Original column title.
        title: String,
    },
    /// Trailing text after the numeric scalar was not a valid unit token.
    InvalidTrailingUnit {
        /// Original column title.
        title: String,
    },
}

impl std::fmt::Display for ColumnTitleHookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("column title is empty and has no numeric hook value"),
            Self::InvalidAssignment { title } => {
                write!(f, "column title `{title}` contains an invalid numeric hook assignment")
            }
            Self::MissingNumericValue { title } => {
                write!(f, "column title `{title}` does not start with an unambiguous finite numeric value")
            }
            Self::NonFiniteValue { title } => {
                write!(f, "column title `{title}` contains a non-finite numeric value")
            }
            Self::InvalidTrailingUnit { title } => {
                write!(f, "column title `{title}` has an invalid or ambiguous trailing unit")
            }
        }
    }
}

impl std::error::Error for ColumnTitleHookError {}

/// Strictly parse an unambiguous finite numeric scalar from a column header title
/// (`HookSource::ColumnTitleValue` and `ParameterScope::ColumnTitleConstant`).
///
/// Accepts:
/// - Standalone finite numbers (`"10"`, `"-2.5"`, `"1e-7"`),
/// - Finite numbers followed by a trailing unit (`"10 nM"`, `"25.0 uM"`, `"5%"`), and
/// - Explicit assignments (`"[I] = 10 nM"`, `"Conc = 2.5 uM"`).
///
/// Rejects embedded hyphens or digits inside un-assigned identifiers and ranges
/// (`"Inhibitor-10 nM"`, `"IL-6 10 ng/mL"`, `"CO2 5%"`, `"Hot1.5"`, `"10-20 nM"`),
/// multiple numeric tokens (`"10 20"`), empty titles, and non-finite literals (`"NaN"`, `"inf"`).
///
/// # Errors
/// Returns [`ColumnTitleHookError`] if `title` is empty, malformed, ambiguous, or non-finite.
pub fn parse_column_title_numeric_hook(title: &str) -> Result<f64, ColumnTitleHookError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(ColumnTitleHookError::EmptyTitle);
    }

    let value_part = if let Some((lhs, rhs)) = trimmed.split_once('=') {
        if rhs.contains('=') || !is_valid_hook_assignment_lhs(lhs) {
            return Err(ColumnTitleHookError::InvalidAssignment { title: title.to_owned() });
        }
        let rhs_trimmed = rhs.trim();
        if rhs_trimmed.is_empty() {
            return Err(ColumnTitleHookError::MissingNumericValue { title: title.to_owned() });
        }
        rhs_trimmed
    } else {
        trimmed
    };

    parse_strict_number_with_unit(value_part, title)
}

fn is_valid_hook_assignment_lhs(lhs: &str) -> bool {
    let l = lhs.trim();
    if l.is_empty() || l.parse::<f64>().is_ok() || !l.chars().any(char::is_alphabetic) {
        return false;
    }
    let Some(first) = l.chars().next() else {
        return false;
    };
    if first != '[' && !first.is_alphabetic() {
        return false;
    }
    let mut bracket_depth: i32 = 0;
    for ch in l.chars() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth -= 1;
                if bracket_depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    bracket_depth == 0
}

fn parse_strict_number_with_unit(expr: &str, original_title: &str) -> Result<f64, ColumnTitleHookError> {
    let (head, tail_opt) = match expr.split_once(char::is_whitespace) {
        Some((h, t)) => (h, Some(t.trim())),
        None => (expr, None),
    };
    let num_token = head.strip_suffix('%').unwrap_or(head);
    let Ok(val) = num_token.parse::<f64>() else {
        return Err(ColumnTitleHookError::MissingNumericValue { title: original_title.to_owned() });
    };
    if !val.is_finite() {
        return Err(ColumnTitleHookError::NonFiniteValue { title: original_title.to_owned() });
    }
    if let Some(tail) = tail_opt
        && !tail.is_empty()
    {
        for tok in tail.split_whitespace() {
            let first_ok = tok.chars().next().is_some_and(|c| c.is_alphabetic() || matches!(c, '%' | '°'));
            if !first_ok || tok.contains('=') || tok.chars().any(|c| c.is_ascii_digit()) {
                return Err(ColumnTitleHookError::InvalidTrailingUnit { title: original_title.to_owned() });
            }
        }
    }
    Ok(if val == 0.0 { 0.0 } else { val })
}
