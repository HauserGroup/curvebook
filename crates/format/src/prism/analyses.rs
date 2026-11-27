//! External `.prism` analysis translation, 3-hop `preferredResultSheet` imported-result ingestion,
//! data-driven mapping table with textbook justifications, and cached-vs-recomputed diagnostics (§5.2).

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AreaUnderCurveSpec, AucBaseline, ChecklistItem, ChecklistStatus, DescriptiveSpec, ErrorPropagationMode,
    FactorialAnovaSpec, FitMethod, MathTransformPreset, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec,
    OneWayMethod, OutlierHandling, OutlierSpec, ProjectDocument, RepeatedMeasuresDesign, ReplicateFitMode, ResultCell, ResultScalarEntry,
    ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnAggregation, SurvivalAnalysisSpec, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec,
    WeightingScheme, default_column_label,
};

use crate::archive::{BoundedJsonOutcome, parse_bounded_json_outcome};
use crate::csv::{CsvOptions, parse_delimited_records};
use crate::error::{FormatError, ImportWarning, ImportWarningKind};
use crate::prism::schema::{
    ArchiveIdIndex, ExternalDataSetJson, ExternalDataSheetJson, ExternalDocumentIndex, IdLookup, parse_archive_color, parse_rich_text_value,
};
use crate::pzfx::{allocate_valid_unique_sheet_id, clamp_title};
use crate::zip::ZipIndex;

/// Maximum number of cached result tabs imported per analysis sheet.
const MAX_TABS_PER_ANALYSIS: usize = 32;

/// Maximum number of rows imported per cached result view tab.
const MAX_RESULT_TAB_ROWS: usize = 10_000;

/// Maximum number of columns imported per cached result view tab.
const MAX_RESULT_TAB_COLS: usize = 256;

/// Maximum number of cached scalar entries extracted from `results.json` per analysis.
const MAX_CACHED_SCALARS_PER_ANALYSIS: usize = 256;

/// Verification status for a foreign `.prism` analysis mapping row (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisVerificationStatus {
    /// Recomputed CurveBook outputs matched cached corpus results within tolerance AND CurveBook's
    /// own statistical kernel passes its textbook D1 oracle.
    CorpusVerified,
    /// Parameter mapping is documented and implemented, but live translation remains off by default
    /// until corpus-wide cached-vs-recomputed verification completes.
    Unverified,
}

/// Data-driven specification mapping one external `.prism` `analysisClass` identifier to CurveBook
/// semantics with a textbook justification and numeric comparison tolerance (§5.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisMappingRow {
    /// Exact external `analysisClass` wire identifier (for example `"TRNSFORM"` or `"ONE_WAY_ANOVA"`).
    pub foreign_class: &'static str,
    /// Target CurveBook `AnalysisKind` family label.
    pub curvebook_kind_label: &'static str,
    /// Whether this mapping is cleared for default live translation (`CorpusVerified`) or imported
    /// as a read-only snapshot (`Unverified`).
    pub verification: AnalysisVerificationStatus,
    /// Textbook statistical or mathematical definition justifying semantic equivalence.
    pub justification: &'static str,
    /// Documented relative tolerance for cached-vs-recomputed scalar agreement checks.
    pub relative_tolerance: f64,
    /// Documented absolute tolerance for cached-vs-recomputed scalar agreement checks near zero.
    pub absolute_tolerance: f64,
}

/// Complete data-driven mapping table covering the 11 corpus-observed `.prism` analysis classes.
///
/// Per §5.2, every row starts as [`AnalysisVerificationStatus::Unverified`] so imported `.prism`
/// analyses default to read-only v9 `ResultSheet` snapshots until corpus recomputation verifies
/// agreement against CurveBook's D1 oracles.
pub const ANALYSIS_MAPPINGS: &[AnalysisMappingRow] = &[
    AnalysisMappingRow {
        foreign_class: "T_TESTS_AND_NONPARAMETRIC",
        curvebook_kind_label: "TwoGroupComparison",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Student (1908) pooled t-test, Welch (1947) unequal-variance t-test, Mann & Whitney (1947) rank-sum test, Wilcoxon (1945) / Pratt (1959) signed-rank test, and Smirnov (1948) two-sample KS test.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "ONE_WAY_ANOVA",
        curvebook_kind_label: "OneWayAnova",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Fisher (1925) one-way ANOVA, Brown & Forsythe (1974) / Welch (1951) heteroscedastic ANOVA, Greenhouse & Geisser (1959) RM ANOVA, Kruskal & Wallis (1952), and Friedman (1937).",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "TWO_WAY_ANOVA",
        curvebook_kind_label: "FactorialAnova",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Maxwell, Delaney, & Kelley (2017) Type III factorial ANOVA and Kenward & Roger (1997) REML linear mixed-effects models for repeated-measures factorial designs.",
        relative_tolerance: 1e-5,
        absolute_tolerance: 1e-8,
    },
    AnalysisMappingRow {
        foreign_class: "NONLINEAR_REGRESSION",
        curvebook_kind_label: "NonlinearRegression",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "More (1978) Levenberg-Marquardt nonlinear least squares, Motulsky & Christopoulos (2004) profile-likelihood and Wald parameter intervals, and Motulsky & Brown (2006) ROUT outlier detection.",
        relative_tolerance: 1e-4,
        absolute_tolerance: 1e-7,
    },
    AnalysisMappingRow {
        foreign_class: "NORMALITY_TESTS",
        curvebook_kind_label: "DescriptiveStats",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Shapiro & Wilk (1965) / Royston (1995), D'Agostino & Pearson (1973) omnibus K^2, Anderson & Darling (1954) A^2*, and Lilliefors (1967) normality and lognormality tests.",
        relative_tolerance: 1e-5,
        absolute_tolerance: 1e-8,
    },
    AnalysisMappingRow {
        foreign_class: "TRNSFORM",
        curvebook_kind_label: "Transform::Math",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Element-wise scalar mathematical transforms (log10, ln, exp10, exp, reciprocal, sqrt, square, logit, probit, arcsin-sqrt, affine scaling) with endpoint or first-order delta-method error propagation.",
        relative_tolerance: 1e-9,
        absolute_tolerance: 1e-12,
    },
    AnalysisMappingRow {
        foreign_class: "NORMALIZE",
        curvebook_kind_label: "Transform::Normalize",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Linear rescaling (y - a0) / (a100 - a0) onto [0%, 100%] or [0, 1] using column extrema, boundary rows, or fixed numeric anchors.",
        relative_tolerance: 1e-9,
        absolute_tolerance: 1e-12,
    },
    AnalysisMappingRow {
        foreign_class: "DESCRIPTIVE_STATISTICS",
        curvebook_kind_label: "DescriptiveStats",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Univariate sample moments (mean, SD, SEM, CV, skewness, kurtosis, geometric mean), Hyndman & Fan (1996) Type-7 quantiles, and Student-t confidence intervals.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "AREA_UNDER_CURVE",
        curvebook_kind_label: "AreaUnderCurve",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Composite trapezoidal quadrature above a fixed or first-point baseline with peak filtering and Bailer (1988) independent-cohort AUC standard error.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "IDENTIFY_OUTLIERS",
        curvebook_kind_label: "OutlierDetection",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Grubbs (1969) extreme studentized deviate (single and iterative ESD) and Motulsky & Brown (2006) univariate ROUT FDR outlier detection.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "SURVIVAL",
        curvebook_kind_label: "Survival::KaplanMeier",
        verification: AnalysisVerificationStatus::Unverified,
        justification: "Kaplan & Meier (1958) product-limit survival estimator, Mantel (1966) log-rank test, Gehan (1965) / Breslow (1970) generalized Wilcoxon test, Tarone (1975) trend test, and Kalbfleisch & Prentice (2002) log-log Greenwood CIs.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
];

/// Look up the [`AnalysisMappingRow`] for an external `analysisClass` identifier.
#[must_use]
pub fn find_analysis_mapping(foreign_class: &str) -> Option<&'static AnalysisMappingRow> {
    ANALYSIS_MAPPINGS.iter().find(|row| row.foreign_class.eq_ignore_ascii_case(foreign_class.trim()))
}

/// Outcome of translating an external `.prism` analysis `parameters.json` into a CurveBook
/// [`AnalysisKind`].
#[derive(Debug, Clone, PartialEq)]
pub struct TranslatedAnalysis {
    pub kind: AnalysisKind,
    pub verification: AnalysisVerificationStatus,
    /// Human-readable descriptions of options or equation settings that blocked lossless live translation.
    pub blocking_options: Vec<String>,
}

/// Single numeric disagreement observed between a recomputed CurveBook scalar and a cached
/// `.prism` scalar (§5.2).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedScalarDiff {
    pub key: String,
    pub data_set_index: Option<usize>,
    pub recomputed: f64,
    pub cached: f64,
    pub abs_diff: f64,
    pub rel_diff: f64,
}

/// Summary of comparing a recomputed [`ResultSheet`] against cached external scalars (§5.2).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CachedResultDiagnostic {
    pub compared_count: usize,
    pub disagreements: Vec<CachedScalarDiff>,
    /// Number of compared values per CurveBook scalar key (filled by
    /// [`compare_recomputed_against_cached_results`]).
    pub compared_by_key: BTreeMap<String, usize>,
}

/// How a crosswalked cached scalar is compared with CurveBook's recomputed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrosswalkComparison {
    /// Signed values must agree.
    Signed,
    /// Only magnitudes are compared. Used for quantities whose sign depends on which group is
    /// subtracted from which (a mean difference or a *t* statistic), where CurveBook keeps its own
    /// documented subtraction order.
    Magnitude,
}

/// Declarative link from one field of a cached `.prism` `results.json` to one CurveBook result
/// scalar, used only to verify a mapping row against the corpus (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedScalarLink {
    /// External `analysisClass` identifier the link applies to.
    pub foreign_class: &'static str,
    /// Dotted path below the `results.json` `content` object. A `*` segment matches every key of
    /// an object. The data-set index is read from `id.col.idx` on the nearest enclosing object;
    /// fields with no such ancestor compare against whole-analysis (`data_set_index: None`) scalars.
    pub cached_path: &'static str,
    /// [`ResultScalarEntry::key`] emitted by CurveBook for the same quantity.
    pub curvebook_key: &'static str,
    pub comparison: CrosswalkComparison,
}

/// Maximum number of cached values one link may yield from a single `results.json`.
const MAX_VALUES_PER_LINK: usize = 1024;

const fn link(foreign_class: &'static str, cached_path: &'static str, curvebook_key: &'static str) -> CachedScalarLink {
    CachedScalarLink { foreign_class, cached_path, curvebook_key, comparison: CrosswalkComparison::Signed }
}

const fn link_magnitude(foreign_class: &'static str, cached_path: &'static str, curvebook_key: &'static str) -> CachedScalarLink {
    CachedScalarLink { foreign_class, cached_path, curvebook_key, comparison: CrosswalkComparison::Magnitude }
}

const TT: &str = "T_TESTS_AND_NONPARAMETRIC";
const OW: &str = "ONE_WAY_ANOVA";
const TW: &str = "TWO_WAY_ANOVA";
const DS: &str = "DESCRIPTIVE_STATISTICS";
const SV: &str = "SURVIVAL";
const NL: &str = "NONLINEAR_REGRESSION";

/// Cached-result crosswalk. Every link names one quantity with the same textbook definition on
/// both sides; quantities whose definitions differ (for example hazard-ratio estimators that use
/// different names for different methods) are deliberately left out rather than adjusted.
pub const CACHED_SCALAR_LINKS: &[CachedScalarLink] = &[
    // Two-group tests (Student 1908, Welch 1947, Mann & Whitney 1947, Wilcoxon 1945). The sign of
    // `t` follows the group subtraction order, so only its magnitude is compared.
    link(TT, "unpairedTTestModel.unpairedTTest.pValue.value", "PValue"),
    link_magnitude(TT, "unpairedTTestModel.unpairedTTest.t", "t"),
    link(TT, "welchTTestModel.welchTTest.pValue.value", "PValue"),
    link_magnitude(TT, "welchTTestModel.welchTTest.t", "t"),
    link(TT, "pairedTTestModel.pairedTTest.pValue.value", "PValue"),
    link_magnitude(TT, "pairedTTestModel.pairedTTest.t", "t"),
    link(TT, "mannWhitneyTestModel.mannWhitneyTest.pValue.value", "PValue"),
    link(TT, "wilcoxonTestModel.wilcoxonTest.pValue.value", "PValue"),
    // One-way designs (Fisher 1925; Kruskal & Wallis 1952; Friedman 1937).
    link(OW, "ordinaryAnovaModel.anovaTable.colFactor.fTest.f", "F"),
    link(OW, "ordinaryAnovaModel.anovaTable.colFactor.fTest.pValue.value", "P value"),
    link(OW, "ordinaryAnovaModel.anovaTable.colFactor.r2", "R squared"),
    link(OW, "ordinaryAnovaModel.anovaTable.residual.ms", "MS residual"),
    link(OW, "repeatedMeasuresGlmAnovaModel.anovaTable.colFactor.fTest.f", "F"),
    link(OW, "repeatedMeasuresGlmAnovaModel.anovaTable.colFactor.fTest.pValue.value", "P value"),
    link(OW, "repeatedMeasuresGlmAnovaModel.anovaTable.residual.ms", "MS residual"),
    link(OW, "kruskalWallisTestModel.kruskalWallisTest.pValue.value", "P value"),
    link(OW, "friedmanTestModel.friedmanTest.pValue.value", "P value"),
    // Two-way factorial designs (Maxwell, Delaney & Kelley 2017), ordinary and repeated measures.
    link(TW, "ordinaryAnovaModel.anovaTable.rowFactor.fTest.f", "Row Factor: F"),
    link(TW, "ordinaryAnovaModel.anovaTable.rowFactor.fTest.pValue.value", "Row Factor: P"),
    link(TW, "ordinaryAnovaModel.anovaTable.colFactor.fTest.f", "Column Factor: F"),
    link(TW, "ordinaryAnovaModel.anovaTable.colFactor.fTest.pValue.value", "Column Factor: P"),
    link(TW, "ordinaryAnovaModel.anovaTable.rowFactorVsColFactor.fTest.f", "Row Factor × Column Factor: F"),
    link(TW, "ordinaryAnovaModel.anovaTable.rowFactorVsColFactor.fTest.pValue.value", "Row Factor × Column Factor: P"),
    link(TW, "ordinaryAnovaModel.anovaTable.residual.ms", "MS residual"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.rowFactor.fTest.f", "Row Factor: F"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.rowFactor.fTest.pValue.value", "Row Factor: P"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.colFactor.fTest.f", "Column Factor: F"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.colFactor.fTest.pValue.value", "Column Factor: P"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.rowFactorVsColFactor.fTest.f", "Row Factor × Column Factor: F"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.rowFactorVsColFactor.fTest.pValue.value", "Row Factor × Column Factor: P"),
    link(TW, "repeatedMeasuresGlmAnovaModel.anovaTable.residual.ms", "MS residual"),
    // Per-data-set descriptive statistics (sample moments; Type-7 median).
    link(DS, "samples.*.value.n", "N"),
    link(DS, "samples.*.value.mean", "Mean"),
    link(DS, "samples.*.value.sd", "SD"),
    link(DS, "samples.*.value.meanSE", "SEM"),
    link(DS, "samples.*.value.median", "Median"),
    // Survival curve comparison (Mantel 1966 log-rank; Gehan 1965 / Breslow 1970).
    link(SV, "curveComparison.logrankMantelCoxTest.chiSquared", "Log-rank chi-square"),
    link(SV, "curveComparison.logrankMantelCoxTest.pValue.value", "Log-rank P value"),
    link(SV, "curveComparison.gehanBreslowWilcoxonTest.chiSquared", "Gehan-Breslow-Wilcoxon chi-square"),
    link(SV, "curveComparison.gehanBreslowWilcoxonTest.pValue.value", "Gehan-Breslow-Wilcoxon P"),
    // Per-data-set dose-response parameter estimates (More 1978 Levenberg-Marquardt).
    link(NL, "models.*.value.parameters.Top.*.value.estimate", "Top"),
    link(NL, "models.*.value.parameters.Bottom.*.value.estimate", "Bottom"),
    link(NL, "models.*.value.parameters.Span.*.value.estimate", "Span"),
    link(NL, "models.*.value.parameters.LogIC50.*.value.estimate", "LogIC50"),
    link(NL, "models.*.value.parameters.LogEC50.*.value.estimate", "LogEC50"),
    link(NL, "models.*.value.parameters.HillSlope.*.value.estimate", "HillSlope"),
    // Per-data-set total area (composite trapezoidal rule) and outlier counts (Motulsky & Brown 2006).
    link("AREA_UNDER_CURVE", "samples.*.value.summary.totalArea", "TotalArea"),
    link("IDENTIFY_OUTLIERS", "samples.*.value.outlierCount", "OutlierCount"),
];

/// Read every cached value that `link` selects from a `results.json` document, paired with the
/// data-set index of its nearest `id.col.idx` ancestor.
#[must_use]
pub fn cached_values_for_link(link: &CachedScalarLink, results_val: &serde_json::Value) -> Vec<(Option<usize>, f64)> {
    let root = results_val.get("content").unwrap_or(results_val);
    let segments: Vec<&str> = link.cached_path.split('.').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    collect_link_values(root, &segments, None, &mut out);
    out
}

fn collect_link_values(val: &serde_json::Value, segments: &[&str], col: Option<usize>, out: &mut Vec<(Option<usize>, f64)>) {
    if out.len() >= MAX_VALUES_PER_LINK {
        return;
    }
    let col = val
        .get("id")
        .and_then(|id| id.get("col"))
        .and_then(|c| c.get("idx"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .or(col);
    match segments.split_first() {
        None => {
            if let Some(v) = val.as_f64().filter(|v| v.is_finite()) {
                out.push((col, v));
            }
        }
        Some((&"*", rest)) => {
            if let Some(map) = val.as_object() {
                for child in map.values() {
                    collect_link_values(child, rest, col, out);
                }
            }
        }
        Some((segment, rest)) => {
            if let Some(child) = val.get(*segment) {
                collect_link_values(child, rest, col, out);
            }
        }
    }
}

/// Compare a recomputed [`ResultSheet`] against a cached `results.json` through
/// [`CACHED_SCALAR_LINKS`], within `max(abs_tol, rel_tol * max(|recomputed|, |cached|))`.
///
/// A cached value is compared only when CurveBook emitted a scalar with the linked key and the same
/// data-set scope; unmatched values are skipped rather than counted as agreement. CurveBook never
/// adjusts its own outputs to match cached values.
#[must_use]
pub fn compare_recomputed_against_cached_results(
    recomputed: &ResultSheet,
    foreign_class: &str,
    results_val: &serde_json::Value,
    abs_tol: f64,
    rel_tol: f64,
) -> CachedResultDiagnostic {
    let mut diag = CachedResultDiagnostic::default();
    let abs_t = if abs_tol.is_finite() && abs_tol >= 0.0 { abs_tol } else { 0.0 };
    let rel_t = if rel_tol.is_finite() && rel_tol >= 0.0 { rel_tol } else { 0.0 };
    let class = foreign_class.trim();
    for link in CACHED_SCALAR_LINKS.iter().filter(|l| l.foreign_class.eq_ignore_ascii_case(class)) {
        for (col, cached) in cached_values_for_link(link, results_val) {
            let Some(rec) =
                recomputed.scalars.iter().find(|e| e.key.eq_ignore_ascii_case(link.curvebook_key) && e.data_set_index == col && e.value.is_finite())
            else {
                continue;
            };
            let (a, b) = match link.comparison {
                CrosswalkComparison::Signed => (rec.value, cached),
                CrosswalkComparison::Magnitude => (rec.value.abs(), cached.abs()),
            };
            diag.compared_count += 1;
            *diag.compared_by_key.entry(link.curvebook_key.to_owned()).or_insert(0) += 1;
            let abs_diff = (a - b).abs();
            let scale = a.abs().max(b.abs());
            let rel_diff = if scale > 0.0 { abs_diff / scale } else { 0.0 };
            if abs_diff > abs_t.max(rel_t * scale) {
                diag.disagreements.push(CachedScalarDiff {
                    key: link.curvebook_key.to_owned(),
                    data_set_index: col,
                    recomputed: rec.value,
                    cached,
                    abs_diff,
                    rel_diff,
                });
            }
        }
    }
    diag
}

/// Build a caution [`ChecklistItem`] disclosing disagreements between CurveBook's recomputed
/// output and cached external values (§5.2). Returns `None` when no disagreements occurred.
#[must_use]
pub fn build_disagreement_checklist_item(foreign_class: &str, diagnostic: &CachedResultDiagnostic) -> Option<ChecklistItem> {
    let first = diagnostic.disagreements.first()?;
    let metric = format!(
        "{}: CurveBook = {:.6}, cached = {:.6} (abs diff = {:.3e}, {} of {} differed)",
        first.key,
        first.recomputed,
        first.cached,
        first.abs_diff,
        diagnostic.disagreements.len(),
        diagnostic.compared_count
    );
    let explanation = format!(
        "CurveBook kept its own live recomputation for `{foreign_class}` rather than adjusting to cached values. Inspect the difference against the cited statistical definition."
    );
    Some(ChecklistItem {
        assumption: String::from("Cached external result comparison"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(metric),
        explanation,
        remediation: None,
    })
}

/// Translate an external `.prism` `parameters.json` document into a CurveBook [`AnalysisKind`]
/// and any blocking option notes. Returns `None` when `foreign_class` is unmapped.
#[must_use]
pub fn translate_analysis_parameters(foreign_class: &str, params_json: &serde_json::Value) -> Option<TranslatedAnalysis> {
    let row = find_analysis_mapping(foreign_class)?;
    let content = params_json.get("content").and_then(serde_json::Value::as_object).or_else(|| params_json.as_object());
    let mut blocking = Vec::new();
    let kind = match row.foreign_class {
        "T_TESTS_AND_NONPARAMETRIC" => translate_two_group(content, &mut blocking),
        "ONE_WAY_ANOVA" => translate_one_way_anova(content, &mut blocking),
        "TWO_WAY_ANOVA" => translate_two_way_anova(content, &mut blocking),
        "NONLINEAR_REGRESSION" => translate_nonlinear(content, &mut blocking),
        "NORMALITY_TESTS" => translate_normality_tests(content, &mut blocking),
        "TRNSFORM" => translate_math_transform(content, &mut blocking),
        "NORMALIZE" => translate_normalize(content, &mut blocking),
        "DESCRIPTIVE_STATISTICS" => translate_descriptive(content, &mut blocking),
        "AREA_UNDER_CURVE" => translate_auc(content, &mut blocking),
        "IDENTIFY_OUTLIERS" => translate_outliers(content, &mut blocking),
        "SURVIVAL" => translate_survival(content, &mut blocking),
        _ => return None,
    };
    Some(TranslatedAnalysis { kind, verification: row.verification, blocking_options: blocking })
}

type JsonMap = serde_json::Map<String, serde_json::Value>;

fn get_str<'a>(map: Option<&'a JsonMap>, key: &str) -> Option<&'a str> {
    map?.get(key)?.as_str()
}

fn get_obj<'a>(map: Option<&'a JsonMap>, key: &str) -> Option<&'a JsonMap> {
    map?.get(key)?.as_object()
}

fn get_bool(map: Option<&JsonMap>, key: &str) -> Option<bool> {
    map?.get(key)?.as_bool()
}

fn get_f64(map: Option<&JsonMap>, key: &str) -> Option<f64> {
    map?.get(key)?.as_f64().filter(|v| v.is_finite())
}

fn translate_two_group(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let design = get_str(content, "experimentalDesign").unwrap_or("UNPAIRED_T_TEST");
    let method = match design {
        "UNPAIRED_T_TEST" => TwoGroupMethod::UnpairedT,
        "WELCH_T_TEST" | "UNPAIRED_WELCH_T_TEST" => TwoGroupMethod::WelchT,
        "PAIRED_T_TEST" => TwoGroupMethod::PairedT,
        "RATIO_PAIRED_T_TEST" => TwoGroupMethod::RatioPairedT,
        "LOGNORMAL_UNPAIRED_T_TEST" => TwoGroupMethod::LognormalUnpairedT,
        "MANN_WHITNEY" | "MANN_WHITNEY_TEST" => TwoGroupMethod::MannWhitneyU,
        "WILCOXON" | "WILCOXON_TEST" | "WILCOXON_MATCHED_PAIRS" => TwoGroupMethod::WilcoxonSignedRank,
        "KOLMOGOROV_SMIRNOV" | "KOLMOGOROV_SMIRNOV_TEST" => TwoGroupMethod::KolmogorovSmirnov,
        other => {
            blocking.push(format!("unmapped experimentalDesign `{other}`"));
            TwoGroupMethod::UnpairedT
        }
    };
    let options = get_obj(content, "options");
    let two_tailed = match get_str(options, "pValueTailType") {
        None | Some("P_VALUE_TWO_TAILED") => true,
        Some("P_VALUE_ONE_TAILED") => false,
        Some(other) => {
            blocking.push(format!("unmapped pValueTailType `{other}`"));
            true
        }
    };
    AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, two_tailed, compute_effect_sizes: true, ..TwoGroupSpec::default() })
}

/// Translate `postTests` into a [`MultiplicitySpec`].
///
/// `multipleComparisons.type` selects the branch (`STATISTICAL` → `statistical.test`, `FDR` →
/// `fdr.test`, `NO_CORRECTION` → unadjusted), and `pairwiseTests.pairwiseDesign.type` selects the
/// comparison family. Only all-pairs families and Dunnett's many-to-one test translate; a Dunnett
/// control keeps its sheet data-set index and must lie in the analysed selection
/// ([`check_control_in_selection`]). Ambiguous test names and selected-pair families block.
fn parse_multiplicity(post_tests: Option<&JsonMap>, allow_dunnett: bool, blocking: &mut Vec<String>) -> MultiplicitySpec {
    let mut spec = MultiplicitySpec::default();
    let Some(pt) = post_tests else {
        return spec;
    };
    if matches!(get_str(Some(pt), "type"), None | Some("NO_TESTS")) {
        return spec;
    }
    let pairwise = get_obj(Some(pt), "pairwiseTests");
    let mc = get_obj(pairwise, "multipleComparisons");
    let test = match get_str(mc, "type") {
        None | Some("NO_CORRECTION") => None,
        Some("STATISTICAL") => get_str(get_obj(mc, "statistical"), "test").or_else(|| get_str(mc, "statistical")),
        Some("FDR") => get_str(get_obj(mc, "fdr"), "test").or_else(|| get_str(mc, "fdr")),
        Some(other) => {
            blocking.push(format!("unmapped multipleComparisons type `{other}`"));
            None
        }
    };
    let design_obj = get_obj(pairwise, "pairwiseDesign");
    let family = get_str(design_obj, "type").unwrap_or("ALL_VS_ALL");
    let Some(test) = test.map(str::to_ascii_uppercase).filter(|t| t != "NO_TEST" && t != "NONE" && t != "NO_CORRECTION") else {
        // Unadjusted comparisons over a control or selected-pairs family would otherwise be
        // reported for every pair.
        if family != "ALL_VS_ALL" {
            blocking.push(format!("unadjusted pairwise comparison family `{family}`"));
        }
        return spec;
    };
    spec.method = match (family, test.as_str()) {
        ("CONTROL_VS_ALL", "DUNNET" | "DUNNETT") if allow_dunnett => {
            let control = get_obj(get_obj(design_obj, "controlID"), "col")
                .and_then(|c| c.get("idx"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|i| usize::try_from(i).ok());
            match control {
                Some(control_column) => MultiplicityMethod::Dunnett { control_column },
                None => {
                    blocking.push(String::from("Dunnett control group is missing"));
                    MultiplicityMethod::None
                }
            }
        }
        ("ALL_VS_ALL", t) => match t {
            "TUKEY" | "TUKEY_HSD" => MultiplicityMethod::TukeyHsd,
            "HOLM_SIDAK" => MultiplicityMethod::HolmSidak,
            "BONFERRONI_DUNN" | "BONFERRONI" => MultiplicityMethod::Bonferroni,
            "SIDAK" => MultiplicityMethod::Sidak,
            "DUNNET_T3" | "DUNNETT_T3" => MultiplicityMethod::DunnettT3,
            "GAMES_HOWELL" => MultiplicityMethod::GamesHowell,
            "TAMHANE_T2" => MultiplicityMethod::TamhaneT2,
            "DUNN" => MultiplicityMethod::DunnRankSum,
            "BENJAMINI_HOCHBERG" => MultiplicityMethod::BenjaminiHochbergFdr,
            "BENJAMINI_YEKUTIELI" => MultiplicityMethod::BenjaminiYekutieliFdr,
            "TWO_STAGE_BKY" | "BENJAMINI_KRIEGER_YEKUTIELI" => MultiplicityMethod::TwoStageBkyFdr,
            "FISHER_LSD" | "UNCORRECTED_FISHER_LSD" => MultiplicityMethod::FisherLsd,
            other => {
                blocking.push(format!("unmapped multipleComparisons test `{other}`"));
                MultiplicityMethod::None
            }
        },
        (family, t) => {
            blocking.push(format!("unmapped pairwise comparison family `{family}` with `{t}`"));
            MultiplicityMethod::None
        }
    };
    spec
}

/// Check that a Dunnett control (a sheet data-set index, which is how CurveBook identifies the
/// control group) is among the analysed data sets. An empty selection analyses every data set.
pub fn check_control_in_selection(kind: &AnalysisKind, selected: &[usize]) -> Result<(), String> {
    let AnalysisKind::OneWayAnova(spec) = kind else {
        return Ok(());
    };
    let MultiplicityMethod::Dunnett { control_column } = spec.multiplicity.method else {
        return Ok(());
    };
    if selected.is_empty() || selected.contains(&control_column) {
        Ok(())
    } else {
        Err(String::from("Dunnett control group is not among the analysed data sets"))
    }
}

fn translate_one_way_anova(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let rm = get_obj(content, "repeatedMeasures");
    let assume_sphericity = get_bool(rm, "assumeSphericity").unwrap_or(false);
    let design = get_str(content, "experimentalDesign").unwrap_or("ORDINARY_ANOVA");
    let method = match design {
        "ORDINARY_ANOVA" => OneWayMethod::OrdinaryAnova,
        "BROWN_FORSYTHE_ANOVA" | "BROWN_FORSYTHE_AND_WELCH" | "BROWN_FORSYTHE_WELCH" | "WELCH_ANOVA" => OneWayMethod::BrownForsytheAndWelch,
        "REPEATED_MEASURES_ANOVA" => OneWayMethod::RepeatedMeasures { geisser_greenhouse: !assume_sphericity },
        "KRUSKAL_WALLIS" | "KRUSKAL_WALLIS_TEST" => OneWayMethod::KruskalWallis,
        "FRIEDMAN_TEST" | "FRIEDMAN" => OneWayMethod::Friedman,
        other => {
            blocking.push(format!("unmapped experimentalDesign `{other}`"));
            OneWayMethod::OrdinaryAnova
        }
    };
    let multiplicity = parse_multiplicity(get_obj(content, "postTests"), true, blocking);
    AnalysisKind::OneWayAnova(OneWayAnovaSpec { method, compute_effect_sizes: true, multiplicity, ..OneWayAnovaSpec::default() })
}

fn translate_two_way_anova(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let design = get_str(content, "experimentalDesign").unwrap_or("ORDINARY_ANOVA");
    let repeated_measures = match design {
        "ORDINARY_ANOVA" => RepeatedMeasuresDesign::None,
        "REPEATED_MEASURES_BY_ROWS" => RepeatedMeasuresDesign::RowFactorMatched,
        "REPEATED_MEASURES_BY_COLS" => RepeatedMeasuresDesign::ColumnFactorMatched,
        "REPEATED_MEASURES_BY_BOTH" | "REPEATED_MEASURES_BY_BOTH_FACTORS" => RepeatedMeasuresDesign::BothFactorsMatched,
        other => {
            blocking.push(format!("unmapped experimentalDesign `{other}`"));
            RepeatedMeasuresDesign::None
        }
    };
    let rm = get_obj(content, "repeatedMeasures");
    let mixed_effects_reml = get_str(rm, "method").is_some_and(|m| m.eq_ignore_ascii_case("MIXED_EFFECTS") || m.eq_ignore_ascii_case("REML"));
    // Two-way Dunnett controls name a level of one factor; that mapping is not yet verified.
    let multiplicity = parse_multiplicity(get_obj(content, "postTests"), false, blocking);
    AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        repeated_measures,
        mixed_effects_reml,
        compute_effect_sizes: true,
        multiplicity,
        ..FactorialAnovaSpec::default()
    })
}

/// Map documented built-in external equation names to canonical CurveBook `ParametricModel` IDs
/// only where parameterizations (e.g., `LogEC50` vs `EC50` and Hill slope sign) match identically.
fn map_nonlinear_equation_alias(name: &str) -> Option<&'static str> {
    let norm = name.trim().to_ascii_lowercase();
    match norm.as_str() {
        "log(agonist) vs. response -- variable slope (four parameters)" | "log_agonist_vs_response_4pl" => Some("log_agonist_vs_response_4pl"),
        "log(inhibitor) vs. response -- variable slope (four parameters)" | "log_inhibitor_vs_response_4pl" => Some("log_inhibitor_vs_response_4pl"),
        "log(agonist) vs. response (three parameters)" | "log_agonist_vs_response_3pl" => Some("log_agonist_vs_response_3pl"),
        "log(inhibitor) vs. response (three parameters)" | "log_inhibitor_vs_response_3pl" => Some("log_inhibitor_vs_response_3pl"),
        "one phase decay" | "one_phase_decay" => Some("one_phase_decay"),
        "one phase association" | "one_phase_association" => Some("one_phase_association"),
        "michaelis-menten" | "michaelis_menten" => Some("michaelis_menten"),
        "straight line" | "linear" | "straight_line" => Some("straight_line"),
        _ => None,
    }
}

fn translate_nonlinear(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let eq_obj = get_obj(get_obj(content, "model"), "equation");
    let raw_eq_name = get_str(eq_obj, "name").unwrap_or("");
    let eq_id = if let Some(mapped) = map_nonlinear_equation_alias(raw_eq_name) {
        mapped
    } else {
        blocking.push(format!("unmapped nonlinear equation `{raw_eq_name}`"));
        "log_agonist_vs_response_4pl"
    };

    let mut spec = NonlinearFitSpec::new(eq_id);
    let data = get_obj(content, "data");
    if let Some(rep_mode) = get_str(data, "handleReplicatesAs") {
        spec.replicate_fit_mode = match rep_mode {
            "AVERAGE" => ReplicateFitMode::RowMeansOnly,
            "INDIVIDUAL" => ReplicateFitMode::IndividualReplicates,
            other => {
                blocking.push(format!("unmapped handleReplicatesAs `{other}`"));
                ReplicateFitMode::IndividualReplicates
            }
        };
    }

    let method = get_obj(content, "method");
    if let Some(fit_m) = get_str(method, "fittingMethod") {
        spec.method = match fit_m {
            "LS" | "LEAST_SQUARES" => FitMethod::LeastSquares,
            "POISSON" => FitMethod::PoissonDeviance,
            "ROBUST" => FitMethod::RobustCauchy,
            other => {
                blocking.push(format!("unmapped fittingMethod `{other}`"));
                FitMethod::LeastSquares
            }
        };
    }
    if let Some(w_type) = get_str(get_obj(method, "weighting"), "type") {
        spec.weighting = match w_type {
            "NO_WEIGHTING" => WeightingScheme::None,
            "RELATIVE" | "ONE_OVER_Y2" => WeightingScheme::RelativeSquare,
            "POISSON" | "ONE_OVER_Y" => WeightingScheme::Relative,
            "ONE_OVER_X" => WeightingScheme::InverseX,
            "ONE_OVER_X2" => WeightingScheme::InverseX2,
            "OBSERVED_SD" => WeightingScheme::ObservedSd,
            other => {
                blocking.push(format!("unmapped weighting `{other}`"));
                WeightingScheme::None
            }
        };
    }
    if let Some(out_m) = get_str(get_obj(method, "outliers"), "method") {
        match out_m {
            "DO_NOT_HANDLE" | "NONE" => spec.outlier_handling = OutlierHandling::None,
            "ROUT_ELIMINATE" => {
                spec.outlier_handling = OutlierHandling::Eliminate;
                spec.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
            }
            "ROUT_COUNT" => spec.outlier_handling = OutlierHandling::IdentifyAndCountOnly,
            other => blocking.push(format!("unmapped outlier method `{other}`")),
        }
    }
    if let Some(ci_m) = get_str(get_obj(content, "confidence"), "ciMethod") {
        match ci_m {
            "PROFILE_LIKELIHOOD" | "ASYMMETRICAL" => spec.profile_likelihood_ci = true,
            "WALD" | "SYMMETRICAL" => spec.profile_likelihood_ci = false,
            other => blocking.push(format!("unmapped ciMethod `{other}`")),
        }
    }
    AnalysisKind::NonlinearRegression(spec)
}

/// Map `data.handleReplicatesAs` onto CurveBook replicate aggregation. Corpus spellings are
/// `AVERAGE` (row means) and `COLUMN` (all replicate values pooled per data set). `SUB_COLUMN`
/// (one summary per subcolumn) blocks live translation until its per-subcolumn result scope is
/// verified, and any other value blocks rather than silently defaulting to row means.
fn parse_subcolumn_agg(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> SubcolumnAggregation {
    match get_str(get_obj(content, "data"), "handleReplicatesAs") {
        None | Some("AVERAGE") => SubcolumnAggregation::RowMeans,
        Some("COLUMN") => SubcolumnAggregation::PooledGrandSample,
        Some("SUB_COLUMN") => {
            blocking.push(String::from("per-subcolumn replicate handling `SUB_COLUMN` is not yet corpus-verified"));
            SubcolumnAggregation::PerSubcolumn
        }
        Some(other) => {
            blocking.push(format!("unmapped handleReplicatesAs `{other}`"));
            SubcolumnAggregation::RowMeans
        }
    }
}

fn translate_normality_tests(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let calc = get_obj(content, "calc");
    let tests = get_obj(calc, "tests");
    // Exact corpus wire spelling is `shaproWilks` (missing `i`).
    let any_test = get_bool(tests, "shaproWilks").unwrap_or(false)
        || get_bool(tests, "shapiroWilk").unwrap_or(false)
        || get_bool(tests, "dAgostinoPearson").unwrap_or(false)
        || get_bool(tests, "andersonDarling").unwrap_or(false)
        || get_bool(tests, "kolmogorovSmirnov").unwrap_or(false);
    if tests.is_some() && !any_test {
        blocking.push(String::from("no recognized normality test enabled in calc.tests"));
    }
    AnalysisKind::DescriptiveStats(DescriptiveSpec {
        aggregation: parse_subcolumn_agg(content, blocking),
        custom_upper_percentile: None,
        run_normality_battery: true,
        hypothetical_value: None,
        normality_only: false,
        emit_qq_tabs: false,
    })
}

fn parse_transform_fn(axis_obj: Option<&JsonMap>, blocking: &mut Vec<String>) -> Option<MathTransformPreset> {
    let obj = axis_obj?;
    if !get_bool(Some(obj), "enabled").unwrap_or(false) {
        return None;
    }
    let fn_name = get_str(Some(obj), "function").unwrap_or("");
    let k = get_f64(Some(obj), "k").unwrap_or(1.0);
    match fn_name.to_ascii_uppercase().as_str() {
        "LOG10" | "LOG" => Some(MathTransformPreset::Log10),
        "LN" => Some(MathTransformPreset::Ln),
        "LOG2" => Some(MathTransformPreset::Log2),
        "EXP10" | "TEN_TO_X" | "ANTILOG10" => Some(MathTransformPreset::Exp10),
        "EXP" | "E_TO_X" => Some(MathTransformPreset::Exp),
        "RECIPROCAL" | "ONE_OVER_X" => Some(MathTransformPreset::Reciprocal),
        "SQRT" => Some(MathTransformPreset::Sqrt),
        "SQUARE" | "X_SQUARED" => Some(MathTransformPreset::Square),
        "LOGIT" => Some(MathTransformPreset::Logit),
        "PROBIT" => Some(MathTransformPreset::Probit),
        "ARCSIN_SQRT" => Some(MathTransformPreset::ArcsinSqrt),
        "MULTIPLY" => Some(MathTransformPreset::LinearScale { slope: k, intercept: 0.0 }),
        "ADD" => Some(MathTransformPreset::LinearScale { slope: 1.0, intercept: k }),
        other => {
            blocking.push(format!("unmapped transform function `{other}`"));
            None
        }
    }
}

fn translate_math_transform(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let calc = get_obj(content, "calc");
    let std_fn = get_obj(calc, "standardFunction");
    if get_str(calc, "transformType").is_some_and(|t| !t.eq_ignore_ascii_case("STANDARD")) {
        blocking.push(String::from("non-standard transformType"));
    }
    let x_transform = parse_transform_fn(get_obj(std_fn, "xTransform"), blocking);
    let y_transform = parse_transform_fn(get_obj(std_fn, "yTransform"), blocking);
    let error_propagation = match get_str(std_fn, "handleSDAs") {
        Some("DELTA_METHOD") => ErrorPropagationMode::FirstOrderDeltaMethod,
        _ => ErrorPropagationMode::ReplicateEndpoints,
    };
    AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform,
        y_transform,
        error_propagation,
        pharmacology_preset: None,
        interchange_xy: false,
        zero_x_substitute: None,
        asymmetric_ci_policy: Default::default(),
    })
}

fn parse_normalize_anchor(anchor_obj: Option<&JsonMap>, default_anchor: NormalizeAnchor, blocking: &mut Vec<String>) -> NormalizeAnchor {
    let Some(obj) = anchor_obj else {
        return default_anchor;
    };
    match get_str(Some(obj), "defineAs").unwrap_or("").to_ascii_uppercase().as_str() {
        "" => default_anchor,
        "SMALLEST" | "MIN" => NormalizeAnchor::ColumnMin,
        "LARGEST" | "MAX" => NormalizeAnchor::ColumnMax,
        "FIRST" | "FIRST_ROW" => NormalizeAnchor::FirstRow,
        "LAST" | "LAST_ROW" => NormalizeAnchor::LastRow,
        "SUM" => NormalizeAnchor::ColumnSum,
        "VALUE" | "CUSTOM" => match get_f64(Some(obj), "customValue") {
            Some(v) => NormalizeAnchor::FixedValue(v),
            None => {
                blocking.push(String::from("non-numeric or linked custom normalize anchor"));
                default_anchor
            }
        },
        other => {
            blocking.push(format!("unmapped normalize anchor `{other}`"));
            default_anchor
        }
    }
}

fn translate_normalize(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let calc = get_obj(content, "calc");
    let zero_anchor = parse_normalize_anchor(get_obj(calc, "value_0"), NormalizeAnchor::ColumnMin, blocking);
    let hundred_anchor = parse_normalize_anchor(get_obj(calc, "value_100"), NormalizeAnchor::ColumnMax, blocking);
    let output_percent = !matches!(get_str(get_obj(content, "options"), "reportResultsAs"), Some("FRACTIONS") | Some("FRACTION"));
    AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set: true })
}

fn translate_descriptive(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let pct = get_f64(get_obj(content, "options"), "percentileLevel").filter(|&p| (50.0..100.0).contains(&p));
    AnalysisKind::DescriptiveStats(DescriptiveSpec {
        aggregation: parse_subcolumn_agg(content, blocking),
        custom_upper_percentile: pct,
        run_normality_battery: false,
        hypothetical_value: None,
        normality_only: false,
        emit_qq_tabs: false,
    })
}

fn translate_auc(content: Option<&JsonMap>, _blocking: &mut Vec<String>) -> AnalysisKind {
    let calc = get_obj(content, "calc");
    let baseline_obj = get_obj(calc, "baseline");
    let baseline = if get_bool(baseline_obj, "useSpecificValue").unwrap_or(true) {
        AucBaseline::Fixed { value: get_f64(baseline_obj, "value").unwrap_or(0.0) }
    } else {
        AucBaseline::FirstPoint
    };
    let peak_h = get_obj(get_obj(calc, "peak"), "height");
    let min_peak_height = get_f64(peak_h, "minimumYUnitCount").filter(|&v| v >= 0.0).unwrap_or(0.0);
    let min_area_fraction = get_f64(peak_h, "minimumAmplitudePercentage").map(|p| (p / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0);
    AnalysisKind::AreaUnderCurve(AreaUnderCurveSpec { baseline, min_peak_height, min_area_fraction, ..AreaUnderCurveSpec::default() })
}

fn translate_outliers(content: Option<&JsonMap>, blocking: &mut Vec<String>) -> AnalysisKind {
    let calc = get_obj(content, "calc");
    let method = get_str(calc, "method").unwrap_or("ROUT");
    let spec = match method.to_ascii_uppercase().as_str() {
        "ROUT" => OutlierSpec::Rout { fdr_q_percent: get_f64(calc, "q").filter(|&q| q > 0.0 && q <= 100.0).unwrap_or(1.0) },
        "GRUBBS_SINGLE" | "GRUBBS" => {
            OutlierSpec::Grubbs { alpha: get_f64(calc, "alpha").filter(|&a| a > 0.0 && a < 1.0).unwrap_or(0.05), iterative: false }
        }
        "GRUBBS_ITERATIVE" | "ESD" => {
            OutlierSpec::Grubbs { alpha: get_f64(calc, "alpha").filter(|&a| a > 0.0 && a < 1.0).unwrap_or(0.05), iterative: true }
        }
        other => {
            blocking.push(format!("unmapped outlier method `{other}`"));
            OutlierSpec::Rout { fdr_q_percent: 1.0 }
        }
    };
    AnalysisKind::OutlierDetection(spec)
}

fn translate_survival(content: Option<&JsonMap>, _blocking: &mut Vec<String>) -> AnalysisKind {
    let calc = get_obj(content, "calc");
    let log_rank_mantel_cox = get_bool(calc, "logrankTest").unwrap_or(false) || get_bool(calc, "twoGroupsLogrankTest").unwrap_or(true);
    let gehan_breslow_wilcoxon =
        get_bool(calc, "gehanBreslowWilcoxonTest").unwrap_or(false) || get_bool(calc, "twoGroupsGehanBreslowWilcoxonTest").unwrap_or(false);
    let trend_test = get_bool(calc, "logrankTestForTrend").unwrap_or(false);
    let asymmetrical_greenwood_ci = !get_str(calc, "ciMethod").is_some_and(|m| m.eq_ignore_ascii_case("SYMMETRICAL"));
    AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox,
        gehan_breslow_wilcoxon,
        trend_test,
        asymmetrical_greenwood_ci,
        multiplicity: MultiplicitySpec::default(),
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
        event_code: None,
        censored_code: None,
        treat_other_as: Default::default(),
        ratio_median_ci: false,
        mv_status_columns: Vec::new(),
        mv_group_columns: Vec::new(),
    })
}

/// Context passed into [`import_external_analyses`] to keep argument counts bounded.
pub(crate) struct AnalysisImportContext<'a> {
    pub id_index: &'a ArchiveIdIndex,
    pub doc_index: &'a ExternalDocumentIndex,
    pub data_uid_to_sheet_id: &'a BTreeMap<String, SheetId>,
    pub data_set_positions: &'a ExternalDataSetPositions,
    pub uid_to_sheet_id: &'a mut BTreeMap<String, SheetId>,
    pub used_ids: &'a mut HashSet<String>,
    pub warnings: &'a mut Vec<ImportWarning>,
}

/// Import all listed `.prism` analyses via the 3-hop `preferredResultSheet` chain into v9
/// `ResultSheet`s (`ResultProvenance::Imported` by default until mapping rows are `CorpusVerified`).
pub(crate) fn import_external_analyses(
    zip_index: &mut ZipIndex<'_>,
    doc: &mut ProjectDocument,
    ctx: &mut AnalysisImportContext<'_>,
) -> Result<(), FormatError> {
    for (idx, analysis_uid) in ctx.doc_index.analysis_sheet_ids.iter().enumerate() {
        let sheet_val = match load_analysis_json(zip_index, ctx.id_index, analysis_uid, "sheet.json", ctx.warnings)? {
            Some(v) => v,
            None => continue,
        };
        let Some(sheet_obj) = sheet_val.as_object() else {
            continue;
        };

        let attr = ctx.doc_index.sheet_attrs.get(analysis_uid);
        let raw_title = attr
            .and_then(|a| a.title.clone())
            .or_else(|| sheet_obj.get("title").map(|v| parse_rich_text_value(v).1))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("Analysis {}", idx + 1));
        let analysis_class = sheet_obj.get("analysisClass").and_then(serde_json::Value::as_str).unwrap_or("UNKNOWN").trim();
        let color_tag = attr
            .and_then(|a| a.highlight_color)
            .or_else(|| sheet_obj.get("highlightColor").and_then(serde_json::Value::as_str).and_then(parse_archive_color));

        let source_sheet_id = resolve_primary_input_sheet(sheet_obj, analysis_uid, ctx.data_uid_to_sheet_id, ctx.uid_to_sheet_id);
        let params_val = load_analysis_json(zip_index, ctx.id_index, analysis_uid, "parameters.json", ctx.warnings)?;
        let mut translation = params_val.as_ref().and_then(|p| translate_analysis_parameters(analysis_class, p));
        let selection =
            resolve_selected_data_sets(sheet_obj, primary_input_data_sheet_uid(sheet_obj, ctx.data_uid_to_sheet_id), ctx.data_set_positions);
        let selected_data_sets = match selection {
            Ok(sel) => sel,
            Err(reason) => {
                if let Some(tr) = translation.as_mut() {
                    tr.blocking_options.push(reason);
                }
                Vec::new()
            }
        };
        if let Some(tr) = translation.as_mut()
            && let Err(reason) = check_control_in_selection(&tr.kind, &selected_data_sets)
        {
            tr.blocking_options.push(reason);
        }

        let sid = allocate_valid_unique_sheet_id(Some(analysis_uid), "result", idx + 1, ctx.used_ids, ctx.warnings);
        let clamped_title = clamp_title(&raw_title, sid.as_str(), ctx.warnings);

        let can_run_live =
            translation.as_ref().is_some_and(|tr| tr.verification == AnalysisVerificationStatus::CorpusVerified && tr.blocking_options.is_empty())
                && source_sheet_id.as_ref().is_some_and(|src_id| {
                    doc.data_sheet(src_id).is_some_and(|ds| translation.as_ref().is_some_and(|tr| tr.kind.is_compatible_with(ds.archetype.kind())))
                });

        let mut result_sheet = if can_run_live
            && let Some(ref tr) = translation
            && let Some(ref src_id) = source_sheet_id
        {
            let mut spec = AnalysisSpec::new(sid.clone(), src_id.clone(), tr.kind.clone())?;
            spec.selected_data_sets = selected_data_sets;
            spec.set_custom_title(Some(clamped_title.clone()))?;
            ResultSheet::new(spec)?
        } else {
            let reason = build_snapshot_reason(analysis_class, translation.as_ref());
            let clamped_kind = clamp_title(if analysis_class.is_empty() { "External analysis" } else { analysis_class }, sid.as_str(), ctx.warnings);
            let clamped_reason = clamp_title(&reason, sid.as_str(), ctx.warnings);
            let mut spec = AnalysisSpec::new_imported_snapshot(sid.clone(), source_sheet_id, clamped_kind.clone())?;
            if clamped_title != clamped_kind {
                spec.set_custom_title(Some(clamped_title.clone()))?;
            }
            // Strip leading extension dot so canonical provenance matches `imported:<format_id>`.
            let canonical_format = ".prism".trim_start_matches('.');
            let mut rs = ResultSheet::new_imported(spec, canonical_format, clamped_kind, clamped_reason)?;
            rs.tabs = import_result_tabs_3hop(zip_index, ctx.id_index, analysis_uid, sheet_obj, ctx.warnings)?;
            if let Some(results_val) = load_analysis_json(zip_index, ctx.id_index, analysis_uid, "results.json", ctx.warnings)? {
                rs.scalars = extract_cached_scalars(&results_val);
            }
            ctx.warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedAnalysis,
                format!("analyses/{analysis_uid}"),
                format!("analysis `{clamped_title}` (`{analysis_class}`) imported as read-only snapshot: {reason}"),
            ));
            rs
        };

        result_sheet.color_tag = color_tag;
        ctx.uid_to_sheet_id.insert(analysis_uid.clone(), result_sheet.id.clone());
        doc.add_result_sheet(result_sheet)?;
    }
    Ok(())
}

fn build_snapshot_reason(analysis_class: &str, translation: Option<&TranslatedAnalysis>) -> String {
    match translation {
        Some(tr) if !tr.blocking_options.is_empty() => {
            format!("untranslated options for `{analysis_class}`: {}", tr.blocking_options.join("; "))
        }
        Some(_) => {
            format!("mapping for `{analysis_class}` is unverified against corpus recomputation; preserved cached result")
        }
        None => format!("analysis class `{analysis_class}` has no live CurveBook mapping; preserved cached result"),
    }
}

/// External data-set UID → (owning data-sheet UID, zero-based position in that sheet's
/// `table.dataSets` list). The position equals the imported CurveBook data-set index because the
/// importer appends Y data sets in `table.dataSets` order.
pub type ExternalDataSetPositions = BTreeMap<String, (String, usize)>;

/// UID of the first `inputSheets` entry that names an imported data sheet.
#[must_use]
pub fn primary_input_data_sheet_uid<'a>(sheet_obj: &'a JsonMap, data_uid_to_sheet_id: &BTreeMap<String, SheetId>) -> Option<&'a str> {
    sheet_obj.get("inputSheets")?.as_array()?.iter().find_map(|item| {
        let uid = item.as_str().or_else(|| item.get("uid").and_then(serde_json::Value::as_str))?;
        data_uid_to_sheet_id.contains_key(uid).then_some(uid)
    })
}

/// Resolve the analysed data-set subset of an external analysis `sheet.json` into CurveBook
/// [`AnalysisSpec::selected_data_sets`] indices, preserving the listed order.
///
/// Returns an empty selection (analyse every data set) unless `isAllDataAnalyzed` is `false`.
/// Otherwise every `inputDataSets[].uid` must resolve to a Y data set of `source_sheet_uid`;
/// any unresolved, foreign-sheet, or duplicate entry yields `Err` with a blocking reason, so the
/// analysis stays a read-only snapshot rather than silently running on different columns.
pub fn resolve_selected_data_sets(
    sheet_obj: &JsonMap,
    source_sheet_uid: Option<&str>,
    positions: &ExternalDataSetPositions,
) -> Result<Vec<usize>, String> {
    if sheet_obj.get("isAllDataAnalyzed").and_then(serde_json::Value::as_bool) != Some(false) {
        return Ok(Vec::new());
    }
    let Some(source_uid) = source_sheet_uid else {
        return Err(String::from("unresolved input data sheet for selected data sets"));
    };
    let Some(items) = sheet_obj.get("inputDataSets").and_then(serde_json::Value::as_array).filter(|a| !a.is_empty()) else {
        return Err(String::from("`isAllDataAnalyzed` is false but `inputDataSets` is empty"));
    };
    let mut selected = Vec::with_capacity(items.len());
    for item in items {
        let uid = item.as_str().or_else(|| item.get("uid").and_then(serde_json::Value::as_str));
        let Some((owner, pos)) = uid.and_then(|u| positions.get(u)) else {
            return Err(String::from("unresolved input data set"));
        };
        if owner != source_uid {
            return Err(String::from("input data set belongs to another data sheet"));
        }
        if selected.contains(pos) {
            return Err(String::from("duplicate input data set"));
        }
        selected.push(*pos);
    }
    Ok(selected)
}

fn resolve_primary_input_sheet(
    sheet_obj: &JsonMap,
    analysis_uid: &str,
    data_uid_to_sheet_id: &BTreeMap<String, SheetId>,
    uid_to_sheet_id: &BTreeMap<String, SheetId>,
) -> Option<SheetId> {
    let inputs = sheet_obj.get("inputSheets")?.as_array()?;
    for item in inputs {
        let uid = item.as_str().or_else(|| item.get("uid").and_then(serde_json::Value::as_str));
        if let Some(u) = uid
            && let Some(mapped) = data_uid_to_sheet_id.get(u)
        {
            return Some(mapped.clone());
        }
    }
    for item in inputs {
        let uid = item.as_str().or_else(|| item.get("uid").and_then(serde_json::Value::as_str));
        if let Some(u) = uid
            && u != analysis_uid
            && let Some(mapped) = uid_to_sheet_id.get(u)
        {
            return Some(mapped.clone());
        }
    }
    None
}

fn load_analysis_json(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    analysis_uid: &str,
    filename: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<serde_json::Value>, FormatError> {
    let path = match id_index.find_analysis_entry(analysis_uid, filename) {
        IdLookup::Found(p) => p,
        IdLookup::Missing => {
            if filename == "sheet.json" {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    format!("analyses/{analysis_uid}"),
                    format!("missing analysis `{filename}` for `{analysis_uid}`; skipping analysis"),
                ));
            }
            return Ok(None);
        }
        IdLookup::Ambiguous(candidates) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                format!("analyses/{analysis_uid}"),
                format!("ambiguous `{filename}` paths for `{analysis_uid}` ({} candidates); skipping", candidates.len()),
            ));
            return Ok(None);
        }
    };
    let Some(bytes) = zip_index.read_entry(&path)? else {
        return Ok(None);
    };
    match parse_bounded_json_outcome(&bytes) {
        BoundedJsonOutcome::Value(v) => Ok(Some(v)),
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                path,
                format!("could not parse analysis `{filename}` for `{analysis_uid}`: {err}"),
            ));
            Ok(None)
        }
    }
}

/// Follow the exact 3-hop chain:
/// 1. `analyses/<aUid>/sheet.json` -> `preferredResultSheet` (first) and `resultSheets[].uid`
/// 2. `analyses/<aUid>/result_sheets/<rsUid>.json` -> `name`, `dataSheet` (`<viewUid>`)
/// 3. `data/sheets/<viewUid>/sheet.json` -> `table.uid` -> `data/tables/<tableUid>/data.csv`
fn import_result_tabs_3hop(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    analysis_uid: &str,
    sheet_obj: &JsonMap,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Vec<ResultTab>, FormatError> {
    let mut ordered_rs: Vec<(String, Option<String>)> = Vec::new();
    let mut seen_rs = HashSet::new();

    if let Some(pref) = sheet_obj.get("preferredResultSheet").and_then(serde_json::Value::as_str).filter(|s| !s.trim().is_empty())
        && seen_rs.insert(pref.to_owned())
    {
        ordered_rs.push((pref.to_owned(), None));
    }
    if let Some(arr) = sheet_obj.get("resultSheets").and_then(serde_json::Value::as_array) {
        for item in arr {
            let uid = item.as_str().or_else(|| item.get("uid").and_then(serde_json::Value::as_str));
            let title = item.get("title").map(|v| parse_rich_text_value(v).1).filter(|s| !s.trim().is_empty());
            if let Some(u) = uid.filter(|s| !s.trim().is_empty()) {
                if seen_rs.insert(u.to_owned()) {
                    ordered_rs.push((u.to_owned(), title));
                } else if let Some(existing) = ordered_rs.iter_mut().find(|(id, _)| id == u)
                    && existing.1.is_none()
                {
                    existing.1 = title;
                }
            }
        }
    }

    let mut tabs = Vec::new();
    for (tab_idx, (rs_uid, fallback_title)) in ordered_rs.into_iter().take(MAX_TABS_PER_ANALYSIS).enumerate() {
        let Some(tab) = load_single_result_tab(zip_index, id_index, analysis_uid, &rs_uid, fallback_title.as_deref(), tab_idx, warnings)? else {
            continue;
        };
        tabs.push(tab);
    }
    Ok(tabs)
}

fn load_single_result_tab(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    analysis_uid: &str,
    rs_uid: &str,
    fallback_title: Option<&str>,
    tab_idx: usize,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<ResultTab>, FormatError> {
    let rs_path = match id_index.find_analysis_result_sheet_json(analysis_uid, rs_uid) {
        IdLookup::Found(p) => p,
        IdLookup::Missing => return Ok(None),
        IdLookup::Ambiguous(candidates) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                format!("analyses/{analysis_uid}/result_sheets/{rs_uid}"),
                format!("ambiguous result sheet `{rs_uid}` ({} candidates); skipping tab", candidates.len()),
            ));
            return Ok(None);
        }
    };
    let Some(rs_bytes) = zip_index.read_entry(&rs_path)? else {
        return Ok(None);
    };
    let rs_val = match parse_bounded_json_outcome(&rs_bytes) {
        BoundedJsonOutcome::Value(v) => v,
        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
        BoundedJsonOutcome::SyntaxOrType(_) => return Ok(None),
    };
    let Some(rs_obj) = rs_val.as_object() else {
        return Ok(None);
    };
    let tab_title = rs_obj
        .get("name")
        .map(|v| parse_rich_text_value(v).1)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| fallback_title.map(str::to_owned))
        .unwrap_or_else(|| format!("Tab {}", tab_idx + 1));
    let Some(view_uid) = rs_obj.get("dataSheet").and_then(serde_json::Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };

    let IdLookup::Found(view_sheet_path) = id_index.find_data_sheet_json(view_uid) else {
        return Ok(None);
    };
    let Some(view_bytes) = zip_index.read_entry(&view_sheet_path)? else {
        return Ok(None);
    };
    let view_val = match parse_bounded_json_outcome(&view_bytes) {
        BoundedJsonOutcome::Value(v) => v,
        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
        BoundedJsonOutcome::SyntaxOrType(_) => return Ok(None),
    };
    let Some(view_sheet) = ExternalDataSheetJson::from_json(&view_val) else {
        return Ok(None);
    };
    let Some(table_uid) = view_sheet.table.uid.as_deref() else {
        return Ok(None);
    };
    let IdLookup::Found(csv_path) = id_index.find_table_csv(table_uid) else {
        return Ok(None);
    };
    let Some(csv_bytes) = zip_index.read_entry(&csv_path)? else {
        return Ok(None);
    };
    let Ok(csv_text) = std::str::from_utf8(&csv_bytes) else {
        return Ok(None);
    };
    let opts = CsvOptions { header_rows: 0, parse_asterisk_exclusions: false, ..CsvOptions::csv() };
    let Ok(records) = parse_delimited_records(csv_text, &opts) else {
        return Ok(None);
    };

    let has_row_titles = view_sheet.table.row_titles_data_set.is_some();
    let mut headers = resolve_view_column_headers(zip_index, id_index, &view_sheet)?;
    let max_csv_cols = records.iter().map(Vec::len).max().unwrap_or(0);
    let data_col_start = usize::from(has_row_titles);
    let inferred_data_cols = max_csv_cols.saturating_sub(data_col_start);
    let col_count = headers.len().max(inferred_data_cols).clamp(1, MAX_RESULT_TAB_COLS);
    while headers.len() < col_count {
        headers.push(default_column_label(headers.len()));
    }
    headers.truncate(col_count);

    let mut row_labels = Vec::new();
    let mut rows = Vec::new();
    for rec in records.iter().take(MAX_RESULT_TAB_ROWS) {
        let label = if has_row_titles { rec.first().map(|s| s.trim().to_owned()).unwrap_or_default() } else { String::new() };
        let mut cells = Vec::with_capacity(col_count);
        for c in 0..col_count {
            let token = rec.get(data_col_start + c).map(String::as_str).unwrap_or("").trim();
            cells.push(parse_result_cell_token(token));
        }
        row_labels.push(label);
        rows.push(cells);
    }

    let tab_id = format!("imported-tab-{}", tab_idx + 1);
    let clamped_title = clamp_title(&tab_title, &tab_id, warnings);
    Ok(Some(ResultTab {
        id: tab_id,
        title: clamped_title,
        kind: if tab_idx == 0 { ResultTabKind::MainSummary } else { ResultTabKind::Diagnostics },
        column_headers: headers,
        row_labels,
        rows,
        footnotes: vec![String::from("Imported read-only cached result table from external `.prism` archive.")],
    }))
}

fn resolve_view_column_headers(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    view_sheet: &ExternalDataSheetJson,
) -> Result<Vec<String>, FormatError> {
    let mut headers = Vec::new();
    if let Some(x_uid) = view_sheet.table.x_data_set.as_deref()
        && !view_sheet.table.x_data_set_format.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("series"))
    {
        let x_title = read_dataset_title(zip_index, id_index, x_uid)?.unwrap_or_else(|| String::from("X"));
        headers.push(x_title);
    }
    for (idx, ds_uid) in view_sheet.table.data_sets.iter().take(MAX_RESULT_TAB_COLS).enumerate() {
        let title = read_dataset_title(zip_index, id_index, ds_uid)?.unwrap_or_else(|| default_column_label(idx));
        headers.push(title);
    }
    Ok(headers)
}

fn read_dataset_title(zip_index: &mut ZipIndex<'_>, id_index: &ArchiveIdIndex, uid: &str) -> Result<Option<String>, FormatError> {
    let IdLookup::Found(path) = id_index.find_dataset_json(uid) else {
        return Ok(None);
    };
    let Some(bytes) = zip_index.read_entry(&path)? else {
        return Ok(None);
    };
    match parse_bounded_json_outcome(&bytes) {
        BoundedJsonOutcome::Value(val) => Ok(ExternalDataSetJson::from_json(&val).and_then(|ds| ds.title_plain)),
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(_) => Ok(None),
    }
}

fn parse_result_cell_token(token: &str) -> ResultCell {
    if token.is_empty() {
        return ResultCell::Empty;
    }
    if let Ok(v) = token.parse::<f64>()
        && v.is_finite()
    {
        return ResultCell::Number { value: v, prefix: None, units: None };
    }
    ResultCell::Text(token.to_owned())
}

/// Extract bounded finite numeric scalar entries from `analyses/<uid>/results.json` for read-only
/// inspection and future cached-vs-recomputed comparison.
fn extract_cached_scalars(results_val: &serde_json::Value) -> Vec<ResultScalarEntry> {
    let mut out = Vec::new();
    let root = results_val.get("content").unwrap_or(results_val);
    collect_scalars_recursive(root, "", 0, &mut out);
    out
}

fn collect_scalars_recursive(val: &serde_json::Value, prefix: &str, depth: usize, out: &mut Vec<ResultScalarEntry>) {
    if depth > 6 || out.len() >= MAX_CACHED_SCALARS_PER_ANALYSIS {
        return;
    }
    match val {
        serde_json::Value::Number(n) => {
            if !prefix.is_empty()
                && let Some(v) = n.as_f64()
                && v.is_finite()
            {
                out.push(ResultScalarEntry { key: prefix.to_owned(), data_set_index: None, value: v, units: None });
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if out.len() >= MAX_CACHED_SCALARS_PER_ANALYSIS {
                    break;
                }
                if matches!(k.as_str(), "$id" | "@class" | "version" | "status" | "id" | "warning" | "error") {
                    continue;
                }
                let next_key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                collect_scalars_recursive(v, &next_key, depth + 1, out);
            }
        }
        _ => {}
    }
}
