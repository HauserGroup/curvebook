//! `ResultSheet` model: multi-tab statistical/regression outputs, unrounded *p*-values, profile-likelihood
//! `CiBound` endpoints (`Finite` | `Open`), hookable scalar outputs, live pairwise comparison
//! records, fitted curve overlays, and plain-language `ChecklistItem` diagnostics
//! (`docs/architecture.md`, `docs/statistics-implementation.md` §6, `docs/curve-fitting-implementation.md` §2).

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
    Input,
    State,
    Format,
    Io,
    Policy,
    Compute,
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
    BadInput,
    NotFound,
    AlreadyExists,
    Conflict,
    FrozenSheet,
    CycleDetected,
    UnsupportedFormat,
    UnsupportedVersion,
    CorruptData,
    IoError,
    PathEscape,
    ResourceLimit,
    SolverFailed,
    InsufficientData,
    Cancelled,
    GuiOnlyViolation,
    InternalPanic,
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
    #[default]
    Dirty,
    UpToDate,
    Failed {
        code: ErrorCode,
        message: String,
    },
}

/// Confidence-interval endpoint supporting unbounded profile-likelihood plateaus (`CiBound::Open`;
/// `docs/curve-fitting-implementation.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CiBound {
    Finite(f64),
    Open,
}

/// Typed cell inside a [`ResultTab`].
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultCell {
    #[default]
    Empty,
    Number {
        value: f64,
        /// Optional diagnostic prefix such as `"~"` on ambiguous nonlinear parameters.
        prefix: Option<String>,
        units: Option<String>,
    },
    ConfidenceInterval {
        lower: CiBound,
        upper: CiBound,
    },
    /// Stores the exact unrounded `raw` probability alongside display strings.
    PValue {
        raw: f64,
        formatted: String,
        stars: String,
    },
    Text(String),
    Boolean(bool),
}

/// Semantic category of a sub-tab inside a [`ResultSheet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultTabKind {
    #[default]
    MainSummary,
    AnovaTable,
    MultipleComparisons,
    ParametersAndConfidenceIntervals,
    ModelComparison,
    Residuals,
    DescriptiveSummary,
    InterpolatedUnknowns,
    CurveCoordinates,
    NumberAtRisk,
    Diagnostics,
    CovarianceMatrix,
    NormalQq,
    BaselineSurvival,
    LogMinusLog,
}

/// Single tabular view within a [`ResultSheet`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResultTab {
    pub id: String,
    pub title: String,
    pub kind: ResultTabKind,
    pub column_headers: Vec<String>,
    pub row_labels: Vec<String>,
    pub rows: Vec<Vec<ResultCell>>,
    pub footnotes: Vec<String>,
}

/// Named scalar published by a [`ResultSheet`] for constant hooking (`{{result:fit-1:EC50}}`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResultScalarEntry {
    pub key: String,
    pub data_set_index: Option<usize>,
    pub value: f64,
    pub units: Option<String>,
}

/// Pairwise group comparison record consumed by live significance brackets on [`crate::GraphSpec`]
/// (`docs/graphing-and-layouts.md` §4).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairwiseComparisonRecord {
    pub group_a: usize,
    pub group_b: usize,
    pub label_a: String,
    pub label_b: String,
    pub estimate_diff: Option<f64>,
    /// Exact unrounded adjusted *p*-value (FWER) or *q*-value (FDR).
    pub raw_p_or_q: f64,
    pub is_fdr_q: bool,
    pub significant: bool,
}

/// Sampled curve coordinate plus optional confidence and prediction band bounds for `curvebook-chart`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CurveSamplePoint {
    pub x: f64,
    pub y: f64,
    pub ci_lower: Option<f64>,
    pub ci_upper: Option<f64>,
    pub pred_lower: Option<f64>,
    pub pred_upper: Option<f64>,
}

/// Evaluated regression curve overlay associated with a data set on an upstream `DataSheet`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FittedCurveOverlay {
    pub data_set_index: usize,
    pub equation_id: String,
    pub converged: bool,
    pub ambiguous: bool,
    pub points: Vec<CurveSamplePoint>,
}

/// Recommended non-destructive alternative statistical method for a violated assumption (`QOL-7` / `C2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RemediationKind {
    WelchT,
    MannWhitneyU,
    WilcoxonSignedRank,
    OneSampleWilcoxon,
    WelchAnova,
    KruskalWallis,
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

/// Severity status of an assumption check in [`ChecklistItem`] (`docs/statistics-implementation.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ChecklistStatus {
    Pass,
    Warn,
    Violated,
    Info,
}

/// Plain-language statistical assumption diagnostic attached to a [`ResultSheet`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChecklistItem {
    pub assumption: String,
    pub status: ChecklistStatus,
    pub diagnostic_metric: Option<String>,
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<RemediationKind>,
}

/// Complete output node produced by running an [`AnalysisSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResultSheet {
    pub id: SheetId,
    pub analysis: AnalysisSpec,
    pub status: ResultStatus,
    /// Present when this result is a derived data transform (`Transform`, `Normalize`, `Transpose`,
    /// `Simulate`) that can feed downstream statistical analyses, curve fits, and graphs.
    pub analyzable_data: Option<DataSheet>,
    pub tabs: Vec<ResultTab>,
    pub scalars: Vec<ResultScalarEntry>,
    pub pairwise_comparisons: Vec<PairwiseComparisonRecord>,
    pub fitted_curves: Vec<FittedCurveOverlay>,
    pub checklist: Vec<ChecklistItem>,
    pub floating_notes: Vec<FloatingNote>,
    pub color_tag: Option<Rgba8>,
}

impl ResultSheet {
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
