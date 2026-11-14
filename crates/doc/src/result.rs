//! `ResultSheet` model: multi-tab statistical/regression outputs, unrounded *p*-values, profile-likelihood
//! `CiBound` endpoints (`Finite` | `Open`), hookable scalar outputs, live pairwise comparison
//! records, fitted curve overlays, and plain-language `ChecklistItem` diagnostics
//! (`docs/architecture.md`, `docs/statistics-implementation.md` §6, `docs/curve-fitting-implementation.md` §2).

use lumencraft_palette::Rgba8;

use crate::analysis::AnalysisSpec;
use crate::error::DocError;
use crate::ids::{FloatingNote, SheetId};
use crate::links::DataSheet;

/// Recalculation status of a [`ResultSheet`] inside `lumencraft-engine`'s reactive DAG.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultStatus {
    #[default]
    Dirty,
    UpToDate,
    Failed {
        code: String,
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

/// Sampled curve coordinate plus optional confidence and prediction band bounds for `lumencraft-chart`.
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
        let needle = key.trim();
        self.scalars
            .iter()
            .find(|s| s.key.trim().eq_ignore_ascii_case(needle) && (data_set_index.is_none() || s.data_set_index == data_set_index))
            .map(|s| s.value)
            .filter(|v| v.is_finite())
    }
}
