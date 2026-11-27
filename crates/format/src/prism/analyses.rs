//! External `.prism` analysis translation, 3-hop `preferredResultSheet` imported-result ingestion,
//! data-driven mapping table with textbook justifications, and cached-vs-recomputed diagnostics (§5.2).

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AreaUnderCurveSpec, AucBaseline, DataSheet, DescriptiveSpec, ErrorPropagationMode, FactorialAnovaSpec,
    FactorialComparisonFamilyMode, FitMethod, MathTransformPreset, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, OneWayMethod, OutlierHandling, OutlierSpec, PairwiseComparisonRecord, ProjectDocument, RepeatedMeasuresDesign, ReplicateFitMode,
    ResultCell, ResultScalarEntry, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnAggregation, SurvivalAnalysisSpec, TableArchetype,
    TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, WeightingScheme, default_column_label,
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

/// Minimum corpus-compared analyses required to promote a `.prism` analysis design to live recompute.
pub const MIN_VERIFIED_DESIGN_ANALYSES: usize = 20;

/// Minimum distinct corpus source records required to promote a `.prism` analysis design.
pub const MIN_VERIFIED_DESIGN_SOURCES: usize = 2;

/// Numerator of the minimum full-agreement ratio (`99 / 100` = 99%) required to promote a design.
pub const MIN_VERIFIED_DESIGN_AGREEMENT_NUM: usize = 99;

/// Denominator of the minimum full-agreement ratio (`99 / 100` = 99%) required to promote a design.
pub const MIN_VERIFIED_DESIGN_AGREEMENT_DEN: usize = 100;

/// Verification status for a translated foreign `.prism` analysis design (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisVerificationStatus {
    /// Recomputed CurveBook outputs matched cached corpus results within tolerance across the
    /// promotion bar AND CurveBook's own statistical kernel passes its textbook D1 oracle.
    CorpusVerified,
    /// Parameter mapping is documented and implemented, but live translation remains off by default
    /// until corpus-wide cached-vs-recomputed verification clears the per-design promotion bar.
    Unverified,
}

/// Data-driven specification mapping one external `.prism` `analysisClass` identifier to CurveBook
/// semantics with a textbook justification and numeric comparison tolerance (§5.2).
///
/// Per-design promotion status lives in [`VERIFIED_ANALYSIS_DESIGNS`], keyed by
/// [`AnalysisDesignKey`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisMappingRow {
    /// Exact external `analysisClass` wire identifier (for example `"TRNSFORM"` or `"ONE_WAY_ANOVA"`).
    pub foreign_class: &'static str,
    /// Target CurveBook `AnalysisKind` family label.
    pub curvebook_kind_label: &'static str,
    /// Textbook statistical or mathematical definition justifying semantic equivalence.
    pub justification: &'static str,
    /// Documented relative tolerance for cached-vs-recomputed scalar agreement checks.
    pub relative_tolerance: f64,
    /// Documented absolute tolerance for cached-vs-recomputed scalar agreement checks near zero.
    pub absolute_tolerance: f64,
}

/// Complete data-driven mapping table covering the 11 corpus-observed `.prism` analysis classes.
///
/// Live-recompute promotion is decided per design in [`VERIFIED_ANALYSIS_DESIGNS`]; any design not
/// listed there (or carrying blocking options) imports as a read-only v9 `ResultSheet` snapshot.
pub const ANALYSIS_MAPPINGS: &[AnalysisMappingRow] = &[
    AnalysisMappingRow {
        foreign_class: "T_TESTS_AND_NONPARAMETRIC",
        curvebook_kind_label: "TwoGroupComparison",
        justification: "Student (1908) pooled t-test, Welch (1947) unequal-variance t-test, Mann & Whitney (1947) rank-sum test, Wilcoxon (1945) / Pratt (1959) signed-rank test, and Smirnov (1948) two-sample KS test.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "ONE_WAY_ANOVA",
        curvebook_kind_label: "OneWayAnova",
        justification: "Fisher (1925) one-way ANOVA, Brown & Forsythe (1974) / Welch (1951) heteroscedastic ANOVA, Greenhouse & Geisser (1959) RM ANOVA, Kruskal & Wallis (1952), and Friedman (1937).",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "TWO_WAY_ANOVA",
        curvebook_kind_label: "FactorialAnova",
        justification: "Maxwell, Delaney, & Kelley (2017) Type III factorial ANOVA and Kenward & Roger (1997) REML linear mixed-effects models for repeated-measures factorial designs.",
        relative_tolerance: 1e-5,
        absolute_tolerance: 1e-8,
    },
    AnalysisMappingRow {
        foreign_class: "NONLINEAR_REGRESSION",
        curvebook_kind_label: "NonlinearRegression",
        justification: "More (1978) Levenberg-Marquardt nonlinear least squares, Motulsky & Christopoulos (2004) profile-likelihood and Wald parameter intervals, and Motulsky & Brown (2006) ROUT outlier detection.",
        relative_tolerance: 1e-4,
        absolute_tolerance: 1e-7,
    },
    AnalysisMappingRow {
        foreign_class: "NORMALITY_TESTS",
        curvebook_kind_label: "DescriptiveStats",
        justification: "Shapiro & Wilk (1965) / Royston (1995), D'Agostino & Pearson (1973) omnibus K^2, Anderson & Darling (1954) A^2*, and Lilliefors (1967) normality and lognormality tests.",
        relative_tolerance: 1e-5,
        absolute_tolerance: 1e-8,
    },
    AnalysisMappingRow {
        foreign_class: "TRNSFORM",
        curvebook_kind_label: "Transform::Math",
        justification: "Element-wise scalar mathematical transforms (log10, ln, exp10, exp, reciprocal, sqrt, square, logit, probit, arcsin-sqrt, affine scaling) with endpoint or first-order delta-method error propagation.",
        relative_tolerance: 1e-9,
        absolute_tolerance: 1e-12,
    },
    AnalysisMappingRow {
        foreign_class: "NORMALIZE",
        curvebook_kind_label: "Transform::Normalize",
        justification: "Linear rescaling (y - a0) / (a100 - a0) onto [0%, 100%] or [0, 1] using column extrema, boundary rows, or fixed numeric anchors.",
        relative_tolerance: 1e-9,
        absolute_tolerance: 1e-12,
    },
    AnalysisMappingRow {
        foreign_class: "DESCRIPTIVE_STATISTICS",
        curvebook_kind_label: "DescriptiveStats",
        justification: "Univariate sample moments (mean, SD, SEM, CV, skewness, kurtosis, geometric mean), Hyndman & Fan (1996) Type-7 quantiles, and Student-t confidence intervals.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "AREA_UNDER_CURVE",
        curvebook_kind_label: "AreaUnderCurve",
        justification: "Composite trapezoidal quadrature above a fixed or first-point baseline with peak filtering and Bailer (1988) independent-cohort AUC standard error.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "IDENTIFY_OUTLIERS",
        curvebook_kind_label: "OutlierDetection",
        justification: "Grubbs (1969) extreme studentized deviate (single and iterative ESD) and Motulsky & Brown (2006) univariate ROUT FDR outlier detection.",
        relative_tolerance: 1e-6,
        absolute_tolerance: 1e-9,
    },
    AnalysisMappingRow {
        foreign_class: "SURVIVAL",
        curvebook_kind_label: "Survival::KaplanMeier",
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

/// Tail qualifier for an external `.prism` analysis design key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DesignTail {
    /// Analysis class has no one-tailed vs. two-tailed option.
    NotApplicable,
    /// Two-tailed hypothesis test (`P_VALUE_TWO_TAILED` or absent default).
    TwoTailed,
    /// One-tailed hypothesis test (`P_VALUE_ONE_TAILED`).
    OneTailed,
}

/// Canonical per-design key identifying an external `.prism` analysis configuration for
/// corpus verification and live-recompute promotion.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnalysisDesignKey {
    pub foreign_class: String,
    pub experimental_design: Option<String>,
    pub tail: DesignTail,
    pub post_test_family: Option<String>,
    pub post_test: Option<String>,
}

impl AnalysisDesignKey {
    /// Stable canonical label shared between `curvebook-format` and `xtask interop-corpus`.
    #[must_use]
    pub fn canonical_label(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for AnalysisDesignKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.foreign_class)?;
        if let Some(design) = &self.experimental_design {
            write!(f, " | {design}")?;
        }
        match self.tail {
            DesignTail::NotApplicable => {}
            DesignTail::TwoTailed => f.write_str(" | two-tailed")?,
            DesignTail::OneTailed => f.write_str(" | one-tailed")?,
        }
        match (&self.post_test_family, &self.post_test) {
            (Some(family), Some(test)) => write!(f, " | {family}:{test}")?,
            (Some(family), None) => write!(f, " | {family}:<none>")?,
            (None, Some(test)) => write!(f, " | <none>:{test}")?,
            (None, None) => {}
        }
        Ok(())
    }
}

/// Corpus evidence snapshot justifying live-recompute promotion for one [`VerifiedAnalysisDesign`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusDesignEvidence {
    pub analyses_compared: usize,
    pub distinct_source_records: usize,
    pub analyses_fully_agreed: usize,
    pub verified_on: &'static str,
}

impl CorpusDesignEvidence {
    /// Returns `true` iff this design clears all promotion thresholds (`>= 20` analyses compared,
    /// `>= 2` distinct source records, `>= 99%` full agreement, and a non-empty verification date).
    #[must_use]
    pub const fn meets_bar(&self) -> bool {
        if self.analyses_compared < MIN_VERIFIED_DESIGN_ANALYSES
            || self.distinct_source_records < MIN_VERIFIED_DESIGN_SOURCES
            || self.analyses_fully_agreed > self.analyses_compared
            || self.verified_on.is_empty()
        {
            return false;
        }
        let lhs = (self.analyses_fully_agreed as u128) * (MIN_VERIFIED_DESIGN_AGREEMENT_DEN as u128);
        let rhs = (self.analyses_compared as u128) * (MIN_VERIFIED_DESIGN_AGREEMENT_NUM as u128);
        lhs >= rhs
    }
}

/// Declarative row authorizing default live-recompute promotion for one corpus-verified `.prism`
/// analysis design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedAnalysisDesign {
    pub foreign_class: &'static str,
    pub experimental_design: Option<&'static str>,
    pub tail: DesignTail,
    pub post_test_family: Option<&'static str>,
    pub post_test: Option<&'static str>,
    pub requires_pairwise_check: bool,
    pub evidence: CorpusDesignEvidence,
    pub notes: &'static str,
}

impl VerifiedAnalysisDesign {
    /// Owned [`AnalysisDesignKey`] corresponding to this static table row.
    #[must_use]
    pub fn design_key(&self) -> AnalysisDesignKey {
        AnalysisDesignKey {
            foreign_class: self.foreign_class.to_owned(),
            experimental_design: self.experimental_design.map(str::to_owned),
            tail: self.tail,
            post_test_family: self.post_test_family.map(str::to_owned),
            post_test: self.post_test.map(str::to_owned),
        }
    }
}

/// Corpus-verified `.prism` analysis designs cleared for default live recomputation.
///
/// Every entry was empirically audited against the local interop corpus via
/// `cargo xtask interop-corpus --strict --observe` and clears all three promotion bars
/// (`>= 20` analyses compared, `>= 2` distinct source records, `>= 99%` full agreement, plus
/// non-zero pairwise agreement checks for post-test designs).
pub const VERIFIED_ANALYSIS_DESIGNS: &[VerifiedAnalysisDesign] = &[
    VerifiedAnalysisDesign {
        foreign_class: TT,
        experimental_design: Some("UNPAIRED_T_TEST"),
        tail: DesignTail::TwoTailed,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 571, distinct_source_records: 39, analyses_fully_agreed: 570, verified_on: "2026-11-27" },
        notes: "1/571 disagreement audited as stale cached results.json; direct recomputation from raw archive data.csv matches CurveBook to 12+ digits.",
    },
    VerifiedAnalysisDesign {
        foreign_class: TT,
        experimental_design: Some("PAIRED_T_TEST"),
        tail: DesignTail::TwoTailed,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 213, distinct_source_records: 13, analyses_fully_agreed: 213, verified_on: "2026-11-27" },
        notes: "100% agreement across all compared paired t-test scalars (|t| and two-tailed p).",
    },
    VerifiedAnalysisDesign {
        foreign_class: TT,
        experimental_design: Some("WELCH_T_TEST"),
        tail: DesignTail::TwoTailed,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 154, distinct_source_records: 19, analyses_fully_agreed: 154, verified_on: "2026-11-27" },
        notes: "100% agreement across all compared Welch t-test scalars (|t| and two-tailed p).",
    },
    VerifiedAnalysisDesign {
        foreign_class: OW,
        experimental_design: Some("ORDINARY_ANOVA"),
        tail: DesignTail::NotApplicable,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 22, distinct_source_records: 5, analyses_fully_agreed: 22, verified_on: "2026-11-27" },
        notes: "100% agreement across omnibus F, P value, R squared, and MS residual.",
    },
    VerifiedAnalysisDesign {
        foreign_class: OW,
        experimental_design: Some("ORDINARY_ANOVA"),
        tail: DesignTail::NotApplicable,
        post_test_family: Some("ALL_VS_ALL"),
        post_test: Some("TUKEY"),
        requires_pairwise_check: true,
        evidence: CorpusDesignEvidence { analyses_compared: 423, distinct_source_records: 37, analyses_fully_agreed: 421, verified_on: "2026-11-27" },
        notes: "2/423 omnibus disagreements audited as stale cached results.json against raw data.csv; pairwise adjusted p uses 1e-5 external-cache tolerance for AS-190/Copenhaver-Holland tail quadrature while CurveBook stays exact (1e-13..1e-15) and omnibus/|mean diff| remain at 1e-9/1e-6.",
    },
    VerifiedAnalysisDesign {
        foreign_class: OW,
        experimental_design: Some("ORDINARY_ANOVA"),
        tail: DesignTail::NotApplicable,
        post_test_family: Some("CONTROL_VS_ALL"),
        post_test: Some("DUNNETT"),
        requires_pairwise_check: true,
        evidence: CorpusDesignEvidence { analyses_compared: 37, distinct_source_records: 17, analyses_fully_agreed: 37, verified_on: "2026-11-27" },
        notes: "100% omnibus and pairwise agreement (238/238 pairwise quantities) when Dunnett control lies in the analysed selection.",
    },
    VerifiedAnalysisDesign {
        foreign_class: TW,
        experimental_design: Some("ORDINARY_ANOVA"),
        tail: DesignTail::NotApplicable,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 71, distinct_source_records: 13, analyses_fully_agreed: 71, verified_on: "2026-11-27" },
        notes: "100% agreement across all 7 omnibus Type III ANOVA scalars; additive models with includeInteractionTerm == false remain blocked.",
    },
    VerifiedAnalysisDesign {
        foreign_class: DS,
        experimental_design: None,
        tail: DesignTail::NotApplicable,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 139, distinct_source_records: 20, analyses_fully_agreed: 139, verified_on: "2026-11-27" },
        notes: "100% agreement across N, Mean, SD, SEM, and Median (2,240/2,240 scalars); SUB_COLUMN replicate handling remains blocked.",
    },
    VerifiedAnalysisDesign {
        foreign_class: "AREA_UNDER_CURVE",
        experimental_design: None,
        tail: DesignTail::NotApplicable,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 84, distinct_source_records: 6, analyses_fully_agreed: 84, verified_on: "2026-11-27" },
        notes: "100% agreement across TotalArea (249/249 scalars).",
    },
    VerifiedAnalysisDesign {
        foreign_class: "IDENTIFY_OUTLIERS",
        experimental_design: None,
        tail: DesignTail::NotApplicable,
        post_test_family: None,
        post_test: None,
        requires_pairwise_check: false,
        evidence: CorpusDesignEvidence { analyses_compared: 96, distinct_source_records: 10, analyses_fully_agreed: 96, verified_on: "2026-11-27" },
        notes: "100% agreement across OutlierCount; SUB_COLUMN and row-averaged replicates on multi-subcolumn tables remain blocked.",
    },
];

fn normalize_experimental_design_alias(raw: &str) -> String {
    let upper = raw.trim().to_ascii_uppercase();
    match upper.as_str() {
        "WELCH_T_TEST" | "UNPAIRED_WELCH_T_TEST" => String::from("WELCH_T_TEST"),
        "BROWN_FORSYTHE_ANOVA" | "BROWN_FORSYTHE_AND_WELCH" | "BROWN_FORSYTHE_WELCH" | "WELCH_ANOVA" => String::from("BROWN_FORSYTHE_ANOVA"),
        "MANN_WHITNEY" | "MANN_WHITNEY_TEST" => String::from("MANN_WHITNEY_TEST"),
        "WILCOXON" | "WILCOXON_TEST" | "WILCOXON_MATCHED_PAIRS" => String::from("WILCOXON_TEST"),
        "KRUSKAL_WALLIS" | "KRUSKAL_WALLIS_TEST" => String::from("KRUSKAL_WALLIS_TEST"),
        "FRIEDMAN" | "FRIEDMAN_TEST" => String::from("FRIEDMAN_TEST"),
        "REPEATED_MEASURES_BY_BOTH" | "REPEATED_MEASURES_BY_BOTH_FACTORS" => String::from("REPEATED_MEASURES_BY_BOTH_FACTORS"),
        _ => upper,
    }
}

fn normalize_post_test_alias(upper: &str) -> String {
    match upper {
        "TUKEY" | "TUKEY_HSD" => String::from("TUKEY"),
        "DUNNET" | "DUNNETT" => String::from("DUNNETT"),
        _ => upper.to_owned(),
    }
}

fn normalize_post_test_choice(family: String, raw_test: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = raw_test.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let upper = raw.to_ascii_uppercase();
    if matches!(upper.as_str(), "NO_TEST" | "NONE") {
        return (None, None);
    }
    if upper == "NO_CORRECTION" {
        return (Some(family), Some(String::from("NO_CORRECTION")));
    }
    (Some(family), Some(normalize_post_test_alias(&upper)))
}

fn extract_post_test_key(content: Option<&JsonMap>) -> (Option<String>, Option<String>) {
    let Some(pt) = get_obj(content, "postTests") else {
        return (None, None);
    };
    let Some(pt_type) = get_str(Some(pt), "type").map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    if pt_type == "NO_TESTS" {
        return (None, None);
    }
    let Some(pairwise) = get_obj(Some(pt), "pairwiseTests") else {
        return (Some(pt_type), None);
    };
    let design_obj = get_obj(Some(pairwise), "pairwiseDesign");
    let family =
        get_str(design_obj, "type").map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).unwrap_or_else(|| String::from("ALL_VS_ALL"));
    let mc = get_obj(Some(pairwise), "multipleComparisons");
    let mc_type = get_str(mc, "type").map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty());
    match mc_type.as_deref() {
        Some("NO_CORRECTION") => (Some(family), Some(String::from("NO_CORRECTION"))),
        Some("STATISTICAL") => {
            let raw = get_str(get_obj(mc, "statistical"), "test").or_else(|| get_str(mc, "statistical"));
            normalize_post_test_choice(family, raw)
        }
        Some("FDR") => {
            let raw = get_str(get_obj(mc, "fdr"), "test").or_else(|| get_str(mc, "fdr"));
            normalize_post_test_choice(family, raw)
        }
        Some(other) => (Some(family), Some(other.to_owned())),
        None => (Some(family), None),
    }
}

/// Extract the canonical [`AnalysisDesignKey`] for an external `.prism` analysis from its
/// `analysisClass` and `parameters.json`, normalizing known corpus aliases.
#[must_use]
pub fn extract_analysis_design_key(foreign_class: &str, params_json: &serde_json::Value) -> AnalysisDesignKey {
    let class = foreign_class.trim().to_ascii_uppercase();
    let content = params_json.get("content").and_then(serde_json::Value::as_object).or_else(|| params_json.as_object());
    let experimental_design =
        get_str(content, "experimentalDesign").map(str::trim).filter(|s| !s.is_empty()).map(normalize_experimental_design_alias);
    let tail = if class == TT {
        match get_str(get_obj(content, "options"), "pValueTailType").map(|s| s.trim().to_ascii_uppercase()).as_deref() {
            Some("P_VALUE_ONE_TAILED") => DesignTail::OneTailed,
            None | Some("P_VALUE_TWO_TAILED") => DesignTail::TwoTailed,
            Some(_) => DesignTail::NotApplicable,
        }
    } else {
        DesignTail::NotApplicable
    };
    let (post_test_family, post_test) = extract_post_test_key(content);
    AnalysisDesignKey { foreign_class: class, experimental_design, tail, post_test_family, post_test }
}

/// Look up a corpus-verified promotion entry matching `key`.
#[must_use]
pub fn find_verified_analysis_design(key: &AnalysisDesignKey) -> Option<&'static VerifiedAnalysisDesign> {
    VERIFIED_ANALYSIS_DESIGNS.iter().find(|row| {
        row.foreign_class.eq_ignore_ascii_case(&key.foreign_class)
            && match (row.experimental_design, key.experimental_design.as_deref()) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            }
            && row.tail == key.tail
            && match (row.post_test_family, key.post_test_family.as_deref()) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            }
            && match (row.post_test, key.post_test.as_deref()) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            }
    })
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

/// Bound relation carried by a cached external `.prism` value (for example `pValue.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CachedBound {
    /// Cached value is an exact numeric point (`P_VALUE_EQUAL` or absent `state`).
    Equal,
    /// Cached value is an upper bound (`P_VALUE_LESS`, e.g. `p < 1e-15`).
    LessThan,
    /// Cached value is a lower bound (`P_VALUE_GREATER`, e.g. `p > 0.1`).
    GreaterThan,
}

impl CachedBound {
    /// Check whether `recomputed` agrees with `cached` under this bound and tolerances.
    #[must_use]
    pub fn agrees(self, recomputed: f64, cached: f64, abs_tol: f64, rel_tol: f64) -> bool {
        let abs_t = if abs_tol.is_finite() && abs_tol >= 0.0 { abs_tol } else { 0.0 };
        let rel_t = if rel_tol.is_finite() && rel_tol >= 0.0 { rel_tol } else { 0.0 };
        match self {
            Self::Equal => {
                let abs_diff = (recomputed - cached).abs();
                let scale = recomputed.abs().max(cached.abs());
                abs_diff <= abs_t.max(rel_t * scale)
            }
            Self::LessThan => recomputed <= cached + abs_t,
            Self::GreaterThan => recomputed + abs_t >= cached,
        }
    }
}

/// Single numeric disagreement observed between a recomputed CurveBook scalar or pairwise quantity
/// and a cached `.prism` value (§5.2).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedScalarDiff {
    /// Canonical CurveBook scalar or pairwise key (never contains user titles; safe for aggregate diagnostics).
    pub key: String,
    /// Concise mathematical symbol for user-facing warnings (for example `"p"`, `"q"`, `"t"`, `"F"`).
    pub symbol: String,
    /// Human-readable quantity label for user-facing warnings (may include the group or pair label).
    pub quantity_label: String,
    pub data_set_index: Option<usize>,
    pub recomputed: f64,
    pub cached: f64,
    pub bound: CachedBound,
    pub abs_diff: f64,
    pub rel_diff: f64,
}

/// Summary of comparing a recomputed [`ResultSheet`] against cached external scalars and pairwise
/// post-test results (§5.2).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CachedResultDiagnostic {
    /// Total scalar + pairwise quantities compared.
    pub compared_count: usize,
    /// Number of pairwise comparison quantities (adjusted *p*/*q* and mean differences) compared.
    pub pairwise_compared_count: usize,
    /// Number of pairwise comparison quantities that disagreed beyond tolerance.
    pub pairwise_disagreements: usize,
    pub disagreements: Vec<CachedScalarDiff>,
    /// Number of compared values per canonical CurveBook scalar/pairwise key.
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

/// Declarative link from a cached `.prism` `multipleComparisons.tests` object to
/// [`ResultSheet::pairwise_comparisons`] (§5.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedPairwiseLink {
    /// External `analysisClass` identifier the link applies to.
    pub foreign_class: &'static str,
    /// Dotted path below `results.json` `content` pointing to the `multipleComparisons.tests` object.
    pub tests_path: &'static str,
    /// Field name for contrast magnitude (for example `"meansDifference"`), or `None` for rank tests.
    pub diff_field: Option<&'static str>,
    /// Optional absolute tolerance override used solely for cached pairwise *p*/*q* comparisons
    /// (keeping omnibus scalars and pairwise `|meansDifference|` at `abs_tol`/`rel_tol`).
    ///
    /// Set to `Some(1e-5)` for `ordinaryAnovaModel.multipleComparisons.tests` because external
    /// `.prism` Studentized-range tail quadrature (AS-190/Copenhaver-Holland) exhibits up to
    /// `4.8e-6` absolute error at small `df` (`5..8`) and large `q` (`10..33`), while CurveBook's
    /// exact quadrature remains unchanged (`1e-13..1e-15`).
    pub p_abs_tol: Option<f64>,
}

/// Single pre-extracted cached scalar reference from a `.prism` `results.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedScalarRef {
    pub key: String,
    pub data_set_index: Option<usize>,
    pub value: f64,
    pub bound: CachedBound,
    pub comparison: CrosswalkComparison,
}

/// Single pre-extracted cached pairwise post-test reference from a `.prism` `results.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedPairwiseRef {
    pub col_a: usize,
    pub col_b: usize,
    /// `(value, bound, is_fdr_q)` extracted from `pValueAdjusted`, `qValue`, or `pValueIndividual`.
    pub p_or_q: Option<(f64, CachedBound, bool)>,
    pub means_diff: Option<f64>,
    /// Optional absolute tolerance override applied solely to `p_or_q` comparisons.
    pub p_abs_tol: Option<f64>,
}

/// Transient cached scalar and pairwise reference extracted from a `.prism` `results.json`
/// for post-recompute agreement checks (never persisted to `.curvebook`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CachedAnalysisReference {
    pub scalars: Vec<CachedScalarRef>,
    pub pairwise: Vec<CachedPairwiseRef>,
}

/// Transient 3-hop cached snapshot payload preserved alongside a promoted live `.prism` analysis
/// so `curvebook-engine` can fall back cleanly if first recalculation fails on degenerate input.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedAnalysisFallback {
    pub original_kind_label: String,
    pub tabs: Vec<ResultTab>,
    pub scalars: Vec<ResultScalarEntry>,
}

/// Transient metadata emitted in [`crate::ImportReport::promoted_analyses`] for each promoted
/// live `.prism` analysis; consumed by `curvebook-engine` after first recalculation and never
/// serialized into `.curvebook` files.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotedAnalysisRef {
    pub sheet_id: SheetId,
    pub analysis_uid: String,
    pub title: String,
    pub foreign_class: String,
    pub abs_tol: f64,
    pub rel_tol: f64,
    pub reference: CachedAnalysisReference,
    pub fallback: ImportedAnalysisFallback,
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
const NT: &str = "NORMALITY_TESTS";
const SV: &str = "SURVIVAL";
const NL: &str = "NONLINEAR_REGRESSION";

/// Cached-result scalar crosswalk. Every link names one quantity with the same textbook definition
/// on both sides; quantities whose definitions differ (for example hazard-ratio estimators that use
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
    link(OW, "kruskalWallisTestModel.kruskalWallisTest.statistic", "Kruskal-Wallis H"),
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
    // Per-data-set normality and lognormality battery (Royston 1995; D'Agostino & Pearson 1973;
    // Anderson & Darling 1954 / Stephens 1986; Lilliefors 1967). Cached `.prism` files store
    // unadjusted Anderson-Darling A^2 (`andersonDarlingTest.A2`) alongside the Stephens Case 3
    // p-value; CurveBook publishes both `AndersonDarlingA2` (unadjusted) and `AndersonDarlingA2Star`
    // (Stephens-modified). Lilliefors KS p-values (`kolmogorovSmirnovTest.pValue.value`) are left
    // out because cached files use the Dallal & Wilkinson (1986) analytic approximation and censor
    // every p > 0.10 to `0.1` (`state: "P_VALUE_GREATER"`), whereas CurveBook computes continuous
    // Lilliefors upper-tail probabilities in `[0, 1]` via deterministic Monte Carlo PCHIP tables.
    link(NT, "samples.*.value.normalityModel.shapiroWilkTest.W", "ShapiroWilkW"),
    link(NT, "samples.*.value.normalityModel.shapiroWilkTest.pValue.value", "ShapiroWilkP"),
    link(NT, "samples.*.value.normalityModel.dAgostinoPearsonTest.K2", "DAgostinoPearsonK2"),
    link(NT, "samples.*.value.normalityModel.dAgostinoPearsonTest.pValue.value", "DAgostinoPearsonP"),
    link(NT, "samples.*.value.normalityModel.andersonDarlingTest.A2", "AndersonDarlingA2"),
    link(NT, "samples.*.value.normalityModel.andersonDarlingTest.pValue.value", "AndersonDarlingP"),
    link(NT, "samples.*.value.normalityModel.kolmogorovSmirnovTest.distance", "KolmogorovSmirnovD"),
    link(NT, "samples.*.value.lognormalityModel.shapiroWilkTest.W", "LognormalShapiroWilkW"),
    link(NT, "samples.*.value.lognormalityModel.shapiroWilkTest.pValue.value", "LognormalShapiroWilkP"),
    link(NT, "samples.*.value.lognormalityModel.dAgostinoPearsonTest.K2", "LognormalDAgostinoPearsonK2"),
    link(NT, "samples.*.value.lognormalityModel.dAgostinoPearsonTest.pValue.value", "LognormalDAgostinoPearsonP"),
    link(NT, "samples.*.value.lognormalityModel.andersonDarlingTest.A2", "LognormalAndersonDarlingA2"),
    link(NT, "samples.*.value.lognormalityModel.andersonDarlingTest.pValue.value", "LognormalAndersonDarlingP"),
    link(NT, "samples.*.value.lognormalityModel.kolmogorovSmirnovTest.distance", "LognormalKolmogorovSmirnovD"),
    link(NT, "samples.*.value.relativeLikelihoodTest.probabilityNormal", "PNormal"),
    link(NT, "samples.*.value.relativeLikelihoodTest.probabilityLognormal", "PLognormal"),
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

/// Declarative pairwise post-test crosswalk covering one-way ANOVA models.
pub const CACHED_PAIRWISE_LINKS: &[CachedPairwiseLink] = &[
    CachedPairwiseLink {
        foreign_class: OW,
        tests_path: "ordinaryAnovaModel.multipleComparisons.tests",
        diff_field: Some("meansDifference"),
        // External `.prism` AS-190/Copenhaver-Holland Studentized-range tail quadrature has up to
        // 4.8e-6 absolute error at small df (5..8) and large q (10..33); CurveBook keeps its own
        // exact (1e-13..1e-15) quadrature and keeps omnibus/mean-diff checks at 1e-9/1e-6.
        p_abs_tol: Some(1e-5),
    },
    CachedPairwiseLink {
        foreign_class: OW,
        tests_path: "repeatedMeasuresGlmAnovaModel.multipleComparisons.tests",
        diff_field: Some("meansDifference"),
        p_abs_tol: None,
    },
    CachedPairwiseLink {
        foreign_class: OW,
        tests_path: "brownForsytheAnovaModel.multipleComparisons.tests",
        diff_field: Some("meansDifference"),
        p_abs_tol: None,
    },
    CachedPairwiseLink { foreign_class: OW, tests_path: "kruskalWallisTestModel.multipleComparisons.tests", diff_field: None, p_abs_tol: None },
];

fn parse_cached_bound(enclosing: &serde_json::Value) -> CachedBound {
    match enclosing.get("state").and_then(serde_json::Value::as_str).map(str::trim) {
        Some(s) if s.eq_ignore_ascii_case("P_VALUE_LESS") => CachedBound::LessThan,
        Some(s) if s.eq_ignore_ascii_case("P_VALUE_GREATER") => CachedBound::GreaterThan,
        _ => CachedBound::Equal,
    }
}

/// Read every cached value that `link` selects from a `results.json` document, paired with the
/// data-set index of its nearest `id.col.idx` ancestor.
#[must_use]
pub fn cached_values_for_link(link: &CachedScalarLink, results_val: &serde_json::Value) -> Vec<(Option<usize>, f64)> {
    cached_values_with_bounds_for_link(link, results_val).into_iter().map(|(col, v, _)| (col, v)).collect()
}

fn cached_values_with_bounds_for_link(link: &CachedScalarLink, results_val: &serde_json::Value) -> Vec<(Option<usize>, f64, CachedBound)> {
    let root = results_val.get("content").unwrap_or(results_val);
    let segments: Vec<&str> = link.cached_path.split('.').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    collect_link_values_with_bounds(root, &segments, None, &mut out);
    out
}

fn collect_link_values_with_bounds(val: &serde_json::Value, segments: &[&str], col: Option<usize>, out: &mut Vec<(Option<usize>, f64, CachedBound)>) {
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
                out.push((col, v, CachedBound::Equal));
            }
        }
        Some((&"value", [])) => {
            if let Some(child) = val.get("value")
                && let Some(v) = child.as_f64().filter(|v| v.is_finite())
            {
                out.push((col, v, parse_cached_bound(val)));
            }
        }
        Some((&"*", rest)) => {
            if let Some(map) = val.as_object() {
                for child in map.values() {
                    collect_link_values_with_bounds(child, rest, col, out);
                }
            }
        }
        Some((segment, rest)) => {
            if let Some(child) = val.get(*segment) {
                collect_link_values_with_bounds(child, rest, col, out);
            }
        }
    }
}

fn extract_column_only_idx(entry: &serde_json::Value) -> Option<usize> {
    let id = entry.get("id")?.as_object()?;
    if id.contains_key("row") {
        return None;
    }
    id.get("col").and_then(|c| c.get("idx")).and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok())
}

fn extract_bounded_number(val: &serde_json::Value) -> Option<(f64, CachedBound)> {
    if let Some(n) = val.as_f64().filter(|v| v.is_finite()) {
        return Some((n, CachedBound::Equal));
    }
    let n = val.get("value").and_then(serde_json::Value::as_f64).filter(|v| v.is_finite())?;
    Some((n, parse_cached_bound(val)))
}

fn collect_pairwise_refs_for_link(link: &CachedPairwiseLink, results_val: &serde_json::Value, out: &mut Vec<CachedPairwiseRef>) {
    let mut cur = results_val.get("content").unwrap_or(results_val);
    for seg in link.tests_path.split('.').filter(|s| !s.is_empty()) {
        let Some(next) = cur.get(seg) else {
            return;
        };
        cur = next;
    }
    let Some(tests_map) = cur.as_object() else {
        return;
    };
    let mut emitted_for_link = 0usize;
    let mut seen_pairs: HashSet<(usize, usize)> = HashSet::new();
    for outer in tests_map.values() {
        let Some(col_a) = extract_column_only_idx(outer) else {
            continue;
        };
        let Some(inner_map) = outer.get("value").and_then(serde_json::Value::as_object) else {
            continue;
        };
        for inner in inner_map.values() {
            if emitted_for_link >= MAX_VALUES_PER_LINK {
                return;
            }
            let Some(col_b) = extract_column_only_idx(inner) else {
                continue;
            };
            if col_a == col_b {
                continue;
            }
            let Some(val_obj) = inner.get("value").and_then(serde_json::Value::as_object) else {
                continue;
            };
            if let Some(status) = val_obj.get("status").and_then(serde_json::Value::as_str)
                && !status.eq_ignore_ascii_case("VALID")
            {
                continue;
            }
            let p_or_q = val_obj
                .get("pValueAdjusted")
                .and_then(extract_bounded_number)
                .map(|(v, b)| (v, b, false))
                .or_else(|| val_obj.get("qValue").and_then(extract_bounded_number).map(|(v, b)| (v, b, true)))
                .or_else(|| val_obj.get("pValueIndividual").and_then(extract_bounded_number).map(|(v, b)| (v, b, false)));
            let means_diff = link
                .diff_field
                .and_then(|field| val_obj.get(field))
                .and_then(|v| v.as_f64().or_else(|| v.get("value").and_then(serde_json::Value::as_f64)))
                .filter(|v| v.is_finite());
            if p_or_q.is_none() && means_diff.is_none() {
                continue;
            }
            let pair_key = (col_a.min(col_b), col_a.max(col_b));
            if !seen_pairs.insert(pair_key) {
                continue;
            }
            out.push(CachedPairwiseRef { col_a, col_b, p_or_q, means_diff, p_abs_tol: link.p_abs_tol });
            emitted_for_link += 1;
        }
    }
}

/// Extract all crosswalked scalar and pairwise references from a `.prism` `results.json` document
/// in one bounded pass.
#[must_use]
pub fn extract_cached_analysis_reference(foreign_class: &str, results_val: &serde_json::Value) -> CachedAnalysisReference {
    let class = foreign_class.trim();
    let mut reference = CachedAnalysisReference::default();
    for scalar_link in CACHED_SCALAR_LINKS.iter().filter(|l| l.foreign_class.eq_ignore_ascii_case(class)) {
        for (data_set_index, value, bound) in cached_values_with_bounds_for_link(scalar_link, results_val) {
            reference.scalars.push(CachedScalarRef {
                key: scalar_link.curvebook_key.to_owned(),
                data_set_index,
                value,
                bound,
                comparison: scalar_link.comparison,
            });
        }
    }
    for pw_link in CACHED_PAIRWISE_LINKS.iter().filter(|l| l.foreign_class.eq_ignore_ascii_case(class)) {
        collect_pairwise_refs_for_link(pw_link, results_val, &mut reference.pairwise);
    }
    reference
}

fn scalar_display_meta(curvebook_key: &str, data_set_index: Option<usize>) -> (String, String) {
    let (sym, base) = match curvebook_key {
        "PValue" | "P value" => ("p", "P value"),
        "Row Factor: P" => ("p", "Row Factor P value"),
        "Column Factor: P" => ("p", "Column Factor P value"),
        "Row Factor × Column Factor: P" => ("p", "Row Factor × Column Factor P value"),
        "Log-rank P value" => ("p", "Log-rank P value"),
        "Gehan-Breslow-Wilcoxon P" => ("p", "Gehan-Breslow-Wilcoxon P value"),
        "t" => ("t", "t statistic"),
        "F" => ("F", "F statistic"),
        "Row Factor: F" => ("F", "Row Factor F statistic"),
        "Column Factor: F" => ("F", "Column Factor F statistic"),
        "Row Factor × Column Factor: F" => ("F", "Row Factor × Column Factor F statistic"),
        "R squared" => ("R²", "R squared"),
        "MS residual" => ("MS", "MS residual"),
        "N" => ("n", "N"),
        "Mean" => ("mean", "Mean"),
        "SD" => ("SD", "SD"),
        "SEM" => ("SEM", "SEM"),
        "Median" => ("median", "Median"),
        "Log-rank chi-square" => ("χ²", "Log-rank chi-square"),
        "Gehan-Breslow-Wilcoxon chi-square" => ("χ²", "Gehan-Breslow-Wilcoxon chi-square"),
        "TotalArea" => ("AUC", "Total area"),
        "OutlierCount" => ("outliers", "Outlier count"),
        other => (other, other),
    };
    let label = match data_set_index {
        Some(idx) => format!("{base} (column {})", idx + 1),
        None => base.to_owned(),
    };
    (sym.to_owned(), label)
}

fn compare_scalar_ref(recomputed: &ResultSheet, scalar_ref: &CachedScalarRef, abs_t: f64, rel_t: f64, diag: &mut CachedResultDiagnostic) {
    let Some(rec) = recomputed
        .scalars
        .iter()
        .find(|e| e.key.eq_ignore_ascii_case(&scalar_ref.key) && e.data_set_index == scalar_ref.data_set_index && e.value.is_finite())
    else {
        return;
    };
    let (a, b) = match scalar_ref.comparison {
        CrosswalkComparison::Signed => (rec.value, scalar_ref.value),
        CrosswalkComparison::Magnitude => (rec.value.abs(), scalar_ref.value.abs()),
    };
    diag.compared_count += 1;
    *diag.compared_by_key.entry(scalar_ref.key.clone()).or_insert(0) += 1;
    if !scalar_ref.bound.agrees(a, b, abs_t, rel_t) {
        let abs_diff = (a - b).abs();
        let scale = a.abs().max(b.abs());
        let rel_diff = if scale > 0.0 { abs_diff / scale } else { 0.0 };
        let (symbol, quantity_label) = scalar_display_meta(&scalar_ref.key, scalar_ref.data_set_index);
        diag.disagreements.push(CachedScalarDiff {
            key: scalar_ref.key.clone(),
            symbol,
            quantity_label,
            data_set_index: scalar_ref.data_set_index,
            recomputed: rec.value,
            cached: scalar_ref.value,
            bound: scalar_ref.bound,
            abs_diff,
            rel_diff,
        });
    }
}

fn pairwise_display_label(cmp: &PairwiseComparisonRecord) -> String {
    let a = if cmp.label_a.trim().is_empty() { default_column_label(cmp.group_a) } else { cmp.label_a.clone() };
    let b = if cmp.label_b.trim().is_empty() { default_column_label(cmp.group_b) } else { cmp.label_b.clone() };
    format!("{a} vs {b}")
}

fn compare_pairwise_ref(recomputed: &ResultSheet, pw: &CachedPairwiseRef, abs_t: f64, rel_t: f64, diag: &mut CachedResultDiagnostic) {
    let target_pair = (pw.col_a.min(pw.col_b), pw.col_a.max(pw.col_b));
    let matched = match pw.p_or_q {
        Some((_, _, is_fdr_q)) => recomputed
            .pairwise_comparisons
            .iter()
            .find(|cmp| (cmp.group_a.min(cmp.group_b), cmp.group_a.max(cmp.group_b)) == target_pair && cmp.is_fdr_q == is_fdr_q),
        None => recomputed.pairwise_comparisons.iter().find(|cmp| (cmp.group_a.min(cmp.group_b), cmp.group_a.max(cmp.group_b)) == target_pair),
    };
    let Some(cmp) = matched else {
        return;
    };
    let pair_label = pairwise_display_label(cmp);

    if let Some((cached_p, bound, is_fdr_q)) = pw.p_or_q
        && cmp.raw_p_or_q.is_finite()
    {
        let (key, symbol, qty_suffix) =
            if is_fdr_q { ("Pairwise FDR q", "q", "FDR q-value") } else { ("Pairwise adjusted P", "p", "adjusted p-value") };
        let p_abs = pw.p_abs_tol.filter(|t| t.is_finite() && *t >= 0.0).map_or(abs_t, |override_tol| abs_t.max(override_tol));
        diag.compared_count += 1;
        diag.pairwise_compared_count += 1;
        *diag.compared_by_key.entry(key.to_owned()).or_insert(0) += 1;
        if !bound.agrees(cmp.raw_p_or_q, cached_p, p_abs, rel_t) {
            let abs_diff = (cmp.raw_p_or_q - cached_p).abs();
            let scale = cmp.raw_p_or_q.abs().max(cached_p.abs());
            let rel_diff = if scale > 0.0 { abs_diff / scale } else { 0.0 };
            diag.pairwise_disagreements += 1;
            diag.disagreements.push(CachedScalarDiff {
                key: key.to_owned(),
                symbol: symbol.to_owned(),
                quantity_label: format!("{pair_label} {qty_suffix}"),
                data_set_index: None,
                recomputed: cmp.raw_p_or_q,
                cached: cached_p,
                bound,
                abs_diff,
                rel_diff,
            });
        }
    }

    if let Some(cached_diff) = pw.means_diff
        && let Some(rec_diff) = cmp.estimate_diff.filter(|v| v.is_finite())
    {
        let key = "Pairwise mean difference";
        let a = rec_diff.abs();
        let b = cached_diff.abs();
        diag.compared_count += 1;
        diag.pairwise_compared_count += 1;
        *diag.compared_by_key.entry(key.to_owned()).or_insert(0) += 1;
        if !CachedBound::Equal.agrees(a, b, abs_t, rel_t) {
            let abs_diff = (a - b).abs();
            let scale = a.max(b);
            let rel_diff = if scale > 0.0 { abs_diff / scale } else { 0.0 };
            diag.pairwise_disagreements += 1;
            diag.disagreements.push(CachedScalarDiff {
                key: key.to_owned(),
                symbol: String::from("|mean diff|"),
                quantity_label: format!("{pair_label} mean difference"),
                data_set_index: None,
                recomputed: a,
                cached: b,
                bound: CachedBound::Equal,
                abs_diff,
                rel_diff,
            });
        }
    }
}

/// Compare a recomputed [`ResultSheet`] against a pre-extracted [`CachedAnalysisReference`]
/// (both scalars and pairwise post-test comparisons) within
/// `max(abs_tol, rel_tol * max(|recomputed|, |cached|))`, respecting [`CachedBound`] inequalities.
///
/// Unmatched cached entries are skipped rather than counted as agreement. CurveBook never adjusts
/// its own outputs to match cached values.
#[must_use]
pub fn compare_recomputed_against_reference(
    recomputed: &ResultSheet,
    reference: &CachedAnalysisReference,
    abs_tol: f64,
    rel_tol: f64,
) -> CachedResultDiagnostic {
    let mut diag = CachedResultDiagnostic::default();
    let abs_t = if abs_tol.is_finite() && abs_tol >= 0.0 { abs_tol } else { 0.0 };
    let rel_t = if rel_tol.is_finite() && rel_tol >= 0.0 { rel_tol } else { 0.0 };
    for scalar_ref in &reference.scalars {
        compare_scalar_ref(recomputed, scalar_ref, abs_t, rel_t, &mut diag);
    }
    for pw in &reference.pairwise {
        compare_pairwise_ref(recomputed, pw, abs_t, rel_t, &mut diag);
    }
    diag
}

/// Compare a recomputed [`ResultSheet`] against a cached `results.json` through
/// [`CACHED_SCALAR_LINKS`] and [`CACHED_PAIRWISE_LINKS`].
#[must_use]
pub fn compare_recomputed_against_cached_results(
    recomputed: &ResultSheet,
    foreign_class: &str,
    results_val: &serde_json::Value,
    abs_tol: f64,
    rel_tol: f64,
) -> CachedResultDiagnostic {
    let reference = extract_cached_analysis_reference(foreign_class, results_val);
    compare_recomputed_against_reference(recomputed, &reference, abs_tol, rel_tol)
}

/// Translate an external `.prism` `parameters.json` document into a CurveBook [`AnalysisKind`]
/// and any blocking option notes. Returns `None` when `foreign_class` is unmapped.
#[must_use]
pub fn translate_analysis_parameters(foreign_class: &str, params_json: &serde_json::Value) -> Option<TranslatedAnalysis> {
    let row = find_analysis_mapping(foreign_class)?;
    let content = params_json.get("content").and_then(serde_json::Value::as_object).or_else(|| params_json.as_object());
    let design_key = extract_analysis_design_key(row.foreign_class, params_json);
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
    let verification = if blocking.is_empty() && find_verified_analysis_design(&design_key).is_some() {
        AnalysisVerificationStatus::CorpusVerified
    } else {
        AnalysisVerificationStatus::Unverified
    };
    Some(TranslatedAnalysis { kind, verification, blocking_options: blocking })
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

struct ParsedPostTests {
    multiplicity: MultiplicitySpec,
    control_factor: Option<usize>,
    comparison_family_mode: FactorialComparisonFamilyMode,
}

impl Default for ParsedPostTests {
    fn default() -> Self {
        Self { multiplicity: MultiplicitySpec::default(), control_factor: None, comparison_family_mode: FactorialComparisonFamilyMode::Standard }
    }
}

fn map_two_way_means_design_type(
    pairwise: Option<&JsonMap>,
    family_label: &str,
    blocking: &mut Vec<String>,
) -> Option<(usize, FactorialComparisonFamilyMode, &'static str)> {
    let Some(means_design) = get_str(pairwise, "meansDesignType") else {
        blocking.push(format!("two-way {family_label} comparison is missing `meansDesignType`"));
        return None;
    };
    match means_design {
        "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SEPARATE_FAMILIES" => {
            Some((1_usize, FactorialComparisonFamilyMode::PerSliceFamilies, "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SEPARATE_FAMILIES"))
        }
        "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SEPARATE_FAMILIES" => {
            Some((0_usize, FactorialComparisonFamilyMode::PerSliceFamilies, "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SEPARATE_FAMILIES"))
        }
        "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SINGLE_FAMILY" => {
            Some((1_usize, FactorialComparisonFamilyMode::TwoFactorFamilies, "COMPARE_CELLS_BY_COLUMNS_WITHIN_ROWS_SINGLE_FAMILY"))
        }
        "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SINGLE_FAMILY" => {
            Some((0_usize, FactorialComparisonFamilyMode::TwoFactorFamilies, "COMPARE_CELLS_BY_ROWS_WITHIN_COLUMNS_SINGLE_FAMILY"))
        }
        "COMPARE_COLUMNS" => Some((1_usize, FactorialComparisonFamilyMode::Standard, "COMPARE_COLUMNS")),
        "COMPARE_ROWS" => Some((0_usize, FactorialComparisonFamilyMode::Standard, "COMPARE_ROWS")),
        "COMPARE_CELLS" => {
            blocking.push(format!("two-way {family_label} `COMPARE_CELLS` requires two-dimensional cell coordinates"));
            None
        }
        other => {
            blocking.push(format!("unmapped two-way {family_label} meansDesignType `{other}`"));
            None
        }
    }
}

fn parse_pair_member_indices(member: &serde_json::Value) -> (Option<usize>, Option<usize>) {
    let obj = member.as_object();
    let col_idx = get_obj(obj, "col").and_then(|c| c.get("idx")).and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let row_idx = get_obj(obj, "row").and_then(|r| r.get("idx")).and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok());
    (row_idx, col_idx)
}

fn parse_control_vs_all_target(
    pairwise: Option<&JsonMap>,
    design_obj: Option<&JsonMap>,
    is_two_way: bool,
    blocking: &mut Vec<String>,
) -> Option<(usize, Option<usize>, FactorialComparisonFamilyMode)> {
    let control_id = get_obj(design_obj, "controlID");
    let col_idx = get_obj(control_id, "col").and_then(|c| c.get("idx")).and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let row_idx = get_obj(control_id, "row").and_then(|r| r.get("idx")).and_then(serde_json::Value::as_u64).and_then(|i| usize::try_from(i).ok());

    if !is_two_way {
        if row_idx.is_some() {
            blocking.push(String::from("one-way control-versus-all controlID specifies a row index"));
            return None;
        }
        return match col_idx {
            Some(c) => Some((c, None, FactorialComparisonFamilyMode::Standard)),
            None => {
                blocking.push(String::from("control-versus-all control group is missing"));
                None
            }
        };
    }

    let (target_factor, family_mode, means_design) = map_two_way_means_design_type(pairwise, "control-versus-all", blocking)?;
    match (target_factor, row_idx, col_idx) {
        (0, Some(r), None) => Some((r, Some(0), family_mode)),
        (1, None, Some(c)) => Some((c, Some(1), family_mode)),
        _ => {
            blocking.push(format!("two-way control-versus-all controlID does not match `{means_design}`"));
            None
        }
    }
}

struct ParsedSelectedPairs {
    pairs: Vec<(usize, usize)>,
    control_factor: Option<usize>,
    family_mode: FactorialComparisonFamilyMode,
}

fn parse_selected_pairs_target(
    pairwise: Option<&JsonMap>,
    design_obj: Option<&JsonMap>,
    is_two_way: bool,
    blocking: &mut Vec<String>,
) -> Option<ParsedSelectedPairs> {
    let (target_factor, family_mode, means_design) = if is_two_way {
        let (tf, fm, md) = map_two_way_means_design_type(pairwise, "selected-pairs", blocking)?;
        (Some(tf), fm, Some(md))
    } else {
        (None, FactorialComparisonFamilyMode::Standard, None)
    };

    let Some(raw_list) = design_obj.and_then(|d| d.get("selectedPairIDList")).and_then(serde_json::Value::as_array) else {
        blocking.push(String::from("selected-pairs comparison is missing `selectedPairIDList`"));
        return None;
    };
    if raw_list.is_empty() {
        blocking.push(String::from("selected-pairs comparison has empty `selectedPairIDList`"));
        return None;
    }

    let mut pairs = Vec::new();
    let mut seen = HashSet::new();
    let mut null_slots = 0_usize;

    for entry in raw_list {
        let Some(pair_arr) = entry.as_array() else {
            blocking.push(String::from("selected-pairs entry is not a two-element pair"));
            return None;
        };
        let [first, second] = pair_arr.as_slice() else {
            blocking.push(String::from("selected-pairs entry does not contain two members"));
            return None;
        };
        if first.is_null() && second.is_null() {
            null_slots += 1;
            continue;
        }
        if first.is_null() || second.is_null() {
            blocking.push(String::from("selected-pairs entry has an incomplete pair"));
            return None;
        }
        let (r0, c0) = parse_pair_member_indices(first);
        let (r1, c1) = parse_pair_member_indices(second);
        let (idx_a, idx_b) = match target_factor {
            None => match (r0, c0, r1, c1) {
                (None, Some(a), None, Some(b)) => (a, b),
                _ => {
                    blocking.push(String::from("one-way selected-pairs member is missing `col.idx` or specifies a row index"));
                    return None;
                }
            },
            Some(0) => match (r0, c0, r1, c1) {
                (Some(a), None, Some(b), None) => (a, b),
                _ => {
                    let md = means_design.unwrap_or("COMPARE_ROWS");
                    blocking.push(format!("two-way selected-pairs member does not match `{md}`"));
                    return None;
                }
            },
            Some(1) => match (r0, c0, r1, c1) {
                (None, Some(a), None, Some(b)) => (a, b),
                _ => {
                    let md = means_design.unwrap_or("COMPARE_COLUMNS");
                    blocking.push(format!("two-way selected-pairs member does not match `{md}`"));
                    return None;
                }
            },
            Some(_) => {
                blocking.push(String::from("unmapped two-way selected-pairs factor axis"));
                return None;
            }
        };
        if idx_a == idx_b {
            blocking.push(format!("selected-pairs list contains self-pair ({idx_a}, {idx_b})"));
            return None;
        }
        let canon = (idx_a.min(idx_b), idx_a.max(idx_b));
        if !seen.insert(canon) {
            blocking.push(format!("selected-pairs list contains duplicate pair ({idx_a}, {idx_b})"));
            return None;
        }
        pairs.push((idx_a, idx_b));
    }

    if pairs.is_empty() {
        if null_slots > 0 {
            blocking.push(String::from("selected-pairs list contains only null placeholders (targets omitted)"));
        } else {
            blocking.push(String::from("selected-pairs comparison has empty `selectedPairIDList`"));
        }
        return None;
    }

    Some(ParsedSelectedPairs { pairs, control_factor: target_factor, family_mode })
}

/// Translate `postTests` into a [`MultiplicitySpec`] plus optional two-way control-factor and
/// comparison-family routing.
///
/// `multipleComparisons.type` selects the branch (`STATISTICAL` → `statistical.test`, `FDR` →
/// `fdr.test`, `NO_CORRECTION` → unadjusted), and `pairwiseTests.pairwiseDesign.type` selects the
/// comparison family. `BONFERRONI_DUNN` denotes single-step Bonferroni (Dunn 1961) and
/// `BONFERRONI_SIDAK` denotes single-step Šidák (1967), with no distinguishing sub-field in
/// `parameters.json` (see `docs/evidence/interop-format-notes.md`). `ALL_VS_ALL`,
/// `CONTROL_VS_ALL`, and one-way / 1D two-way `SELECTED_VS_SELECTED` families translate; column
/// targets keep their sheet data-set indices and must lie in the analysed selection
/// ([`check_control_in_selection`]). Trailing `[null, null]` placeholder slots in
/// `selectedPairIDList` are skipped when at least one valid pair is present; empty lists,
/// all-null placeholder lists, incomplete pairs, `STATISTICAL` with `NO_TEST`, unknown test names,
/// and 2D `COMPARE_CELLS` targets stay explicitly blocked.
fn parse_multiplicity(post_tests: Option<&JsonMap>, is_two_way: bool, blocking: &mut Vec<String>) -> ParsedPostTests {
    let mut parsed = ParsedPostTests::default();
    let Some(pt) = post_tests else {
        return parsed;
    };
    if matches!(get_str(Some(pt), "type"), None | Some("NO_TESTS")) {
        return parsed;
    }
    let pairwise = get_obj(Some(pt), "pairwiseTests");
    let mc = get_obj(pairwise, "multipleComparisons");
    let mc_type = get_str(mc, "type");
    let design_obj = get_obj(pairwise, "pairwiseDesign");
    let family = get_str(design_obj, "type").unwrap_or("ALL_VS_ALL");

    let raw_test = match mc_type {
        Some("NO_CORRECTION") => {
            if family == "CONTROL_VS_ALL" {
                if let Some((ctrl, control_factor, family_mode)) = parse_control_vs_all_target(pairwise, design_obj, is_two_way, blocking) {
                    parsed.multiplicity.method = MultiplicityMethod::FisherLsd;
                    parsed.multiplicity.control_column = Some(ctrl);
                    parsed.control_factor = control_factor;
                    parsed.comparison_family_mode = family_mode;
                }
            } else if family == "SELECTED_VS_SELECTED" {
                if let Some(sel) = parse_selected_pairs_target(pairwise, design_obj, is_two_way, blocking) {
                    parsed.multiplicity.method = MultiplicityMethod::FisherLsd;
                    parsed.multiplicity.selected_pairs = Some(sel.pairs);
                    parsed.control_factor = sel.control_factor;
                    parsed.comparison_family_mode = sel.family_mode;
                }
            } else if family != "ALL_VS_ALL" {
                blocking.push(format!("unadjusted pairwise comparison family `{family}`"));
            }
            return parsed;
        }
        None => {
            if family != "ALL_VS_ALL" {
                blocking.push(format!("unadjusted pairwise comparison family `{family}`"));
            }
            return parsed;
        }
        Some("STATISTICAL") => get_str(get_obj(mc, "statistical"), "test").or_else(|| get_str(mc, "statistical")),
        Some("FDR") => get_str(get_obj(mc, "fdr"), "test").or_else(|| get_str(mc, "fdr")),
        Some(other) => {
            blocking.push(format!("unmapped multipleComparisons type `{other}`"));
            return parsed;
        }
    };

    let raw_upper = raw_test.map(str::to_ascii_uppercase);
    let Some(test) = raw_upper.as_deref().filter(|t| *t != "NO_TEST" && *t != "NONE" && *t != "NO_CORRECTION") else {
        if family == "CONTROL_VS_ALL" || family == "SELECTED_VS_SELECTED" {
            let token = raw_upper.as_deref().unwrap_or("NONE");
            blocking.push(format!("unspecified statistical test `{token}` for pairwise comparison family `{family}`"));
        } else if family != "ALL_VS_ALL" {
            blocking.push(format!("unadjusted pairwise comparison family `{family}`"));
        }
        return parsed;
    };

    match family {
        "CONTROL_VS_ALL" => {
            let mapped = match test {
                "DUNNET" | "DUNNETT" => Some(None),
                "HOLM_SIDAK" => Some(Some(MultiplicityMethod::HolmSidak)),
                "BONFERRONI_DUNN" | "BONFERRONI" => Some(Some(MultiplicityMethod::Bonferroni)),
                "BONFERRONI_SIDAK" | "SIDAK" => Some(Some(MultiplicityMethod::Sidak)),
                "DUNN" if !is_two_way => Some(Some(MultiplicityMethod::DunnRankSum)),
                "DUNNET_T3" | "DUNNETT_T3" => Some(Some(MultiplicityMethod::DunnettT3)),
                "TAMHANE_T2" => Some(Some(MultiplicityMethod::TamhaneT2)),
                "FISHER_LSD" | "UNCORRECTED_FISHER_LSD" => Some(Some(MultiplicityMethod::FisherLsd)),
                "BENJAMINI_HOCHBERG" => Some(Some(MultiplicityMethod::BenjaminiHochbergFdr)),
                "BENJAMINI_YEKUTIELI" => Some(Some(MultiplicityMethod::BenjaminiYekutieliFdr)),
                "TWO_STAGE_BKY" | "BENJAMINI_KRIEGER_YEKUTIELI" => Some(Some(MultiplicityMethod::TwoStageBkyFdr)),
                _ => None,
            };
            let Some(method_opt) = mapped else {
                blocking.push(format!("unmapped pairwise comparison family `{family}` with `{test}`"));
                return parsed;
            };
            if let Some((ctrl, control_factor, family_mode)) = parse_control_vs_all_target(pairwise, design_obj, is_two_way, blocking) {
                match method_opt {
                    None => {
                        parsed.multiplicity.method = MultiplicityMethod::Dunnett { control_column: ctrl };
                        parsed.multiplicity.control_column = None;
                    }
                    Some(method) => {
                        parsed.multiplicity.method = method;
                        parsed.multiplicity.control_column = Some(ctrl);
                    }
                }
                parsed.control_factor = control_factor;
                parsed.comparison_family_mode = family_mode;
            }
        }
        "SELECTED_VS_SELECTED" => {
            let mapped = match test {
                "HOLM_SIDAK" => Some(MultiplicityMethod::HolmSidak),
                "BONFERRONI_DUNN" | "BONFERRONI" => Some(MultiplicityMethod::Bonferroni),
                "BONFERRONI_SIDAK" | "SIDAK" => Some(MultiplicityMethod::Sidak),
                "DUNN" if !is_two_way => Some(MultiplicityMethod::DunnRankSum),
                "DUNNET_T3" | "DUNNETT_T3" => Some(MultiplicityMethod::DunnettT3),
                "TAMHANE_T2" => Some(MultiplicityMethod::TamhaneT2),
                "FISHER_LSD" | "UNCORRECTED_FISHER_LSD" => Some(MultiplicityMethod::FisherLsd),
                "BENJAMINI_HOCHBERG" => Some(MultiplicityMethod::BenjaminiHochbergFdr),
                "BENJAMINI_YEKUTIELI" => Some(MultiplicityMethod::BenjaminiYekutieliFdr),
                "TWO_STAGE_BKY" | "BENJAMINI_KRIEGER_YEKUTIELI" => Some(MultiplicityMethod::TwoStageBkyFdr),
                _ => None,
            };
            let Some(method) = mapped else {
                blocking.push(format!("unmapped pairwise comparison family `{family}` with `{test}`"));
                return parsed;
            };
            if let Some(sel) = parse_selected_pairs_target(pairwise, design_obj, is_two_way, blocking) {
                parsed.multiplicity.method = method;
                parsed.multiplicity.selected_pairs = Some(sel.pairs);
                parsed.control_factor = sel.control_factor;
                parsed.comparison_family_mode = sel.family_mode;
            }
        }
        "ALL_VS_ALL" => {
            parsed.multiplicity.method = match test {
                "TUKEY" | "TUKEY_HSD" => MultiplicityMethod::TukeyHsd,
                "HOLM_SIDAK" => MultiplicityMethod::HolmSidak,
                "BONFERRONI_DUNN" | "BONFERRONI" => MultiplicityMethod::Bonferroni,
                "BONFERRONI_SIDAK" | "SIDAK" => MultiplicityMethod::Sidak,
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
            };
        }
        other => {
            blocking.push(format!("unmapped pairwise comparison family `{other}` with `{test}`"));
        }
    }
    parsed
}

/// Check that control-versus-all and selected-pairs column indices (sheet data-set indices on
/// one-way ANOVA and column-factor two-way ANOVA) lie within the analysed data sets. An empty
/// selection analyses every data set.
pub fn check_control_in_selection(kind: &AnalysisKind, selected: &[usize]) -> Result<(), String> {
    if selected.is_empty() {
        return Ok(());
    }
    let mult = match kind {
        AnalysisKind::OneWayAnova(spec) => Some(&spec.multiplicity),
        AnalysisKind::FactorialAnova(spec) if spec.control_factor == Some(1) => Some(&spec.multiplicity),
        _ => None,
    };
    let Some(mult) = mult else {
        return Ok(());
    };
    if let Some(control_column) = mult.effective_control_column()
        && !selected.contains(&control_column)
    {
        return Err(String::from("control group is not among the analysed data sets"));
    }
    if let Some(pairs) = mult.selected_pairs.as_deref()
        && pairs.iter().any(|(a, b)| !selected.contains(a) || !selected.contains(b))
    {
        return Err(String::from("selected comparison pair references a data set outside the analysed selection"));
    }
    Ok(())
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
    let parsed = parse_multiplicity(get_obj(content, "postTests"), false, blocking);
    AnalysisKind::OneWayAnova(OneWayAnovaSpec { method, compute_effect_sizes: true, multiplicity: parsed.multiplicity, ..OneWayAnovaSpec::default() })
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
    if get_bool(get_obj(content, "options"), "includeInteractionTerm") == Some(false) {
        blocking.push(String::from("additive two-way ANOVA without interaction term is not yet translated"));
    }
    let parsed = parse_multiplicity(get_obj(content, "postTests"), true, blocking);
    AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        repeated_measures,
        mixed_effects_reml,
        compute_effect_sizes: true,
        multiplicity: parsed.multiplicity,
        control_factor: parsed.control_factor,
        comparison_family_mode: parsed.comparison_family_mode,
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
    match get_str(get_obj(content, "data"), "handleReplicatesAs") {
        None | Some("AVERAGE") | Some("COLUMN") => {}
        Some("SUB_COLUMN") => {
            blocking.push(String::from("per-subcolumn replicate handling `SUB_COLUMN` is not yet corpus-verified"));
        }
        Some(other) => {
            blocking.push(format!("unmapped handleReplicatesAs `{other}`"));
        }
    }
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

fn source_sheet_max_y_subcolumns(source_ds: &DataSheet) -> usize {
    match &source_ds.archetype {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.subcolumn_titles.len()).max().unwrap_or(1),
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => 1,
    }
}

/// Validate analysis options whose translation depends on the resolved source [`DataSheet`] layout
/// (for example, replicate aggregation modes on multi-subcolumn tables).
pub fn check_source_sheet_option_compatibility(foreign_class: &str, params_json: &serde_json::Value, source_ds: &DataSheet) -> Result<(), String> {
    if foreign_class.trim().eq_ignore_ascii_case("IDENTIFY_OUTLIERS") {
        let content = params_json.get("content").and_then(serde_json::Value::as_object).or_else(|| params_json.as_object());
        let handle_reps = get_str(get_obj(content, "data"), "handleReplicatesAs");
        if matches!(handle_reps, None | Some("AVERAGE")) && source_sheet_max_y_subcolumns(source_ds) > 1 {
            return Err(String::from("outlier detection with row-averaged replicates on a multi-subcolumn table is not yet translated"));
        }
    }
    Ok(())
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
    pub promoted_analyses: &'a mut Vec<PromotedAnalysisRef>,
}

struct LivePromotionRequest<'a> {
    sid: &'a SheetId,
    src_id: &'a SheetId,
    analysis_uid: &'a str,
    analysis_class: &'a str,
    clamped_title: &'a str,
    kind: &'a AnalysisKind,
    selected_data_sets: Vec<usize>,
    sheet_obj: &'a JsonMap,
}

fn build_promoted_live_analysis(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    req: LivePromotionRequest<'_>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(ResultSheet, PromotedAnalysisRef), FormatError> {
    let mut spec = AnalysisSpec::new(req.sid.clone(), req.src_id.clone(), req.kind.clone())?;
    spec.selected_data_sets = req.selected_data_sets;
    spec.set_custom_title(Some(req.clamped_title.to_owned()))?;
    let rs = ResultSheet::new(spec)?;

    let results_val = load_analysis_json(zip_index, id_index, req.analysis_uid, "results.json", warnings)?;
    let reference = results_val.as_ref().map(|rv| extract_cached_analysis_reference(req.analysis_class, rv)).unwrap_or_default();
    let fallback_tabs = import_result_tabs_3hop(zip_index, id_index, req.analysis_uid, req.sheet_obj, warnings)?;
    let fallback_scalars = results_val.as_ref().map(extract_cached_scalars).unwrap_or_default();
    let (abs_tol, rel_tol) =
        find_analysis_mapping(req.analysis_class).map(|row| (row.absolute_tolerance, row.relative_tolerance)).unwrap_or((1e-9, 1e-6));
    let original_kind_label =
        clamp_title(if req.analysis_class.is_empty() { "External analysis" } else { req.analysis_class }, req.sid.as_str(), warnings);

    let promoted = PromotedAnalysisRef {
        sheet_id: rs.id.clone(),
        analysis_uid: req.analysis_uid.to_owned(),
        title: req.clamped_title.to_owned(),
        foreign_class: req.analysis_class.to_owned(),
        abs_tol,
        rel_tol,
        reference,
        fallback: ImportedAnalysisFallback { original_kind_label, tabs: fallback_tabs, scalars: fallback_scalars },
    };
    Ok((rs, promoted))
}

/// Import all listed `.prism` analyses: corpus-verified designs without blocking options become
/// live `ResultSheet`s (`ResultProvenance::Computed`) with transient [`PromotedAnalysisRef`]
/// entries; all other analyses import via the 3-hop `preferredResultSheet` chain as read-only
/// `ResultProvenance::Imported` snapshots.
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
        if let Some(tr) = translation.as_mut()
            && let Some(ref params) = params_val
            && let Some(ref src_id) = source_sheet_id
            && let Some(src_ds) = doc.data_sheet(src_id)
            && let Err(reason) = check_source_sheet_option_compatibility(analysis_class, params, src_ds)
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
            let (rs, promoted) = build_promoted_live_analysis(
                zip_index,
                ctx.id_index,
                LivePromotionRequest {
                    sid: &sid,
                    src_id,
                    analysis_uid,
                    analysis_class,
                    clamped_title: &clamped_title,
                    kind: &tr.kind,
                    selected_data_sets,
                    sheet_obj,
                },
                ctx.warnings,
            )?;
            ctx.promoted_analyses.push(promoted);
            rs
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
        Some(tr) if tr.verification == AnalysisVerificationStatus::CorpusVerified => {
            format!("input data sheet for `{analysis_class}` is missing or incompatible for live recompute; preserved cached result")
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::zip::{ZipLimits, ZipReadOptions, ZipWriter};
    use curvebook_doc::{ResultProvenance, ResultStatus};
    use serde_json::json;

    #[test]
    fn build_promoted_live_analysis_captures_reference_and_3hop_fallback() {
        let mut writer = ZipWriter::new();
        writer
            .add_file(
                "analyses/an-1/results.json",
                serde_json::to_vec(&json!({
                    "content": {
                        "ordinaryAnovaModel": {
                            "anovaTable": {
                                "colFactor": { "fTest": { "f": 9.5, "pValue": { "state": "P_VALUE_LESS", "value": 1e-15 } } }
                            },
                            "multipleComparisons": {
                                "tests": {
                                    "uA": {
                                        "id": { "col": { "idx": 2 } },
                                        "value": {
                                            "uB": {
                                                "id": { "col": { "idx": 0 } },
                                                "value": {
                                                    "status": "VALID",
                                                    "meansDifference": -6.25,
                                                    "pValueAdjusted": { "state": "P_VALUE_EQUAL", "value": 0.004 }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        writer
            .add_file(
                "analyses/an-1/result_sheets/rs-main.json",
                serde_json::to_vec(&json!({"uid": "rs-main", "name": "ANOVA table", "dataSheet": "view-1"})).unwrap(),
            )
            .unwrap();
        writer
            .add_file(
                "data/sheets/view-1/sheet.json",
                serde_json::to_vec(&json!({
                    "uid": "view-1",
                    "flags": ["ANALYSIS_VIEW"],
                    "table": { "uid": "t-view", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-rt", "dataSets": ["ds-val"] }
                }))
                .unwrap(),
            )
            .unwrap();
        writer
            .add_file("data/sets/ds-rt.json", serde_json::to_vec(&json!({"title": "Row", "replicates": [{"valueType": "string"}]})).unwrap())
            .unwrap();
        writer
            .add_file("data/sets/ds-val.json", serde_json::to_vec(&json!({"title": "Value", "replicates": [{"valueType": "string"}]})).unwrap())
            .unwrap();
        writer.add_file("data/tables/t-view/data.csv", b"F,9.5\n".to_vec()).unwrap();
        let bytes = writer.finish().unwrap();

        let mut zip_index = ZipIndex::from_bytes_with_options(&bytes, ZipLimits::default(), ZipReadOptions::external_interop()).unwrap();
        let id_index = ArchiveIdIndex::from_zip_index(&zip_index);
        let sheet_val = json!({
            "uid": "an-1",
            "analysisClass": "ONE_WAY_ANOVA",
            "preferredResultSheet": "rs-main",
            "resultSheets": [{ "uid": "rs-main", "title": "ANOVA table" }]
        });
        let kind = AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
            ..OneWayAnovaSpec::default()
        });
        let mut warnings = Vec::new();
        let (rs, promoted) = build_promoted_live_analysis(
            &mut zip_index,
            &id_index,
            LivePromotionRequest {
                sid: &SheetId::new("res-an-1"),
                src_id: &SheetId::new("data-1"),
                analysis_uid: "an-1",
                analysis_class: "ONE_WAY_ANOVA",
                clamped_title: "One-way ANOVA",
                kind: &kind,
                selected_data_sets: vec![0, 2],
                sheet_obj: sheet_val.as_object().unwrap(),
            },
            &mut warnings,
        )
        .unwrap();

        assert!(warnings.is_empty());
        assert_eq!(rs.status, ResultStatus::Dirty);
        assert_eq!(rs.provenance, ResultProvenance::Computed);
        assert_eq!(rs.analysis.selected_data_sets, vec![0, 2]);
        assert_eq!(rs.analysis.custom_title.as_deref(), Some("One-way ANOVA"));
        assert_eq!(promoted.sheet_id, SheetId::new("res-an-1"));
        assert_eq!(promoted.analysis_uid, "an-1");
        assert_eq!(promoted.foreign_class, "ONE_WAY_ANOVA");
        assert!((promoted.abs_tol - 1e-9).abs() < 1e-15);
        assert!((promoted.rel_tol - 1e-6).abs() < 1e-15);
        assert_eq!(promoted.reference.scalars.len(), 2);
        assert!(promoted.reference.scalars.iter().any(|s| s.key == "P value" && s.bound == CachedBound::LessThan));
        assert_eq!(promoted.reference.pairwise.len(), 1);
        assert_eq!(promoted.reference.pairwise[0].col_a, 2);
        assert_eq!(promoted.reference.pairwise[0].col_b, 0);
        assert_eq!(promoted.reference.pairwise[0].p_or_q, Some((0.004, CachedBound::Equal, false)));
        assert_eq!(promoted.reference.pairwise[0].means_diff, Some(-6.25));
        assert_eq!(promoted.reference.pairwise[0].p_abs_tol, Some(1e-5));
        assert_eq!(promoted.fallback.original_kind_label, "ONE_WAY_ANOVA");
        assert_eq!(promoted.fallback.tabs.len(), 1);
        assert_eq!(promoted.fallback.tabs[0].title, "ANOVA table");
        assert!(!promoted.fallback.scalars.is_empty());
    }
}
