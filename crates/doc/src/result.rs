//! `ResultSheet` model: multi-tab statistical/regression outputs, unrounded *p*-values, profile-likelihood
//! `CiBound` endpoints (`Finite` | `Open`), hookable scalar outputs, live pairwise comparison
//! records, fitted curve overlays, and plain-language `ChecklistItem` diagnostics
//! (`docs/architecture.md`, `docs/implementation/statistics-implementation.md` §6, `docs/implementation/curve-fitting-implementation.md` §2).

use std::fmt;

use curvebook_palette::Rgba8;

use crate::analysis::AnalysisSpec;
use crate::error::DocError;
use crate::ids::{FloatingNote, SheetId};
use crate::links::DataSheet;

/// High-level grouping for structured [`ErrorCode`] diagnostics across L1/L2 crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Caller-supplied parameter, coordinate, or domain validation failure.
    Input,
    /// Document state conflict (missing sheet, duplicate ID, frozen sheet, or dependency cycle).
    State,
    /// Archive, schema version, or import file-format parsing failure.
    Format,
    /// Operating-system filesystem or stream I/O failure.
    Io,
    /// Security, sandbox, resource-budget, or `--gui-only` channel policy rejection.
    Policy,
    /// Statistical, regression, or numerical solver failure (non-convergence, singular design, or insufficient observations).
    Compute,
    /// Unexpected internal invariant failure or caught panic boundary diagnostic.
    Internal,
}

impl ErrorCategory {
    /// Complete ordered list of all canonical [`ErrorCategory`] variants.
    pub const ALL: [Self; 7] = [Self::Input, Self::State, Self::Format, Self::Io, Self::Policy, Self::Compute, Self::Internal];

    /// Canonical `snake_case` identifier for this error category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::State => "state",
            Self::Format => "format",
            Self::Io => "io",
            Self::Policy => "policy",
            Self::Compute => "compute",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable machine-readable error code for structured diagnostics across document, engine, format, and UI layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// Invalid parameter value, malformed expression, out-of-bounds cell coordinate, or incompatible table archetype (`"bad_input"`).
    BadInput,
    /// Target sheet, column, checkpoint, or resource identifier does not exist (`"not_found"`).
    NotFound,
    /// Attempted to create a sheet or entity whose identifier is already in use (`"already_exists"`).
    AlreadyExists,
    /// Concurrent or mutually exclusive state conflict (`"conflict"`).
    Conflict,
    /// Edit attempted on a [`DataSheet`] locked with `frozen == true` (`"frozen_sheet"`).
    FrozenSheet,
    /// Directed cycle detected in formula, constant-hook, or linked-data dependencies (`"cycle_detected"`).
    CycleDetected,
    /// Unrecognized or unsupported external file container or variant (`"unsupported_format"`).
    UnsupportedFormat,
    /// Persisted schema version is newer than this release supports (refusing destructive downgrade/overwrite; `"unsupported_version"`).
    UnsupportedVersion,
    /// Malformed archive, checksum mismatch, or corrupt serialized payload (`"corrupt_data"`).
    CorruptData,
    /// Filesystem read, atomic write, or subprocess I/O error (`"io_error"`).
    IoError,
    /// Archive entry or caller path attempted directory traversal (`..` or absolute escape; `"path_escape"`).
    PathEscape,
    /// Dimension, cell count, archive entry count, decompression size, or timeout exceeded configured safety limits (`"resource_limit"`).
    ResourceLimit,
    /// Iterative numerical, statistical, or nonlinear regression solver failed to converge or encountered a singular system (`"solver_failed"`).
    SolverFailed,
    /// Too few active observations, events, or degrees of freedom for the requested statistical or regression model (`"insufficient_data"`).
    InsufficientData,
    /// Long-running computation or user workflow was explicitly cancelled (`"cancelled"`).
    Cancelled,
    /// Direct non-GUI mutation or read attempted while strict `--gui-only` policy is active (`"gui_only_violation"`).
    GuiOnlyViolation,
    /// Caught panic converted into a structured error at the `guard.rs` defence-in-depth boundary (`"internal_panic"`).
    InternalPanic,
    /// Fallback code for unclassified internal failures or unrecognized legacy error strings (`"internal_error"`).
    InternalError,
}

impl ErrorCode {
    /// Complete ordered list of all canonical [`ErrorCode`] variants.
    pub const ALL: [Self; 18] = [
        Self::BadInput,
        Self::NotFound,
        Self::AlreadyExists,
        Self::Conflict,
        Self::FrozenSheet,
        Self::CycleDetected,
        Self::UnsupportedFormat,
        Self::UnsupportedVersion,
        Self::CorruptData,
        Self::IoError,
        Self::PathEscape,
        Self::ResourceLimit,
        Self::SolverFailed,
        Self::InsufficientData,
        Self::Cancelled,
        Self::GuiOnlyViolation,
        Self::InternalPanic,
        Self::InternalError,
    ];

    /// Canonical stable `snake_case` identifier for this error code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadInput => "bad_input",
            Self::NotFound => "not_found",
            Self::AlreadyExists => "already_exists",
            Self::Conflict => "conflict",
            Self::FrozenSheet => "frozen_sheet",
            Self::CycleDetected => "cycle_detected",
            Self::UnsupportedFormat => "unsupported_format",
            Self::UnsupportedVersion => "unsupported_version",
            Self::CorruptData => "corrupt_data",
            Self::IoError => "io_error",
            Self::PathEscape => "path_escape",
            Self::ResourceLimit => "resource_limit",
            Self::SolverFailed => "solver_failed",
            Self::InsufficientData => "insufficient_data",
            Self::Cancelled => "cancelled",
            Self::GuiOnlyViolation => "gui_only_violation",
            Self::InternalPanic => "internal_panic",
            Self::InternalError => "internal_error",
        }
    }

    /// Coarse [`ErrorCategory`] classification for this error code.
    #[must_use]
    pub const fn category(self) -> ErrorCategory {
        match self {
            Self::BadInput => ErrorCategory::Input,
            Self::NotFound | Self::AlreadyExists | Self::Conflict | Self::FrozenSheet | Self::CycleDetected => ErrorCategory::State,
            Self::UnsupportedFormat | Self::UnsupportedVersion | Self::CorruptData => ErrorCategory::Format,
            Self::IoError => ErrorCategory::Io,
            Self::PathEscape | Self::ResourceLimit | Self::GuiOnlyViolation => ErrorCategory::Policy,
            Self::SolverFailed | Self::InsufficientData | Self::Cancelled => ErrorCategory::Compute,
            Self::InternalPanic | Self::InternalError => ErrorCategory::Internal,
        }
    }

    /// Parse a canonical `snake_case` code or known legacy alias, mapping unknown legacy strings to
    /// [`Self::InternalError`] so historical snapshots remain readable.
    #[must_use]
    pub fn from_code_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "bad_input" | "invalid_input" | "validation_error" => Self::BadInput,
            "not_found" | "missing_source" | "sheet_not_found" => Self::NotFound,
            "already_exists" | "duplicate_id" => Self::AlreadyExists,
            "conflict" => Self::Conflict,
            "frozen_sheet" | "frozen" => Self::FrozenSheet,
            "cycle_detected" | "cycle" => Self::CycleDetected,
            "unsupported_format" => Self::UnsupportedFormat,
            "unsupported_version" => Self::UnsupportedVersion,
            "corrupt_data" | "corrupt" => Self::CorruptData,
            "io_error" | "io" => Self::IoError,
            "path_escape" => Self::PathEscape,
            "resource_limit" => Self::ResourceLimit,
            "solver_failed" | "analysis_error" => Self::SolverFailed,
            "insufficient_data" => Self::InsufficientData,
            "cancelled" | "canceled" => Self::Cancelled,
            "gui_only_violation" => Self::GuiOnlyViolation,
            "internal_panic" | "panic" => Self::InternalPanic,
            _ => Self::InternalError,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ErrorCodeVisitor;
        impl serde::de::Visitor<'_> for ErrorCodeVisitor {
            type Value = ErrorCode;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a snake_case error code string")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ErrorCode::from_code_str(value))
            }
        }
        deserializer.deserialize_str(ErrorCodeVisitor)
    }
}

/// Recalculation status of a [`ResultSheet`] inside `curvebook-engine`'s reactive DAG.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultStatus {
    /// Upstream inputs or [`AnalysisSpec`] parameters changed and recalculation is pending.
    #[default]
    Dirty,
    /// `tabs`, `scalars`, `pairwise_comparisons`, `fitted_curves`, and `checklist` reflect the latest upstream state.
    UpToDate,
    /// Most recent recalculation failed; [`ResultSheet::lookup_scalar`] returns `None` while failed so stale values never propagate through hooks.
    Failed {
        /// Machine-readable failure classification.
        code: ErrorCode,
        /// Human-readable diagnostic message explaining why the analysis failed.
        message: String,
    },
}

/// Confidence-interval endpoint supporting unbounded profile-likelihood plateaus (`CiBound::Open`;
/// `docs/implementation/curve-fitting-implementation.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CiBound {
    /// Resolved finite endpoint in the parameter's or effect size's native scale.
    Finite(f64),
    /// Profile-likelihood or score threshold was not crossed within the identifiable search domain (rendered as `"???"` / open interval rather than `NaN`).
    Open,
}

/// Typed cell inside a [`ResultTab`].
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultCell {
    /// Blank table cell (not applicable or suppressed entry).
    #[default]
    Empty,
    /// Scalar estimate, test statistic, degrees of freedom, or count.
    Number {
        /// Numeric value in the row/column's reported scale (guaranteed finite after result sanitization; non-identifiable values use `Empty` or `Text` rather than `NaN`).
        value: f64,
        /// Optional diagnostic prefix such as `"~"` on ambiguous nonlinear parameters.
        prefix: Option<String>,
        /// Optional physical or percentage unit suffix (for example `"%"` or `"nM"`), or `None` when dimensionless.
        units: Option<String>,
    },
    /// Two-sided confidence interval at the owning [`AnalysisSpec::confidence_level`].
    ConfidenceInterval {
        /// Lower confidence bound (`CiBound::Open` when unbounded on the left).
        lower: CiBound,
        /// Upper confidence bound (`CiBound::Open` when unbounded on the right).
        upper: CiBound,
    },
    /// Stores the exact unrounded `raw` probability alongside display strings.
    PValue {
        /// Unrounded probability `p` (or multiplicity-adjusted *p* / FDR *q*) in `[0.0, 1.0]`.
        raw: f64,
        /// Pre-formatted publication string rendered via [`AnalysisSpec::p_value_format`] (for example `"< 0.0001"` or `".002"`).
        formatted: String,
        /// Significance summary token (`"ns"`, `"*"`, `"**"`, `"***"`, or `"****"`) under [`AnalysisSpec::p_value_format`].
        stars: String,
    },
    /// Plain-text label, conclusion, CLD letter group, or categorical note.
    Text(String),
    /// Logical pass/fail or significance flag (`true` / `false`).
    Boolean(bool),
}

/// Semantic category of a sub-tab inside a [`ResultSheet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultTabKind {
    /// Primary test summary, omnibus statistics, or effect-size overview.
    #[default]
    MainSummary,
    /// Sum-of-squares, degrees-of-freedom, mean-square, *F*, and *p* decomposition table.
    AnovaTable,
    /// Post-hoc pairwise or vs-control multiplicity-adjusted comparison table (with optional CLD letters).
    MultipleComparisons,
    /// Regression parameter estimates, standard errors, confidence intervals, and identifiability notes.
    ParametersAndConfidenceIntervals,
    /// Nested extra-sum-of-squares *F*, likelihood-ratio, or Burnham–Anderson AICc model comparison table.
    ModelComparison,
    /// Observation-level fitted values, raw/weighted/studentized residuals, and outlier audit flags.
    Residuals,
    /// Per-group or per-column sample sizes, means, SDs, SEMs, medians, quartiles, and CIs.
    DescriptiveSummary,
    /// Inverse (`X` from `Y`) or forward (`Y` from `X`) calibration/standard-curve interpolations with CIs.
    InterpolatedUnknowns,
    /// Sampled `(X, Y)` curve coordinates and optional confidence/prediction band envelopes.
    CurveCoordinates,
    /// Kaplan–Meier subject counts remaining at risk across follow-up time landmarks.
    NumberAtRisk,
    /// Normality, homoscedasticity, collinearity (VIF), sphericity, or convergence diagnostic summary.
    Diagnostics,
    /// Parameter variance–covariance or correlation matrix from linear/nonlinear/GLM estimation.
    CovarianceMatrix,
    /// Theoretical normal quantiles paired with observed/residual values for Q–Q plotting.
    NormalQq,
    /// Breslow baseline survival schedule `S_0(t)` and cumulative hazard `H_0(t)` for Cox PH models.
    BaselineSurvival,
    /// Complementary log-log `ln(-ln S(t))` vs `ln(t)` coordinates for proportional-hazards visual checks.
    LogMinusLog,
}

/// Single tabular view within a [`ResultSheet`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResultTab {
    /// Stable tab identifier within `ResultSheet::tabs` (used by embedded result-table annotations on graphs and layouts).
    pub id: String,
    /// User-facing tab header title.
    pub title: String,
    /// Semantic role of the tab for automated UI/graph inspection.
    pub kind: ResultTabKind,
    /// Column header titles (`rows[r].len() == column_headers.len()`).
    pub column_headers: Vec<String>,
    /// Row stub labels (`row_labels.len() == rows.len()`).
    pub row_labels: Vec<String>,
    /// Row-major matrix of typed [`ResultCell`] entries.
    pub rows: Vec<Vec<ResultCell>>,
    /// Plain-text methodological notes, df/adjustment conventions, and filtering disclosures rendered beneath the table.
    pub footnotes: Vec<String>,
}

/// Named scalar published by a [`ResultSheet`] for constant hooking (`{{result:fit-1:EC50}}`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResultScalarEntry {
    /// Canonical parameter or statistic key matched case-insensitively by [`ResultSheet::lookup_scalar`] (for example `"t"`, `"df"`, `"p"`, `"LogEC50"`, or `"AUC"`).
    pub key: String,
    /// Zero-based upstream data-set index when the scalar is series-specific; `None` for omnibus or whole-table scalars.
    pub data_set_index: Option<usize>,
    /// Finite unrounded numeric value in the reported quantity's native units.
    pub value: f64,
    /// Optional physical or percentage unit string; `None` when dimensionless or inherited from the source axis.
    pub units: Option<String>,
}

/// Pairwise group comparison record consumed by live significance brackets on [`crate::GraphSpec`]
/// (`docs/implementation/graphing-and-layouts.md` §4).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairwiseComparisonRecord {
    /// Zero-based index of the first compared group or column on the plotted categorical axis.
    pub group_a: usize,
    /// Zero-based index of the second compared group or column on the plotted categorical axis.
    pub group_b: usize,
    /// Display label of `group_a`.
    pub label_a: String,
    /// Display label of `group_b`.
    pub label_b: String,
    /// Point estimate of the contrast (`mean_a - mean_b`, Hodges–Lehmann shift, log-rank ratio, etc.); `None` for omnibus/rank procedures that do not define an additive location contrast.
    pub estimate_diff: Option<f64>,
    /// Exact unrounded adjusted *p*-value (FWER/unadjusted, `is_fdr_q == false`) or *q*-value (FDR procedures such as Benjamini–Hochberg, Benjamini–Krieger–Yekutieli, or Benjamini–Yekutieli, `is_fdr_q == true`) in `[0.0, 1.0]`.
    pub raw_p_or_q: f64,
    /// `true` when `raw_p_or_q` is an FDR *q*-value compared against `fdr_q`, `false` when it is an unadjusted or FWER-adjusted *p*-value compared against `alpha = 1 - confidence_level`.
    pub is_fdr_q: bool,
    /// `true` when the comparison crosses the configured family-wise `alpha` or FDR `Q` threshold.
    pub significant: bool,
}

/// Sampled curve coordinate plus optional confidence and prediction band bounds for `curvebook-chart`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CurveSamplePoint {
    /// Independent variable coordinate in data-space *X* units.
    pub x: f64,
    /// Fitted curve, survival step, or ROC sensitivity coordinate in data-space *Y* units.
    pub y: f64,
    /// Lower two-sided confidence band bound at [`AnalysisSpec::confidence_level`]; `None` when bands are disabled or not identifiable at `x`.
    pub ci_lower: Option<f64>,
    /// Upper two-sided confidence band bound at [`AnalysisSpec::confidence_level`]; `None` when bands are disabled or not identifiable at `x`.
    pub ci_upper: Option<f64>,
    /// Lower two-sided single-observation prediction band bound at [`AnalysisSpec::confidence_level`]; `None` when prediction bands are disabled or undefined for the model family.
    pub pred_lower: Option<f64>,
    /// Upper two-sided single-observation prediction band bound at [`AnalysisSpec::confidence_level`]; `None` when prediction bands are disabled or undefined for the model family.
    pub pred_upper: Option<f64>,
}

/// Evaluated regression curve overlay associated with a data set on an upstream `DataSheet`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FittedCurveOverlay {
    /// Zero-based data-set column index on the upstream [`DataSheet`].
    pub data_set_index: usize,
    /// Canonical built-in equation ID (for example `"log_agonist_vs_response_4pl"`), `"linear"`, `"deming"`, or `"custom_dsl"`.
    pub equation_id: String,
    /// `true` when the regression solver converged cleanly for this series.
    pub converged: bool,
    /// `true` when the Jacobian/covariance was numerically rank-deficient (`HitAmbiguous` in Motulsky & Christopoulos 2004; rendered with a dashed warning stroke when plotted).
    pub ambiguous: bool,
    /// Ordered sequence of evaluated `(x, y)` coordinates and optional band envelopes across the plotted domain.
    pub points: Vec<CurveSamplePoint>,
}

/// Recommended non-destructive alternative statistical method for a violated assumption (`QOL-7` / `C2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RemediationKind {
    /// Unpaired Welch *t*-test with Welch–Satterthwaite degrees of freedom (Welch 1947; recommended when equal-variance assumptions fail between two groups).
    WelchT,
    /// Exact/asymptotic Mann–Whitney *U* rank-sum test (Mann & Whitney 1947; recommended when unpaired normality fails).
    MannWhitneyU,
    /// Paired Wilcoxon signed-rank test (Wilcoxon 1945; Pratt 1959; recommended when paired differences violate normality).
    WilcoxonSignedRank,
    /// One-sample Wilcoxon signed-rank test against a hypothetical median (recommended when one-sample normality fails).
    OneSampleWilcoxon,
    /// Brown–Forsythe and Welch heteroscedastic one-way ANOVA (Brown & Forsythe 1974; Welch 1951; recommended when homoscedasticity fails across `>= 3` groups).
    WelchAnova,
    /// Kruskal–Wallis one-way rank ANOVA (Kruskal & Wallis 1952; recommended when normality fails across `>= 3` independent groups).
    KruskalWallis,
    /// Friedman nonparametric repeated-measures rank test (Friedman 1937; recommended when repeated-measures normality fails).
    Friedman,
}

impl RemediationKind {
    /// Complete ordered list of all canonical [`RemediationKind`] variants.
    pub const ALL: [Self; 7] =
        [Self::WelchT, Self::MannWhitneyU, Self::WilcoxonSignedRank, Self::OneSampleWilcoxon, Self::WelchAnova, Self::KruskalWallis, Self::Friedman];

    /// Engine shorthand identifier accepted by `analysis.run` / `stats.run`.
    #[must_use]
    pub const fn engine_kind(self) -> &'static str {
        match self {
            Self::WelchT => "welch_t",
            Self::MannWhitneyU => "mann_whitney",
            Self::WilcoxonSignedRank => "wilcoxon",
            Self::OneSampleWilcoxon => "one_sample_wilcoxon",
            Self::WelchAnova => "welch_anova",
            Self::KruskalWallis => "kruskal_wallis",
            Self::Friedman => "friedman",
        }
    }

    /// Short human-readable label for the sibling remediation analysis.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WelchT => "Welch's t test",
            Self::MannWhitneyU => "Mann-Whitney U test",
            Self::WilcoxonSignedRank => "Wilcoxon matched-pairs test",
            Self::OneSampleWilcoxon => "One-sample Wilcoxon test",
            Self::WelchAnova => "Brown-Forsythe & Welch ANOVA",
            Self::KruskalWallis => "Kruskal-Wallis test",
            Self::Friedman => "Friedman test",
        }
    }
}

/// Severity status of an assumption check in [`ChecklistItem`] (`docs/implementation/statistics-implementation.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ChecklistStatus {
    /// Tested statistical assumption passed at the diagnostic threshold.
    Pass,
    /// Mild departure, low sample size, or borderline diagnostic warranting inspection.
    Warn,
    /// Tested assumption was clearly violated (often paired with a [`ChecklistItem::remediation`] suggestion).
    Violated,
    /// Informational disclosure (for example excluded cells, listwise deletion counts, or model parameterizations).
    Info,
}

/// Plain-language statistical assumption diagnostic attached to a [`ResultSheet`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChecklistItem {
    /// Short name of the assumption or data-hygiene check (for example `"Normality of residuals"` or `"Homogeneity of variances"`).
    pub assumption: String,
    /// Pass/warning/violation/informational severity badge.
    pub status: ChecklistStatus,
    /// Formatted test statistic and *p*-value backing the check (for example `"Shapiro-Wilk W = 0.912, p = 0.018"`); `None` for non-quantitative informational notes.
    pub diagnostic_metric: Option<String>,
    /// Plain-language interpretation of what the check found and how it affects inference.
    pub explanation: String,
    /// Optional non-destructive sibling test that can be spawned in one action when an assumption is [`ChecklistStatus::Warn`] or [`ChecklistStatus::Violated`]; `None` when no automated alternative applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<RemediationKind>,
}

/// Complete output node produced by running an [`AnalysisSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResultSheet {
    /// Project-unique sheet identifier (matches `analysis.id`).
    pub id: SheetId,
    /// Persisted [`AnalysisSpec`] configuration that produced (or will produce) this result sheet.
    pub analysis: AnalysisSpec,
    /// Current reactive recalculation state (`Dirty`, `UpToDate`, or `Failed`).
    pub status: ResultStatus,
    /// Present when this result is a derived data transform (`Transform`, `Normalize`, `Transpose`,
    /// `Simulate`) that can feed downstream statistical analyses, curve fits, and graphs; `None` for terminal statistical/regression result sheets.
    pub analyzable_data: Option<DataSheet>,
    /// Ordered tabular output views (`MainSummary`, `AnovaTable`, `MultipleComparisons`, `Residuals`, etc.).
    pub tabs: Vec<ResultTab>,
    /// Machine-readable scalar outputs addressable via [`Self::lookup_scalar`] and `{{result:...}}` hooks.
    pub scalars: Vec<ResultScalarEntry>,
    /// Pairwise comparison records consumed by live significance brackets and compact letter displays on graphs.
    pub pairwise_comparisons: Vec<PairwiseComparisonRecord>,
    /// Sampled regression, survival, or ROC curves with optional confidence/prediction bands for graph overlays.
    pub fitted_curves: Vec<FittedCurveOverlay>,
    /// Source observations excluded by ROUT when fitting the curves in this result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eliminated_outliers: Vec<crate::SourceCellId>,
    /// Plain-language assumption and data-hygiene diagnostics.
    pub checklist: Vec<ChecklistItem>,
    /// Sticky-note callouts attached to this result sheet.
    pub floating_notes: Vec<FloatingNote>,
    /// Optional navigator color swatch; `None` uses no color badge.
    pub color_tag: Option<Rgba8>,
}

impl ResultSheet {
    /// Construct an initially [`ResultStatus::Dirty`] empty `ResultSheet` wrapping `analysis`, rejecting empty IDs.
    pub fn new(analysis: AnalysisSpec) -> Result<Self, DocError> {
        if analysis.id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        Ok(Self {
            id: analysis.id.clone(),
            analysis,
            status: ResultStatus::Dirty,
            analyzable_data: None,
            tabs: Vec::new(),
            scalars: Vec::new(),
            pairwise_comparisons: Vec::new(),
            fitted_curves: Vec::new(),
            eliminated_outliers: Vec::new(),
            checklist: Vec::new(),
            floating_notes: Vec::new(),
            color_tag: None,
        })
    }

    /// Effective navigator title delegated from `self.analysis`.
    #[must_use]
    pub fn title(&self) -> &str {
        self.analysis.effective_title()
    }

    /// Resolve a hooked scalar by case-insensitive parameter key and optional dataset index.
    #[must_use]
    pub fn lookup_scalar(&self, key: &str, data_set_index: Option<usize>) -> Option<f64> {
        if matches!(self.status, ResultStatus::Failed { .. }) {
            return None;
        }
        let needle = key.trim();
        self.scalars
            .iter()
            .find(|s| s.key.trim().eq_ignore_ascii_case(needle) && (data_set_index.is_none() || s.data_set_index == data_set_index))
            .map(|s| s.value)
            .filter(|v| v.is_finite())
    }
}
