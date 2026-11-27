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
pub mod summary_matrix;

pub use descriptor::*;
pub use summary_matrix::*;

const fn default_col_b() -> usize {
    1
}

const fn default_true() -> bool {
    true
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

// ============================================================================
// Shared Model Formula Specification (`GlmSpec`, `CoxProportionalHazards`, `FactorialAnovaSpec`)
// ============================================================================

/// Single term in a linear predictor formula shared by [`GlmSpec`],
/// [`SurvivalAnalysisSpec::CoxProportionalHazards`], and [`FactorialAnovaSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FormulaTerm {
    /// Main effect of zero-based predictor column index `0`.
    Main(usize),
    /// Two-way interaction between predictor column indices `(a, b)`.
    Interaction2(usize, usize),
    /// Three-way interaction among predictor column indices `(a, b, c)`.
    Interaction3(usize, usize, usize),
    /// Polynomial power transform `X_column^degree` (for example `degree = 2` or `3`).
    Power {
        /// Zero-based continuous predictor column index.
        column: usize,
        /// Positive integer exponent (`>= 2`).
        degree: u8,
    },
    /// Square-root transform `sqrt(X_col)` of a non-negative continuous column.
    Sqrt(usize),
}

/// Explicit linear-predictor formula specification shared across generalized linear models,
/// Cox proportional hazards models, and selective-term factorial ANOVA.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ModelFormulaSpec {
    /// When `true` (default), includes an intercept column in the design matrix (ignored by Cox PH baseline hazard).
    #[serde(default = "default_true")]
    pub include_intercept: bool,
    /// Ordered model terms compiled into the design matrix.
    #[serde(default)]
    pub terms: Vec<FormulaTerm>,
}

impl Default for ModelFormulaSpec {
    fn default() -> Self {
        Self { include_intercept: true, terms: Vec::new() }
    }
}

// ============================================================================
// Multiplicity & FDR Specifications (`docs/implementation/statistics-implementation.md` §4)
// ============================================================================

/// Familywise error-rate (FWER) or false-discovery-rate (FDR) procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MultiplicityMethod {
    /// No post-hoc multiple-comparisons adjustment (unadjusted pairwise comparisons or omnibus test only).
    #[default]
    #[serde(alias = "none")]
    None,
    /// Tukey's Honestly Significant Difference all-pairwise Studentized range test under pooled MSE (Tukey 1953; Kramer 1956 for unequal sample sizes).
    #[serde(alias = "tukey", alias = "tukey_hsd")]
    TukeyHsd,
    /// Dunnett's many-to-one test comparing every treatment group against a designated control column under pooled MSE (Dunnett 1955).
    Dunnett {
        /// Zero-based index of the reference control group column within the analyzed groups.
        control_column: usize,
    },
    /// Single-step Šídák FWER adjustment `p_adj = 1 - (1 - p)^m` assuming positive orthant dependence / independence (Šídák 1967).
    #[serde(alias = "sidak")]
    Sidak,
    /// Classical single-step Bonferroni FWER bound `p_adj = min(1, m * p)` valid under arbitrary dependence (Dunn 1961).
    #[serde(alias = "bonferroni")]
    Bonferroni,
    /// Sequential step-down Holm–Šídák FWER procedure (`1 - (1 - p_(i))^(m - i + 1)` with monotonicity enforcement; Holm 1979; Šídák 1967).
    #[serde(alias = "holm_sidak")]
    HolmSidak,
    /// Dunnett's T3 all-pairwise Studentized maximum modulus test for unequal variances and small sample sizes using Welch–Satterthwaite *df* (Dunnett 1980).
    #[serde(alias = "dunnett_t3")]
    DunnettT3,
    /// Games–Howell all-pairwise Studentized range test for heteroscedastic groups with Welch–Satterthwaite *df* (Games & Howell 1976).
    #[serde(alias = "games_howell")]
    GamesHowell,
    /// Tamhane's T2 conservative all-pairwise Šídák-*t* test for unequal variances using Welch–Satterthwaite *df* (Tamhane 1979).
    #[serde(alias = "tamhane_t2")]
    TamhaneT2,
    /// Dunn's nonparametric rank-sum post-hoc comparisons following Kruskal–Wallis or Friedman tests with Bonferroni adjustment (Dunn 1964).
    #[serde(alias = "dunn", alias = "dunn_rank_sum")]
    DunnRankSum,
    /// Orthogonal linear polynomial trend contrast across ordered columns (`1 df` linear slope test; Zar 2010).
    #[serde(alias = "linear_trend")]
    LinearTrend,
    /// Benjamini–Hochberg linear step-up False Discovery Rate procedure emitting *q*-values (Benjamini & Hochberg 1995).
    #[serde(alias = "benjamini_hochberg", alias = "bh", alias = "fdr")]
    BenjaminiHochbergFdr,
    /// Benjamini–Yekutieli conservative harmonic-sum FDR procedure valid under arbitrary test dependence (Benjamini & Yekutieli 2001).
    #[serde(alias = "benjamini_yekutieli", alias = "by")]
    BenjaminiYekutieliFdr,
    /// Two-stage adaptive linear step-up Benjamini–Krieger–Yekutieli FDR procedure estimating `pi_0` (Benjamini, Krieger, & Yekutieli 2006).
    #[serde(alias = "benjamini_krieger_yekutieli", alias = "bky")]
    TwoStageBkyFdr,
    /// Student–Newman–Keuls stepwise range test (Newman 1939; Keuls 1952; does not strongly control FWER for `> 3` groups and omits simultaneous CIs).
    #[serde(alias = "newman_keuls", alias = "student_newman_keuls", alias = "snk")]
    NewmanKeuls,
    /// Unadjusted Fisher's Least Significant Difference pairwise *t* comparisons using the omnibus residual MSE and *df* (Fisher 1935).
    #[serde(alias = "fisher_lsd", alias = "lsd", alias = "uncorrected_fisher_lsd")]
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
    #[serde(alias = "ln_y", alias = "ln")]
    Ln,
    /// Base-10 logarithm `log10(u)` (maps `u <= 0` to empty).
    #[serde(alias = "log10_y", alias = "log10")]
    Log10,
    /// Base-2 logarithm `log2(u)` (maps `u <= 0` to empty).
    #[serde(alias = "log2_y", alias = "log2")]
    Log2,
    /// Base-10 antilogarithm `10^u`.
    #[serde(alias = "exp10_y", alias = "exp10", alias = "10_pow_y")]
    Exp10,
    /// Natural exponential `exp(u)`.
    #[serde(alias = "exp_y", alias = "exp", alias = "e_pow_y")]
    Exp,
    /// Multiplicative inverse `1 / u` (maps `u == 0` to empty).
    #[serde(alias = "reciprocal_y", alias = "reciprocal", alias = "1/y")]
    Reciprocal,
    /// Principal square root `sqrt(u)` (maps `u < 0` to empty).
    #[serde(alias = "sqrt")]
    Sqrt,
    /// Quadratic square `u^2`.
    #[serde(alias = "square")]
    Square,
    /// Unit-proportion log-odds `ln(u / (1 - u))` for `u in (0, 1)`.
    #[serde(alias = "logit")]
    Logit,
    /// Finney's +5 probit `5 + Phi^{-1}(u)` (inverse standard normal CDF plus 5) for unit proportions `u in (0, 1)`.
    #[serde(alias = "probit")]
    Probit,
    /// Angular variance-stabilizing Freeman–Tukey / Bliss transform `arcsin(sqrt(u))` in radians for proportions `u in [0, 1]` (Zar 2010).
    #[serde(alias = "arcsin_sqrt")]
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
    #[serde(alias = "asymmetric", alias = "replicate_endpoints")]
    ReplicateEndpoints,
    /// First-order Taylor delta-method approximation `SD(g(Y)) ~= |g'(mean)| * SD(Y)`.
    #[serde(alias = "delta", alias = "delta_method", alias = "first_order_delta")]
    FirstOrderDeltaMethod,
}

/// Anchor value for `[0%, 100%]` normalization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NormalizeAnchor {
    /// Minimum active value observed in the series (or across the whole table when `per_data_set == false`).
    #[serde(alias = "smallest", alias = "min", alias = "column_min", alias = "columnmin")]
    ColumnMin,
    /// Maximum active value observed in the series (or across the whole table when `per_data_set == false`).
    #[serde(alias = "largest", alias = "max", alias = "column_max", alias = "columnmax")]
    ColumnMax,
    /// Sum of active values in the series (for fraction-of-total scaling when paired with `FixedValue(0.0)`).
    #[serde(alias = "sum", alias = "column_sum", alias = "columnsum")]
    ColumnSum,
    /// Active value (or replicate mean) in the first non-empty row (`row == 0`).
    #[serde(alias = "first", alias = "first_row", alias = "firstrow")]
    FirstRow,
    /// Active value (or replicate mean) in the last non-empty row.
    #[serde(alias = "last", alias = "last_row", alias = "lastrow")]
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
    #[serde(alias = "difference", alias = "subtract")]
    Difference,
    /// Multiplicative fold ratio `Y / Baseline` (maps `Baseline == 0` to empty).
    #[serde(alias = "ratio", alias = "divide")]
    Ratio,
    /// Percentage change `100 * (Y - Baseline) / Baseline`.
    #[serde(alias = "percent", alias = "percent_change", alias = "pct_change", alias = "pctchange", alias = "percentchange")]
    PercentChange,
    /// Unitless fractional change `(Y - Baseline) / Baseline`.
    #[serde(alias = "fraction", alias = "fractional_change", alias = "fractionalchange")]
    FractionalChange,
    /// Additive combination `Y + Baseline`.
    #[serde(alias = "add", alias = "sum")]
    Add,
    /// Multiplicative scaling `Y * Baseline`.
    #[serde(alias = "multiply", alias = "product")]
    Multiply,
}

/// Generalized baseline source for [`TransformAnalysisSpec::RemoveBaseline`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BaselineKind {
    /// Row-by-row mean across `columns`.
    ColumnsRowWise(Vec<usize>),
    /// Single grand mean across `columns` and all rows.
    ColumnsMean(Vec<usize>),
    /// Constant scalar baseline.
    Constant(f64),
    /// Per-column mean over inclusive row slice `start_row..=end_row`.
    RowRangeMean {
        /// Zero-based inclusive starting row index of the baseline window.
        start_row: usize,
        /// Zero-based inclusive ending row index of the baseline window (`end_row >= start_row`).
        end_row: usize,
    },
    /// Per-column linear baseline interpolated between active-observation centroids `(x_start, y_start)`
    /// across rows `0..first_k` and `(x_end, y_end)` across rows `(row_count - last_k)..row_count`.
    FirstAndLastLinear {
        /// Number of leading rows (`>= 1`) whose active observations define `(x_start, y_start)`.
        first_k: usize,
        /// Number of trailing rows (`>= 1`) whose active observations define `(x_end, y_end)`.
        last_k: usize,
    },
    /// Paired every-other-column baseline matching adjacent column pairs `(2k, 2k + 1)`.
    PairedEveryOtherColumn,
    /// Per-column ordinary least-squares linear baseline fitted against *X* over the first
    /// `first_k` and last `last_k` rows.
    OlsLinearWithX {
        /// Number of leading rows (`>= 1`) included in the OLS baseline fit.
        first_k: usize,
        /// Number of trailing rows (`>= 1`) included in the OLS baseline fit.
        last_k: usize,
    },
}

/// Classical linearizing pharmacology and enzyme-kinetics XY transforms.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PharmacologyTransformPreset {
    /// Lineweaver–Burk double-reciprocal transform `(1/X, 1/Y)`.
    #[serde(alias = "lineweaver_burk")]
    LineweaverBurk,
    /// Eadie–Hofstee linearizing transform `(Y/X, Y)`.
    #[serde(alias = "eadie_hofstee")]
    EadieHofstee,
    /// Hanes–Woolf linearizing transform `(X, X/Y)`.
    #[serde(alias = "hanes_woolf")]
    HanesWoolf,
    /// Scatchard binding transform `(Y, Y/X)` (`Bound` vs `Bound/Free`).
    #[serde(alias = "scatchard")]
    Scatchard,
    /// Hill linearizing transform `(log10(X), log10(Y / (Y_max - Y)))`.
    Hill {
        /// Optional upper asymptote `Y_max`; `None` uses the observed series maximum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y_max: Option<f64>,
    },
}

/// Policy controlling how pre-summarized confidence-interval error bars propagate under nonlinear transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum AsymmetricCiTransformPolicy {
    /// Delegate to `error_propagation` (`ReplicateEndpoints` or `FirstOrderDeltaMethod`).
    #[default]
    #[serde(alias = "default")]
    Default,
    /// Preserve exact asymmetric confidence limits `[g(lower), g(upper)]`, reversing order on monotone decreasing transforms.
    #[serde(alias = "asymmetric_endpoints", alias = "exact_endpoints")]
    AsymmetricEndpoints,
    /// Omit transformed confidence-interval limits when nonlinear scaling invalidates symmetric spread.
    #[serde(alias = "omit")]
    Omit,
}

/// Sampling constraint design for `2 × 2` contingency-table Monte Carlo simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ContingencySimDesign {
    /// Multinomial sampling with fixed grand total `N` and cell probabilities `[p00, p01, p10, p11]`.
    #[default]
    #[serde(alias = "fixed_grand_total")]
    FixedGrandTotal,
    /// Independent binomial sampling per row with fixed row totals `[n_row0, n_row1]`.
    #[serde(alias = "fixed_row_totals")]
    FixedRowTotals,
    /// Independent binomial sampling per column with fixed column totals `[n_col0, n_col1]`.
    #[serde(alias = "fixed_column_totals")]
    FixedColumnTotals,
    /// Independent Poisson draws for each cell given expected counts `[lambda00, lambda01, lambda10, lambda11]`.
    #[serde(alias = "poisson_counts", alias = "poisson")]
    PoissonCounts,
}

/// Parameters for deterministic Monte Carlo simulation of a `2 × 2` contingency table.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ContingencySimulationSpec {
    /// Sampling design constraint (`FixedGrandTotal`, `FixedRowTotals`, `FixedColumnTotals`, or `PoissonCounts`).
    #[serde(default)]
    pub design: ContingencySimDesign,
    /// Row-major cell probabilities or expected Poisson rates `[a, b, c, d]`.
    #[serde(default)]
    pub cell_params: [f64; 4],
    /// Fixed grand total `[N]` or marginal totals `[m0, m1]` (unused by `PoissonCounts`).
    #[serde(default)]
    pub fixed_totals: Vec<usize>,
}

/// Parameters for deterministic Monte Carlo simulation of a multi-group [`TableKind::Column`] table
/// with optional random subject effects and outlier contamination.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ColumnSimulationSpec {
    /// True population means `mu_g` for each simulated column.
    #[serde(default)]
    pub column_means: Vec<f64>,
    /// Optional dynamic [`HookSource`] mean overrides aligned with `column_means`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooked_means: Vec<Option<HookSource>>,
    /// Within-group residual standard deviation (`>= 0`).
    #[serde(default)]
    pub within_sd: f64,
    /// Optional subject/row random-effect standard deviation (`>= 0`) inducing matched-row correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random_effect_sd: Option<f64>,
    /// Fraction of simulated cells in `[0, 1]` contaminated as outliers.
    #[serde(default)]
    pub outlier_fraction: f64,
    /// Additive outlier shift expressed in multiples of `within_sd`.
    #[serde(default)]
    pub outlier_shift_sd: f64,
}

/// Target table archetype and generative model for [`TransformAnalysisSpec::Simulate`].
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum SimulationMode {
    /// Classic XY parametric curve simulation (default).
    #[default]
    XyCurve,
    /// Simulated `2 × 2` contingency count table.
    Contingency2x2(ContingencySimulationSpec),
    /// Simulated multi-group [`TableKind::Column`] table with optional random effects and outliers.
    Column(ColumnSimulationSpec),
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
        /// Optional biochemical/pharmacology linearizing transform preset applied to `(X, Y)` pairs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pharmacology_preset: Option<PharmacologyTransformPreset>,
        /// When `true`, exchanges *X* and *Y* coordinates during the transform.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        interchange_xy: bool,
        /// Optional finite substitute replacing `X == 0` prior to logarithmic *X* transforms (for example `1e-12`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zero_x_substitute: Option<f64>,
        /// Policy governing asymmetric confidence-interval propagation under nonlinear transforms.
        #[serde(default, skip_serializing_if = "is_default")]
        asymmetric_ci_policy: AsymmetricCiTransformPolicy,
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
        /// Algebraic baseline combination rule (`Difference`, `Ratio`, `PercentChange`, `FractionalChange`, `Add`, or `Multiply`).
        operation: BaselineOperation,
        /// Optional generalized baseline definition overriding `baseline_columns` (`None` uses `BaselineKind::ColumnsRowWise(baseline_columns)`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline_kind: Option<BaselineKind>,
        /// When `true`, preserves unselected columns unchanged in the output table instead of dropping them.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        retain_unselected_columns: bool,
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
        /// When `true`, drops rows with any missing/excluded value across selected variables during extraction.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        complete_rows_only: bool,
        /// Optional zero-based subject/block identifier column aligning matched subjects across groups.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject_column: Option<usize>,
        /// Optional zero-based column supplying custom row titles on the rearranged output table.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row_title_column: Option<usize>,
        /// Optional zero-based frequency/count column when extracting a [`TableKind::Contingency`] table.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contingency_count_column: Option<usize>,
        /// When `true`, prefixes rearranged column/row labels with source variable names.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        include_variable_name_prefixes: bool,
    },
    /// Deterministic Monte Carlo XY curve, `2 × 2` contingency, or column simulation using `curvebook-math::DeterministicRng`.
    Simulate {
        /// 64-bit SplitMix64/Xoshiro256** PRNG seed ensuring bit-reproducible simulated replicates across platforms.
        seed: u64,
        /// Number of *X* grid rows (`>= 1`) spaced linearly across `[x_min, x_max]` (or rows per column in `Column` mode).
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
        /// Optional simulation target mode (`None` or `Some(SimulationMode::XyCurve)` uses XY curve simulation).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        simulation_mode: Option<SimulationMode>,
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

/// Per-row summary statistic family emitted by [`RowStatisticsSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RowSummaryKind {
    /// Row arithmetic mean and sample SD (with active replicate count `N`).
    #[default]
    #[serde(alias = "mean_sd")]
    MeanSd,
    /// Row arithmetic mean and SEM (with active replicate count `N`).
    #[serde(alias = "mean_sem")]
    MeanSem,
    /// Row arithmetic mean and coefficient of variation `%CV` (with active replicate count `N`).
    #[serde(alias = "mean_cv")]
    MeanCv,
    /// Row arithmetic mean and two-sided Student-*t* confidence interval at [`AnalysisSpec::confidence_level`].
    #[serde(alias = "mean_ci")]
    MeanCi,
    /// Row arithmetic mean with minimum and maximum bounds.
    #[serde(alias = "mean_min_max")]
    MeanMinMax,
    /// Row sum (total) across active replicates.
    #[serde(alias = "total", alias = "sum")]
    Total,
    /// Row median with Q1/Q3 quartiles and IQR.
    #[serde(alias = "median_quartiles")]
    MedianQuartiles,
    /// Row median with minimum and maximum bounds.
    #[serde(alias = "median_min_max")]
    MedianMinMax,
    /// Row median with symmetric custom percentiles (`100 - upper_percentile` and `upper_percentile`).
    #[serde(alias = "median_percentiles")]
    MedianPercentiles,
    /// Row geometric mean and geometric SD factor over strictly positive replicates.
    #[serde(alias = "geometric_mean_sd", alias = "geomean_sd")]
    GeometricMeanSd,
    /// Row geometric mean and back-transformed confidence interval over strictly positive replicates.
    #[serde(alias = "geometric_mean_ci", alias = "geomean_ci")]
    GeometricMeanCi,
}

/// Row-wise replicate summary statistics specification (`crates/transform`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RowStatisticsSpec {
    /// Summary statistic family computed across each row's active replicates.
    pub summary: RowSummaryKind,
    /// Optional upper percentile in `(50.0, 100.0)` used by [`RowSummaryKind::MedianPercentiles`] (defaults to `90.0` when `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_percentile: Option<f64>,
}

impl Default for RowStatisticsSpec {
    fn default() -> Self {
        Self { summary: RowSummaryKind::MeanSd, upper_percentile: None }
    }
}

/// Denominator basis for [`FractionOfTotalSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum FractionDenominator {
    /// Divide each cell by its column total.
    #[default]
    #[serde(alias = "column", alias = "column_total")]
    ColumnTotal,
    /// Divide each cell by its row total.
    #[serde(alias = "row", alias = "row_total")]
    RowTotal,
    /// Divide each cell by the table grand total.
    #[serde(alias = "grand", alias = "grand_total", alias = "total")]
    GrandTotal,
    /// Compute column, row, and grand-total fractions in separate output tabs.
    #[serde(alias = "all", alias = "all_three")]
    AllThree,
}

/// Binomial confidence-interval method for integer-count [`FractionOfTotalSpec`] outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ProportionCiMethod {
    /// Omit binomial proportion confidence intervals.
    #[default]
    #[serde(alias = "none")]
    None,
    /// Wilson (1927) score interval for binomial proportions.
    #[serde(alias = "wilson")]
    Wilson,
    /// Exact Clopper–Pearson (1934) beta-quantile binomial confidence interval.
    #[serde(alias = "clopper_pearson", alias = "exact")]
    ClopperPearson,
}

/// Fraction-of-total reduction specification (`crates/transform`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FractionOfTotalSpec {
    /// Normalization denominator (`ColumnTotal`, `RowTotal`, `GrandTotal`, or `AllThree`).
    pub denominator: FractionDenominator,
    /// When `true`, outputs percentages (`0..=100`); when `false`, outputs unit fractions (`0..=1`).
    pub output_percent: bool,
    /// Optional binomial proportion confidence-interval method for non-negative integer count tables.
    pub ci_method: ProportionCiMethod,
}

impl Default for FractionOfTotalSpec {
    fn default() -> Self {
        Self { denominator: FractionDenominator::ColumnTotal, output_percent: true, ci_method: ProportionCiMethod::None }
    }
}

/// Numerical differentiation, cumulative integration, or smoothing operator on XY curves.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum CurveCalculusOp {
    /// First numerical derivative `dY/dX` along ordered *X*.
    #[default]
    #[serde(alias = "first_derivative", alias = "derivative")]
    FirstDerivative,
    /// Second numerical derivative `d^2Y/dX^2` along ordered *X*.
    #[serde(alias = "second_derivative")]
    SecondDerivative,
    /// Cumulative trapezoidal integral `int_{x_0}^x (Y(t) - baseline) dt`.
    CumulativeIntegral {
        /// Horizontal reference baseline subtracted prior to integration (default `0.0`).
        #[serde(default)]
        baseline: f64,
    },
    /// Savitzky–Golay polynomial local smoothing filter (Savitzky & Golay 1964).
    SmoothSavitzkyGolay {
        /// Odd smoothing neighborhood width (`>= 3`).
        window: usize,
        /// Polynomial degree (`< window`, commonly `2` or `3`).
        order: usize,
    },
    /// Centered unweighted moving-average smoothing over `window` adjacent points.
    SmoothMovingAverage {
        /// Positive smoothing window width (`>= 2`).
        window: usize,
    },
    /// Cleveland (1979) locally weighted scatterplot smoothing (LOWESS).
    SmoothLowess {
        /// Neighborhood span fraction in `(0, 1]`.
        span: f64,
        /// Number of Tukey bisquare robustness iterations (default `3`).
        #[serde(default = "default_lowess_robustness_iters")]
        robustness_iters: usize,
    },
}

/// XY curve differentiation, cumulative integration, and smoothing specification (`crates/transform::curve`).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CurveCalculusSpec {
    /// Numerical differentiation, integration, or smoothing operator applied to each selected XY series.
    pub operation: CurveCalculusOp,
}

/// Comparison predicate applied to a tracked scalar in [`MonteCarloHitRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MonteCarloHitOp {
    /// Strictly less than `threshold` (`value < threshold`).
    #[default]
    #[serde(alias = "lt", alias = "less_than")]
    LessThan,
    /// Less than or equal to `threshold` (`value <= threshold`).
    #[serde(alias = "le", alias = "less_or_equal")]
    LessOrEqual,
    /// Strictly greater than `threshold` (`value > threshold`).
    #[serde(alias = "gt", alias = "greater_than")]
    GreaterThan,
    /// Greater than or equal to `threshold` (`value >= threshold`).
    #[serde(alias = "ge", alias = "greater_or_equal")]
    GreaterOrEqual,
    /// Inclusive interval predicate (`threshold <= value <= upper_threshold`).
    #[serde(alias = "between")]
    Between,
}

/// Downstream result scalar recorded on every iteration of a [`MonteCarloSpec`] run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MonteCarloTrackedScalar {
    /// Output column label for the tracked scalar in the Monte Carlo summary tabs.
    pub label: String,
    /// Downstream `ResultSheet` supplying this scalar (`None` uses [`MonteCarloSpec::downstream_result_sheet_id`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sheet_id: Option<SheetId>,
    /// Scalar key looked up via [`crate::ResultSheet::lookup_scalar`].
    pub parameter_key: String,
    /// Optional zero-based dataset index within the downstream result sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_set_index: Option<usize>,
}

/// Hit classification predicate evaluated against one entry in [`MonteCarloSpec::tracked_scalars`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MonteCarloHitRule {
    /// Zero-based index into [`MonteCarloSpec::tracked_scalars`].
    pub scalar_index: usize,
    /// Comparison operator (`LessThan`, `LessOrEqual`, `GreaterThan`, `GreaterOrEqual`, or `Between`).
    pub op: MonteCarloHitOp,
    /// Primary cutoff (or lower bound for [`MonteCarloHitOp::Between`]).
    pub threshold: f64,
    /// Inclusive upper bound when `op == MonteCarloHitOp::Between`; `None` for single-sided operators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_threshold: Option<f64>,
}

const fn default_mc_iterations() -> usize {
    1_000
}

/// Chained Monte Carlo simulation specification orchestrated by `curvebook-engine` (`L5`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MonteCarloSpec {
    /// Deterministic 64-bit master PRNG seed deriving reproducible per-iteration seeds.
    pub seed: u64,
    /// Number of Monte Carlo iterations (`1..=Self::MAX_ITERATIONS`, default `1_000`).
    #[serde(default = "default_mc_iterations")]
    pub iterations: usize,
    /// Downstream `ResultSheet` re-evaluated on each simulated dataset; `None` summarizes simulated data only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downstream_result_sheet_id: Option<SheetId>,
    /// Ordered downstream scalars recorded on every iteration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracked_scalars: Vec<MonteCarloTrackedScalar>,
    /// Conjunction of predicates partitioning simulated iterations into `Hits`, `Not Hits`, and `All`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hit_rules: Vec<MonteCarloHitRule>,
    /// When `true`, skips iterations whose downstream nonlinear fit was labeled `"Flagged"`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_flagged_fits: bool,
}

impl MonteCarloSpec {
    /// Hard upper safety limit on iterations per Monte Carlo execution (`100_000`).
    pub const MAX_ITERATIONS: usize = 100_000;
}

impl Default for MonteCarloSpec {
    fn default() -> Self {
        Self {
            seed: 42,
            iterations: default_mc_iterations(),
            downstream_result_sheet_id: None,
            tracked_scalars: Vec::new(),
            hit_rules: Vec::new(),
            exclude_flagged_fits: false,
        }
    }
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
    /// When `true`, runs as a standalone Normality & Lognormality analysis omitting univariate descriptive moment tables.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub normality_only: bool,
    /// When `true`, emits Normal and Lognormal Q–Q coordinate tabs alongside normality/lognormality test tables.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emit_qq_tabs: bool,
}

impl Default for DescriptiveSpec {
    fn default() -> Self {
        Self {
            aggregation: SubcolumnAggregation::RowMeans,
            custom_upper_percentile: None,
            run_normality_battery: true,
            hypothetical_value: None,
            normality_only: false,
            emit_qq_tabs: false,
        }
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
    /// One-sample ratio *t*-test on `ln(Y / hypothetical)` across strictly positive observations, reporting the geometric mean ratio and back-transformed CI.
    OneSampleRatioT {
        /// Strictly positive null geometric mean / hypothetical value (`> 0.0`).
        hypothetical: f64,
    },
    /// Unpaired Welch unequal-variance *t*-test on natural-log-transformed strictly positive observations (`ln(Y)`), reporting the geometric mean ratio and back-transformed CI.
    LognormalWelchT,
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

/// Lognormal or ratio transformation mode for row-wise [`MultipleRowTestsSpec`] comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LognormalRowTestMode {
    /// Analyze raw untransformed observations (default).
    #[default]
    #[serde(alias = "none")]
    None,
    /// Analyze natural-log-transformed strictly positive observations (`ln(Y)`) and back-transform geometric mean ratios.
    #[serde(alias = "lognormal")]
    Lognormal,
    /// Perform paired ratio *t*-tests on `ln(B / A)` across strictly positive matched subcolumns.
    #[serde(alias = "ratio_paired", alias = "ratio")]
    RatioPaired,
}

impl LognormalRowTestMode {
    /// Return `true` when `self` is [`Self::None`].
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
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
    /// Optional lognormal or ratio-paired log-space transformation mode (defaults to `LognormalRowTestMode::None`).
    #[serde(default, skip_serializing_if = "LognormalRowTestMode::is_none")]
    pub lognormal_mode: LognormalRowTestMode,
}

impl Default for MultipleRowTestsSpec {
    fn default() -> Self {
        Self {
            paired: false,
            nonparametric: false,
            assume_equal_sd: true,
            multiplicity: MultiplicitySpec::default(),
            lognormal_mode: LognormalRowTestMode::None,
        }
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
    #[serde(alias = "bonferroni")]
    Bonferroni,
    /// Step-down Holm–Bonferroni FWER procedure `(m - i + 1) * p_(i)` (Holm 1979).
    #[serde(alias = "holm", alias = "holm_bonferroni", alias = "holm_1979")]
    HolmBonferroni,
    /// Single-step Šídák FWER bound `1 - (1 - p)^m` (Šídák 1967).
    #[serde(alias = "sidak", alias = "šidák", alias = "šídák", alias = "sidak_1967")]
    Sidak,
    /// Step-down Holm–Šídák FWER procedure `1 - (1 - p_(i))^(m - i + 1)` (Holm 1979; Šídák 1967).
    #[serde(alias = "holm_sidak", alias = "holm_šidák", alias = "holm_šídák")]
    HolmSidak,
    /// Benjamini–Hochberg linear step-up FDR *q*-values (Benjamini & Hochberg 1995).
    #[default]
    #[serde(alias = "bh", alias = "benjamini_hochberg", alias = "benjamini_hochberg_fdr", alias = "fdr_bh")]
    BenjaminiHochberg,
    /// Benjamini–Krieger–Yekutieli two-stage adaptive FDR *q*-values (Benjamini, Krieger, & Yekutieli 2006).
    #[serde(alias = "bky", alias = "two_stage_bky", alias = "two_stage_bky_fdr", alias = "benjamini_krieger_yekutieli", alias = "bky_2006")]
    TwoStageBky,
    /// Benjamini–Yekutieli harmonic-factor FDR *q*-values valid under arbitrary dependence (Benjamini & Yekutieli 2001).
    #[serde(alias = "by", alias = "benjamini_yekutieli", alias = "benjamini_yekutieli_fdr", alias = "fdr_by")]
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

/// Centre used for the absolute deviations `|x_ij - c_i|` in a standalone Levene-type
/// homogeneity-of-variance test (`R19.2`; `crates/stats::variance_homogeneity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LeveneCenter {
    /// Group mean `c_i = mean(x_i)` (Levene 1960; Olkin et al., *Contributions to Probability and Statistics*, pp. 278–292).
    #[serde(alias = "mean", alias = "levene", alias = "levene_mean")]
    Mean,
    /// Group median `c_i = median(x_i)` (Brown & Forsythe 1974, *JASA* 69(346):364–367), robust to skewed
    /// and heavy-tailed data and therefore the default.
    #[default]
    #[serde(alias = "median", alias = "brown_forsythe", alias = "levene_median")]
    Median,
}

impl LeveneCenter {
    /// Both centring choices in display order (default first).
    pub const ALL: [Self; 2] = [Self::Median, Self::Mean];

    /// Canonical method label for result tables, methods reports, and parameter dialogs.
    #[must_use]
    pub const fn method_label(self) -> &'static str {
        match self {
            Self::Mean => "Levene (mean-centered)",
            Self::Median => "Brown-Forsythe (median-centered Levene)",
        }
    }

    /// Short lowercase centre name (`"mean"` or `"median"`).
    #[must_use]
    pub const fn center_name(self) -> &'static str {
        match self {
            Self::Mean => "mean",
            Self::Median => "median",
        }
    }
}

/// Reference baseline policy for [`AreaUnderCurveSpec`] (`R17.7` / `R19.2`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AucBaseline {
    /// Constant horizontal reference *Y* baseline (default `0.0`).
    Fixed {
        /// Finite baseline *Y* value.
        value: f64,
    },
    /// Use each series's *Y* value at the start of its active integration window
    /// (`x_min`, linearly interpolated if between sampled knots; first finite point when `x_min` is omitted).
    FirstPoint,
}

impl Default for AucBaseline {
    fn default() -> Self {
        Self::Fixed { value: 0.0 }
    }
}

/// Standalone trapezoidal Area Under the Curve (AUC) and peak-detection specification (`R17.7` / `R19.2`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AreaUnderCurveSpec {
    /// Horizontal reference baseline (default [`AucBaseline::Fixed`] at `0.0`).
    #[serde(default)]
    pub baseline: AucBaseline,
    /// Optional lower *X* integration bound (linearly interpolated when falling between sampled knots).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_min: Option<f64>,
    /// Optional upper *X* integration bound (linearly interpolated when falling between sampled knots).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_max: Option<f64>,
    /// Whether replicate time-point means come from independent cohorts/subjects (Bailer 1988).
    /// Default `false`: leaves `NetAreaSe` blank on longitudinal / repeated-measures assays.
    #[serde(default)]
    pub independent_time_points: bool,
    /// Minimum peak height above baseline required to retain a peak (default `0.0`).
    #[serde(default)]
    pub min_peak_height: f64,
    /// Minimum fraction of positive area in `[0.0, 1.0]` required to retain a peak (default `0.0`).
    #[serde(default)]
    pub min_area_fraction: f64,
}

impl Default for AreaUnderCurveSpec {
    fn default() -> Self {
        Self {
            baseline: AucBaseline::default(),
            x_min: None,
            x_max: None,
            independent_time_points: false,
            min_peak_height: 0.0,
            min_area_fraction: 0.0,
        }
    }
}

/// Standalone Levene / Brown–Forsythe homogeneity-of-variance test over the selected columns of a
/// raw-replicate `Column` or `MultipleVariables` table (`R19.2`).
///
/// Each selected column is one group; the test is a one-way ANOVA *F* on `|x_ij - c_i|`
/// (Levene 1960; Brown & Forsythe 1974).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct LeveneTestSpec {
    /// Deviation centre (default [`LeveneCenter::Median`], the Brown–Forsythe variant).
    #[serde(default)]
    pub center: LeveneCenter,
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
    /// Ordinary homoscedastic one-way ANOVA on natural-log-transformed strictly positive observations (`ln(Y)`), back-transforming geometric mean ratios.
    #[serde(alias = "lognormal_ordinary", alias = "lognormal_one_way_anova")]
    LognormalOrdinary,
    /// Heteroscedastic Brown–Forsythe and Welch one-way ANOVA on natural-log-transformed strictly positive observations (`ln(Y)`).
    #[serde(alias = "lognormal_welch_bf", alias = "lognormal_welch_anova")]
    LognormalWelchBf,
    /// Repeated-measures one-way ANOVA (or REML mixed model) on natural-log-transformed strictly positive observations (`ln(Y)`).
    LognormalRepeatedMeasures {
        /// When `true`, multiplies omnibus `(df_treatment, df_residual)` by the Greenhouse–Geisser sphericity estimate `epsilon_hat`.
        #[serde(default)]
        geisser_greenhouse: bool,
    },
}

/// Denominator degrees-of-freedom approximation for REML mixed-effects ANOVA and nested models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MixedModelDfMethod {
    /// Small-sample adjusted `Phi_A` covariance and scaled `F*` statistic (Kenward & Roger, 1997).
    #[default]
    #[serde(alias = "kenward_roger", alias = "kenwardroger", alias = "kr")]
    KenwardRoger,
    /// Satterthwaite moment-matched denominator *df* using unadjusted Wald covariance `Phi` (Fai & Cornelius, 1996).
    #[serde(alias = "satterthwaite", alias = "sat", alias = "fai_cornelius")]
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
    #[serde(alias = "none", alias = "no_matching")]
    None,
    /// Subjects correspond to subcolumns within each column group and are measured repeatedly across row-factor levels.
    #[serde(alias = "row", alias = "row_factor", alias = "row_factor_matched")]
    RowFactorMatched,
    /// Subjects correspond to `(row, subcol)` slots and are measured repeatedly across column-factor data sets.
    #[serde(alias = "column", alias = "col", alias = "column_factor", alias = "column_factor_matched")]
    ColumnFactorMatched,
    /// Doubly repeated-measures design where each subcolumn `s` is the same subject measured across every `(row, column)` combination.
    #[serde(alias = "both", alias = "both_factors", alias = "both_factors_matched")]
    BothFactorsMatched,
}

/// Residual covariance structure fitted by REML for repeated-measures mixed models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ResidualCovariance {
    /// Random-intercept / compound-symmetry covariance with equal variance and correlation across timepoints.
    #[default]
    #[serde(alias = "compound_symmetry", alias = "compoundsymmetry", alias = "cs", alias = "random_intercept")]
    CompoundSymmetry,
    /// Unstructured positive-definite `t x t` covariance matrix (`t(t + 1) / 2` parameters via Cholesky REML).
    #[serde(alias = "unstructured", alias = "un", alias = "general")]
    Unstructured,
}

/// Sphericity epsilon adjustment applied to repeated-measures ANOVA *F*-test *p*-values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RmSphericityCorrection {
    /// Uncorrected standard ANOVA *F*-test degrees of freedom assuming exact sphericity (`epsilon = 1`).
    #[default]
    #[serde(alias = "none")]
    None,
    /// Multiply numerator and denominator degrees of freedom by Greenhouse–Geisser `epsilon_hat` (Greenhouse & Geisser 1959).
    #[serde(alias = "greenhouse_geisser", alias = "geisser_greenhouse", alias = "gg")]
    GreenhouseGeisser,
    /// Multiply numerator and denominator degrees of freedom by Huynh–Feldt / Lecoutre `epsilon_tilde` (Huynh & Feldt 1976; Lecoutre 1991).
    #[serde(alias = "huynh_feldt", alias = "hf")]
    HuynhFeldt,
}

/// Policy for handling zero or negative random-effect variance components in REML mixed models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RemlZeroVariancePolicy {
    /// Constrain random-effect variance components to be `>= 0` (standard bounded REML, default).
    #[default]
    #[serde(alias = "bound_at_zero", alias = "bounded")]
    BoundAtZero,
    /// Drop random-effect terms whose variance component converges to zero and refit the reduced model.
    #[serde(alias = "drop_zero_random_term", alias = "drop")]
    DropZeroRandomTerm,
    /// Allow unconstrained negative variance component estimates.
    #[serde(alias = "unbounded")]
    Unbounded,
    /// Fall back to ordinary ANOVA / legacy moment-based EMS when REML hits the zero boundary.
    #[serde(alias = "legacy_fallback", alias = "fallback")]
    LegacyFallback,
}

/// Family partitioning mode for post-hoc multiple comparisons in factorial ANOVA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum FactorialComparisonFamilyMode {
    /// Standard simple-effects / main-effects comparison families (default).
    #[default]
    #[serde(alias = "standard")]
    Standard,
    /// Compare only cell means differing in exactly one factor level.
    #[serde(alias = "cells_differing_by_one_factor", alias = "one_factor_diff")]
    CellsDifferingByOneFactor,
    /// Treat each simple-effect slice (per row or per column) as its own comparison family.
    #[serde(alias = "per_slice_families", alias = "per_slice")]
    PerSliceFamilies,
    /// Partition comparisons into separate families per two-factor margin.
    #[serde(alias = "two_factor_families")]
    TwoFactorFamilies,
    /// Pool all pairwise comparisons across the factorial table into one grand family.
    #[serde(alias = "single_grand_family", alias = "grand_family")]
    SingleGrandFamily,
    /// Compare only explicit cell index pairs listed in [`FactorialAnovaSpec::preselected_pairs`].
    #[serde(alias = "preselected_pairs", alias = "preselected")]
    PreselectedPairs,
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
    /// Sphericity epsilon adjustment (`None`, `GreenhouseGeisser`, or `HuynhFeldt`) applied to RM factorial *F*-tests.
    #[serde(default, skip_serializing_if = "is_default")]
    pub sphericity_correction: RmSphericityCorrection,
    /// Policy when REML mixed-effects variance components converge to zero or negative values.
    #[serde(default, skip_serializing_if = "is_default")]
    pub reml_zero_variance_policy: RemlZeroVariancePolicy,
    /// Family partitioning rule for post-hoc factorial multiple comparisons.
    #[serde(default, skip_serializing_if = "is_default")]
    pub comparison_family_mode: FactorialComparisonFamilyMode,
    /// Explicit `(cell_a, cell_b)` index pairs evaluated when `comparison_family_mode == FactorialComparisonFamilyMode::PreselectedPairs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preselected_pairs: Vec<(usize, usize)>,
    /// When `true`, reverses pairwise contrast subtraction order (`B - A` instead of `A - B`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub swap_direction: bool,
    /// When `true`, runs Spearman rank heteroscedasticity testing of `|residual|` vs predicted values.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heteroscedasticity_test: bool,
    /// Optional explicit selective-term [`ModelFormulaSpec`] overriding `max_interaction_order` on `MultipleVariables` factorial ANOVA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<ModelFormulaSpec>,
    /// Optional explicit three-way repeated-measures flags `[factor_0, factor_1, factor_2]` (`[factor1_rm, factor2_rm, row_rm]` on `GroupedTable`; matching `multifactor_columns[0..3]` on `MultipleVariablesTable`) covering all 8 three-way RM topologies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub three_way_rm_factors: Option<[bool; 3]>,
    /// When `true`, emits a consolidated tidy data tab for downstream inspection/graphing of three-way tables.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emit_consolidated_data_tab: bool,
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
            sphericity_correction: RmSphericityCorrection::None,
            reml_zero_variance_policy: RemlZeroVariancePolicy::BoundAtZero,
            comparison_family_mode: FactorialComparisonFamilyMode::Standard,
            preselected_pairs: Vec::new(),
            swap_direction: false,
            heteroscedasticity_test: false,
            formula: None,
            three_way_rm_factors: None,
            emit_consolidated_data_tab: false,
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
    #[serde(alias = "unweighted", alias = "none")]
    Unweighted,
    /// Cicchetti–Allison linear ordinal disagreement weights `w_{ij} = 1 - |i - j| / (k - 1)` (Cohen 1968).
    #[serde(alias = "linear")]
    Linear,
    /// Fleiss–Cohen quadratic ordinal disagreement weights `w_{ij} = 1 - ((i - j) / (k - 1))^2` (Fleiss, Cohen, & Everitt 1969).
    #[serde(alias = "quadratic")]
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
    #[serde(alias = "g2", alias = "likelihood_ratio_g2")]
    LikelihoodRatioG2,
    /// Fisher's exact conditional hypergeometric test (two-sided minimum-likelihood sum on `2 × 2` and Freeman–Halton network generalization on `R × C`; Fisher 1935; Freeman & Halton 1951).
    #[serde(alias = "fisher", alias = "fisher_exact")]
    FisherExact,
    /// Barnard's unconditional exact CSM/Boschloo-style supremum test over nuisance proportion `pi in (0, 1)` on `2 × 2` tables (Barnard 1945).
    #[serde(alias = "barnard", alias = "barnard_exact")]
    BarnardExact,
    /// Cochran–Armitage `1 df` linear trend test in proportions across ordered columns of a `2 × C` or `R × 2` table (Cochran 1954; Armitage 1955).
    #[serde(alias = "trend", alias = "chi_square_for_trend", alias = "cochran_armitage", alias = "cochran_armitage_trend")]
    CochranArmitageTrend,
    /// McNemar's paired discordance test on matched `2 × 2` tables with exact binomial and mid-*p* values plus discordant odds ratio `(b / c)` (McNemar 1947).
    #[serde(alias = "mcnemar", alias = "mcnemar_paired")]
    McNemarPaired,
    /// Two-rater Cohen's kappa inter-rater agreement on square `k × k` tables with asymptotic Fleiss–Cohen–Everitt standard error (Cohen 1960, 1968).
    CohenKappa {
        /// Nominal (`Unweighted`) vs ordinal (`Linear` or `Quadratic`) disagreement weights.
        #[serde(default)]
        weighting: KappaWeighting,
    },
    /// Multi-rater Fleiss' kappa across `N` subjects (rows) and `k` categories (columns) with constant or row-sum rater counts (Fleiss 1971).
    #[serde(alias = "fleiss_kappa")]
    FleissKappa,
    /// Exact two-sided Clopper–Pearson / Sterne one-sample binomial test against [`ContingencySpec::null_proportion`] (Clopper & Pearson 1934).
    #[serde(alias = "exact_binomial", alias = "binomial")]
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
    #[serde(alias = "breslow")]
    Breslow,
    /// Efron (1977) tied-event partial likelihood approximation (default; closely matches exact discrete partial likelihood under moderate ties).
    #[default]
    #[serde(alias = "efron")]
    Efron,
    /// Exact discrete partial likelihood summing over all `C(|R_i|, d_i)` subsets of tied failures (Cox 1972; Kalbfleisch & Prentice 2002).
    #[serde(alias = "exact")]
    Exact,
}

/// Handling policy for status column values matching neither `event_code` nor `censored_code` in survival analyses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SurvivalOtherValuePolicy {
    /// Ignore/exclude observations whose status value matches neither code (default).
    #[default]
    #[serde(alias = "exclude", alias = "ignore")]
    Exclude,
    /// Treat unrecognized status values as right-censored.
    #[serde(alias = "censor", alias = "censored")]
    Censor,
    /// Treat unrecognized status values as events/failures.
    #[serde(alias = "event", alias = "failure")]
    Event,
}

/// Confidence-interval calculation method for GLM and Cox proportional hazards regression parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RegressionCiMethod {
    /// Symmetric asymptotic Wald confidence interval `beta_hat ± z * SE` (default).
    #[default]
    #[serde(alias = "wald")]
    Wald,
    /// Asymmetrical Venzon–Moolgavkar profile-likelihood confidence interval.
    #[serde(alias = "profile_likelihood", alias = "profile")]
    ProfileLikelihood,
}

/// Named covariate profile used to predict survival curves `S(t | x_0)` from a Cox proportional hazards model.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct CovariateProfile {
    /// Human-readable profile label displayed in output curve headers.
    pub label: String,
    /// Explicit `(column_index, value)` assignments for continuous covariates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub numeric_values: Vec<(usize, f64)>,
    /// Explicit `(column_index, level_label)` assignments for categorical covariates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categorical_values: Vec<(usize, String)>,
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
        /// Optional custom numeric code representing an observed event (defaults to `1.0` when `None`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_code: Option<f64>,
        /// Optional custom numeric code representing right-censoring (defaults to `0.0` when `None`; swapping with `event_code` enables reverse-KM follow-up).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        censored_code: Option<f64>,
        /// Policy applied to numeric status values matching neither `event_code` nor `censored_code`.
        #[serde(default, skip_serializing_if = "is_default")]
        treat_other_as: SurvivalOtherValuePolicy,
        /// When `true`, reports the ratio of median survival times and its confidence interval for two-group comparisons.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ratio_median_ci: bool,
        /// Optional multiple status column indices for `MultipleVariables` multi-endpoint Kaplan–Meier analysis.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mv_status_columns: Vec<usize>,
        /// Optional multiple grouping column indices for stratified `MultipleVariables` Kaplan–Meier analysis.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mv_group_columns: Vec<usize>,
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
        /// Optional explicit [`ModelFormulaSpec`] supporting 2-/3-way interactions and nonlinear predictor terms.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        formula: Option<ModelFormulaSpec>,
        /// Optional covariate profiles for predicted survival curves `S_hat(t | x_0)`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        covariate_profiles: Vec<CovariateProfile>,
        /// Optional reduced nested [`ModelFormulaSpec`] evaluated via partial-likelihood ratio test against the primary model.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nested_comparison: Option<ModelFormulaSpec>,
        /// When `true`, emits Cox–Snell and covariate residual diagnostic tabs.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        emit_cox_snell_residuals: bool,
        /// Confidence-interval method (`Wald` or `ProfileLikelihood`) for hazard ratios and coefficients.
        #[serde(default, skip_serializing_if = "is_default")]
        ci_method: RegressionCiMethod,
        /// Optional custom numeric code representing an observed event (defaults to `1.0` when `None`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_code: Option<f64>,
        /// Optional custom numeric code representing right-censoring (defaults to `0.0` when `None`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        censored_code: Option<f64>,
        /// Policy applied to numeric status values matching neither `event_code` nor `censored_code`.
        #[serde(default, skip_serializing_if = "is_default")]
        treat_other_as: SurvivalOtherValuePolicy,
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
    #[serde(alias = "linear")]
    Linear,
    /// Binomial Bernoulli error (`Y in {0, 1}`) with canonical logit link `ln(p / (1 - p)) = X beta` fitted by IRLS, reporting odds ratios `exp(beta_j)` and Hosmer–Lemeshow / Tjur `R^2` diagnostics.
    #[serde(alias = "logistic")]
    Logistic,
    /// Poisson count error (`Y in {0, 1, 2, ...}`) with canonical log link `ln(E[Y | X]) = X beta` fitted by IRLS, reporting rate ratios `exp(beta_j)` and Pearson/deviance dispersion diagnostics.
    #[serde(alias = "poisson")]
    Poisson,
}

/// Observation weighting specification for [`GlmSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum GlmWeightSpec {
    /// Unweighted estimation (default).
    #[default]
    None,
    /// Analytical response/predictor weighting scheme (`1/Y^2`, `1/|Y|`, `1/|Y|^k`, `1/|X|`, `1/X^2`).
    Scheme(WeightingScheme),
    /// Explicit zero-based non-negative weight column index on the source sheet.
    Column(usize),
}

/// Dispersion / covariance estimation mode for [`GlmSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum GlmDispersionMode {
    /// Canonical model dispersion (`phi = 1` for Logistic/Poisson, residual MSE for Linear; default).
    #[default]
    #[serde(alias = "canonical")]
    Canonical,
    /// Quasi-Poisson Pearson chi-square dispersion scaling (`phi_hat = X^2 / df`).
    #[serde(alias = "quasi_poisson", alias = "quasipoisson")]
    QuasiPoisson,
    /// Heteroscedasticity-consistent sandwich covariance estimator (Huber–White HC).
    #[serde(alias = "robust_sandwich", alias = "sandwich", alias = "hc")]
    RobustSandwich,
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
    /// Optional explicit [`ModelFormulaSpec`] supporting intercept toggle, 2-/3-way interactions, powers, and square roots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<ModelFormulaSpec>,
    /// Optional reduced nested [`ModelFormulaSpec`] compared against the primary model via Extra-SS *F* (Linear) or deviance LRT (Logistic/Poisson) and `Delta AICc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_comparison: Option<ModelFormulaSpec>,
    /// Observation weighting specification (`None`, analytical `WeightingScheme`, or explicit weight column).
    #[serde(default, skip_serializing_if = "is_default")]
    pub weights: GlmWeightSpec,
    /// Dispersion / covariance estimation mode (`Canonical`, `QuasiPoisson`, or `RobustSandwich`).
    #[serde(default, skip_serializing_if = "is_default")]
    pub dispersion_mode: GlmDispersionMode,
    /// Optional classification probability cutoff in `(0, 1)` for Logistic GLMs (`None` defaults to `0.5`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_cutoff: Option<f64>,
    /// Confidence-interval method (`Wald` or `ProfileLikelihood`) for regression parameters and odds/rate ratios.
    #[serde(default, skip_serializing_if = "is_default")]
    pub ci_method: RegressionCiMethod,
    /// When `true`, emits row-level leverage `h_ii`, Cook's `D`, DFBETAs, and standardized/studentized/deviance residuals.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emit_influence_diagnostics: bool,
}

impl Default for GlmSpec {
    fn default() -> Self {
        Self {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: Vec::new(),
            reference_levels: Vec::new(),
            compute_vif: true,
            formula: None,
            nested_comparison: None,
            weights: GlmWeightSpec::None,
            dispersion_mode: GlmDispersionMode::Canonical,
            classification_cutoff: None,
            ci_method: RegressionCiMethod::Wald,
            emit_influence_diagnostics: false,
        }
    }
}

/// Pairwise distance metric for hierarchical and multivariate clustering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterDistance {
    /// L2 Euclidean distance `sqrt(sum_k (x_{ik} - x_{jk})^2)`.
    #[default]
    #[serde(alias = "euclidean", alias = "l2")]
    Euclidean,
    /// L1 city-block Manhattan distance `sum_k |x_{ik} - x_{jk}|`.
    #[serde(alias = "manhattan", alias = "l1")]
    Manhattan,
    /// Correlation dissimilarity `1 - r_{ij}` based on Pearson's correlation across features.
    #[serde(alias = "pearson", alias = "correlation")]
    Pearson,
    /// Angular cosine dissimilarity `1 - (x_i . x_j) / (||x_i||_2 ||x_j||_2)`.
    #[serde(alias = "cosine")]
    Cosine,
}

/// Agglomerative linkage criterion for hierarchical clustering (Lance & Williams 1967).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterLinkage {
    /// Ward's minimum within-cluster variance increase (`Delta ESS`; Ward 1963).
    #[default]
    #[serde(alias = "ward", alias = "ward_d2", alias = "ward.d2")]
    Ward,
    /// Farthest-neighbor maximum pairwise inter-cluster distance.
    #[serde(alias = "complete")]
    Complete,
    /// Nearest-neighbor minimum pairwise inter-cluster distance.
    #[serde(alias = "single")]
    Single,
    /// Unweighted Pair Group Method with Arithmetic Mean (`|A| * |B|`-weighted average pairwise distance; Sokal & Michener 1958).
    #[serde(alias = "upgma", alias = "average")]
    Upgma,
    /// Weighted Pair Group Method with Arithmetic Mean (equal `0.5` weight to each merged branch).
    #[serde(alias = "wpgma", alias = "weighted_average")]
    Wpgma,
    /// Squared/Euclidean distance between cluster centroids (UPGMC).
    #[serde(alias = "centroid", alias = "upgmc")]
    Centroid,
    /// Gower's unweighted midpoint centroid linkage (WPGMC; Gower 1967).
    #[serde(alias = "median", alias = "wpgmc")]
    Median,
}

/// Partitioning algorithm for *k*-means clustering inside [`MultivariateSpec::Clustering`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum KMeansAlgorithm {
    /// Classical Lloyd (1982) batch assignment and centroid update iterations (default).
    #[default]
    #[serde(alias = "lloyd")]
    Lloyd,
    /// Hartigan–Wong (1979) live-transfer greedy local search minimizing within-cluster SSE.
    #[serde(alias = "hartigan_wong", alias = "hw")]
    HartiganWong,
    /// MacQueen (1967) incremental online centroid update algorithm.
    #[serde(alias = "macqueen", alias = "mac_queen")]
    MacQueen,
}

/// Clustering orientation for hierarchical clustering inside [`MultivariateSpec::Clustering`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ClusterDirection {
    /// Cluster rows / observations (default).
    #[default]
    #[serde(alias = "rows")]
    Rows,
    /// Cluster columns / variables.
    #[serde(alias = "columns", alias = "cols")]
    Columns,
    /// Cluster both rows and columns independently (biclustering / clustered heatmap).
    #[serde(alias = "both")]
    Both,
}

/// Component retention criterion for Principal Component Analysis ([`MultivariateSpec::Pca`]).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum PcaRetentionRule {
    /// Default Kaiser (`lambda > 1` when standardized) or Horn parallel-analysis rule when a seed is provided.
    #[default]
    KaiserOrParallel,
    /// Retain all principal components with eigenvalue strictly greater than the specified threshold.
    EigenvalueCutoff(f64),
    /// Retain an explicit fixed number of leading principal components (`>= 1`).
    FixedCount(usize),
    /// Retain the smallest prefix of components explaining at least the target cumulative variance percentage in `(0, 100]`.
    CumulativeVariancePercent(f64),
    /// Retain every non-trivial principal component.
    SelectAll,
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
        /// Principal component retention rule (`KaiserOrParallel`, `EigenvalueCutoff`, `FixedCount`, `CumulativeVariancePercent`, or `SelectAll`).
        #[serde(default, skip_serializing_if = "is_default")]
        retention_rule: PcaRetentionRule,
        /// When `true`, emits additional diagnostic matrix tabs (standardized data, covariance/correlation, squared cosines, and contributions).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        emit_extra_matrices: bool,
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
        /// When `true`, evaluates the 17-index consensus optimal-*k* suite (including Tibshirani et al. 2001 Gap statistic) and emits cluster metrics tabs.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        consensus_optimal_k: bool,
        /// Partitioning algorithm used for *k*-means (`Lloyd`, `HartiganWong`, or `MacQueen`).
        #[serde(default, skip_serializing_if = "is_default")]
        kmeans_algorithm: KMeansAlgorithm,
        /// Orientation for hierarchical clustering (`Rows`, `Columns`, or `Both`).
        #[serde(default, skip_serializing_if = "is_default")]
        cluster_direction: ClusterDirection,
        /// Optional dendrogram cut height extracting flat cluster assignments from hierarchical merges.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cut_height: Option<f64>,
        /// When `true`, emits a one-way ANOVA feature-separation summary tab across *k*-means clusters.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        emit_anova_tab: bool,
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

/// Measurement-error variance specification for Model II Deming regression
/// (Deming 1943; Linnet 1990, 1993).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum DemingErrorSpec {
    /// Equal measurement-error variances in *X* and *Y* (`lambda = sigma_x^2 / sigma_y^2 = 1.0`, orthogonal / major-axis Deming regression).
    #[default]
    #[serde(alias = "equal_error", alias = "equal")]
    EqualError,
    /// Explicit measurement-error standard deviations for *X* and *Y*, yielding `lambda = (sd_x / sd_y)^2`.
    StandardDeviations {
        /// Measurement-error standard deviation of *X* (`sd_x > 0`).
        sd_x: f64,
        /// Measurement-error standard deviation of *Y* (`sd_y > 0`).
        sd_y: f64,
    },
    /// Explicit error variance ratio `lambda = sigma_x^2 / sigma_y^2` (`lambda > 0`).
    VarianceRatio {
        /// Ratio of *X* error variance to *Y* error variance (`sigma_x^2 / sigma_y^2 > 0`).
        lambda: f64,
    },
}

impl DemingErrorSpec {
    /// Returns `true` when this error model is the default [`Self::EqualError`].
    #[must_use]
    pub const fn is_equal_error(&self) -> bool {
        matches!(self, Self::EqualError)
    }

    /// Resolve the positive finite error variance ratio `lambda = sigma_x^2 / sigma_y^2`.
    ///
    /// Returns `None` when any supplied standard deviation or variance ratio is non-finite or `<= 0.0`,
    /// or when `(sd_x / sd_y)^2` overflows or underflows to `0.0`.
    #[must_use]
    pub fn resolved_lambda(self) -> Option<f64> {
        match self {
            Self::EqualError => Some(1.0),
            Self::StandardDeviations { sd_x, sd_y } => {
                if !sd_x.is_finite() || !sd_y.is_finite() || sd_x <= 0.0 || sd_y <= 0.0 {
                    return None;
                }
                let ratio = sd_x / sd_y;
                let lambda = ratio * ratio;
                (lambda.is_finite() && lambda > 0.0).then_some(lambda)
            }
            Self::VarianceRatio { lambda } => (lambda.is_finite() && lambda > 0.0).then_some(lambda),
        }
    }
}

/// Simple ordinary least-squares (Model I) or Deming orthogonal/errors-in-variables (Model II) linear regression specification (`crates/fit::linear`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearRegressionSpec {
    /// When `true`, fits Deming (1943) Model II regression accounting for measurement error in both *X* and *Y* (configured via [`Self::deming_error`], defaulting to equal-variance ratio `lambda = 1`) with jackknife SEs; when `false`, fits ordinary least-squares `Y = intercept + slope * X` (plus ANCOVA slope/intercept comparison across `>= 2` datasets).
    pub deming_model_ii: bool,
    /// When `true`, constrains `intercept = 0.0` (`Y = slope * X`, `df = n - 1`). Retained for backward compatibility; superseded by [`Self::fixed_point`] when `Some((x0, y0))`.
    pub force_through_origin: bool,
    /// Optional arbitrary fixed-point constraint `(X0, Y0)` forcing `Y - Y0 = slope * (X - X0)` (`df = n - 1`). Takes precedence over [`Self::force_through_origin`] when `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_point: Option<(f64, f64)>,
    /// When `true`, reports standard-addition unknown concentration `-X_intercept` with negated, order-swapped Fieller (1940) confidence limits `[-U, -L]`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub standard_addition: bool,
    /// When `true`, extends the fitted regression line plot range to include the estimated *X*-intercept (`Y = 0`) when finite.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extend_line_to_x_intercept: bool,
    /// Measurement-error variance specification for Model II Deming regression (`EqualError` by default).
    #[serde(default, skip_serializing_if = "DemingErrorSpec::is_equal_error")]
    pub deming_error: DemingErrorSpec,
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

    /// Effective fixed-point constraint `(X0, Y0)`, resolving [`Self::fixed_point`] first and falling back to `(0.0, 0.0)` when [`Self::force_through_origin`] is `true`.
    #[must_use]
    pub const fn effective_fixed_point(&self) -> Option<(f64, f64)> {
        if let Some(pt) = self.fixed_point {
            Some(pt)
        } else if self.force_through_origin {
            Some((0.0, 0.0))
        } else {
            None
        }
    }
}

impl Default for LinearRegressionSpec {
    fn default() -> Self {
        Self {
            deming_model_ii: false,
            force_through_origin: false,
            fixed_point: None,
            standard_addition: false,
            extend_line_to_x_intercept: false,
            deming_error: DemingErrorSpec::EqualError,
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

/// Standalone simple logistic regression specification on single-subcolumn `XY` tables with
/// binary outcomes `Y in {0, 1}` (`crates/stats::simple_logistic`; Hosmer, Lemeshow & Sturdivant 2013;
/// Tjur 2009; Heinze & Schemper 2002).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SimpleLogisticSpec {
    /// When `true` (default), includes a `Row Classification` output tab listing observed `(X_i, Y_i)`,
    /// predicted probabilities `P_hat(X_i)` with confidence intervals, and automatic predictions for
    /// appended `(X_new, blank Y)` rows.
    pub emit_row_classification: bool,
    /// When `true` (default), samples two-sided confidence bands around the fitted sigmoid probability
    /// curve at [`AnalysisSpec::confidence_level`].
    pub confidence_bands: bool,
    /// Classification probability threshold in `(0, 1)` used to classify predicted probabilities into
    /// `0`/`1` in the `Row Classification` tab and confusion summary (default `0.5`).
    pub classification_cutoff: f64,
    /// Optional fitted-curve starting *X*, overriding the automatic data-driven minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_x_min: Option<f64>,
    /// Optional fitted-curve ending *X*, overriding the automatic data-driven maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_x_max: Option<f64>,
    /// Optional number of sigmoid curve segments (`1..=10_000`; default `100`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_segments: Option<usize>,
}

impl SimpleLogisticSpec {
    /// Default number of segments for the fitted sigmoid probability curve.
    pub const DEFAULT_PLOT_SEGMENTS: usize = 100;
    /// Maximum allowed number of segments for the fitted sigmoid probability curve.
    pub const MAX_PLOT_SEGMENTS: usize = 10_000;
    /// Default classification probability threshold (`0.5`).
    pub const DEFAULT_CLASSIFICATION_CUTOFF: f64 = 0.5;
}

impl Default for SimpleLogisticSpec {
    fn default() -> Self {
        Self {
            emit_row_classification: true,
            confidence_bands: true,
            classification_cutoff: Self::DEFAULT_CLASSIFICATION_CUTOFF,
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
    #[serde(alias = "none", alias = "unweighted")]
    None,
    /// `1 / Y^2` (constant `%CV` proportional scatter).
    #[serde(alias = "relative_square", alias = "inverse_y2", alias = "1/y2", alias = "relative_1_y2")]
    RelativeSquare,
    /// `1 / |Y|` (Poisson-like variance proportional to mean response).
    #[serde(alias = "relative", alias = "inverse_y", alias = "1/y")]
    Relative,
    /// General power-of-*Y* weight `1 / |Y|^k` with caller-supplied exponent `k`.
    GeneralPowerY(f64),
    /// `1 / |X|` weighting inversely proportional to independent variable magnitude.
    #[serde(alias = "inverse_x", alias = "1/x")]
    InverseX,
    /// `1 / X^2` weighting inversely proportional to squared *X*.
    #[serde(alias = "inverse_x2", alias = "1/x2")]
    InverseX2,
    /// Empirical replicate variance weighting `1 / SD_i^2` across replicates at each row.
    #[serde(alias = "observed_sd")]
    ObservedSd,
}

/// Basis used to evaluate response-dependent weights (`RelativeSquare`, `Relative`, `GeneralPowerY`).
///
/// Reference: Carroll, R. J. & Ruppert, D. (1988). *Transformation and Weighting in Regression*. Chapman & Hall.
/// Reference: Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum WeightingSource {
    /// Static weights computed from observed responses `y_i` (legacy/default for backward compatibility).
    #[default]
    #[serde(alias = "observed_y", alias = "observed", alias = "data")]
    ObservedY,
    /// Iteratively reweighted least squares (IRLS) weights computed from fitted curve heights `y_hat_i = f(x_i; theta)`.
    #[serde(alias = "predicted_y", alias = "predicted", alias = "curve", alias = "y_curve", alias = "irls")]
    PredictedY,
}

/// Outlier detection and elimination policy for nonlinear regression (Motulsky & Brown 2006).
///
/// Reference: Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123. doi:10.1186/1471-2105-7-123
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum OutlierHandling {
    /// No separate ROUT outlier handling override (uses [`FitMethod`] directly).
    #[default]
    #[serde(alias = "none", alias = "off", alias = "disabled")]
    None,
    /// Identify and count ROUT outliers (emitting the outlier audit table and counts) while retaining all observations in the least-squares fit.
    #[serde(alias = "identify_and_count_only", alias = "count_only", alias = "identify_only", alias = "count")]
    IdentifyAndCountOnly,
    /// Identify and exclude ROUT outliers before refitting least squares on the retained inliers.
    #[serde(alias = "eliminate", alias = "remove", alias = "exclude")]
    Eliminate,
}

/// How replicate observations within each table row enter the nonlinear regression objective.
///
/// Reference: Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateFitMode {
    /// Treat each replicate subcell as an individual observation (or expand `(Mean, SD, n)` sufficient statistics with `n` degrees of freedom).
    #[default]
    #[serde(alias = "individual_replicates", alias = "individual", alias = "replicates", alias = "all")]
    IndividualReplicates,
    /// Collapse each row's active replicates to its unweighted sample mean with `1` observation per row (for technical replicates sharing one biological unit).
    #[serde(alias = "row_means_only", alias = "row_means", alias = "means_only", alias = "means")]
    RowMeansOnly,
}

/// Convergence strictness preset controlling the Levenberg–Marquardt stopping criterion.
///
/// Reference: Moré, J. J. (1978). The Levenberg–Marquardt algorithm: implementation and theory. *Lecture Notes in Mathematics* 630, 105–116. doi:10.1007/BFb0067700
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ConvergenceStrictness {
    /// Machine-precision MINPACK-1 `ftol`/`xtol`/`gtol` stopping criteria (default).
    #[default]
    #[serde(alias = "default", alias = "minpack", alias = "machine")]
    Default,
    /// Fast stopping: `2` consecutive accepted iterations changing objective SS by `< 0.01%` (`1e-4`).
    #[serde(alias = "quick", alias = "fast")]
    Quick,
    /// Balanced stopping: `5` consecutive accepted iterations changing objective SS by `< 0.0001%` (`1e-6`).
    #[serde(alias = "medium", alias = "standard")]
    Medium,
    /// Tight stopping: `5` consecutive accepted iterations changing objective SS by `< 1e-8%` (`1e-10`).
    #[serde(alias = "strict", alias = "tight")]
    Strict,
}

/// Regression objective function (`docs/implementation/curve-fitting-implementation.md` §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum FitMethod {
    /// Ordinary or weighted sum-of-squares minimization via trust-region Levenberg–Marquardt (Moré 1978).
    #[default]
    #[serde(alias = "ordinary", alias = "ols", alias = "least_squares")]
    LeastSquares,
    /// Exact Poisson log-likelihood deviance minimization `2 * sum (y_i ln(y_i / mu_i) - (y_i - mu_i))` for non-negative count responses `y_i >= 0` and `mu_i > 0` (McCullagh & Nelder 1989).
    #[serde(alias = "poisson", alias = "poisson_deviance")]
    PoissonDeviance,
    /// Outlier-resistant Lorentzian/Cauchy M-estimator minimizing `sum ln(1 + (r_i / RSDR)^2)` without deleting points (Motulsky & Brown 2006; asymptotic SEs/CIs are intentionally omitted under robust loss).
    #[serde(alias = "robust", alias = "cauchy")]
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

/// Parameter identifiability and ambiguity reporting mode for nonlinear regression
/// (`docs/implementation/curve-fitting-implementation.md` §2.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum IdentifiabilityMode {
    /// Assemble the converged objective Hessian `H` (`2 J^T W J` under WLS, Fisher information
    /// under Poisson deviance) and greedily delete rows/columns until `kappa(H) <= 10 / sqrt(eps_mach)`,
    /// marking dropped parameters `"Unstable"` while preserving well-conditioned parameters' SE/CIs.
    #[default]
    #[serde(alias = "identify_unstable_parameters", alias = "unstable")]
    IdentifyUnstableParameters,
    /// Legacy dependency criterion (`max_k Dependency_k > 0.9999` or rank-deficient Jacobian),
    /// labeling the fit `"Ambiguous"`, prefixing tangled estimates with `~`, and omitting CIs.
    #[serde(alias = "identify_ambiguous_fits", alias = "ambiguous")]
    IdentifyAmbiguousFits,
    /// Report raw converged parameter values unconditionally without unstable/ambiguous tagging.
    #[serde(alias = "report_all", alias = "neither", alias = "none")]
    ReportAll,
}

/// Statistical test criterion used to compare two candidate nonlinear models (§2.4, §2.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonMethod {
    /// Nested extra-sum-of-squares *F*-test (Motulsky & Christopoulos 2004).
    #[default]
    #[serde(alias = "extra_sum_of_squares_f", alias = "f_test", alias = "f")]
    ExtraSumOfSquaresF,
    /// Small-sample corrected Akaike Information Criterion (`AICc`, Burnham & Anderson 2002).
    #[serde(alias = "aicc", alias = "aic")]
    Aicc,
    /// Nested deviance / Gaussian likelihood-ratio chi-square test (Wilks 1938).
    #[serde(alias = "likelihood_ratio_test", alias = "lrt", alias = "likelihood_ratio")]
    LikelihoodRatioTest,
}

/// Scientific hypothesis question evaluated by nonlinear model comparison (§2.5.2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonQuestion {
    /// Compare the primary equation against a different candidate built-in equation.
    AlternativeEquation {
        /// Built-in equation ID of the competing/null model.
        alternative_equation_id: String,
    },
    /// Test whether selected parameters share a single global value across datasets (`H_0`)
    /// versus differing per dataset (`H_1`, e.g., parallel Hill slopes or shared plateaus).
    ParameterInvarianceAcrossDatasets {
        /// Equation parameter names constrained to [`ParameterScope::SharedGlobal`] under `H_0`
        /// and freed to [`ParameterScope::Local`] under `H_1`.
        shared_params: Vec<String>,
    },
    /// Test whether one global curve (all non-constant parameters [`ParameterScope::SharedGlobal`], `H_0`)
    /// fits all datasets as well as separate per-dataset curves (all non-constant parameters [`ParameterScope::Local`], `H_1`).
    SingleGlobalCurveVsSeparate,
    /// Test whether constraining a single parameter to a theoretical constant (`theta_p = c`, `H_0`,
    /// e.g., `HillSlope = 1.0`) degrades fit relative to leaving `param` free (`H_1`).
    ParameterEqualsTheoretical {
        /// Exact equation parameter name constrained under `H_0`.
        param: String,
        /// Theoretical null value `c` imposed under `H_0`.
        theoretical_value: f64,
    },
}

/// Automated 6-rule fit-quality flag criteria labeling dubious nonlinear fits `"Flagged"` (§2.5.4).
///
/// Each rule threshold is optional; a fit is labeled `"Flagged"` whenever any configured rule is violated.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct FitFlagRules {
    /// Flag when unweighted `R^2` is strictly below this threshold (in `[0, 1]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_r_squared: Option<f64>,
    /// Flag when the analyzed observation count `N` is strictly below this threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_observations: Option<usize>,
    /// Flag when `max_k Dependency_k` strictly exceeds this threshold (in `[0, 1]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_dependency: Option<f64>,
    /// Flag when any computed residual normality test *p*-value is strictly below this alpha (in `(0, 1)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_normality_p: Option<f64>,
    /// Flag when the runs test or replicates lack-of-fit *p*-value is strictly below this alpha (in `(0, 1)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_runs_or_lof_p: Option<f64>,
    /// Flag when the ROUT outlier count strictly exceeds this maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rout_outliers: Option<usize>,
    /// When `true`, exclude flagged replicates from batch/Monte Carlo parameter summary tabulations.
    #[serde(default)]
    pub exclude_flagged_from_summary: bool,
}

impl FitFlagRules {
    /// Return `true` when at least one of the 6 automated flag rules is configured.
    #[must_use]
    pub const fn has_any_rule(&self) -> bool {
        self.min_r_squared.is_some()
            || self.min_observations.is_some()
            || self.max_dependency.is_some()
            || self.min_normality_p.is_some()
            || self.min_runs_or_lof_p.is_some()
            || self.max_rout_outliers.is_some()
    }
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

impl ModelComparisonSpec {
    /// Statistical criterion represented by this legacy model-comparison specification.
    #[must_use]
    pub const fn method(&self) -> ModelComparisonMethod {
        match self {
            Self::ExtraSumOfSquaresF { .. } => ModelComparisonMethod::ExtraSumOfSquaresF,
            Self::Aicc { .. } => ModelComparisonMethod::Aicc,
            Self::LikelihoodRatioTest { .. } => ModelComparisonMethod::LikelihoodRatioTest,
        }
    }

    /// Alternative built-in equation ID stored in this legacy model-comparison specification.
    #[must_use]
    pub fn alternative_equation_id(&self) -> &str {
        match self {
            Self::ExtraSumOfSquaresF { alternative_equation_id }
            | Self::Aicc { alternative_equation_id }
            | Self::LikelihoodRatioTest { alternative_equation_id } => alternative_equation_id.as_str(),
        }
    }
}

/// Inequality relation enforced between two fitted parameters by [`RelationalConstraint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum RelationalConstraintOp {
    /// Strict ordering `lhs > rhs + margin`.
    #[default]
    #[serde(alias = "gt", alias = "greater_than")]
    GreaterThan,
    /// Non-strict ordering `lhs >= rhs + margin`.
    #[serde(alias = "ge", alias = "greater_or_equal")]
    GreaterOrEqual,
    /// Strict ordering `lhs < rhs - margin`.
    #[serde(alias = "lt", alias = "less_than")]
    LessThan,
    /// Non-strict ordering `lhs <= rhs - margin`.
    #[serde(alias = "le", alias = "less_or_equal")]
    LessOrEqual,
}

/// Relational inequality constraint between two named equation parameters (for example `KFast > KSlow`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RelationalConstraint {
    /// Left-hand parameter name matched against `ParametricModel::param_names`.
    pub lhs_param: String,
    /// Ordering relation between `lhs_param` and `rhs_param`.
    pub op: RelationalConstraintOp,
    /// Right-hand parameter name matched against `ParametricModel::param_names`.
    pub rhs_param: String,
    /// Non-negative separation margin `epsilon >= 0.0` (default `0.0`).
    #[serde(default)]
    pub margin: f64,
}

/// Theoretical probability distribution family evaluated by [`TheoreticalDistributionSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum TheoreticalDistributionFamily {
    /// Continuous Gaussian/normal distribution `N(mean, sd^2)`.
    #[default]
    #[serde(alias = "normal", alias = "gaussian")]
    Normal,
    /// Continuous Student's *t* distribution with `df` degrees of freedom.
    #[serde(alias = "student_t", alias = "t")]
    StudentT,
    /// Continuous Fisher–Snedecor *F* distribution with `(df1, df2)` degrees of freedom.
    #[serde(alias = "f", alias = "fisher_f")]
    FisherF,
    /// Continuous chi-square distribution with `df` degrees of freedom.
    #[serde(alias = "chi_square", alias = "chi2")]
    ChiSquare,
    /// Discrete binomial probability mass function `Binomial(n, p)`.
    #[serde(alias = "binomial")]
    Binomial,
    /// Discrete Poisson probability mass function `Poisson(lambda)`.
    #[serde(alias = "poisson")]
    Poisson,
}

/// Tail- or interval-shading region applied to continuous [`TheoreticalDistributionSpec`] curves.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum DistributionTailShade {
    /// No shaded tail region (default).
    #[default]
    None,
    /// Shade lower tail `X <= cutoff`.
    LowerTail {
        /// Upper boundary of the lower shaded region.
        cutoff: f64,
    },
    /// Shade upper tail `X >= cutoff`.
    UpperTail {
        /// Lower boundary of the upper shaded region.
        cutoff: f64,
    },
    /// Shade both outer tails `X <= lower` and `X >= upper`.
    TwoTailed {
        /// Upper boundary of the lower tail.
        lower: f64,
        /// Lower boundary of the upper tail.
        upper: f64,
    },
    /// Shade central interval `lower <= X <= upper`.
    Between {
        /// Inclusive lower endpoint.
        lower: f64,
        /// Inclusive upper endpoint.
        upper: f64,
    },
}

/// Theoretical continuous PDF/CDF or discrete PMF function-plotting specification (§6.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TheoreticalDistributionSpec {
    /// Probability distribution family (`Normal`, `StudentT`, `FisherF`, `ChiSquare`, `Binomial`, or `Poisson`).
    #[serde(default)]
    pub family: TheoreticalDistributionFamily,
    /// Ordered distribution parameters (`[mean, sd]`, `[df]`, `[df1, df2]`, `[n, p]`, or `[lambda]`).
    #[serde(default)]
    pub parameters: Vec<f64>,
    /// Optional dynamic [`HookSource`] overrides aligned with `parameters`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_hooks: Vec<Option<HookSource>>,
    /// Optional tail- or central-interval shading for continuous distributions.
    #[serde(default, skip_serializing_if = "is_default")]
    pub tail_shade: DistributionTailShade,
    /// When `true`, evaluates the cumulative distribution function (CDF) instead of the PDF/PMF.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cumulative: bool,
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
    /// Zero-based continuous dependent response column indices on [`TableKind::MultipleVariables`] sources (backward-compatible extension of `mv_y_column`; fits `M` responses x `G` groups).
    #[serde(default)]
    pub mv_y_columns: Vec<usize>,
    /// Basis for response-dependent weighting (`ObservedY` static weights vs `PredictedY` curve-height IRLS).
    #[serde(default)]
    pub weighting_source: WeightingSource,
    /// ROUT outlier handling mode (`None`, `IdentifyAndCountOnly`, or `Eliminate`).
    #[serde(default)]
    pub outlier_handling: OutlierHandling,
    /// When `true` and ROUT outlier detection runs, emits an analyzable `"Cleaned data"` tab (and `ResultSheet::analyzable_data`) with ROUT outliers excluded.
    #[serde(default)]
    pub emit_cleaned_data_tab: bool,
    /// Whether replicate subcolumns enter individually (`IndividualReplicates`) or collapse to unweighted row means (`RowMeansOnly`).
    #[serde(default)]
    pub replicate_fit_mode: ReplicateFitMode,
    /// Convergence strictness preset (`Default` MINPACK machine precision, `Quick`, `Medium`, or `Strict`).
    #[serde(default)]
    pub convergence_strictness: ConvergenceStrictness,
    /// Three-way identifiability reporting mode (`IdentifyUnstableParameters`, `IdentifyAmbiguousFits`,
    /// or `ReportAll`; §2.5.1). `None` preserves legacy solver configuration while result formatting
    /// still safely masks ambiguous/collinear parameters as `"Unstable"` by default (`R19.7`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifiability_mode: Option<IdentifiabilityMode>,
    /// Explicit override for `Std. Error` column visibility on the nonlinear parameter table (§2.5.1).
    /// `None` omits asymptotic SEs by default under asymmetrical profile-likelihood CIs (`profile_likelihood_ci == true`)
    /// and includes them under symmetrical Wald CIs (`profile_likelihood_ci == false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_standard_errors: Option<bool>,
    /// Extended 4-question model-comparison hypothesis (§2.5.2). When `Some(_)`, specifies the
    /// scientific comparison question (`AlternativeEquation`, `ParameterInvarianceAcrossDatasets`,
    /// `SingleGlobalCurveVsSeparate`, or `ParameterEqualsTheoretical`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_question: Option<ModelComparisonQuestion>,
    /// Statistical test criterion (`ExtraSumOfSquaresF`, `Aicc`, or `LikelihoodRatioTest`) used
    /// with `comparison_question` when `comparison` is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_method: Option<ModelComparisonMethod>,
    /// Pre-check before Extra-SS *F*, LRT, or `AICc`: if one candidate fit is `Ambiguous`,
    /// `Unstable`, or `Flagged` while the other converges cleanly, select the clean model
    /// immediately without running a statistical test (§2.5.2).
    #[serde(default)]
    pub prefer_well_defined_fit: bool,
    /// Optional parameter (or derived scalar) name summarized across fitted datasets into a
    /// dedicated analyzable parameter summary `ResultTab` (§2.5.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_parameter: Option<String>,
    /// When `true` and `summary_parameter` is `Some(_)`, automatically create a linked secondary
    /// bar or XY `GraphSheet` plotting the summarized parameter across datasets (§2.5.3).
    #[serde(default)]
    pub create_summary_graph: bool,
    /// Optional significant-digit precision (`1..=15`) controlling displayed formatting on
    /// parameter tables and Embedded Result tables (§2.5.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_significant_digits: Option<u8>,
    /// Automated 6-rule fit-quality flag criteria labeling dubious fits `"Flagged"` (§2.5.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_flag_rules: Option<FitFlagRules>,
    /// Relational parameter inequality constraints enforced via smooth gap reparameterization (`theta_a > theta_b + margin`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relational_constraints: Vec<RelationalConstraint>,
    /// Optional theoretical probability distribution plotting configuration (`Plot a function` §6.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_distribution: Option<TheoreticalDistributionSpec>,
    /// When `true`, emits extended goodness-of-fit metrics (Adjusted `R^2`, Weighted `R^2`, RMSE, and Poisson pseudo-`R^2` / NLL / dispersion).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emit_extra_gof: bool,
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
            mv_y_columns: Vec::new(),
            weighting_source: WeightingSource::ObservedY,
            outlier_handling: OutlierHandling::None,
            emit_cleaned_data_tab: false,
            replicate_fit_mode: ReplicateFitMode::IndividualReplicates,
            convergence_strictness: ConvergenceStrictness::Default,
            identifiability_mode: None,
            show_standard_errors: None,
            comparison_question: None,
            comparison_method: None,
            prefer_well_defined_fit: false,
            summary_parameter: None,
            create_summary_graph: false,
            output_significant_digits: None,
            fit_flag_rules: None,
            relational_constraints: Vec::new(),
            plot_distribution: None,
            emit_extra_gof: false,
        }
    }

    /// Resolve the effective [`IdentifiabilityMode`], honoring explicit `identifiability_mode`
    /// first and falling back to `identify_unstable_parameters` for backwards compatibility.
    #[must_use]
    pub const fn effective_identifiability_mode(&self) -> Option<IdentifiabilityMode> {
        if let Some(mode) = self.identifiability_mode {
            Some(mode)
        } else if self.identify_unstable_parameters {
            Some(IdentifiabilityMode::IdentifyUnstableParameters)
        } else {
            None
        }
    }

    /// Return `true` when the `Std. Error` column should be shown on the parameter table (§2.5.1).
    ///
    /// Defaults to omitting asymptotic SEs under asymmetrical profile-likelihood CIs
    /// (`profile_likelihood_ci == true`) and showing them under symmetrical Wald CIs
    /// (`profile_likelihood_ci == false`), unless overridden by `show_standard_errors`.
    #[must_use]
    pub fn should_show_standard_errors(&self) -> bool {
        self.show_standard_errors.unwrap_or(!self.profile_likelihood_ci)
    }

    /// Resolve the effective `(ModelComparisonMethod, ModelComparisonQuestion)` pair when either
    /// `comparison_question` or legacy `comparison` is configured.
    #[must_use]
    pub fn effective_comparison(&self) -> Option<(ModelComparisonMethod, ModelComparisonQuestion)> {
        if let Some(question) = &self.comparison_question {
            let method = self
                .comparison_method
                .or_else(|| self.comparison.as_ref().map(ModelComparisonSpec::method))
                .unwrap_or(ModelComparisonMethod::ExtraSumOfSquaresF);
            return Some((method, question.clone()));
        }
        self.comparison.as_ref().map(|spec| {
            (spec.method(), ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: spec.alternative_equation_id().to_owned() })
        })
    }

    /// Return `max_iters` clamped into `1..=Self::HARD_MAX_ITERS`.
    #[must_use]
    pub fn clamped_max_iters(&self) -> usize {
        self.max_iters.clamp(1, Self::HARD_MAX_ITERS)
    }

    /// Resolve effective `MultipleVariables` response columns, preferring `mv_y_columns` when
    /// non-empty and falling back to legacy singular `mv_y_column`.
    #[must_use]
    pub fn effective_mv_y_columns(&self) -> Vec<usize> {
        if !self.mv_y_columns.is_empty() {
            self.mv_y_columns.clone()
        } else if let Some(yc) = self.mv_y_column {
            vec![yc]
        } else {
            Vec::new()
        }
    }

    /// Resolve effective ROUT outlier handling combining `method` and `outlier_handling`.
    #[must_use]
    pub const fn effective_outlier_handling(&self) -> OutlierHandling {
        match (self.method, self.outlier_handling) {
            (FitMethod::RoutOutlierElimination { .. }, OutlierHandling::IdentifyAndCountOnly) => OutlierHandling::IdentifyAndCountOnly,
            (FitMethod::RoutOutlierElimination { .. }, OutlierHandling::None | OutlierHandling::Eliminate) => OutlierHandling::Eliminate,
            (FitMethod::LeastSquares, OutlierHandling::IdentifyAndCountOnly) => OutlierHandling::IdentifyAndCountOnly,
            (FitMethod::LeastSquares, OutlierHandling::Eliminate) => OutlierHandling::Eliminate,
            _ => OutlierHandling::None,
        }
    }
}

// ============================================================================
// Model-Free Spline & LOWESS Curve Fitting (`docs/implementation/curve-fitting-implementation.md` §6.1)
// ============================================================================

/// Model-free XY curve-fitting algorithm (`docs/implementation/curve-fitting-implementation.md` §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SplineLowessMethod {
    /// Natural cubic spline ($C^2$ piecewise cubic through sorted distinct knots with zero endpoint second derivatives; de Boor 1978).
    #[default]
    #[serde(alias = "natural_cubic_spline", alias = "natural_spline", alias = "cubic_spline", alias = "spline")]
    NaturalCubicSpline,
    /// Akima subspline ($C^1$ piecewise cubic using 5-point secant slope weights suppressing overshoot; Akima 1970).
    #[serde(alias = "akima_subspline", alias = "akima", alias = "akima_spline")]
    AkimaSubspline,
    /// Penalized cubic smoothing spline minimizing weighted RSS plus curvature penalty parameterized by knot count or effective degrees of freedom (Reinsch 1967).
    #[serde(alias = "smoothing_spline", alias = "reinsch", alias = "penalized_spline")]
    SmoothingSpline,
    /// Tricube-weighted local linear regression with Tukey bisquare robustness passes (Cleveland 1979).
    #[serde(alias = "lowess", alias = "loess")]
    Lowess,
    /// Dense piecewise-linear curve in data space resampled onto `segment_count` `(X, Y)` vertices (distinct from straight graph-space connectors on log axes).
    #[serde(alias = "point_to_point", alias = "point-to-point", alias = "dense_point_to_point", alias = "piecewise_linear")]
    PointToPoint,
}

impl SplineLowessMethod {
    /// All five model-free XY curve-fitting methods in canonical order.
    pub const ALL: [Self; 5] = [Self::NaturalCubicSpline, Self::AkimaSubspline, Self::SmoothingSpline, Self::Lowess, Self::PointToPoint];

    /// Neutral human-readable label for titles, inspectors, and methods reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NaturalCubicSpline => "Natural cubic spline",
            Self::AkimaSubspline => "Akima subspline",
            Self::SmoothingSpline => "Smoothing spline",
            Self::Lowess => "LOWESS",
            Self::PointToPoint => "Dense point-to-point curve",
        }
    }

    /// Canonical snake-case equation identifier stored on [`crate::FittedCurveOverlay::equation_id`].
    #[must_use]
    pub const fn equation_id(self) -> &'static str {
        match self {
            Self::NaturalCubicSpline => "natural_cubic_spline",
            Self::AkimaSubspline => "akima_subspline",
            Self::SmoothingSpline => "smoothing_spline",
            Self::Lowess => "lowess",
            Self::PointToPoint => "point_to_point",
        }
    }
}

/// Window-span preset for Cleveland (1979) LOWESS smoothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LowessPreset {
    /// Broad smoothing window (`f = 0.75`).
    #[serde(alias = "coarse")]
    Coarse,
    /// Balanced default smoothing window (`f = 0.50`).
    #[default]
    #[serde(alias = "medium")]
    Medium,
    /// Tight local smoothing window (`f = 0.25`).
    #[serde(alias = "fine")]
    Fine,
}

impl LowessPreset {
    /// All three LOWESS window presets in canonical order.
    pub const ALL: [Self; 3] = [Self::Coarse, Self::Medium, Self::Fine];

    /// Neighborhood span fraction `f` in `(0, 1]` corresponding to this preset.
    #[must_use]
    pub const fn fraction(self) -> f64 {
        match self {
            Self::Coarse => 0.75,
            Self::Medium => 0.50,
            Self::Fine => 0.25,
        }
    }

    /// Neutral human-readable label for UI selectors and methods reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse (f = 0.75)",
            Self::Medium => "Medium (f = 0.50)",
            Self::Fine => "Fine (f = 0.25)",
        }
    }
}

const fn default_spline_segment_count() -> usize {
    SplineLowessSpec::DEFAULT_SEGMENTS
}

const fn default_lowess_robustness_iters() -> usize {
    SplineLowessSpec::DEFAULT_LOWESS_ROBUSTNESS_ITERS
}

/// Dedicated XY model-free curve-fitting specification (`Spline / LOWESS`; `docs/implementation/curve-fitting-implementation.md` §6.1).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SplineLowessSpec {
    /// Model-free curve algorithm (`NaturalCubicSpline`, `AkimaSubspline`, `SmoothingSpline`, `Lowess`, or `PointToPoint`).
    pub method: SplineLowessMethod,
    /// Number of polyline vertices sampled along `[x_min, x_max]` (clamped into `2..=Self::MAX_SEGMENTS`; LOWESS enforces `>= 4 * N`).
    #[serde(default = "default_spline_segment_count")]
    pub segment_count: usize,
    /// Optional target knot count (`>= 2`) for [`SplineLowessMethod::SmoothingSpline`]; used when `smoothing_df` is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothing_knots: Option<usize>,
    /// Optional explicit effective degrees of freedom (`df` in `[2, G]`) for [`SplineLowessMethod::SmoothingSpline`] (Reinsch 1967); overrides `smoothing_knots` when `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothing_df: Option<f64>,
    /// Window-span preset for [`SplineLowessMethod::Lowess`] (`Coarse` = 0.75, `Medium` = 0.50, `Fine` = 0.25) used when `lowess_fraction` is `None`.
    pub lowess_preset: LowessPreset,
    /// Optional custom LOWESS neighborhood fraction `f` in `(0, 1]`; overrides `lowess_preset` when `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowess_fraction: Option<f64>,
    /// Number of Tukey bisquare robustness passes for [`SplineLowessMethod::Lowess`] (default `3`, matching Cleveland 1979).
    #[serde(default = "default_lowess_robustness_iters")]
    pub lowess_robustness_iters: usize,
    /// Optional explicit lower *X* coordinate for curve evaluation; `None` uses the observed data minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_x_min: Option<f64>,
    /// Optional explicit upper *X* coordinate for curve evaluation; `None` uses the observed data maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_x_max: Option<f64>,
    /// When `true`, emits a [`crate::ResultTabKind::CurveCoordinates`] tab containing the sampled `(X, Y)` curve table.
    pub emit_curve_coordinates_tab: bool,
    /// When `true`, interpolates unpaired unknown rows (`S(x_unk)` for `(X, blank Y)` and root-finding `S(x) = y_unk` for `(blank X, Y)`).
    pub interpolate_unknowns: bool,
}

impl SplineLowessSpec {
    /// Default polyline vertex count (`200`).
    pub const DEFAULT_SEGMENTS: usize = 200;
    /// Minimum polyline vertex count (`2`).
    pub const MIN_SEGMENTS: usize = 2;
    /// Maximum polyline vertex count (`10_000`).
    pub const MAX_SEGMENTS: usize = 10_000;
    /// Default Tukey bisquare robustness iteration count (`3`).
    pub const DEFAULT_LOWESS_ROBUSTNESS_ITERS: usize = 3;
    /// Hard ceiling on LOWESS robustness iterations (`20`).
    pub const MAX_LOWESS_ROBUSTNESS_ITERS: usize = 20;
    /// Minimum recommended sample size for LOWESS before emitting an assumption warning (`20`).
    pub const LOWESS_MIN_RECOMMENDED_N: usize = 20;

    /// Resolve the active LOWESS neighborhood fraction (`lowess_fraction` override or `lowess_preset.fraction()`).
    #[must_use]
    pub fn effective_lowess_fraction(&self) -> f64 {
        self.lowess_fraction.unwrap_or_else(|| self.lowess_preset.fraction())
    }
}

impl Default for SplineLowessSpec {
    fn default() -> Self {
        Self {
            method: SplineLowessMethod::NaturalCubicSpline,
            segment_count: Self::DEFAULT_SEGMENTS,
            smoothing_knots: None,
            smoothing_df: None,
            lowess_preset: LowessPreset::Medium,
            lowess_fraction: None,
            lowess_robustness_iters: Self::DEFAULT_LOWESS_ROBUSTNESS_ITERS,
            plot_x_min: None,
            plot_x_max: None,
            emit_curve_coordinates_tab: false,
            interpolate_unknowns: false,
        }
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
    #[serde(alias = "unpairedt", alias = "twosampleunpairedt", alias = "independentt")]
    TwoSampleUnpairedT,
    /// Paired-differences *t*-test via exact noncentral-*t* distribution (`df = n - 1`, Cohen's `d_z`).
    #[serde(alias = "pairedt", alias = "twosamplepairedt", alias = "dependentt")]
    TwoSamplePairedT,
    /// One-sample *t*-test via exact noncentral-*t* distribution (`df = n - 1`, Cohen's `d`).
    #[serde(alias = "onesamplet", alias = "singlesamplet")]
    OneSampleT,
    /// Balanced `k`-group one-way fixed-effects ANOVA via exact noncentral-*F* distribution (`df_1 = k - 1`, `df_2 = k(n - 1)`, Cohen's `f`).
    #[serde(alias = "onewayanova", alias = "anova")]
    OneWayAnova,
    /// Two independent binomial proportions test (`p_1` vs `p_2`; Fleiss, Levin, & Paik 2003 / Cohen's `h`).
    #[serde(alias = "twoproportions", alias = "proportions")]
    TwoProportions,
    /// Bivariate Pearson correlation test `H_0: rho = 0` (`r` in `(-1, 1)`).
    #[serde(alias = "correlation", alias = "pearsoncorrelation")]
    Correlation,
    /// Contingency or goodness-of-fit chi-square test via exact noncentral `chi^2` distribution (`df > 0`, Cohen's `w`).
    #[serde(alias = "chisquare", alias = "chi2")]
    ChiSquare,
    /// Simple linear regression slope test via exact noncentral-*F* / *t* (`df_1 = 1`, `df_2 = n - 2`, Cohen's `f^2 = R^2 / (1 - R^2)`).
    #[serde(alias = "simplelinearregression", alias = "linearregression", alias = "regression")]
    SimpleLinearRegression,
    /// Two-arm log-rank survival comparison via Schoenfeld (1983) or Freedman (1982) event/sample-size formulas (Hazard Ratio).
    #[serde(alias = "logrank", alias = "log_rank", alias = "logranksurvival", alias = "survival")]
    LogRankSurvival,
}

impl PowerTestFamily {
    /// All nine supported power-analysis test families.
    pub const ALL: [Self; 9] = [
        Self::TwoSampleUnpairedT,
        Self::TwoSamplePairedT,
        Self::OneSampleT,
        Self::OneWayAnova,
        Self::TwoProportions,
        Self::Correlation,
        Self::ChiSquare,
        Self::SimpleLinearRegression,
        Self::LogRankSurvival,
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
            Self::LogRankSurvival => "Log-rank survival test",
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
            Self::LogRankSurvival => "Hazard ratio (HR)",
        }
    }

    /// Return `true` when one-sided alternatives are meaningful for this family.
    #[must_use]
    pub const fn supports_one_sided(self) -> bool {
        !matches!(self, Self::OneWayAnova | Self::ChiSquare)
    }
}

/// Required-events approximation formula for [`PowerTestFamily::LogRankSurvival`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SurvivalPowerMethod {
    /// Schoenfeld (1983) log-hazard-ratio score approximation (default).
    #[default]
    #[serde(alias = "schoenfeld")]
    Schoenfeld,
    /// Freedman (1982) hazard-ratio proportion approximation.
    #[serde(alias = "freedman")]
    Freedman,
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
    #[serde(alias = "samplesize", alias = "n")]
    SampleSize,
    /// Evaluate achieved statistical power `1 - beta` in `(0, 1)` given `sample_size`, `alpha`, and `effect`.
    #[serde(alias = "power")]
    Power,
    /// Solve the minimum detectable effect size given `sample_size`, `alpha`, and `target_power`.
    #[serde(alias = "detectableeffect", alias = "effect", alias = "mde")]
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
    /// Hazard ratio (`HR > 0`, `HR != 1`) and optional per-arm cumulative event probabilities for [`PowerTestFamily::LogRankSurvival`].
    HazardRatio {
        /// Alternative hazard ratio `HR = lambda_2 / lambda_1 > 0` (`HR != 1.0`).
        hazard_ratio: f64,
        /// Optional cumulative event probability in arm 1 (`(0, 1]`) for converting required events to sample size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_prob_group1: Option<f64>,
        /// Optional cumulative event probability in arm 2 (`(0, 1]`) for converting required events to sample size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_prob_group2: Option<f64>,
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
    /// Survival required-events approximation formula (`Schoenfeld` or `Freedman`) for [`PowerTestFamily::LogRankSurvival`].
    #[serde(default, skip_serializing_if = "is_default")]
    pub survival_method: SurvivalPowerMethod,
    /// Optional target or available total event count `D > 0` for [`PowerTestFamily::LogRankSurvival`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_events: Option<f64>,
    /// Optional cumulative event probability `p_1 in (0, 1]` in arm 1 for [`PowerTestFamily::LogRankSurvival`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_prob_group1: Option<f64>,
    /// Optional cumulative event probability `p_2 in (0, 1]` in arm 2 for [`PowerTestFamily::LogRankSurvival`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_prob_group2: Option<f64>,
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
            survival_method: SurvivalPowerMethod::Schoenfeld,
            target_events: None,
            event_prob_group1: None,
            event_prob_group2: None,
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
        if self.target_events.is_some_and(|v| !v.is_finite()) {
            return Some("power.target_events");
        }
        if self.event_prob_group1.is_some_and(|v| !v.is_finite()) {
            return Some("power.event_prob_group1");
        }
        if self.event_prob_group2.is_some_and(|v| !v.is_finite()) {
            return Some("power.event_prob_group2");
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
            PowerEffectSpec::HazardRatio { hazard_ratio, event_prob_group1, event_prob_group2 } => {
                if !hazard_ratio.is_finite() || event_prob_group1.is_some_and(|v| !v.is_finite()) || event_prob_group2.is_some_and(|v| !v.is_finite())
                {
                    Some("power.effect.hazard_ratio")
                } else {
                    None
                }
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
        if let Some(d) = self.target_events
            && d <= 0.0
        {
            return Err(invalid("power.target_events must be > 0"));
        }
        if let Some(p) = self.event_prob_group1
            && (p <= 0.0 || p > 1.0)
        {
            return Err(invalid("power.event_prob_group1 must lie in (0, 1]"));
        }
        if let Some(p) = self.event_prob_group2
            && (p <= 0.0 || p > 1.0)
        {
            return Err(invalid("power.event_prob_group2 must lie in (0, 1]"));
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
            PowerEffectSpec::HazardRatio { hazard_ratio, event_prob_group1, event_prob_group2 } => {
                if *hazard_ratio <= 0.0 {
                    return Err(invalid("power.effect hazard_ratio must be > 0"));
                }
                if event_prob_group1.is_some_and(|p| p <= 0.0 || p > 1.0) || event_prob_group2.is_some_and(|p| p <= 0.0 || p > 1.0) {
                    return Err(invalid("power.effect survival event probabilities must lie in (0, 1]"));
                }
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
            survival_method: SurvivalPowerMethod,
            target_events: Option<f64>,
            event_prob_group1: Option<f64>,
            event_prob_group2: Option<f64>,
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
                    survival_method: d.survival_method,
                    target_events: d.target_events,
                    event_prob_group1: d.event_prob_group1,
                    event_prob_group2: d.event_prob_group2,
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
            survival_method: raw.survival_method,
            target_events: raw.target_events,
            event_prob_group1: raw.event_prob_group1,
            event_prob_group2: raw.event_prob_group2,
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
            "LogRankSurvival",
        ])
        .with_title("PowerTestFamily")
    }
}

impl crate::schema::JsonSchema for SurvivalPowerMethod {
    fn schema_name() -> &'static str {
        "SurvivalPowerMethod"
    }

    fn json_schema() -> crate::schema::SchemaNode {
        crate::schema::enum_schema(&["Schoenfeld", "Freedman"]).with_title("SurvivalPowerMethod")
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
            object_schema(
                vec![(
                    "HazardRatio",
                    object_schema(
                        vec![
                            ("hazard_ratio", number_schema().with_minimum_f64(f64::MIN_POSITIVE)),
                            ("event_prob_group1", nullable_schema(number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))),
                            ("event_prob_group2", nullable_schema(number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))),
                        ],
                        &["hazard_ratio"],
                        false,
                    ),
                )],
                &["HazardRatio"],
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
                ("survival_method", SurvivalPowerMethod::json_schema()),
                ("target_events", nullable_schema(number_schema().with_minimum_f64(0.0))),
                ("event_prob_group1", nullable_schema(number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))),
                ("event_prob_group2", nullable_schema(number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0))),
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

/// Specification payload for an imported read-only result snapshot that CurveBook preserves
/// without natively recomputing.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ImportedSnapshotSpec {
    /// Original analysis kind label reported by the source file (for example `"Nested t test"`).
    pub original_kind_label: String,
}

// ============================================================================
// Unified AnalysisKind & AnalysisSpec
// ============================================================================

/// Discriminated statistical, regression, or transformation specification.
#[allow(clippy::large_enum_variant)]
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
    /// Model-free XY spline (natural cubic, Akima, smoothing), LOWESS, and dense point-to-point curve fitting ([`SplineLowessSpec`]).
    FitSplineLowess(SplineLowessSpec),
    /// Standalone FWER/FDR adjustment of a column of pre-computed *P* values ([`PValueStackSpec`]).
    PValueStack(PValueStackSpec),
    /// Statistical power, sample-size, and minimum-detectable-effect solver ([`PowerAnalysisSpec`]).
    PowerAnalysis(PowerAnalysisSpec),
    /// Standalone single-predictor binary logistic regression on `XY` tables ([`SimpleLogisticSpec`]).
    SimpleLogisticRegression(SimpleLogisticSpec),
    /// Standalone Levene (mean-centered) or Brown–Forsythe (median-centered) homogeneity-of-variance test ([`LeveneTestSpec`]).
    LeveneTest(LeveneTestSpec),
    /// Standalone trapezoidal Area Under the Curve (AUC) and peak detection on `XY` tables ([`AreaUnderCurveSpec`]).
    AreaUnderCurve(AreaUnderCurveSpec),
    /// Preserved read-only result snapshot imported from an external file format ([`ImportedSnapshotSpec`]).
    ImportedSnapshot(ImportedSnapshotSpec),
    /// Per-row replicate summary statistics (totals, means with SD/SEM/%CV/CI, medians/percentiles, geometric means; [`RowStatisticsSpec`]).
    RowStatistics(RowStatisticsSpec),
    /// Row, column, or grand-total proportion/percentage reduction with optional binomial CIs ([`FractionOfTotalSpec`]).
    FractionOfTotal(FractionOfTotalSpec),
    /// XY curve numerical differentiation, cumulative integration, and smoothing ([`CurveCalculusSpec`]).
    CurveCalculus(CurveCalculusSpec),
    /// Chained deterministic Monte Carlo simulation over a downstream analysis ([`MonteCarloSpec`]).
    MonteCarloSimulation(MonteCarloSpec),
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
            Self::DescriptiveStats(d) => {
                if d.normality_only {
                    String::from("Normality and lognormality tests")
                } else {
                    String::from("Descriptive statistics")
                }
            }
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
                TwoGroupMethod::OneSampleRatioT { .. } => String::from("One-sample ratio t test"),
                TwoGroupMethod::LognormalWelchT => String::from("Lognormal Welch's t test"),
            },
            Self::BlandAltman(_) => String::from("Bland-Altman method comparison"),
            Self::MultipleRowTTests(s) => match (s.nonparametric, s.paired, s.lognormal_mode) {
                (false, _, LognormalRowTestMode::RatioPaired) => String::from("Multiple ratio paired t tests"),
                (false, false, LognormalRowTestMode::Lognormal) => String::from("Multiple lognormal t tests"),
                (false, true, LognormalRowTestMode::Lognormal) => String::from("Multiple paired lognormal t tests"),
                (false, false, LognormalRowTestMode::None) => String::from("Multiple unpaired t tests"),
                (false, true, LognormalRowTestMode::None) => String::from("Multiple paired t tests"),
                (true, false, _) => String::from("Multiple Mann-Whitney tests"),
                (true, true, _) => String::from("Multiple Wilcoxon tests"),
            },
            Self::PassingBablok(_) => String::from("Passing-Bablok regression"),
            Self::OneWayAnova(s) => match s.method {
                OneWayMethod::OrdinaryAnova => String::from("Ordinary one-way ANOVA"),
                OneWayMethod::BrownForsytheAndWelch => String::from("Brown-Forsythe and Welch ANOVA"),
                OneWayMethod::RepeatedMeasures { .. } => String::from("RM one-way ANOVA"),
                OneWayMethod::KruskalWallis => String::from("Kruskal-Wallis test"),
                OneWayMethod::Friedman => String::from("Friedman test"),
                OneWayMethod::LognormalOrdinary => String::from("Lognormal one-way ANOVA"),
                OneWayMethod::LognormalWelchBf => String::from("Lognormal Brown-Forsythe and Welch ANOVA"),
                OneWayMethod::LognormalRepeatedMeasures { .. } => String::from("Lognormal RM one-way ANOVA"),
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
            Self::FitSplineLowess(s) => format!("Spline / LOWESS ({})", s.method.label()),
            Self::PValueStack(s) => format!("Analyze stack of P values ({})", s.method.label()),
            Self::PowerAnalysis(s) => s.derive_auto_title(),
            Self::SimpleLogisticRegression(_) => String::from("Simple logistic regression"),
            Self::LeveneTest(s) => match s.center {
                LeveneCenter::Mean => String::from("Levene's test (mean)"),
                LeveneCenter::Median => String::from("Brown-Forsythe test (median)"),
            },
            Self::AreaUnderCurve(_) => String::from("Area under the curve"),
            Self::ImportedSnapshot(s) => {
                let trimmed = s.original_kind_label.trim();
                if trimmed.is_empty() { String::from("Imported result") } else { trimmed.to_owned() }
            }
            Self::RowStatistics(_) => String::from("Row statistics"),
            Self::FractionOfTotal(_) => String::from("Fraction of total"),
            Self::CurveCalculus(c) => match c.operation {
                CurveCalculusOp::FirstDerivative => String::from("First derivative"),
                CurveCalculusOp::SecondDerivative => String::from("Second derivative"),
                CurveCalculusOp::CumulativeIntegral { .. } => String::from("Cumulative integral"),
                CurveCalculusOp::SmoothSavitzkyGolay { .. } => String::from("Smooth curve (Savitzky-Golay)"),
                CurveCalculusOp::SmoothMovingAverage { .. } => String::from("Smooth curve (moving average)"),
                CurveCalculusOp::SmoothLowess { .. } => String::from("Smooth curve (LOWESS)"),
            },
            Self::MonteCarloSimulation(_) => String::from("Monte Carlo simulation"),
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
            Self::Transform(TransformAnalysisSpec::Simulate { simulation_mode: Some(SimulationMode::Column(col_sim)), .. }) => {
                for hook in col_sim.hooked_means.iter().flatten() {
                    out.push(hook.clone());
                }
            }
            Self::NonlinearRegression(fit) => {
                for rule in &fit.parameter_rules {
                    if let ParameterScope::HookedConstant(h) = &rule.scope {
                        out.push(h.clone());
                    }
                }
                if let Some(dist) = &fit.plot_distribution {
                    for hook in dist.parameter_hooks.iter().flatten() {
                        out.push(hook.clone());
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
        if let Self::MonteCarloSimulation(mc) = self {
            if let Some(down_id) = &mc.downstream_result_sheet_id {
                ids.push(down_id.clone());
            }
            for scalar in &mc.tracked_scalars {
                if let Some(res_id) = &scalar.result_sheet_id {
                    ids.push(res_id.clone());
                }
            }
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
    ///
    /// Required (`Some(..)`) for all natively computed analysis kinds; may be `None` strictly for
    /// unlinked [`AnalysisKind::ImportedSnapshot`] result sheets.
    pub source_sheet_id: Option<SheetId>,
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
        let source_sheet_id = Some(source_sheet_id.into());
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

    /// Construct a validated [`AnalysisSpec`] for an imported read-only snapshot, optionally linked to an upstream `source_sheet_id`.
    pub fn new_imported_snapshot(
        id: impl Into<SheetId>,
        source_sheet_id: Option<SheetId>,
        original_kind_label: impl Into<String>,
    ) -> Result<Self, DocError> {
        let kind = AnalysisKind::ImportedSnapshot(ImportedSnapshotSpec { original_kind_label: original_kind_label.into() });
        let auto_title = kind.derive_auto_title();
        let spec = Self {
            id: id.into(),
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
        match (&self.kind, &self.source_sheet_id) {
            (AnalysisKind::ImportedSnapshot(snapshot), maybe_src) => {
                validate_title(&snapshot.original_kind_label)?;
                if let Some(src) = maybe_src {
                    src.validate_non_empty_bounded()?;
                }
            }
            (_, Some(src)) => {
                src.validate_non_empty_bounded()?;
            }
            (_, None) => {
                return Err(DocError::InvalidIdentifier { reason: "source_sheet_id is required for non-imported analyses" });
            }
        }
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
        self.source_sheet_id = Some(new_id);
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
            #[serde(default)]
            source_sheet_id: Option<SheetId>,
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

#[cfg(test)]
mod r18_5_linear_spec_tests {
    use super::{AnalysisKind, DemingErrorSpec, LeveneCenter, LeveneTestSpec, LinearRegressionSpec};
    use crate::error::CURRENT_SCHEMA_VERSION;

    #[test]
    fn linear_regression_spec_backward_compat_and_deming_error_spec() {
        assert_eq!(CURRENT_SCHEMA_VERSION, 9);

        let levene_default: LeveneTestSpec = serde_json::from_str("{}").unwrap();
        assert_eq!(levene_default.center, LeveneCenter::Median);
        let levene_mean = AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Mean });
        let levene_rt: AnalysisKind = serde_json::from_str(&serde_json::to_string(&levene_mean).unwrap()).unwrap();
        assert_eq!(levene_rt, levene_mean);

        let legacy_json = r#"{
            "deming_model_ii": false,
            "force_through_origin": true,
            "confidence_bands": true,
            "prediction_bands": false,
            "interpolate_unknowns": false
        }"#;
        let parsed: LinearRegressionSpec = serde_json::from_str(legacy_json).unwrap();
        assert!(!parsed.deming_model_ii);
        assert!(parsed.force_through_origin);
        assert_eq!(parsed.fixed_point, None);
        assert_eq!(parsed.effective_fixed_point(), Some((0.0, 0.0)));
        assert!(!parsed.standard_addition);
        assert!(!parsed.extend_line_to_x_intercept);
        assert_eq!(parsed.deming_error, DemingErrorSpec::EqualError);

        assert_eq!(DemingErrorSpec::EqualError.resolved_lambda(), Some(1.0));
        assert_eq!(DemingErrorSpec::StandardDeviations { sd_x: 3.0, sd_y: 2.0 }.resolved_lambda(), Some(2.25));
        assert_eq!(DemingErrorSpec::VarianceRatio { lambda: 0.5 }.resolved_lambda(), Some(0.5));
        assert_eq!(DemingErrorSpec::StandardDeviations { sd_x: 0.0, sd_y: 1.0 }.resolved_lambda(), None);
        assert_eq!(DemingErrorSpec::VarianceRatio { lambda: -1.0 }.resolved_lambda(), None);
        assert_eq!(DemingErrorSpec::VarianceRatio { lambda: f64::NAN }.resolved_lambda(), None);

        let custom = LinearRegressionSpec {
            deming_model_ii: true,
            fixed_point: Some((1.5, 2.5)),
            standard_addition: true,
            extend_line_to_x_intercept: true,
            deming_error: DemingErrorSpec::StandardDeviations { sd_x: 0.4, sd_y: 0.2 },
            ..LinearRegressionSpec::default()
        };
        assert_eq!(custom.effective_fixed_point(), Some((1.5, 2.5)));
        let roundtrip: LinearRegressionSpec = serde_json::from_str(&serde_json::to_string(&custom).unwrap()).unwrap();
        assert_eq!(roundtrip, custom);
    }
}
