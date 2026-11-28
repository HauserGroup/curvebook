//! Canonical descriptor-table matrix governing pre-summarized [`SubcolumnMode`] support across
//! every statistical analysis, regression fit, and data transformation that accepts `Column`, `Xy`,
//! or `Grouped` tables.
//!
//! Instead of scattering ad-hoc `if mode.is_pre_summarized()` match arms across domain crates,
//! `curvebook-stats`, `curvebook-fit`, and `curvebook-transform` consult [`summary_mode_support`]
//! backed by [`SUMMARY_MODE_MATRIX`]. Compile-time `const` assertions verify that every
//! user-creatable [`AnalysisKindId`] compatible with `Column`, `Xy`, or `Grouped` tables is
//! exhaustively covered across all eight summary [`SubcolumnMode`] layouts.
//!
//! # References
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25.
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different
//!   population variances are involved. *Biometrika* 34(1–2):28–35.
//! - Welch, B. L. (1951). On the comparison of several mean values: an alternative approach.
//!   *Biometrika* 38(3–4):330–336.
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114.
//! - Brown, M. B., & Forsythe, A. B. (1974b). The small sample behavior of some statistics which
//!   test the equality of several means. *Technometrics* 16(1):129–132.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Draper, N. R., & Smith, H. (1998). *Applied Regression Analysis* (3rd ed.). Wiley.
//! - Seber, G. A. F., & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. J., & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear
//!   and Nonlinear Regression*. Oxford University Press.
//! - Yeh, K. C., & Kwan, K. C. (1978). A comparison of numerical integrating algorithms by
//!   trapezoidal, Lagrange, and spline approximation. *Journal of Pharmacokinetics and
//!   Biopharmaceutics* 6(1):79–98.
//! - Bailer, A. J. (1988). Testing for the equality of area under the curves when using
//!   destructive measurement techniques. *Journal of Pharmacokinetics and Biopharmaceutics*
//!   16(3):303–309.
//! - Ku, H. H. (1966). Notes on the use of propagation of error formulas. *Journal of Research of
//!   the National Bureau of Standards* 70C(4):263–273.

use crate::TableKind;
use crate::cell::SubcolumnMode;

use super::{
    ALL_ANALYSIS_DESCRIPTORS, AnalysisKind, AnalysisKindId, ContingencyMethod, FitMethod, OneWayMethod, OutlierHandling, TransformAnalysisSpec,
    TwoGroupMethod, WeightingScheme,
};

/// All eight pre-summarized [`SubcolumnMode`] layouts in canonical matrix column order.
pub const SUMMARY_SUBCOLUMN_MODES: [SubcolumnMode; 8] = [
    SubcolumnMode::MeanSdN,
    SubcolumnMode::MeanSemN,
    SubcolumnMode::MeanCvN,
    SubcolumnMode::MeanSd,
    SubcolumnMode::MeanSem,
    SubcolumnMode::MeanLowHigh,
    SubcolumnMode::UpperLowerLimits,
    SubcolumnMode::MedianIqr,
];

/// Table archetypes whose primary data columns can carry summary [`SubcolumnMode`] layouts.
pub const SUMMARY_CAPABLE_TABLE_KINDS: [TableKind; 3] = [TableKind::Column, TableKind::Xy, TableKind::Grouped];

/// Execution semantics for a supported `(procedure, table_kind, summary_mode)` matrix cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SummaryModeSemantics {
    /// Exact parametric moment algebra from `(Mean, SD/SEM/%CV, N)`: reconstructs `SD`,
    /// `SS_within = (n - 1) * SD^2`, and total `N` (Zar 2010; Welch 1947; Motulsky & Christopoulos 2004).
    ParametricMoments,
    /// Descriptive reporting of entered spread-only `(Mean, SD)` or `(Mean, SEM)` rows without
    /// fabricating `N`, pooled variance across multiple rows, or inferential CIs.
    SpreadOnlySummary,
    /// Descriptive reporting of entered asymmetric offset or interval-limit rows
    /// (`[Mean, -Error, +Error]` or `[Mean, Lower, Upper]`) without fabricating `N`, `SD`, or `SEM`.
    IntervalLimitSummary,
    /// Descriptive reporting of entered nonparametric quartile rows (`[Median, Q1, Q3]`,
    /// `IQR = Q3 - Q1`) without fabricating arithmetic means, `N`, `SD`, or `SEM`.
    QuartileMedianSummary,
    /// Fits or integrates only the entered central location column (subcolumn `0`) as row-level
    /// unweighted points (`sd = None`, `n = None`), never feeding bounds or quartiles as replicates.
    CentralLocationOnly,
    /// Element-wise propagation across center and error/limit subcolumns (first-order delta method,
    /// replicate endpoint mapping, affine normalization, baseline operations, and bound reordering).
    SubcolumnPropagation,
    /// Structural row/column preservation (`Transpose` on XY/Grouped, `PruneRows`, `SelectAndTransform`,
    /// `RandomSample`), pooling parametric moments or clearing unpoolable intervals on bin averaging.
    StructuralPreservation,
    /// Prospective or synthetic generator (`PowerAnalysis`, `Transform::Simulate`) independent of
    /// the source table's subcolumn cells.
    TableIndependent,
}

/// Typed reason why a `(procedure, table_kind, summary_mode)` cell is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SummaryModeRefusalReason {
    /// Procedure mathematically requires individual raw observations (nonparametric rank/order
    /// tests, normality tests, outlier detection, matched/paired/RM subject blocks, robust/ROUT
    /// or Deming regression, Levene deviations, ROC cutoffs, PCA/clustering/GLM, binary trials).
    RequiresRawReplicates,
    /// Procedure supports parametric `(Mean, SD/SEM/%CV, N)` moments, but this summary mode omits
    /// sample size `N` or parametric variance (`MeanSd`, `MeanSem`, `MeanLowHigh`,
    /// `UpperLowerLimits`, `MedianIqr`).
    MissingSampleSizeOrVariance,
    /// Procedure requests `1/SD^2` inverse-variance weighting (`WeightingScheme::ObservedSd`),
    /// which requires reconstructed `(SD, N)` parametric summary columns.
    MissingSdForWeighting,
    /// Procedure cannot accept pre-summarized multi-subcolumn tuples on this table archetype
    /// (for example binary `0/1` `SimpleLogisticRegression` or `Transpose` on pre-summarized `Column`).
    UnsupportedSummaryMode,
}

impl SummaryModeRefusalReason {
    /// Concise, neutral explanation suitable for typed refusal errors and diagnostics.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::RequiresRawReplicates => "analysis requires individual raw replicate observations rather than pre-summarized columns",
            Self::MissingSampleSizeOrVariance => {
                "procedure requires sample size N and parametric SD/SEM/%CV moments (use MeanSdN, MeanSemN, MeanCvN, or raw replicates)"
            }
            Self::MissingSdForWeighting => {
                "inverse-variance 1/SD^2 weighting requires parametric Mean/SD/N, Mean/SEM/N, or Mean/%CV/N summary columns"
            }
            Self::UnsupportedSummaryMode => "procedure does not support pre-summarized subcolumns on this table archetype",
        }
    }
}

/// Cell verdict in [`SUMMARY_MODE_MATRIX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SummaryModeSupport {
    /// Supported under the documented mathematical semantics and backed by a D1 reference oracle.
    Supported(SummaryModeSemantics),
    /// Deterministically refused with a typed error.
    Refused(SummaryModeRefusalReason),
}

impl SummaryModeSupport {
    /// Return `true` when this cell is [`Self::Supported`].
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported(_))
    }

    /// Return `true` when this cell is [`Self::Refused`].
    #[must_use]
    pub const fn is_refused(self) -> bool {
        matches!(self, Self::Refused(_))
    }
}

/// Exhaustive verdict across all eight pre-summarized [`SubcolumnMode`] layouts for one matrix row.
///
/// Storing one named field per summary mode enforces compile-time exhaustiveness without runtime
/// slice indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryModeRowCells {
    /// `[Mean, SD, N]` (3 subcolumns; parametric moments with `N`).
    pub mean_sd_n: SummaryModeSupport,
    /// `[Mean, SEM, N]` (3 subcolumns; parametric moments with `N`).
    pub mean_sem_n: SummaryModeSupport,
    /// `[Mean, %CV, N]` (3 subcolumns; parametric moments with `N`).
    pub mean_cv_n: SummaryModeSupport,
    /// `[Mean, SD]` (2 subcolumns; spread-only, no `N`).
    pub mean_sd: SummaryModeSupport,
    /// `[Mean, SEM]` (2 subcolumns; spread-only, no `N`).
    pub mean_sem: SummaryModeSupport,
    /// `[Mean, -Error, +Error]` (3 subcolumns; asymmetric offsets, no `N`).
    pub mean_low_high: SummaryModeSupport,
    /// `[Mean, Lower, Upper]` (3 subcolumns; explicit limits, no `N`).
    pub upper_lower_limits: SummaryModeSupport,
    /// `[Median, Q1, Q3]` (3 subcolumns; nonparametric quartile summary, no `N`).
    pub median_iqr: SummaryModeSupport,
}

impl SummaryModeRowCells {
    /// Look up the cell verdict for `mode`, returning `None` for non-summary modes
    /// (`Single` and `RawReplicates(_)`).
    #[must_use]
    pub const fn get(&self, mode: SubcolumnMode) -> Option<SummaryModeSupport> {
        match mode {
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => None,
            SubcolumnMode::MeanSdN => Some(self.mean_sd_n),
            SubcolumnMode::MeanSemN => Some(self.mean_sem_n),
            SubcolumnMode::MeanCvN => Some(self.mean_cv_n),
            SubcolumnMode::MeanSd => Some(self.mean_sd),
            SubcolumnMode::MeanSem => Some(self.mean_sem),
            SubcolumnMode::MeanLowHigh => Some(self.mean_low_high),
            SubcolumnMode::UpperLowerLimits => Some(self.upper_lower_limits),
            SubcolumnMode::MedianIqr => Some(self.median_iqr),
        }
    }

    const fn all(support: SummaryModeSupport) -> Self {
        Self {
            mean_sd_n: support,
            mean_sem_n: support,
            mean_cv_n: support,
            mean_sd: support,
            mean_sem: support,
            mean_low_high: support,
            upper_lower_limits: support,
            median_iqr: support,
        }
    }

    const fn parametric_only(refusal: SummaryModeRefusalReason) -> Self {
        Self {
            mean_sd_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
            mean_sem_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
            mean_cv_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
            mean_sd: SummaryModeSupport::Refused(refusal),
            mean_sem: SummaryModeSupport::Refused(refusal),
            mean_low_high: SummaryModeSupport::Refused(refusal),
            upper_lower_limits: SummaryModeSupport::Refused(refusal),
            median_iqr: SummaryModeSupport::Refused(refusal),
        }
    }
}

/// Canonical discriminant identifying each procedure row in [`SUMMARY_MODE_MATRIX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum SummaryModeProcedure {
    /// Univariate descriptive statistics across `Column`, `Xy`, and `Grouped`.
    DescriptiveStats = 0,
    /// Univariate Grubbs ESD and ROUT outlier detection.
    OutlierDetection = 1,
    /// Parametric unpaired or one-sample *t*-tests (`UnpairedT`, `WelchT`, `OneSampleT`).
    TwoGroupParametricSummary = 2,
    /// Paired, lognormal, or nonparametric two-group / one-sample tests.
    TwoGroupRawOnly = 3,
    /// Bland–Altman method-comparison bias and limits of agreement.
    BlandAltman = 4,
    /// Row-by-row multiple two-group comparisons.
    MultipleRowTTests = 5,
    /// Passing–Bablok nonparametric errors-in-variables linear regression.
    PassingBablok = 6,
    /// Parametric unpaired one-way ANOVA (`OrdinaryAnova`, `BrownForsytheAndWelch`).
    OneWayAnovaParametricSummary = 7,
    /// Repeated-measures or nonparametric one-way ANOVA (`RepeatedMeasures`, `KruskalWallis`, `Friedman`).
    OneWayAnovaRawOnly = 8,
    /// Two-way and three-way grouped factorial ANOVA / REML.
    FactorialAnova = 9,
    /// Exact binomial proportion test on binary `0/1` `Column` tables.
    ContingencyColumnBinary = 10,
    /// Receiver Operating Characteristic (ROC) curve analysis on `Column` tables.
    RocCurve = 11,
    /// Generalized Linear Models (`Linear`, `Logistic`, `Poisson`) on `Column` / `Xy` tables.
    GeneralizedLinearModel = 12,
    /// Multivariate PCA, correlation matrix, clustering, and multiple imputation on `Column` / `Xy`.
    Multivariate = 13,
    /// Standalone FWER/FDR adjustment of a column of *P* values.
    PValueStack = 14,
    /// Prospective statistical power and sample-size calculator (independent of table cells).
    PowerAnalysis = 15,
    /// Single-predictor binary logistic regression on `Xy` tables.
    SimpleLogisticRegression = 16,
    /// Standalone Levene / Brown–Forsythe homogeneity-of-variance test on `Column` tables.
    LeveneTest = 17,
    /// Simple Model I OLS linear regression (`deming_model_ii == false`) on `Xy` / `Grouped`.
    LinearRegressionOls = 18,
    /// Model II Deming orthogonal linear regression (`deming_model_ii == true`) on `Xy` / `Grouped`.
    LinearRegressionDeming = 19,
    /// Nonlinear least-squares or Poisson regression with standard weighting on `Xy` / `Grouped`.
    NonlinearRegressionStandard = 20,
    /// Nonlinear least-squares regression with `WeightingScheme::ObservedSd` (`1/SD^2`) on `Xy` / `Grouped`.
    NonlinearRegressionObservedSdWeighted = 21,
    /// Nonlinear Cauchy robust or ROUT outlier-elimination regression on `Xy` / `Grouped`.
    NonlinearRegressionRobustOrOutlier = 22,
    /// Model-free `Xy` cubic spline, Akima, smoothing spline, LOWESS, and point-to-point curves.
    FitSplineLowess = 23,
    /// Trapezoidal Area Under the Curve (AUC) with optional Bailer (1988) SE on `Xy`.
    AreaUnderCurve = 24,
    /// Element-wise mathematical transforms (`TransformAnalysisSpec::Math`).
    TransformMath = 25,
    /// Affine rescaling normalization (`TransformAnalysisSpec::Normalize`).
    TransformNormalize = 26,
    /// Row-matched baseline operations (`TransformAnalysisSpec::RemoveBaseline`).
    TransformRemoveBaseline = 27,
    /// Matrix transposition of pre-summarized `Column` tables (refused).
    TransformTransposeColumn = 28,
    /// Matrix transposition of pre-summarized `Xy` and `Grouped` tables.
    TransformTransposeXyGrouped = 29,
    /// Row decimation (`TransformAnalysisSpec::PruneRows { average_window: false }`).
    TransformPruneDecimate = 30,
    /// Block-averaged row pruning (`TransformAnalysisSpec::PruneRows { average_window: true }`).
    TransformPruneAverageBins = 31,
    /// Row selection and derived-column transforms (`TransformAnalysisSpec::SelectAndTransform`).
    TransformSelectAndTransform = 32,
    /// Synthetic curve simulation (`TransformAnalysisSpec::Simulate`, table-independent).
    TransformSimulate = 33,
    /// Deterministic random row sampling without replacement (`TransformAnalysisSpec::RandomSample`).
    TransformRandomSample = 34,
    /// Per-row replicate summary statistics (`AnalysisKind::RowStatistics`).
    RowStatistics = 35,
    /// Row, column, or grand-total proportion/percentage reduction (`AnalysisKind::FractionOfTotal`).
    FractionOfTotal = 36,
    /// XY curve differentiation, cumulative integration, and smoothing (`AnalysisKind::CurveCalculus`).
    CurveCalculus = 37,
    /// Chained deterministic Monte Carlo simulation (`AnalysisKind::MonteCarloSimulation`).
    MonteCarloSimulation = 38,
    /// Standalone frequency distribution & histogram binning (`AnalysisKind::FrequencyDistribution`).
    FrequencyDistribution = 39,
    /// General one-way Analysis of Covariance (`AnalysisKind::Ancova` on `Xy` tables).
    Ancova = 40,
}

impl SummaryModeProcedure {
    /// Complete ordered list of all 41 canonical summary-mode matrix procedures.
    pub const ALL: [Self; 41] = [
        Self::DescriptiveStats,
        Self::OutlierDetection,
        Self::TwoGroupParametricSummary,
        Self::TwoGroupRawOnly,
        Self::BlandAltman,
        Self::MultipleRowTTests,
        Self::PassingBablok,
        Self::OneWayAnovaParametricSummary,
        Self::OneWayAnovaRawOnly,
        Self::FactorialAnova,
        Self::ContingencyColumnBinary,
        Self::RocCurve,
        Self::GeneralizedLinearModel,
        Self::Multivariate,
        Self::PValueStack,
        Self::PowerAnalysis,
        Self::SimpleLogisticRegression,
        Self::LeveneTest,
        Self::LinearRegressionOls,
        Self::LinearRegressionDeming,
        Self::NonlinearRegressionStandard,
        Self::NonlinearRegressionObservedSdWeighted,
        Self::NonlinearRegressionRobustOrOutlier,
        Self::FitSplineLowess,
        Self::AreaUnderCurve,
        Self::TransformMath,
        Self::TransformNormalize,
        Self::TransformRemoveBaseline,
        Self::TransformTransposeColumn,
        Self::TransformTransposeXyGrouped,
        Self::TransformPruneDecimate,
        Self::TransformPruneAverageBins,
        Self::TransformSelectAndTransform,
        Self::TransformSimulate,
        Self::TransformRandomSample,
        Self::RowStatistics,
        Self::FractionOfTotal,
        Self::CurveCalculus,
        Self::MonteCarloSimulation,
        Self::FrequencyDistribution,
        Self::Ancova,
    ];

    /// Total number of procedure rows in [`SUMMARY_MODE_MATRIX`].
    pub const COUNT: usize = Self::ALL.len();

    /// Return the static [`SummaryModeMatrixRow`] for this procedure discriminant.
    #[must_use]
    pub const fn row(self) -> &'static SummaryModeMatrixRow {
        match self {
            Self::DescriptiveStats => &SUMMARY_MODE_MATRIX[0],
            Self::OutlierDetection => &SUMMARY_MODE_MATRIX[1],
            Self::TwoGroupParametricSummary => &SUMMARY_MODE_MATRIX[2],
            Self::TwoGroupRawOnly => &SUMMARY_MODE_MATRIX[3],
            Self::BlandAltman => &SUMMARY_MODE_MATRIX[4],
            Self::MultipleRowTTests => &SUMMARY_MODE_MATRIX[5],
            Self::PassingBablok => &SUMMARY_MODE_MATRIX[6],
            Self::OneWayAnovaParametricSummary => &SUMMARY_MODE_MATRIX[7],
            Self::OneWayAnovaRawOnly => &SUMMARY_MODE_MATRIX[8],
            Self::FactorialAnova => &SUMMARY_MODE_MATRIX[9],
            Self::ContingencyColumnBinary => &SUMMARY_MODE_MATRIX[10],
            Self::RocCurve => &SUMMARY_MODE_MATRIX[11],
            Self::GeneralizedLinearModel => &SUMMARY_MODE_MATRIX[12],
            Self::Multivariate => &SUMMARY_MODE_MATRIX[13],
            Self::PValueStack => &SUMMARY_MODE_MATRIX[14],
            Self::PowerAnalysis => &SUMMARY_MODE_MATRIX[15],
            Self::SimpleLogisticRegression => &SUMMARY_MODE_MATRIX[16],
            Self::LeveneTest => &SUMMARY_MODE_MATRIX[17],
            Self::LinearRegressionOls => &SUMMARY_MODE_MATRIX[18],
            Self::LinearRegressionDeming => &SUMMARY_MODE_MATRIX[19],
            Self::NonlinearRegressionStandard => &SUMMARY_MODE_MATRIX[20],
            Self::NonlinearRegressionObservedSdWeighted => &SUMMARY_MODE_MATRIX[21],
            Self::NonlinearRegressionRobustOrOutlier => &SUMMARY_MODE_MATRIX[22],
            Self::FitSplineLowess => &SUMMARY_MODE_MATRIX[23],
            Self::AreaUnderCurve => &SUMMARY_MODE_MATRIX[24],
            Self::TransformMath => &SUMMARY_MODE_MATRIX[25],
            Self::TransformNormalize => &SUMMARY_MODE_MATRIX[26],
            Self::TransformRemoveBaseline => &SUMMARY_MODE_MATRIX[27],
            Self::TransformTransposeColumn => &SUMMARY_MODE_MATRIX[28],
            Self::TransformTransposeXyGrouped => &SUMMARY_MODE_MATRIX[29],
            Self::TransformPruneDecimate => &SUMMARY_MODE_MATRIX[30],
            Self::TransformPruneAverageBins => &SUMMARY_MODE_MATRIX[31],
            Self::TransformSelectAndTransform => &SUMMARY_MODE_MATRIX[32],
            Self::TransformSimulate => &SUMMARY_MODE_MATRIX[33],
            Self::TransformRandomSample => &SUMMARY_MODE_MATRIX[34],
            Self::RowStatistics => &SUMMARY_MODE_MATRIX[35],
            Self::FractionOfTotal => &SUMMARY_MODE_MATRIX[36],
            Self::CurveCalculus => &SUMMARY_MODE_MATRIX[37],
            Self::MonteCarloSimulation => &SUMMARY_MODE_MATRIX[38],
            Self::FrequencyDistribution => &SUMMARY_MODE_MATRIX[39],
            Self::Ancova => &SUMMARY_MODE_MATRIX[40],
        }
    }

    const fn from_transform(tx: &TransformAnalysisSpec, table_kind: TableKind) -> Self {
        match tx {
            TransformAnalysisSpec::Math { .. } => Self::TransformMath,
            TransformAnalysisSpec::Normalize { .. } => Self::TransformNormalize,
            TransformAnalysisSpec::RemoveBaseline { .. } => Self::TransformRemoveBaseline,
            TransformAnalysisSpec::Transpose { .. } => {
                if matches!(table_kind, TableKind::Column) {
                    Self::TransformTransposeColumn
                } else {
                    Self::TransformTransposeXyGrouped
                }
            }
            TransformAnalysisSpec::PruneRows { average_window: false, .. } => Self::TransformPruneDecimate,
            TransformAnalysisSpec::PruneRows { average_window: true, .. } => Self::TransformPruneAverageBins,
            TransformAnalysisSpec::SelectAndTransform { .. } => Self::TransformSelectAndTransform,
            TransformAnalysisSpec::Simulate { .. } => Self::TransformSimulate,
            TransformAnalysisSpec::RandomSample { .. } => Self::TransformRandomSample,
        }
    }

    const fn from_two_group(method: TwoGroupMethod, table_kind: TableKind) -> Option<Self> {
        if !matches!(table_kind, TableKind::Column) {
            return None;
        }
        Some(match method {
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::OneSampleT { .. } => Self::TwoGroupParametricSummary,
            TwoGroupMethod::PairedT
            | TwoGroupMethod::RatioPairedT
            | TwoGroupMethod::LognormalUnpairedT
            | TwoGroupMethod::LognormalWelchT
            | TwoGroupMethod::MannWhitneyU
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::PairedSignTest
            | TwoGroupMethod::KolmogorovSmirnov
            | TwoGroupMethod::OneSampleWilcoxon { .. }
            | TwoGroupMethod::OneSampleRatioT { .. } => Self::TwoGroupRawOnly,
        })
    }

    const fn from_one_way(method: OneWayMethod, table_kind: TableKind) -> Option<Self> {
        if !matches!(table_kind, TableKind::Column) {
            return None;
        }
        Some(match method {
            OneWayMethod::OrdinaryAnova | OneWayMethod::BrownForsytheAndWelch => Self::OneWayAnovaParametricSummary,
            OneWayMethod::RepeatedMeasures { .. }
            | OneWayMethod::KruskalWallis
            | OneWayMethod::Friedman
            | OneWayMethod::LognormalOrdinary
            | OneWayMethod::LognormalWelchBf
            | OneWayMethod::LognormalRepeatedMeasures { .. } => Self::OneWayAnovaRawOnly,
        })
    }

    const fn from_nonlinear(method: FitMethod, outlier_handling: OutlierHandling, weighting: WeightingScheme, table_kind: TableKind) -> Option<Self> {
        if !matches!(table_kind, TableKind::Xy | TableKind::Grouped) {
            return None;
        }
        if matches!(method, FitMethod::RobustCauchy | FitMethod::RoutOutlierElimination { .. }) || !matches!(outlier_handling, OutlierHandling::None)
        {
            Some(Self::NonlinearRegressionRobustOrOutlier)
        } else if matches!(weighting, WeightingScheme::ObservedSd) {
            Some(Self::NonlinearRegressionObservedSdWeighted)
        } else {
            Some(Self::NonlinearRegressionStandard)
        }
    }

    const fn when(cond: bool, proc: Self) -> Option<Self> {
        if cond { Some(proc) } else { None }
    }

    /// Map a concrete [`AnalysisKind`] and [`TableKind`] to its canonical [`SummaryModeProcedure`].
    ///
    /// Matches exhaustively on [`AnalysisKind`] with zero wildcards so adding a new analysis kind
    /// requires explicitly classifying its summary-mode behavior here.
    #[must_use]
    pub const fn from_analysis_kind(kind: &AnalysisKind, table_kind: TableKind) -> Option<Self> {
        let is_col = matches!(table_kind, TableKind::Column);
        let is_xy = matches!(table_kind, TableKind::Xy);
        let is_grp = matches!(table_kind, TableKind::Grouped);
        if !(is_col || is_xy || is_grp) {
            return None;
        }
        match kind {
            AnalysisKind::Transform(tx) => Some(Self::from_transform(tx, table_kind)),
            AnalysisKind::DescriptiveStats(_) => Some(Self::DescriptiveStats),
            AnalysisKind::OutlierDetection(_) => Some(Self::OutlierDetection),
            AnalysisKind::TwoGroupComparison(spec) => Self::from_two_group(spec.method, table_kind),
            AnalysisKind::BlandAltman(_) => Self::when(is_col || is_xy, Self::BlandAltman),
            AnalysisKind::MultipleRowTTests(_) => Some(Self::MultipleRowTTests),
            AnalysisKind::PassingBablok(_) => Self::when(is_col || is_xy, Self::PassingBablok),
            AnalysisKind::OneWayAnova(spec) => Self::from_one_way(spec.method, table_kind),
            AnalysisKind::FactorialAnova(_) => Self::when(is_grp, Self::FactorialAnova),
            AnalysisKind::Contingency(spec) => {
                Self::when(is_col && matches!(spec.method, ContingencyMethod::ExactBinomial), Self::ContingencyColumnBinary)
            }
            AnalysisKind::RocCurve(_) => Self::when(is_col, Self::RocCurve),
            AnalysisKind::GeneralizedLinearModel(_) => Self::when(is_col || is_xy, Self::GeneralizedLinearModel),
            AnalysisKind::Multivariate(_) => Self::when(is_col || is_xy, Self::Multivariate),
            AnalysisKind::LinearRegression(spec) => {
                Self::when(is_xy || is_grp, if spec.deming_model_ii { Self::LinearRegressionDeming } else { Self::LinearRegressionOls })
            }
            AnalysisKind::NonlinearRegression(spec) => Self::from_nonlinear(spec.method, spec.outlier_handling, spec.weighting, table_kind),
            AnalysisKind::PValueStack(_) => Some(Self::PValueStack),
            AnalysisKind::PowerAnalysis(_) => Some(Self::PowerAnalysis),
            AnalysisKind::SimpleLogisticRegression(_) => Self::when(is_xy, Self::SimpleLogisticRegression),
            AnalysisKind::FitSplineLowess(_) => Self::when(is_xy, Self::FitSplineLowess),
            AnalysisKind::LeveneTest(_) => Self::when(is_col, Self::LeveneTest),
            AnalysisKind::AreaUnderCurve(_) => Self::when(is_xy, Self::AreaUnderCurve),
            AnalysisKind::RowStatistics(_) => Some(Self::RowStatistics),
            AnalysisKind::FractionOfTotal(_) => Some(Self::FractionOfTotal),
            AnalysisKind::CurveCalculus(_) => Self::when(is_xy, Self::CurveCalculus),
            AnalysisKind::MonteCarloSimulation(_) => Some(Self::MonteCarloSimulation),
            AnalysisKind::FrequencyDistribution(_) => Some(Self::FrequencyDistribution),
            AnalysisKind::Ancova(_) => Self::when(is_xy, Self::Ancova),
            AnalysisKind::NestedComparison(_) | AnalysisKind::Survival(_) | AnalysisKind::ImportedSnapshot(_) => None,
        }
    }
}

/// One canonical row in [`SUMMARY_MODE_MATRIX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryModeMatrixRow {
    /// Canonical procedure discriminant for this row.
    pub procedure: SummaryModeProcedure,
    /// Parent [`AnalysisKindId`] in [`ALL_ANALYSIS_DESCRIPTORS`].
    pub analysis_kind_id: AnalysisKindId,
    /// Human-readable procedure name for diagnostics and typed error payloads.
    pub label: &'static str,
    /// Summary-capable [`TableKind`] archetypes (`Column`, `Xy`, `Grouped`) governed by this row.
    pub table_kinds: &'static [TableKind],
    /// Published textbook / literature reference backing every supported cell in this row.
    pub reference_citation: &'static str,
    /// Per-summary-mode verdicts across all eight pre-summarized [`SubcolumnMode`] variants.
    pub cells: SummaryModeRowCells,
}

impl SummaryModeMatrixRow {
    /// Return `true` when `table_kind` appears in [`Self::table_kinds`].
    #[must_use]
    pub const fn applies_to_table_kind(&self, table_kind: TableKind) -> bool {
        let mut rest = self.table_kinds;
        while let [head, tail @ ..] = rest {
            if table_kind_eq(*head, table_kind) {
                return true;
            }
            rest = tail;
        }
        false
    }

    /// Look up the cell verdict for `mode` (`None` for raw/single modes).
    #[must_use]
    pub const fn support_for(&self, mode: SubcolumnMode) -> Option<SummaryModeSupport> {
        self.cells.get(mode)
    }
}

const COL_ONLY: &[TableKind] = &[TableKind::Column];
const XY_ONLY: &[TableKind] = &[TableKind::Xy];
const GROUPED_ONLY: &[TableKind] = &[TableKind::Grouped];
const COL_XY: &[TableKind] = &[TableKind::Column, TableKind::Xy];
const XY_GROUPED: &[TableKind] = &[TableKind::Xy, TableKind::Grouped];
const COL_XY_GROUPED: &[TableKind] = &[TableKind::Column, TableKind::Xy, TableKind::Grouped];

const DESCRIPTIVE_CELLS: SummaryModeRowCells = SummaryModeRowCells {
    mean_sd_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sem_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_cv_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sd: SummaryModeSupport::Supported(SummaryModeSemantics::SpreadOnlySummary),
    mean_sem: SummaryModeSupport::Supported(SummaryModeSemantics::SpreadOnlySummary),
    mean_low_high: SummaryModeSupport::Supported(SummaryModeSemantics::IntervalLimitSummary),
    upper_lower_limits: SummaryModeSupport::Supported(SummaryModeSemantics::IntervalLimitSummary),
    median_iqr: SummaryModeSupport::Supported(SummaryModeSemantics::QuartileMedianSummary),
};

const REGRESSION_CENTRAL_OR_MOMENT_CELLS: SummaryModeRowCells = SummaryModeRowCells {
    mean_sd_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sem_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_cv_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sd: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSampleSizeOrVariance),
    mean_sem: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSampleSizeOrVariance),
    mean_low_high: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
    upper_lower_limits: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
    median_iqr: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
};

const NONLINEAR_OBSERVED_SD_CELLS: SummaryModeRowCells = SummaryModeRowCells {
    mean_sd_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sem_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_cv_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sd: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSampleSizeOrVariance),
    mean_sem: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSampleSizeOrVariance),
    mean_low_high: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSdForWeighting),
    upper_lower_limits: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSdForWeighting),
    median_iqr: SummaryModeSupport::Refused(SummaryModeRefusalReason::MissingSdForWeighting),
};

const CURVE_CENTRAL_OR_MOMENT_CELLS: SummaryModeRowCells = SummaryModeRowCells {
    mean_sd_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sem_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_cv_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sd: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
    mean_sem: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
    mean_low_high: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
    upper_lower_limits: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
    median_iqr: SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly),
};

const PRUNE_AVERAGE_BINS_CELLS: SummaryModeRowCells = SummaryModeRowCells {
    mean_sd_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sem_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_cv_n: SummaryModeSupport::Supported(SummaryModeSemantics::ParametricMoments),
    mean_sd: SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation),
    mean_sem: SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation),
    mean_low_high: SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation),
    upper_lower_limits: SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation),
    median_iqr: SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation),
};

/// Exhaustive canonical descriptor table covering every analysis, fit, and transform compatible
/// with `Column`, `Xy`, or `Grouped` tables across all eight summary [`SubcolumnMode`] layouts.
pub const SUMMARY_MODE_MATRIX: [SummaryModeMatrixRow; SummaryModeProcedure::COUNT] = [
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::DescriptiveStats,
        analysis_kind_id: AnalysisKindId::DescriptiveStats,
        label: "Descriptive statistics",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Zar (2010) Ch. 3-4, 10; Cochrane Handbook §6.5.2.10; Tukey (1977); Yeh & Kwan (1978)",
        cells: DESCRIPTIVE_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::OutlierDetection,
        analysis_kind_id: AnalysisKindId::OutlierDetection,
        label: "Outlier detection (Grubbs / ROUT)",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Grubbs (1969); Motulsky & Brown (2006) (requires individual observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TwoGroupParametricSummary,
        analysis_kind_id: AnalysisKindId::TwoGroupComparison,
        label: "Two-group parametric t-test (Student / Welch / One-sample)",
        table_kinds: COL_ONLY,
        reference_citation: "Student (1908); Welch (1947); Satterthwaite (1946); Zar (2010) Ch. 7-8",
        cells: SummaryModeRowCells::parametric_only(SummaryModeRefusalReason::MissingSampleSizeOrVariance),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TwoGroupRawOnly,
        analysis_kind_id: AnalysisKindId::TwoGroupComparison,
        label: "Two-group paired, lognormal, or nonparametric test",
        table_kinds: COL_ONLY,
        reference_citation: "Wilcoxon (1945); Mann & Whitney (1947); Zar (2010) Ch. 8-9 (requires raw observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::BlandAltman,
        analysis_kind_id: AnalysisKindId::BlandAltman,
        label: "Bland-Altman method comparison",
        table_kinds: COL_XY,
        reference_citation: "Bland & Altman (1986, 1999) (requires paired subject observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::MultipleRowTTests,
        analysis_kind_id: AnalysisKindId::MultipleRowTTests,
        label: "Multiple row-by-row t-tests",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Benjamini & Hochberg (1995); Holm (1979) (requires raw row replicates)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::PassingBablok,
        analysis_kind_id: AnalysisKindId::PassingBablok,
        label: "Passing-Bablok regression",
        table_kinds: COL_XY,
        reference_citation: "Passing & Bablok (1983) (nonparametric pairwise slopes require raw pairs)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::OneWayAnovaParametricSummary,
        analysis_kind_id: AnalysisKindId::OneWayAnova,
        label: "One-way parametric ANOVA (Ordinary / Brown-Forsythe / Welch)",
        table_kinds: COL_ONLY,
        reference_citation: "Zar (2010) Ch. 10; Welch (1951); Brown & Forsythe (1974b); Maxwell & Delaney (2004)",
        cells: SummaryModeRowCells::parametric_only(SummaryModeRefusalReason::MissingSampleSizeOrVariance),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::OneWayAnovaRawOnly,
        analysis_kind_id: AnalysisKindId::OneWayAnova,
        label: "One-way repeated-measures or nonparametric ANOVA (RM / Kruskal-Wallis / Friedman)",
        table_kinds: COL_ONLY,
        reference_citation: "Kruskal & Wallis (1952); Friedman (1937); Geisser & Greenhouse (1958) (requires raw observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::FactorialAnova,
        analysis_kind_id: AnalysisKindId::FactorialAnova,
        label: "Factorial two-way / three-way ANOVA & REML",
        table_kinds: GROUPED_ONLY,
        reference_citation: "Maxwell & Delaney (2004); Searle et al. (1992) (requires cell-level raw replicates)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::ContingencyColumnBinary,
        analysis_kind_id: AnalysisKindId::Contingency,
        label: "Exact binomial test (Column binary 0/1)",
        table_kinds: COL_ONLY,
        reference_citation: "Clopper & Pearson (1934); Sterne (1954) (requires raw binary 0/1 observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::RocCurve,
        analysis_kind_id: AnalysisKindId::RocCurve,
        label: "ROC curve analysis",
        table_kinds: COL_ONLY,
        reference_citation: "DeLong et al. (1988); Hanley & McNeil (1982) (requires subject-level scores)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::GeneralizedLinearModel,
        analysis_kind_id: AnalysisKindId::GeneralizedLinearModel,
        label: "Generalized linear model (GLM)",
        table_kinds: COL_XY,
        reference_citation: "McCullagh & Nelder (1989) (requires case-level observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::Multivariate,
        analysis_kind_id: AnalysisKindId::Multivariate,
        label: "Multivariate analysis (PCA / Correlation / Clustering / MI)",
        table_kinds: COL_XY,
        reference_citation: "Jolliffe (2002); Rubin (1987) (requires case-level observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::PValueStack,
        analysis_kind_id: AnalysisKindId::PValueStack,
        label: "P-value stack multiplicity adjustment",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Benjamini & Hochberg (1995); Holm (1979) (requires individual P values)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::PowerAnalysis,
        analysis_kind_id: AnalysisKindId::PowerAnalysis,
        label: "Statistical power & sample size",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Cohen (1988) (prospective specification-driven calculation)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::TableIndependent)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::SimpleLogisticRegression,
        analysis_kind_id: AnalysisKindId::SimpleLogisticRegression,
        label: "Simple logistic regression",
        table_kinds: XY_ONLY,
        reference_citation: "Hosmer, Lemeshow & Sturdivant (2013) (requires single-subcolumn binary 0/1 Y)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::UnsupportedSummaryMode)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::LeveneTest,
        analysis_kind_id: AnalysisKindId::LeveneTest,
        label: "Levene / Brown-Forsythe homogeneity of variances",
        table_kinds: COL_ONLY,
        reference_citation: "Levene (1960); Brown & Forsythe (1974) (absolute deviations require raw observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::LinearRegressionOls,
        analysis_kind_id: AnalysisKindId::LinearRegression,
        label: "Simple linear regression (Model I OLS)",
        table_kinds: XY_GROUPED,
        reference_citation: "Draper & Smith (1998); Zar (2010) Ch. 17 (replicate-weighted moments or row-level centers)",
        cells: REGRESSION_CENTRAL_OR_MOMENT_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::LinearRegressionDeming,
        analysis_kind_id: AnalysisKindId::LinearRegression,
        label: "Deming (Model II) linear regression",
        table_kinds: XY_GROUPED,
        reference_citation: "Deming (1943); Linnet (1990, 1993) (LOO jackknife requires raw observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::NonlinearRegressionStandard,
        analysis_kind_id: AnalysisKindId::NonlinearRegression,
        label: "Nonlinear regression (Least squares / Poisson, standard weighting)",
        table_kinds: XY_GROUPED,
        reference_citation: "Seber & Wild (1989); Motulsky & Christopoulos (2004) Ch. 16",
        cells: REGRESSION_CENTRAL_OR_MOMENT_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::NonlinearRegressionObservedSdWeighted,
        analysis_kind_id: AnalysisKindId::NonlinearRegression,
        label: "Nonlinear regression (1/SD^2 inverse-variance weighting)",
        table_kinds: XY_GROUPED,
        reference_citation: "Seber & Wild (1989); Motulsky & Christopoulos (2004) Ch. 16 (w_i = 1 / SD_i^2)",
        cells: NONLINEAR_OBSERVED_SD_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::NonlinearRegressionRobustOrOutlier,
        analysis_kind_id: AnalysisKindId::NonlinearRegression,
        label: "Nonlinear regression (Cauchy robust / ROUT outlier elimination)",
        table_kinds: XY_GROUPED,
        reference_citation: "Motulsky & Brown (2006) (robust loss and ROUT outlier flagging require raw replicates)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::FitSplineLowess,
        analysis_kind_id: AnalysisKindId::FitSplineLowess,
        label: "Fit spline / LOWESS curve",
        table_kinds: XY_ONLY,
        reference_citation: "Reinsch (1967); Akima (1970); Cleveland (1979) (fits central Y curve)",
        cells: CURVE_CENTRAL_OR_MOMENT_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::AreaUnderCurve,
        analysis_kind_id: AnalysisKindId::AreaUnderCurve,
        label: "Area under the curve (AUC)",
        table_kinds: XY_ONLY,
        reference_citation: "Yeh & Kwan (1978); Bailer (1988) (Bailer SE computed only when parametric N/SD present)",
        cells: CURVE_CENTRAL_OR_MOMENT_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformMath,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Math",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Ku (1966); Bevington & Robinson (2003) (delta-method and endpoint propagation)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::SubcolumnPropagation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformNormalize,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Normalize",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Ku (1966) (affine rescaling with orientation-preserving bound swap)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::SubcolumnPropagation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformRemoveBaseline,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Remove baseline",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Ku (1966) (baseline difference/ratio/percent change with bound swap)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::SubcolumnPropagation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformTransposeColumn,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Transpose (Column)",
        table_kinds: COL_ONLY,
        reference_citation: "Multi-subcolumn Column transpose is refused to preserve group-summary structure",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::UnsupportedSummaryMode)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformTransposeXyGrouped,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Transpose (XY / Grouped)",
        table_kinds: XY_GROUPED,
        reference_citation: "Exact cell-tuple transposition preserving all subcolumn modes into Column / Grouped",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformPruneDecimate,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Prune rows (Every Nth)",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Exact row decimation preserving intact summary tuples",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformPruneAverageBins,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Prune rows (Average bins)",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Chan, Golub & LeVeque (1983); Zar (2010) (exact parametric moment pooling; center-only for unpoolable modes)",
        cells: PRUNE_AVERAGE_BINS_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformSelectAndTransform,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Select and transform",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Row predicate filtering over primary subcolumn 0 preserving summary tuples",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformSimulate,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Simulate XY",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Seeded synthetic equation generator independent of source subcolumn mode",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::TableIndependent)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::TransformRandomSample,
        analysis_kind_id: AnalysisKindId::Transform,
        label: "Transform: Random sample",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Fisher & Yates (1938) (deterministic row sampling without replacement preserving summary tuples)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::StructuralPreservation)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::RowStatistics,
        analysis_kind_id: AnalysisKindId::RowStatistics,
        label: "Row statistics (replicate summaries)",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Zar (2010) (reconstructs parametric moments when N/SD present; central value otherwise)",
        cells: CURVE_CENTRAL_OR_MOMENT_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::FractionOfTotal,
        analysis_kind_id: AnalysisKindId::FractionOfTotal,
        label: "Fraction of total",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Wilson (1927); Clopper & Pearson (1934) (operates on primary subcolumn 0 values)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::CentralLocationOnly)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::CurveCalculus,
        analysis_kind_id: AnalysisKindId::CurveCalculus,
        label: "Curve calculus (differentiate, integrate, smooth)",
        table_kinds: XY_ONLY,
        reference_citation: "Savitzky & Golay (1964); Cleveland (1979); Yeh & Kwan (1978) (operates on central Y curve)",
        cells: CURVE_CENTRAL_OR_MOMENT_CELLS,
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::MonteCarloSimulation,
        analysis_kind_id: AnalysisKindId::MonteCarloSimulation,
        label: "Monte Carlo simulation",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Metropolis & Ulam (1949) (chained seeded simulation independent of source subcolumn mode)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Supported(SummaryModeSemantics::TableIndependent)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::FrequencyDistribution,
        analysis_kind_id: AnalysisKindId::FrequencyDistribution,
        label: "Frequency distribution & histogram",
        table_kinds: COL_XY_GROUPED,
        reference_citation: "Sturges (1926); Scott (1979); Freedman & Diaconis (1981) (binning requires raw observations)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
    SummaryModeMatrixRow {
        procedure: SummaryModeProcedure::Ancova,
        analysis_kind_id: AnalysisKindId::Ancova,
        label: "Analysis of covariance (ANCOVA)",
        table_kinds: XY_ONLY,
        reference_citation: "Maxwell & Delaney (2004); Kutner et al. (2005) (requires observation-level covariate-response pairs)",
        cells: SummaryModeRowCells::all(SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates)),
    },
];

/// Return the complete slice of canonical [`SummaryModeMatrixRow`] entries.
#[must_use]
pub const fn summary_mode_matrix_rows() -> &'static [SummaryModeMatrixRow] {
    &SUMMARY_MODE_MATRIX
}

/// Look up the canonical [`SummaryModeMatrixRow`] governing `kind` on `table_kind`, if `kind`
/// accepts `table_kind` and `table_kind` supports summary subcolumn modes.
#[must_use]
pub const fn summary_mode_row_for_spec(kind: &AnalysisKind, table_kind: TableKind) -> Option<&'static SummaryModeMatrixRow> {
    match SummaryModeProcedure::from_analysis_kind(kind, table_kind) {
        Some(proc_id) => Some(proc_id.row()),
        None => None,
    }
}

/// Evaluate the canonical [`SummaryModeSupport`] verdict for `(kind, table_kind, mode)`.
///
/// Returns `None` when `mode` is not a pre-summarized mode (`Single` or `RawReplicates(_)`) or
/// when `kind` does not accept `table_kind` (which callers reject with their `IncompatibleTable`
/// error first).
#[must_use]
pub const fn summary_mode_support(kind: &AnalysisKind, table_kind: TableKind, mode: SubcolumnMode) -> Option<SummaryModeSupport> {
    if !mode.is_pre_summarized() {
        return None;
    }
    match summary_mode_row_for_spec(kind, table_kind) {
        Some(row) => row.support_for(mode),
        None => None,
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

const fn analysis_kind_id_eq(a: AnalysisKindId, b: AnalysisKindId) -> bool {
    (a as usize) == (b as usize)
}

const fn matrix_covers_kind_and_table(id: AnalysisKindId, tk: TableKind) -> bool {
    let mut rows = SUMMARY_MODE_MATRIX.as_slice();
    while let [row, tail @ ..] = rows {
        if analysis_kind_id_eq(row.analysis_kind_id, id) && row.applies_to_table_kind(tk) {
            return true;
        }
        rows = tail;
    }
    false
}

// Compile-time bijection, completeness, and descriptor-consistency verification.
const _: () = {
    // 1. Every SummaryModeProcedure slot matches its index in SUMMARY_MODE_MATRIX and carries valid metadata.
    let mut i = 0;
    let mut procs = SummaryModeProcedure::ALL.as_slice();
    let mut rows = SUMMARY_MODE_MATRIX.as_slice();
    while let ([p, rest_p @ ..], [r, rest_r @ ..]) = (procs, rows) {
        assert!(*p as usize == i);
        assert!(r.procedure as usize == i);
        assert!(r.procedure.row().procedure as usize == i);
        assert!(!r.label.is_empty());
        assert!(!r.reference_citation.is_empty());
        assert!(!r.table_kinds.is_empty());

        let desc = r.analysis_kind_id.descriptor();
        assert!(desc.user_creatable);
        let mut tks = r.table_kinds;
        while let [tk, rest_tk @ ..] = tks {
            assert!(matches!(*tk, TableKind::Column | TableKind::Xy | TableKind::Grouped));
            assert!(desc.supports_table_kind(*tk));
            tks = rest_tk;
        }

        // Every summary mode in SUMMARY_SUBCOLUMN_MODES resolves to Some(_).
        let mut modes = SUMMARY_SUBCOLUMN_MODES.as_slice();
        while let [m, rest_m @ ..] = modes {
            assert!(m.is_pre_summarized());
            assert!(r.support_for(*m).is_some());
            modes = rest_m;
        }

        procs = rest_p;
        rows = rest_r;
        i += 1;
    }
    assert!(i == SummaryModeProcedure::COUNT);
    assert!(procs.is_empty());
    assert!(rows.is_empty());

    // 2. Every user-creatable AnalysisKindDescriptor compatible with Column, Xy, or Grouped is
    //    covered in SUMMARY_MODE_MATRIX for every one of those table kinds.
    let mut descs = ALL_ANALYSIS_DESCRIPTORS.as_slice();
    while let [desc, rest_d @ ..] = descs {
        if desc.user_creatable {
            let mut tks = SUMMARY_CAPABLE_TABLE_KINDS.as_slice();
            while let [tk, rest_tk @ ..] = tks {
                if desc.supports_table_kind(*tk) {
                    assert!(matrix_covers_kind_and_table(desc.id, *tk));
                }
                tks = rest_tk;
            }
        }
        descs = rest_d;
    }
};
