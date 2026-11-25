//! Clean-room ICMJE/ARRIVE statistical methods and results report generator (`QOL-6`).
//!
//! Produces deterministic [`ReportFormat::PlainText`], [`ReportFormat::Markdown`], and
//! [`ReportFormat::Latex`] narratives directly from stored [`AnalysisSpec`], [`DataSheet`],
//! and [`ResultSheet`] records without mutating any stored numbers.

use std::fmt::Write as _;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, BlandAltmanSpec, ChecklistItem, ChecklistStatus, CiBound, ClusterDistance, ClusterLinkage,
    ContingencyMethod, ContingencySpec, ConvergenceStrictness, CorrelationPowerMethod, CoxTieMethod, DataSheet, DescriptiveSpec,
    ErrorPropagationMode, FactorialAnovaSpec, FitMethod, GlmFamily, GlmSpec, HookSource, IdentifiabilityMode, KappaWeighting, LeveneCenter,
    LeveneTestSpec, LinearRegressionSpec, MathTransformPreset, MixedModelDfMethod, ModelComparisonMethod, ModelComparisonQuestion,
    ModelComparisonSpec, MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec,
    NormalizeAnchor, OneWayAnovaSpec, OneWayMethod, OutlierHandling, OutlierSpec, PValueFormat, PValueStackMethod, PValueStackSpec,
    PairedObservationAccounting, ParameterRule, ParameterScope, PassingBablokSpec, PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection,
    PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument, RepeatedMeasuresDesign, ReplicateFitMode, ResidualCovariance, ResultCell,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, RocCiMethod, RocSpec, SheetId, SimpleLogisticSpec, SimulationNoise, SplineLowessMethod,
    SplineLowessSpec, SubcolumnAggregation, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, TwoGroupMethod,
    TwoGroupSpec, TwoProportionsMethod, WeightingScheme, WeightingSource,
};
use curvebook_fit::{get_builtin_model, resolve_model_from_spec};

use crate::EngineError;

/// Serialization format for [`generate_methods_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportFormat {
    PlainText,
    Markdown,
    Latex,
}

impl ReportFormat {
    /// Parse a format string case-insensitively (`"plain"`, `"plaintext"`, `"text"`,
    /// `"markdown"`, `"md"`, `"latex"`, `"tex"`).
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "plain" | "plaintext" | "plain_text" | "plain-text" | "text" => Some(Self::PlainText),
            "markdown" | "md" => Some(Self::Markdown),
            "latex" | "tex" => Some(Self::Latex),
            _ => None,
        }
    }

    /// Canonical lowercase format identifier (`"plain"`, `"markdown"`, or `"latex"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plain",
            Self::Markdown => "markdown",
            Self::Latex => "latex",
        }
    }
}

/// Generate a deterministic, clean-room ICMJE/ARRIVE statistical methods and summary report for
/// `result_sheet_id` inside `doc` in the requested [`ReportFormat`].
///
/// # Errors
/// Returns [`EngineError`] if `result_sheet_id` is empty or does not match a [`ResultSheet`] in `doc`.
pub fn generate_methods_report(doc: &ProjectDocument, result_sheet_id: &SheetId, format: ReportFormat) -> Result<String, EngineError> {
    if result_sheet_id.as_str().trim().is_empty() {
        return Err(EngineError::new("result sheet identifier must not be empty"));
    }
    let res = doc.result_sheet(result_sheet_id).ok_or_else(|| EngineError::new(format!("result sheet `{result_sheet_id}` not found")))?;
    let source_ds = resolve_source_data_sheet(doc, &res.analysis.source_sheet_id);
    let blocks = build_report_blocks(doc, res, source_ds);
    Ok(render_blocks(&blocks, format))
}

fn resolve_source_data_sheet<'a>(doc: &'a ProjectDocument, source_id: &SheetId) -> Option<&'a DataSheet> {
    if let Some(ds) = doc.data_sheet(source_id) {
        return Some(ds);
    }
    doc.result_sheet(source_id).and_then(|r| r.analyzable_data.as_ref())
}

// ---------------------------------------------------------------------------
// Intermediate Report AST (single prose source across all 3 output formats)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TextRun {
    Plain(String),
    Bold(String),
    Code(String),
    Math { plain: String, markdown: String, latex: String },
}

#[derive(Debug, Clone)]
struct TableBlock {
    caption: Vec<TextRun>,
    headers: Vec<Vec<TextRun>>,
    rows: Vec<Vec<Vec<TextRun>>>,
    footnotes: Vec<Vec<TextRun>>,
}

#[derive(Debug, Clone)]
enum ReportBlock {
    Title(Vec<TextRun>),
    Section(Vec<TextRun>),
    Paragraph(Vec<TextRun>),
    Bullets(Vec<Vec<TextRun>>),
    Table(TableBlock),
}

fn plain(s: impl Into<String>) -> TextRun {
    TextRun::Plain(s.into())
}

fn bold(s: impl Into<String>) -> TextRun {
    TextRun::Bold(s.into())
}

fn code(s: impl Into<String>) -> TextRun {
    TextRun::Code(s.into())
}

fn math(plain_s: impl Into<String>, md_s: impl Into<String>, tex_s: impl Into<String>) -> TextRun {
    TextRun::Math { plain: plain_s.into(), markdown: md_s.into(), latex: tex_s.into() }
}

fn fwer_math(thresh: &str) -> TextRun {
    math(format!("familywise alpha = {thresh}"), format!("familywise alpha = {thresh}"), format!("\\textrm{{familywise }}\\alpha = {thresh}"))
}

fn fdr_math(thresh: &str) -> TextRun {
    math(format!("Q = {thresh}"), format!("Q = {thresh}"), format!("Q = {thresh}"))
}

fn multiplicity_adjustment_clause(lead: impl Into<String>, is_fdr: bool, thresh: &str, suffix: impl Into<String>) -> [TextRun; 3] {
    [plain(lead), if is_fdr { fdr_math(thresh) } else { fwer_math(thresh) }, plain(suffix)]
}

fn bullet_kv<I>(label: &str, value: I) -> Vec<TextRun>
where
    I: IntoIterator<Item = TextRun>,
{
    let mut runs = vec![bold(format!("{label}:")), plain(" ")];
    runs.extend(value);
    runs
}

// ---------------------------------------------------------------------------
// Report Assembly
// ---------------------------------------------------------------------------

fn build_report_blocks(doc: &ProjectDocument, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let mut blocks = Vec::new();
    blocks.push(ReportBlock::Title(vec![plain(format!("Statistical Methods and Results Report: {}", res.title()))]));

    append_study_context_section(&mut blocks, doc, res, source_ds);
    append_methods_section(&mut blocks, doc, res, source_ds);
    append_results_section(&mut blocks, doc, res, source_ds);
    append_diagnostics_section(&mut blocks, res);

    blocks
}

fn append_study_context_section(blocks: &mut Vec<ReportBlock>, doc: &ProjectDocument, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    blocks.push(ReportBlock::Section(vec![plain("Study Context and Data Provenance (ARRIVE / ICMJE)")]));

    let mut bullets = vec![
        bullet_kv("Project", [plain(&doc.title)]),
        bullet_kv("Analysis sheet", [plain(res.title()), plain(" ("), code(res.id.as_str()), plain(")")]),
        bullet_kv("Procedure", [plain(res.analysis.kind.derive_auto_title())]),
        bullet_kv("Recalculation status", [status_run(&res.status)]),
    ];

    if let Some(ds) = source_ds {
        let arch_label = ds.kind().label();
        let rows = ds.archetype.row_count();
        let cols = ds.archetype.column_count();
        let mut src_line = bullet_kv(
            "Source table",
            [plain(&ds.title), plain(" ("), code(ds.id.as_str()), plain(format!("; {arch_label}, {rows} rows x {cols} columns"))],
        );
        if ds.frozen {
            src_line.push(plain("; frozen snapshot"));
        }
        src_line.push(plain(")"));
        bullets.push(src_line);

        let selected_labels = selected_column_labels(ds, &res.analysis.selected_data_sets);
        if !selected_labels.is_empty() {
            let scope = if res.analysis.selected_data_sets.is_empty() { "All columns analyzed" } else { "Selected columns analyzed" };
            bullets.push(bullet_kv(scope, [plain(selected_labels.join(", "))]));
        }

        let excluded_count = count_excluded_cells(ds);
        let documented_count = count_active_exclusion_notes(ds);
        bullets.push(exclusion_audit_bullet(excluded_count, documented_count));
    } else {
        bullets.push(bullet_kv("Source table ID", [code(res.analysis.source_sheet_id.as_str())]));
    }
    if let Some(accounting) = res.paired_observation_accounting() {
        bullets.push(paired_observations_bullet(&accounting));
    }

    blocks.push(ReportBlock::Bullets(bullets));
}

/// Describe complete-case pairing from the counts the analysis recorded, so the report states the
/// observations actually analyzed rather than inferring them from the exclusion mask alone.
fn paired_observations_bullet(acc: &PairedObservationAccounting) -> Vec<TextRun> {
    let pairs = acc.pairs_analyzed;
    let omitted = acc.rows_omitted();
    let detail = if omitted == 0 {
        format!("{pairs} complete pair(s) analyzed; no rows omitted.")
    } else {
        let reasons: Vec<String> = [
            (acc.omitted_missing_partner, "missing a partner value"),
            (acc.omitted_excluded, "containing an excluded cell"),
            (acc.omitted_non_numeric, "containing a non-numeric cell"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, why)| format!("{n} {why}"))
        .collect();
        format!("{pairs} complete pair(s) analyzed; {omitted} row(s) omitted by complete-case pairing ({}).", reasons.join(", "))
    };
    bullet_kv("Paired observations", [plain(detail)])
}

fn status_run(status: &ResultStatus) -> TextRun {
    match status {
        ResultStatus::UpToDate => plain("Up to date"),
        ResultStatus::Dirty => plain("Pending recalculation (Dirty)"),
        ResultStatus::Failed { code: err_code, message } => plain(format!("Failed ({err_code}): {message}")),
    }
}

fn exclusion_audit_bullet(excluded_count: usize, documented_count: usize) -> Vec<TextRun> {
    let detail = if excluded_count == 0 {
        String::from("0 cells masked as excluded prior to analysis.")
    } else {
        format!("{excluded_count} cell(s) masked as excluded prior to analysis ({documented_count} with recorded audit justification note(s)).")
    };
    bullet_kv("Observation exclusions", [plain(detail)])
}

fn clean_percent_from_alpha(alpha: f64) -> f64 {
    if !alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0 {
        return 95.0;
    }
    (((1.0 - alpha) * 100.0) * 1e9).round() / 1e9
}

fn clean_alpha_cutoff(v: f64) -> f64 {
    if !v.is_finite() {
        return v;
    }
    (v * 1e12).round() / 1e12
}

fn effective_report_alpha(doc: &ProjectDocument, res: &ResultSheet) -> (f64, f64) {
    if let AnalysisKind::PowerAnalysis(p_spec) = &res.analysis.kind {
        let eff = resolve_effective_power_spec(doc, res, p_spec);
        let alpha = clean_alpha_cutoff(eff.alpha);
        return (clean_percent_from_alpha(alpha), alpha);
    }
    let spec = &res.analysis;
    (spec.confidence_level.percent(), clean_alpha_cutoff(spec.confidence_level.alpha()))
}

fn append_methods_section(blocks: &mut Vec<ReportBlock>, doc: &ProjectDocument, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    blocks.push(ReportBlock::Section(vec![plain("Statistical and Mathematical Methods")]));

    let spec = &res.analysis;
    let (eff_conf_pct, eff_alpha) = effective_report_alpha(doc, res);
    let conf_pct = format_exact_float(eff_conf_pct);
    let alpha = format_exact_float(eff_alpha);
    let (t2_raw, t3_raw, t4_raw) = if eff_alpha >= 0.05 { (0.01, 0.001, 0.0001) } else { (eff_alpha / 5.0, eff_alpha / 50.0, eff_alpha / 500.0) };
    let t2 = format_exact_float(clean_alpha_cutoff(t2_raw));
    let t3 = format_exact_float(clean_alpha_cutoff(t3_raw));
    let t4 = format_exact_float(clean_alpha_cutoff(t4_raw));
    let mut star_legend = format!("ns for p > {alpha}, * for p <= {alpha}, ** for p <= {t2}, *** for p <= {t3}");
    if spec.p_value_format.star_ceiling() >= 4 {
        let _ = write!(star_legend, ", **** for p <= {t4}");
    }

    blocks.push(ReportBlock::Paragraph(vec![
        plain("Analyses were conducted in CurveBook with "),
        math(format!("{conf_pct}%"), format!("{conf_pct}%"), format!("{conf_pct}\\%")),
        plain(" confidence intervals and nominal significance threshold "),
        math(format!("alpha = {alpha}"), format!("alpha = {alpha}"), format!("\\alpha = {alpha}")),
        plain(" ("),
        math("p < alpha", "p < alpha", "p < \\alpha"),
        plain(" vs. "),
        math("p >= alpha", "p >= alpha", "p \\ge \\alpha"),
        plain("; inclusive test-rejection and star boundary "),
        math("p <= alpha", "p <= alpha", "p \\le \\alpha"),
        plain(format!("; significance stars at alpha = {alpha}: {star_legend}), using stored P-value formatting rule ")),
        plain(p_format_label(spec.p_value_format)),
        plain(". Stored summary estimates use exact integer or short verbatim decimal formatting, or compact 6-decimal fixed/scientific formatting when representations exceed 10-12 characters."),
    ]));

    let mut method_paragraphs = describe_analysis_kind(doc, spec, res, source_ds);
    blocks.append(&mut method_paragraphs);
}

fn describe_analysis_kind(doc: &ProjectDocument, spec: &AnalysisSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(s) => describe_two_group(s, source_ds),
        AnalysisKind::MultipleRowTTests(s) => describe_multiple_row_tests(s, res),
        AnalysisKind::OneWayAnova(s) => describe_one_way_anova(s, res, source_ds),
        AnalysisKind::FactorialAnova(s) => describe_factorial_anova(s, res, source_ds),
        AnalysisKind::NestedComparison(s) => describe_nested(s, res, source_ds),
        AnalysisKind::BlandAltman(s) => describe_bland_altman(s, source_ds, spec.confidence_level.percent()),
        AnalysisKind::PassingBablok(s) => describe_passing_bablok(s, source_ds),
        AnalysisKind::PValueStack(s) => describe_pvalue_stack(s, source_ds),
        AnalysisKind::PowerAnalysis(s) => {
            let effective = resolve_effective_power_spec(doc, res, s);
            vec![ReportBlock::Paragraph(vec![plain(describe_power_analysis_spec(&effective))])]
        }
        AnalysisKind::Contingency(s) => describe_contingency(s, spec, source_ds, res),
        AnalysisKind::Survival(s) => describe_survival(s, res, source_ds),
        AnalysisKind::RocCurve(s) => describe_roc(s),
        AnalysisKind::GeneralizedLinearModel(s) => describe_glm(s, source_ds),
        AnalysisKind::Multivariate(s) => describe_multivariate(s, res, source_ds),
        AnalysisKind::LinearRegression(s) => describe_linear_regression(s),
        AnalysisKind::NonlinearRegression(s) => describe_nonlinear_regression(s, res),
        AnalysisKind::FitSplineLowess(s) => describe_spline_lowess(s),
        AnalysisKind::DescriptiveStats(s) => describe_descriptive(s),
        AnalysisKind::OutlierDetection(s) => describe_outlier_detection(s),
        AnalysisKind::Transform(s) => describe_transform(s),
        AnalysisKind::SimpleLogisticRegression(s) => describe_simple_logistic(s),
        AnalysisKind::LeveneTest(s) => describe_levene_test(s),
    }
}

fn describe_two_group(s: &TwoGroupSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let tail_text = if matches!(s.method, TwoGroupMethod::KolmogorovSmirnov) {
        if s.two_tailed { "two-tailed" } else { "two-tailed (the two-sample Kolmogorov-Smirnov test always evaluates a two-sided D statistic)" }
    } else if s.two_tailed {
        "two-tailed"
    } else {
        "one-tailed"
    };
    let method_desc = match s.method {
        TwoGroupMethod::UnpairedT => String::from(
            "An unpaired Student's t-test assuming equal population variances was performed, accompanied by an F-test and Levene's median test for variance homogeneity.",
        ),
        TwoGroupMethod::WelchT => String::from(
            "Welch's unequal-variance t-test with Welch-Satterthwaite effective degrees of freedom was performed without assuming equal group variances.",
        ),
        TwoGroupMethod::PairedT => String::from(
            "A paired Student's t-test was performed on within-subject paired differences, with pairing effectiveness evaluated via Pearson correlation r.",
        ),
        TwoGroupMethod::RatioPairedT => String::from(
            "A ratio paired t-test was performed on natural-logarithm-transformed within-pair ratios, with results back-transformed to the geometric mean ratio and its confidence interval.",
        ),
        TwoGroupMethod::LognormalUnpairedT => String::from(
            "An unpaired lognormal t-test was performed on natural-logarithm-transformed observations, yielding the geometric mean ratio and corresponding confidence interval.",
        ),
        TwoGroupMethod::MannWhitneyU => String::from(
            "A nonparametric two-sample Mann-Whitney U rank-sum test was performed with Hodges-Lehmann median difference estimation and rank-biserial / Cliff's delta effect sizes.",
        ),
        TwoGroupMethod::WilcoxonSignedRank => String::from(
            "A nonparametric Wilcoxon matched-pairs signed-rank test was performed on paired differences with Hodges-Lehmann pseudomedian estimation.",
        ),
        TwoGroupMethod::KolmogorovSmirnov => {
            String::from("A two-sample Kolmogorov-Smirnov test was performed on empirical cumulative distributions (D statistic).")
        }
        TwoGroupMethod::OneSampleT { hypothetical } => format!(
            "A one-sample Student's t-test compared the observed sample mean against the hypothetical reference value {}.",
            format_exact_float(hypothetical)
        ),
        TwoGroupMethod::OneSampleWilcoxon { hypothetical } => format!(
            "A one-sample Wilcoxon signed-rank test compared the observed sample median against the hypothetical reference median {}.",
            format_exact_float(hypothetical)
        ),
        TwoGroupMethod::PairedSignTest => String::from(
            "An exact two-sample paired sign test evaluated positive versus negative within-pair differences against a binomial null proportion of 0.5 and reported the median difference with an exact order-statistic confidence interval (excluding zero-difference ties from the binomial trial count).",
        ),
    };

    let mut runs = vec![plain(format!("Hypothesis testing was {tail_text}. {method_desc}"))];
    match s.method {
        TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT if s.compute_effect_sizes => {
            runs.push(plain(" Standardized effect size indices (Cohen's d, Hedges' g, Glass's delta, and R^2) were computed."));
        }
        TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT if s.compute_effect_sizes => {
            runs.push(plain(" Standardized paired effect size indices (Cohen's d_z and Hedges' g_z) were computed."));
        }
        TwoGroupMethod::OneSampleT { .. } => {
            runs.push(plain(" Standardized effect size index (Hedges' g) was computed."));
        }
        _ => {}
    }
    if let (Some(g), Some(r)) = (s.mv_group_column, s.mv_response_column) {
        let g_lbl = column_title_at(source_ds, g);
        let r_lbl = column_title_at(source_ds, r);
        runs.push(plain(" Grouping variable "));
        runs.push(code(g_lbl));
        runs.push(plain(" and response variable "));
        runs.push(code(r_lbl));
        runs.push(plain(" were extracted from the multivariable table."));
    }
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_multiple_row_tests(s: &MultipleRowTestsSpec, res: &ResultSheet) -> Vec<ReportBlock> {
    let family = match (s.nonparametric, s.paired, s.assume_equal_sd) {
        (true, true, _) => "Wilcoxon matched-pairs signed-rank tests",
        (true, false, _) => "Mann-Whitney U rank-sum tests",
        (false, true, _) => "paired Student's t-tests",
        (false, false, true) => "unpaired Student's t-tests assuming equal SD",
        (false, false, false) => "Welch's unpaired t-tests without assuming equal SD",
    };
    let mut runs = vec![plain(format!("Row-wise two-group comparisons were evaluated using {family}. "))];
    let mult_spec = MultiplicitySpec { report_confidence_intervals: false, ..s.multiplicity };
    match mult_spec.method {
        MultiplicityMethod::None => {
            runs.push(plain("Multiple comparisons across rows used no post-hoc multiplicity correction (unadjusted raw P values)."));
        }
        MultiplicityMethod::FisherLsd => {
            runs.push(plain("Row-wise P values were left unadjusted (Fisher's unprotected LSD; no multiplicity correction applied across rows)."));
        }
        MultiplicityMethod::Bonferroni
        | MultiplicityMethod::Sidak
        | MultiplicityMethod::HolmSidak
        | MultiplicityMethod::BenjaminiHochbergFdr
        | MultiplicityMethod::BenjaminiYekutieliFdr
        | MultiplicityMethod::TwoStageBkyFdr => {
            runs.extend(describe_multiplicity_runs(&mult_spec, false));
        }
        MultiplicityMethod::TukeyHsd
        | MultiplicityMethod::NewmanKeuls
        | MultiplicityMethod::Dunnett { .. }
        | MultiplicityMethod::DunnettT3
        | MultiplicityMethod::GamesHowell
        | MultiplicityMethod::TamhaneT2
        | MultiplicityMethod::DunnRankSum
        | MultiplicityMethod::LinearTrend => {
            if matches!(res.status, ResultStatus::Failed { .. }) {
                runs.push(plain("The requested ANOVA-only post-hoc procedure is not valid for raw row-wise P-value adjustment."));
            } else {
                let eff_method =
                    if mult_spec.method == MultiplicityMethod::TamhaneT2 { MultiplicityMethod::Sidak } else { MultiplicityMethod::Bonferroni };
                let eff_spec = MultiplicitySpec { method: eff_method, ..mult_spec };
                runs.extend(describe_multiplicity_runs(&eff_spec, false));
            }
        }
    }
    vec![ReportBlock::Paragraph(runs)]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RmEstimator {
    None,
    ClassicalBalancedOls,
    CompoundSymmetryReml,
    UnstructuredReml,
}

fn resolve_one_way_rm_estimator(s: &OneWayAnovaSpec, res: &ResultSheet) -> RmEstimator {
    if !matches!(s.method, OneWayMethod::RepeatedMeasures { .. }) {
        return RmEstimator::None;
    }
    if res.tabs.iter().any(|t| t.id == "unstructured-reml-covariance" || t.id == "unstructured-reml-correlation") {
        return RmEstimator::UnstructuredReml;
    }
    if res.tabs.iter().any(|t| t.id == "mixed-reml") {
        return RmEstimator::CompoundSymmetryReml;
    }
    if res.tabs.iter().any(|t| t.id == "rm-one-way-anova" && !t.title.contains("REML"))
        || res.scalars.iter().any(|sc| sc.key == "Greenhouse-Geisser epsilon")
    {
        return RmEstimator::ClassicalBalancedOls;
    }
    if s.residual_covariance == ResidualCovariance::Unstructured {
        RmEstimator::UnstructuredReml
    } else if s.mixed_effects_reml {
        RmEstimator::CompoundSymmetryReml
    } else {
        RmEstimator::ClassicalBalancedOls
    }
}

fn describe_one_way_anova(s: &OneWayAnovaSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let mut runs = match s.method {
        OneWayMethod::OrdinaryAnova => vec![plain(
            "Group means across three or more independent conditions were compared using ordinary one-way ANOVA assuming homogeneous variances (checked via Brown-Forsythe test).",
        )],
        OneWayMethod::BrownForsytheAndWelch => vec![plain(
            "Groups with potentially unequal variances were compared using Welch's ANOVA (W) and the Brown-Forsythe F* ANOVA without assuming variance homogeneity.",
        )],
        OneWayMethod::RepeatedMeasures { geisser_greenhouse } => describe_rm_one_way_header(s, res, geisser_greenhouse),
        OneWayMethod::KruskalWallis => vec![plain(
            "Independent rank distributions were compared using the nonparametric Kruskal-Wallis H test with tie correction and epsilon-squared effect size.",
        )],
        OneWayMethod::Friedman => {
            vec![plain("Matched subject blocks were compared using the nonparametric Friedman rank test with Kendall's W concordance.")]
        }
    };

    match s.method {
        OneWayMethod::OrdinaryAnova if s.compute_effect_sizes => {
            runs.push(plain(" Variance-explained effect sizes (eta-squared, partial eta-squared, omega-squared, and Cohen's f) were computed."));
        }
        OneWayMethod::RepeatedMeasures { .. } => {
            let rm_est = resolve_one_way_rm_estimator(s, res);
            let is_reml = matches!(rm_est, RmEstimator::CompoundSymmetryReml | RmEstimator::UnstructuredReml);
            if !is_reml || s.compute_effect_sizes {
                runs.push(plain(" Variance-explained effect size (partial eta-squared) was computed."));
            }
        }
        OneWayMethod::OrdinaryAnova | OneWayMethod::BrownForsytheAndWelch | OneWayMethod::KruskalWallis | OneWayMethod::Friedman => {}
    }

    append_one_way_posthoc_runs(&mut runs, s, res, source_ds);
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_rm_one_way_header(s: &OneWayAnovaSpec, res: &ResultSheet, geisser_greenhouse: bool) -> Vec<TextRun> {
    let df_lbl = df_method_label(s.df_method);
    match resolve_one_way_rm_estimator(s, res) {
        RmEstimator::UnstructuredReml => vec![plain(format!(
            "Matched subject blocks were analyzed via a mixed-effects REML model with unstructured residual covariance and {df_lbl} degrees of freedom."
        ))],
        RmEstimator::CompoundSymmetryReml => vec![plain(format!(
            "Matched subject blocks were analyzed via a compound-symmetry mixed-effects REML model with {df_lbl} degrees of freedom."
        ))],
        RmEstimator::ClassicalBalancedOls | RmEstimator::None => {
            if geisser_greenhouse {
                vec![plain(
                    "Matched subject blocks were analyzed via classical repeated-measures one-way ANOVA with the Geisser-Greenhouse sphericity correction applied to treatment degrees of freedom (reporting epsilon_GG, epsilon_HF, and Mauchly's sphericity test).",
                )]
            } else {
                vec![plain(
                    "Matched subject blocks were analyzed via classical repeated-measures one-way ANOVA assuming compound symmetry / sphericity (with Geisser-Greenhouse and Huynh-Feldt epsilon diagnostics reported).",
                )]
            }
        }
    }
}

fn append_one_way_posthoc_runs(runs: &mut Vec<TextRun>, s: &OneWayAnovaSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    let has_ci = result_has_posthoc_ci(res);
    match s.method {
        OneWayMethod::OrdinaryAnova => {
            append_control_clause(runs, &s.multiplicity, source_ds);
            runs.push(plain(" "));
            if s.multiplicity.method == MultiplicityMethod::LinearTrend {
                runs.push(plain("A post-hoc test for linear trend across ordered group means was performed."));
            } else if s.multiplicity.method == MultiplicityMethod::DunnRankSum && matches!(res.status, ResultStatus::Failed { .. }) {
                runs.push(plain("The requested nonparametric rank-sum post-hoc procedure is incompatible with ordinary parametric ANOVA."));
            } else {
                runs.extend(describe_multiplicity_runs(&s.multiplicity, has_ci));
            }
        }
        OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
            if matches!(
                s.multiplicity.method,
                MultiplicityMethod::GamesHowell | MultiplicityMethod::TamhaneT2 | MultiplicityMethod::DunnettT3 | MultiplicityMethod::DunnRankSum
            ) && matches!(res.status, ResultStatus::Failed { .. })
            {
                runs.push(plain(" The requested post-hoc procedure is incompatible with parametric repeated-measures ANOVA."));
                return;
            }
            append_control_clause(runs, &s.multiplicity, source_ds);
            runs.push(plain(" "));
            if s.multiplicity.method == MultiplicityMethod::None {
                runs.push(plain("Post-hoc pairwise comparisons were not requested (none computed)."));
            } else if s.multiplicity.method == MultiplicityMethod::LinearTrend {
                runs.push(plain("A post-hoc test for linear trend across ordered condition means was performed."));
            } else {
                let rm_est = resolve_one_way_rm_estimator(s, res);
                let paired_gg = rm_est == RmEstimator::ClassicalBalancedOls
                    && geisser_greenhouse
                    && matches!(
                        s.multiplicity.method,
                        MultiplicityMethod::Bonferroni
                            | MultiplicityMethod::Sidak
                            | MultiplicityMethod::HolmSidak
                            | MultiplicityMethod::FisherLsd
                            | MultiplicityMethod::BenjaminiHochbergFdr
                            | MultiplicityMethod::BenjaminiYekutieliFdr
                            | MultiplicityMethod::TwoStageBkyFdr
                    );
                if paired_gg {
                    runs.push(plain("Without assuming sphericity, matched-pair difference t contrasts (df = n - 1) were used: "));
                }
                runs.extend(describe_multiplicity_runs(&s.multiplicity, has_ci));
            }
        }
        OneWayMethod::BrownForsytheAndWelch => {
            append_welch_posthoc_runs(runs, &s.multiplicity, res, source_ds, has_ci);
        }
        OneWayMethod::KruskalWallis => {
            runs.push(plain(" "));
            runs.extend(describe_dunn_multiplicity_runs(&s.multiplicity, source_ds, "rank-sum"));
        }
        OneWayMethod::Friedman => match s.multiplicity.method {
            MultiplicityMethod::None
            | MultiplicityMethod::TukeyHsd
            | MultiplicityMethod::NewmanKeuls
            | MultiplicityMethod::GamesHowell
            | MultiplicityMethod::DunnettT3
            | MultiplicityMethod::TamhaneT2
            | MultiplicityMethod::LinearTrend => {
                runs.push(plain(" Post-hoc pairwise comparisons were not computed for the Friedman rank test."));
            }
            MultiplicityMethod::DunnRankSum
            | MultiplicityMethod::Bonferroni
            | MultiplicityMethod::Sidak
            | MultiplicityMethod::HolmSidak
            | MultiplicityMethod::FisherLsd
            | MultiplicityMethod::Dunnett { .. }
            | MultiplicityMethod::BenjaminiHochbergFdr
            | MultiplicityMethod::BenjaminiYekutieliFdr
            | MultiplicityMethod::TwoStageBkyFdr => {
                runs.push(plain(" "));
                runs.extend(describe_dunn_multiplicity_runs(&s.multiplicity, source_ds, "within-block mean rank"));
            }
        },
    }
}

fn append_welch_posthoc_runs(runs: &mut Vec<TextRun>, mult: &MultiplicitySpec, res: &ResultSheet, source_ds: Option<&DataSheet>, has_ci: bool) {
    runs.push(plain(" "));
    match mult.method {
        MultiplicityMethod::None => {
            runs.push(plain("Post-hoc pairwise comparisons were not requested (none computed)."));
        }
        MultiplicityMethod::DunnRankSum => {
            if matches!(res.status, ResultStatus::Failed { .. }) {
                runs.push(plain("The requested rank-sum post-hoc procedure is incompatible with Welch/Brown-Forsythe ANOVA."));
            } else {
                runs.push(plain("Post-hoc pairwise comparisons were not computed."));
            }
        }
        MultiplicityMethod::GamesHowell | MultiplicityMethod::TamhaneT2 | MultiplicityMethod::DunnettT3 => {
            runs.extend(describe_multiplicity_runs(mult, has_ci));
        }
        MultiplicityMethod::TukeyHsd | MultiplicityMethod::NewmanKeuls => {
            let eff_mult = MultiplicitySpec { method: MultiplicityMethod::GamesHowell, ..*mult };
            runs.extend(describe_multiplicity_runs(&eff_mult, has_ci));
        }
        MultiplicityMethod::Dunnett { control_column } => {
            let ctrl_name = column_title_at(source_ds, control_column);
            let thresh = format_exact_float(mult.threshold);
            let ci_note = if mult.report_confidence_intervals && has_ci { " with multiplicity-adjusted confidence intervals" } else { "" };
            runs.push(plain("Control column for post-hoc comparisons: "));
            runs.push(code(ctrl_name));
            runs.extend(multiplicity_adjustment_clause(
                ". Multiple comparisons used unequal-variance Welch t comparisons versus the control group with heteroscedastic Dunnett loadings and Satterthwaite degrees of freedom at ",
                false,
                &thresh,
                format!("{ci_note}."),
            ));
        }
        MultiplicityMethod::LinearTrend => {
            runs.push(plain(
                "A heteroscedastic polynomial test for linear trend across ordered groups was performed using per-group variances and Welch-Satterthwaite denominator degrees of freedom.",
            ));
        }
        MultiplicityMethod::Bonferroni
        | MultiplicityMethod::Sidak
        | MultiplicityMethod::HolmSidak
        | MultiplicityMethod::FisherLsd
        | MultiplicityMethod::BenjaminiHochbergFdr
        | MultiplicityMethod::BenjaminiYekutieliFdr
        | MultiplicityMethod::TwoStageBkyFdr => {
            runs.push(plain("Per-pair Welch unequal-variance t tests with Welch-Satterthwaite degrees of freedom were evaluated: "));
            runs.extend(describe_multiplicity_runs(mult, has_ci));
        }
    }
}

fn append_control_clause(runs: &mut Vec<TextRun>, mult: &MultiplicitySpec, source_ds: Option<&DataSheet>) {
    if let MultiplicityMethod::Dunnett { control_column } = mult.method {
        let ctrl_name = column_title_at(source_ds, control_column);
        runs.push(plain(" Control column for post-hoc comparisons: "));
        runs.push(code(ctrl_name));
        runs.push(plain("."));
    }
}

fn df_method_label(df_method: MixedModelDfMethod) -> &'static str {
    match df_method {
        MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite",
    }
}

fn resolve_factorial_rm_estimator(s: &FactorialAnovaSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> RmEstimator {
    if s.repeated_measures == RepeatedMeasuresDesign::None || !s.multifactor_columns.is_empty() {
        return RmEstimator::None;
    }
    if res.tabs.iter().any(|t| t.id == "unstructured-reml-covariance" || t.id == "unstructured-reml-correlation")
        || res
            .checklist
            .iter()
            .any(|c| c.assumption == "Repeated-measures estimator" && c.diagnostic_metric.as_deref().is_some_and(|m| m.contains("Unstructured")))
    {
        return RmEstimator::UnstructuredReml;
    }
    if res.tabs.iter().any(|t| t.id == "mixed-reml")
        || res
            .checklist
            .iter()
            .any(|c| c.assumption == "Repeated-measures estimator" && c.diagnostic_metric.as_deref().is_some_and(|m| m.contains("Compound Symmetry")))
    {
        return RmEstimator::CompoundSymmetryReml;
    }
    if res.tabs.iter().any(|t| t.id == "rm-sphericity")
        || res
            .checklist
            .iter()
            .any(|c| c.assumption == "Repeated-measures estimator" && c.diagnostic_metric.as_deref().is_some_and(|m| m.contains("Classical")))
    {
        return RmEstimator::ClassicalBalancedOls;
    }
    if s.residual_covariance == ResidualCovariance::Unstructured {
        return RmEstimator::UnstructuredReml;
    }
    if s.mixed_effects_reml || s.three_way {
        return RmEstimator::CompoundSymmetryReml;
    }
    if s.fit_reml_on_missing
        && let Some(ds) = source_ds
        && let TableArchetype::Grouped(grouped) = &ds.archetype
        && grouped.has_missing_replicates()
    {
        return RmEstimator::CompoundSymmetryReml;
    }
    RmEstimator::ClassicalBalancedOls
}

fn describe_factorial_anova(s: &FactorialAnovaSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let design_label = if !s.multifactor_columns.is_empty() {
        format!("multifactor {}-way general linear model (up to order-{} interactions)", s.multifactor_columns.len(), s.max_interaction_order)
    } else if s.three_way {
        String::from("three-way factorial ANOVA")
    } else {
        String::from("two-way factorial ANOVA")
    };
    let ss_label = match s.ss_type {
        SumOfSquaresType::TypeI => "Type I (sequential)",
        SumOfSquaresType::TypeIII => "Type III (partial / marginal)",
    };
    let rm_label = match s.repeated_measures {
        RepeatedMeasuresDesign::None => "no repeated-measures matching (all factors between-subjects)",
        RepeatedMeasuresDesign::RowFactorMatched => "repeated measures matched across the row factor",
        RepeatedMeasuresDesign::ColumnFactorMatched => "repeated measures matched across the column factor",
        RepeatedMeasuresDesign::BothFactorsMatched => "repeated measures matched across both row and column factors",
    };
    let df_lbl = df_method_label(s.df_method);
    let rm_estimator = resolve_factorial_rm_estimator(s, res, source_ds);
    let reml_text = match rm_estimator {
        RmEstimator::None => String::new(),
        RmEstimator::UnstructuredReml => format!(
            " Repeated-measures correlations were modeled with an unstructured residual covariance matrix estimated by Cholesky restricted maximum likelihood (REML) with {df_lbl} degrees of freedom."
        ),
        RmEstimator::CompoundSymmetryReml => format!(
            " Repeated-measures mixed-effects restricted maximum likelihood (REML) with compound-symmetry covariance and {df_lbl} degrees of freedom was fitted."
        ),
        RmEstimator::ClassicalBalancedOls => String::from(
            " Complete repeated-measures cells were analyzed using classical balanced repeated-measures ANOVA (OLS) with Mauchly, Greenhouse-Geisser, and Huynh-Feldt sphericity diagnostics.",
        ),
    };
    let mut runs = vec![plain(format!("Factorial comparisons used {design_label} with {ss_label} sums of squares and {rm_label}.{reml_text}"))];
    if let Some(resp) = s.response_column {
        let resp_lbl = column_title_at(source_ds, resp);
        runs.push(plain(" Response variable: "));
        runs.push(code(resp_lbl));
        runs.push(plain("."));
    }
    if s.compute_effect_sizes {
        let is_reml = matches!(rm_estimator, RmEstimator::UnstructuredReml | RmEstimator::CompoundSymmetryReml);
        if is_reml {
            runs.push(plain(" Partial eta-squared effect sizes were calculated (total eta-squared is omitted for REML mixed-effects fits)."));
        } else {
            runs.push(plain(" Partial eta-squared and eta-squared effect sizes were calculated."));
        }
    }
    runs.push(plain(" "));
    if s.multiplicity.method == MultiplicityMethod::None {
        runs.push(plain("Post-hoc pairwise comparisons were not requested (none computed)."));
    } else {
        runs.extend(describe_multiplicity_runs(&s.multiplicity, result_has_posthoc_ci(res)));
    }
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_nested(s: &NestedAnalysisSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let test_name = if s.two_group_t_test { "nested hierarchical t-test" } else { "nested hierarchical one-way ANOVA" };
    let df_lbl = df_method_label(s.df_method);
    let mut runs = vec![plain(format!(
        "Subcolumn technical/biological replicates nested within clusters were analyzed via a {test_name} (mixed-effects REML variance decomposition of cluster vs. residual variance with {df_lbl} denominator degrees of freedom). "
    ))];
    if s.two_group_t_test {
        runs.push(plain("Post-hoc pairwise comparisons were not computed because the nested t-test compares only two groups."));
        return vec![ReportBlock::Paragraph(runs)];
    }
    if s.multiplicity.method == MultiplicityMethod::None {
        runs.push(plain("Post-hoc pairwise comparisons were not requested (none computed)."));
        return vec![ReportBlock::Paragraph(runs)];
    }
    let has_posthoc_tab = res.tabs.iter().any(|t| t.id == "multiple-comparisons" || t.id == "linear-trend");
    let fewer_than_three = if matches!(res.status, ResultStatus::UpToDate) {
        !has_posthoc_tab
    } else if let Some(ds) = source_ds {
        let count = if res.analysis.selected_data_sets.is_empty() { ds.archetype.column_count() } else { res.analysis.selected_data_sets.len() };
        count < 3
    } else {
        false
    };
    if fewer_than_three {
        runs.push(plain("Post-hoc pairwise comparisons were not computed because fewer than three groups were analyzed."));
    } else if s.multiplicity.method == MultiplicityMethod::LinearTrend {
        runs.push(plain("A post-hoc test for linear trend across ordered group means was performed."));
    } else {
        append_control_clause(&mut runs, &s.multiplicity, source_ds);
        if matches!(s.multiplicity.method, MultiplicityMethod::Dunnett { .. }) {
            runs.push(plain(" "));
        }
        runs.extend(describe_multiplicity_runs(&s.multiplicity, result_has_posthoc_ci(res)));
    }
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_bland_altman(s: &BlandAltmanSpec, source_ds: Option<&DataSheet>, conf_percent: f64) -> Vec<ReportBlock> {
    let (col_a, col_b) = match source_ds.map(|ds| &ds.archetype) {
        Some(TableArchetype::Xy(xy)) if s.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1) => {
            let a = if xy.x_title.trim().is_empty() { String::from("X") } else { xy.x_title.clone() };
            let b = xy
                .data_sets
                .first()
                .map(|c| c.title.as_str())
                .filter(|t| !t.trim().is_empty())
                .map_or_else(|| String::from("Column 1"), str::to_owned);
            (a, b)
        }
        _ => (column_title_at(source_ds, s.column_a), column_title_at(source_ds, s.column_b)),
    };
    let scale = if s.percent_difference { "percentage differences relative to the pair mean" } else { "raw paired differences (A - B)" };
    let conf_pct = format_exact_float(conf_percent);
    vec![ReportBlock::Paragraph(vec![
        plain("Method agreement between "),
        code(col_a),
        plain(" and "),
        code(col_b),
        plain(format!(
            " was evaluated via Bland-Altman analysis of {scale} against pair averages, reporting mean bias, standard deviation of differences, and {conf_pct}% limits of agreement with confidence intervals."
        )),
    ])]
}

fn describe_pvalue_stack(s: &PValueStackSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let (threshold_sym, control, citation) = match s.method {
        PValueStackMethod::Bonferroni => ("alpha", "family-wise error rate (FWER)", "Dunn (1961)"),
        PValueStackMethod::HolmBonferroni => ("alpha", "family-wise error rate (FWER) step-down", "Holm (1979)"),
        PValueStackMethod::Sidak => ("alpha", "family-wise error rate (FWER)", "Šídák (1967)"),
        PValueStackMethod::HolmSidak => ("alpha", "family-wise error rate (FWER) step-down", "Holm (1979) and Holland & Copenhaver (1987)"),
        PValueStackMethod::BenjaminiHochberg => ("Q", "false discovery rate (FDR)", "Benjamini & Hochberg (1995)"),
        PValueStackMethod::TwoStageBky => ("Q", "two-stage adaptive false discovery rate (FDR)", "Benjamini, Krieger, & Yekutieli (2006)"),
        PValueStackMethod::BenjaminiYekutieli => ("Q", "false discovery rate (FDR) under arbitrary dependence", "Benjamini & Yekutieli (2001)"),
    };
    let mut runs = vec![plain("Raw P values ")];
    if let Some(col) = s.column {
        runs.push(plain("on "));
        runs.push(code(column_title_at(source_ds, col)));
    } else {
        runs.push(plain("across the selected P-value column(s)"));
    }
    runs.push(plain(format!(
        " were evaluated for statistical discovery using the {} procedure ({citation}) to control the {control} at {threshold_sym} = {}. Adjusted P values (or FDR q-values) and boolean discovery indicators preserve the original input row order and rank mapping.",
        s.method.label(),
        format_exact_float(s.threshold)
    )));
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_levene_test(s: &LeveneTestSpec) -> Vec<ReportBlock> {
    let text = match s.center {
        LeveneCenter::Median => {
            "Homogeneity of variances across the selected columns was assessed with the Brown-Forsythe test, a one-way ANOVA F test on absolute deviations from each group's median (Brown & Forsythe 1974), with DFn = k - 1 and DFd = N - k."
        }
        LeveneCenter::Mean => {
            "Homogeneity of variances across the selected columns was assessed with Levene's test, a one-way ANOVA F test on absolute deviations from each group's mean (Levene 1960), with DFn = k - 1 and DFd = N - k."
        }
    };
    vec![ReportBlock::Paragraph(vec![plain(text)])]
}

fn describe_passing_bablok(s: &PassingBablokSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let col_x = column_title_at(source_ds, s.x_column);
    let col_y = column_title_at(source_ds, s.y_column);
    vec![ReportBlock::Paragraph(vec![
        plain("Nonparametric Passing-Bablok method-comparison regression of "),
        code(col_y),
        plain(" against "),
        code(col_x),
        plain(
            " computed the shifted median of valid pairwise slopes and median intercept, with rank-based confidence intervals and CUSUM linearity testing against the fitted regression line.",
        ),
    ])]
}

fn contingency_table_is_2x2(spec: &AnalysisSpec, source_ds: Option<&DataSheet>, res: &ResultSheet) -> bool {
    if let Some(ds) = source_ds {
        return match &ds.archetype {
            TableArchetype::Contingency(t) => {
                let n_cols = if spec.selected_data_sets.is_empty() {
                    t.data_sets.len()
                } else {
                    spec.selected_data_sets.iter().filter(|&&idx| idx < t.data_sets.len()).count()
                };
                t.row_count() == 2 && n_cols == 2
            }
            _ => false,
        };
    }
    res.tabs.iter().any(|tab| tab.id == "contingency-effects")
}

fn describe_contingency(s: &ContingencySpec, spec: &AnalysisSpec, source_ds: Option<&DataSheet>, res: &ResultSheet) -> Vec<ReportBlock> {
    if matches!(source_ds.map(|ds| &ds.archetype), Some(TableArchetype::PartsOfWhole(_)))
        && matches!(s.method, ContingencyMethod::PearsonChiSquare { .. })
    {
        let mut runs = vec![plain("Part-of-whole frequencies were evaluated using a one-way chi-square goodness-of-fit test.")];
        if let Some(props) = &s.expected_proportions {
            let formatted: Vec<String> = props.iter().map(|&p| format_exact_float(p)).collect();
            runs.push(plain(format!(" Expected counts were parameterized from custom ratios ({}, normalized to unit sum).", formatted.join(", "))));
        } else {
            runs.push(plain(" Uniform equal expected proportions were assumed across slices."));
        }
        runs.push(plain(" Asymptotic chi-square adequacy assumes expected cell counts "));
        runs.push(math("E >= 5", "E >= 5", "E \\ge 5"));
        runs.push(plain(" (verified in assumption diagnostics)."));
        return vec![ReportBlock::Paragraph(runs)];
    }

    let test_desc = match s.method {
        ContingencyMethod::CohenKappa { weighting } => {
            let weight_desc = match weighting {
                KappaWeighting::Unweighted => "unweighted nominal agreement (identity weights)",
                KappaWeighting::Linear => "linear ordinal agreement weights (1 - |i - j| / (K - 1))",
                KappaWeighting::Quadratic => "quadratic Fleiss-Cohen ordinal agreement weights (1 - ((i - j) / (K - 1))^2)",
            };
            return vec![ReportBlock::Paragraph(vec![plain(format!(
                "Inter-rater agreement across square K x K classifications was quantified using Cohen's kappa with {weight_desc}, reporting observed agreement P_o, chance-expected agreement P_e, asymptotic standard error, Wald confidence interval, and a normal Z-test against kappa = 0."
            ))])];
        }
        ContingencyMethod::FleissKappa => {
            return vec![ReportBlock::Paragraph(vec![plain(
                "Multi-rater nominal agreement across fixed rater counts per subject was evaluated using Fleiss' chance-corrected kappa (reporting overall P_bar, chance-expected agreement P_e, asymptotic standard error, confidence interval, and Z-test against kappa = 0, alongside category-wise conditional kappa estimates).",
            )])];
        }
        ContingencyMethod::ExactBinomial => {
            let p0 = format_exact_float(s.null_proportion.unwrap_or(0.5));
            return vec![ReportBlock::Paragraph(vec![plain(format!(
                "Binary successes versus total trials were compared against the null proportion p0 = {p0} using an exact one-sample binomial test (reporting Sterne and doubled-tail two-sided P values alongside one-sided lower/upper exact tail probabilities), with exact Clopper-Pearson, Wilson score, and Agresti-Coull binomial proportion confidence intervals."
            ))])];
        }
        ContingencyMethod::PearsonChiSquare { yates_continuity: true } => {
            "Pearson's chi-square contingency test with Yates' continuity correction for 2x2 tables"
        }
        ContingencyMethod::PearsonChiSquare { yates_continuity: false } => "Pearson's chi-square contingency test",
        ContingencyMethod::LikelihoodRatioG2 => "Likelihood-ratio G-squared contingency test",
        ContingencyMethod::FisherExact => "Fisher's exact conditional hypergeometric test",
        ContingencyMethod::BarnardExact => {
            "Barnard's unconditional exact score test maximizing nuisance probability across pi in [0, 1] without fixing both margins"
        }
        ContingencyMethod::CochranArmitageTrend => "Cochran-Armitage chi-square test for ordered trend across columns",
        ContingencyMethod::McNemarPaired => "McNemar's test for paired discordant binary proportions",
    };

    let mut runs = vec![plain(format!("Categorical counts were evaluated using {test_desc}."))];
    if let Some(props) = &s.expected_proportions {
        let formatted: Vec<String> = props.iter().map(|&p| format_exact_float(p)).collect();
        runs.push(plain(format!(
            " One-way goodness-of-fit expected counts were parameterized from custom ratios ({}, normalized to unit sum).",
            formatted.join(", ")
        )));
    }
    let is_2x2 = contingency_table_is_2x2(spec, source_ds, res);
    if s.compute_odds_and_risk_ratios && is_2x2 {
        runs.push(plain(
            " For 2x2 tables, effect sizes included the odds ratio (Baptista-Pike exact CI), relative risk (Koopman asymptotic score CI), risk difference (Newcombe score CI), and number needed to treat (NNT).",
        ));
    }
    if s.compute_sensitivity_specificity && is_2x2 {
        runs.push(plain(
            " Diagnostic accuracy metrics (sensitivity, specificity, PPV, NPV) were computed for the 2x2 table with Wilson score intervals.",
        ));
    }
    if s.compute_phi_and_cramers_v {
        runs.push(plain(" Association strength was quantified with phi / Cramer's V."));
    }
    if matches!(s.method, ContingencyMethod::PearsonChiSquare { .. } | ContingencyMethod::LikelihoodRatioG2 | ContingencyMethod::CochranArmitageTrend)
    {
        runs.push(plain(" Asymptotic chi-square adequacy assumes expected cell counts "));
        runs.push(math("E >= 5", "E >= 5", "E \\ge 5"));
        runs.push(plain(" (verified in assumption diagnostics)."));
    }
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_survival(s: &SurvivalAnalysisSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    match s {
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox,
            gehan_breslow_wilcoxon,
            trend_test,
            asymmetrical_greenwood_ci,
            multiplicity,
            mv_time_column,
            mv_status_column,
            mv_group_column,
        } => {
            let ci_mode = if *asymmetrical_greenwood_ci {
                "asymmetrical log-log transformed Greenwood confidence intervals"
            } else {
                "linear Greenwood standard-error confidence intervals"
            };
            let mut tests = Vec::new();
            if *log_rank_mantel_cox {
                tests.push("Mantel-Cox log-rank test");
            }
            if *gehan_breslow_wilcoxon {
                tests.push("Gehan-Breslow-Wilcoxon early-weighted test");
            }
            if *trend_test {
                tests.push("log-rank test for trend");
            }
            let tests_text = if tests.is_empty() { String::from("no inter-cohort hypothesis test selected") } else { tests.join(", ") };

            let mut runs = vec![plain(format!(
                "Time-to-event distributions with right-censoring were estimated by the Kaplan-Meier product-limit method using {ci_mode}. Survival curves were compared via {tests_text}, reporting median survival times, numbers at risk, and hazard ratios (log-rank and Mantel-Haenszel for two cohorts)."
            ))];
            if let (Some(tc), Some(sc)) = (mv_time_column, mv_status_column) {
                let t_lbl = column_title_at(source_ds, *tc);
                let s_lbl = column_title_at(source_ds, *sc);
                let g_lbl = mv_group_column.map_or_else(|| String::from("none"), |gc| column_title_at(source_ds, gc));
                runs.extend([
                    plain(" Extracted from multivariable columns: time="),
                    code(t_lbl),
                    plain(", status="),
                    code(s_lbl),
                    plain(", group="),
                    code(g_lbl),
                    plain("."),
                ]);
            }
            let has_pairwise = res.tabs.iter().any(|t| t.id == "survival-pairwise");
            if matches!(res.status, ResultStatus::UpToDate) && !has_pairwise {
                runs.push(plain(" Fewer than three active cohorts were analyzed, so pairwise log-rank post-hoc comparisons were not computed."));
            } else {
                runs.extend(describe_survival_pairwise_runs(multiplicity, source_ds));
            }
            vec![ReportBlock::Paragraph(runs)]
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, tie_method, .. } => {
            let tie_lbl = match tie_method {
                CoxTieMethod::Breslow => "Breslow",
                CoxTieMethod::Efron => "Efron",
                CoxTieMethod::Exact => "Exact",
            };
            let time_lbl = column_title_at(source_ds, *time_column);
            let status_lbl = column_title_at(source_ds, *status_column);
            let mut runs = vec![
                plain("Semi-parametric Cox proportional hazards regression modeled time "),
                code(time_lbl),
                plain(" and event status "),
                code(status_lbl),
                plain(" across predictors ("),
            ];
            push_code_list(&mut runs, predictor_columns.iter().map(|&c| column_title_at(source_ds, c)));
            runs.push(plain(format!(
                ") using {tie_lbl} tie handling. Reported outputs include regression coefficients (beta), Wald z-statistics, hazard ratios with confidence intervals, Harrell's concordance index (C-index), likelihood-ratio omnibus test, and Schoenfeld residual proportional-hazards tests."
            )));
            vec![ReportBlock::Paragraph(runs)]
        }
    }
}

fn describe_survival_pairwise_runs(mult: &MultiplicitySpec, source_ds: Option<&DataSheet>) -> Vec<TextRun> {
    let thresh = format_exact_float(mult.threshold);
    let mut runs = vec![plain(" For "), math(">= 3", ">= 3", "\\ge 3"), plain(" cohorts, ")];
    match mult.method {
        MultiplicityMethod::None => {
            runs.push(plain("pairwise log-rank comparisons across all cohort pairs were reported without multiplicity adjustment."));
        }
        MultiplicityMethod::FisherLsd => {
            runs.push(plain(
                "pairwise log-rank comparisons across all cohort pairs reported unadjusted Fisher's LSD P values without familywise error control.",
            ));
        }
        MultiplicityMethod::Dunnett { control_column } => {
            let ctrl_lbl = column_title_at(source_ds, control_column);
            runs.push(plain("pairwise log-rank comparisons against control cohort "));
            runs.push(code(ctrl_lbl));
            runs.extend(multiplicity_adjustment_clause(" used Bonferroni FWER adjustment at ", false, &thresh, "."));
        }
        MultiplicityMethod::Bonferroni
        | MultiplicityMethod::Sidak
        | MultiplicityMethod::HolmSidak
        | MultiplicityMethod::BenjaminiHochbergFdr
        | MultiplicityMethod::BenjaminiYekutieliFdr
        | MultiplicityMethod::TwoStageBkyFdr => {
            let eff_spec = MultiplicitySpec { report_confidence_intervals: false, ..*mult };
            runs.push(plain("pairwise log-rank comparisons used "));
            runs.extend(describe_multiplicity_runs(&eff_spec, false));
        }
        MultiplicityMethod::TukeyHsd
        | MultiplicityMethod::NewmanKeuls
        | MultiplicityMethod::GamesHowell
        | MultiplicityMethod::TamhaneT2
        | MultiplicityMethod::DunnettT3
        | MultiplicityMethod::DunnRankSum
        | MultiplicityMethod::LinearTrend => {
            runs.extend(multiplicity_adjustment_clause(
                "pairwise log-rank comparisons across all cohort pairs mapped the ANOVA/rank post-hoc setting to Bonferroni FWER adjustment at ",
                false,
                &thresh,
                ".",
            ));
        }
    }
    runs
}

fn push_code_list<I>(runs: &mut Vec<TextRun>, items: I)
where
    I: IntoIterator<Item = String>,
{
    for (idx, item) in items.into_iter().enumerate() {
        if idx > 0 {
            runs.push(plain(", "));
        }
        runs.push(code(item));
    }
}

fn describe_roc(s: &RocSpec) -> Vec<ReportBlock> {
    let ci_method = match s.ci_method {
        RocCiMethod::DeLong => "DeLong nonparametric variance estimation",
        RocCiMethod::HanleyMcNeil => "Hanley-McNeil binomial standard-error approximation",
    };
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Diagnostic discrimination was quantified via Receiver Operating Characteristic (ROC) curve analysis, computing sensitivity, specificity, Youden's J optimal cutpoint, and area under the curve (AUC) with confidence intervals via {ci_method}."
    ))])]
}

fn describe_glm(s: &GlmSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let family_desc = match s.family {
        GlmFamily::Linear => {
            "ordinary least-squares multiple linear regression (with F-test, adjusted R^2, AICc, Durbin-Watson autocorrelation, and Cook's distance)"
        }
        GlmFamily::Logistic => {
            "multiple logistic regression via iteratively reweighted least squares (reporting log-odds beta, Wald z, odds ratios with CIs, McFadden/Nagelkerke/Tjur pseudo-R^2, Hosmer-Lemeshow goodness of fit, and ROC AUC)"
        }
        GlmFamily::Poisson => {
            "log-link Poisson regression via iteratively reweighted least squares (reporting incidence rate ratios with CIs, deviance, AICc, and Pearson dispersion)"
        }
    };
    let resp_lbl = column_title_at(source_ds, s.response_column);
    let vif_note = if s.compute_vif { " Predictor multicollinearity was evaluated using Variance Inflation Factors (VIF)." } else { "" };
    let mut runs = vec![plain("Multivariable modeling of response "), code(resp_lbl), plain(" on predictors (")];
    push_code_list(&mut runs, s.predictor_columns.iter().map(|&c| column_title_at(source_ds, c)));
    runs.push(plain(format!(") used {family_desc}.{vif_note}")));
    vec![ReportBlock::Paragraph(runs)]
}

fn resolve_mi_effective_settings(res: &ResultSheet, requested_m: usize, requested_iters: usize) -> (usize, usize) {
    let scalar_m = res.lookup_scalar("Imputations (m)", None).filter(|v| v.is_finite() && *v >= 1.0).map(|v| v.round() as usize);
    let scalar_iters = res.lookup_scalar("Max iterations", None).filter(|v| v.is_finite() && *v >= 1.0).map(|v| v.round() as usize);
    if let (Some(m), Some(iters)) = (scalar_m, scalar_iters) {
        return (m, iters);
    }
    if matches!(res.status, ResultStatus::Failed { .. }) {
        return (requested_m, requested_iters);
    }
    (requested_m.clamp(2, 50), requested_iters.clamp(1, 100))
}

fn describe_multivariate(s: &MultivariateSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    match s {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed, pcr_outcome_column } => {
            let matrix_kind =
                if *standardize { "standardized correlation matrix (z-scored variables)" } else { "unstandardized sample covariance matrix" };
            let pa_note = match parallel_analysis_seed {
                Some(seed) => format!(
                    " Component retention combined the Kaiser criterion (eigenvalue >= 1) with Horn's parallel analysis at the 95th percentile (RNG seed {seed})."
                ),
                None => String::from(" Component retention was evaluated using the Kaiser eigenvalue criterion."),
            };
            let prose = if let Some(outcome_col) = pcr_outcome_column {
                let outcome_name = column_title_at(source_ds, *outcome_col);
                format!(
                    "Principal component regression (PCR) decomposed the predictor {matrix_kind} across complete multivariate rows while strictly excluding outcome `{outcome_name}` (column {outcome_col}) to prevent target leakage.{pa_note} Retained orthogonal component scores were then regressed on `{outcome_name}` by ordinary least squares and back-transformed onto the original predictor scale."
                )
            } else {
                format!("Principal component analysis (PCA) decomposed the {matrix_kind} across complete multivariate rows.{pa_note}")
            };
            vec![ReportBlock::Paragraph(vec![plain(prose)])]
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            let corr_kind = if *spearman_rank { "Spearman rank correlation (rho)" } else { "Pearson product-moment correlation (r)" };
            vec![ReportBlock::Paragraph(vec![plain(format!(
                "Pairwise associations were quantified via {corr_kind} using pairwise complete observations, with upper-triangle P values adjusted for false discovery rate via the Benjamini-Hochberg procedure."
            ))])]
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            let dist_desc = match distance {
                ClusterDistance::Euclidean => "Euclidean distance",
                ClusterDistance::Manhattan => "Manhattan (L1) distance",
                ClusterDistance::Pearson => "Pearson correlation dissimilarity (1 - r)",
                ClusterDistance::Cosine => "cosine dissimilarity (1 - cos)",
            };
            let link_desc = match linkage {
                ClusterLinkage::Ward => "Ward.D2 minimum-variance",
                ClusterLinkage::Complete => "complete (furthest-neighbor)",
                ClusterLinkage::Single => "single (nearest-neighbor)",
                ClusterLinkage::Upgma => "UPGMA (unweighted average)",
                ClusterLinkage::Wpgma => "WPGMA (weighted average)",
                ClusterLinkage::Centroid => "UPGMC centroid",
                ClusterLinkage::Median => "WPGMC median",
            };
            let sweep_desc = k_sweep_max.map_or_else(
                || String::from("a candidate-k WCSS elbow and mean-silhouette sweep (k >= 2)"),
                |k_max| format!("a candidate-k WCSS elbow and mean-silhouette sweep up to k = {k_max}"),
            );
            let mode = match k_means_k {
                Some(k) => format!("with k-means++ partitioning into k = {k} clusters alongside {sweep_desc}"),
                None => format!("with k-means++ partitioning selecting the cluster count maximizing mean silhouette across {sweep_desc}"),
            };
            vec![ReportBlock::Paragraph(vec![plain(format!(
                "Unsupervised multivariate clustering was performed on complete observations using {dist_desc} and {link_desc} hierarchical agglomerative linkage (with cophenetic correlation), paired {mode}."
            ))])]
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, seed, response_column, predictor_columns } => {
            let (eff_m, eff_iters) = resolve_mi_effective_settings(res, *imputations_m, *max_iters);
            let clamp_note = if eff_m != *imputations_m || eff_iters != *max_iters {
                format!(" (clamped from requested m = {imputations_m}, max_iters = {max_iters})")
            } else {
                String::new()
            };
            let mut runs = vec![plain(format!(
                "Missing data were handled by Multiple Imputation by Chained Equations (m = {eff_m} imputed datasets, max_iters = {eff_iters}, RNG seed = {seed}{clamp_note}). Estimates were combined using Rubin's (1987) pooling rules (within-imputation variance U, between-imputation variance B, total variance T, relative increase in variance RIV, fraction of missing information FMI, and Barnard-Rubin adjusted degrees of freedom)."
            ))];
            if let Some(resp) = response_column {
                let r_lbl = column_title_at(source_ds, *resp);
                runs.push(plain(" Downstream pooled regression modeled "));
                runs.push(code(r_lbl));
                runs.push(plain(" on ("));
                push_code_list(&mut runs, predictor_columns.iter().map(|&c| column_title_at(source_ds, c)));
                runs.push(plain(")."));
            }
            vec![ReportBlock::Paragraph(runs)]
        }
    }
}

fn describe_spline_lowess(s: &SplineLowessSpec) -> Vec<ReportBlock> {
    let method_text = match s.method {
        SplineLowessMethod::NaturalCubicSpline => String::from(
            "A natural cubic spline (de Boor 1978; C^2 piecewise cubic with zero endpoint second derivatives S''(X_min) = S''(X_max) = 0) was fitted through the sorted distinct (X, mean Y) knots.",
        ),
        SplineLowessMethod::AkimaSubspline => String::from(
            "An Akima subspline (Akima 1970; C^1 piecewise cubic using five-point secant slope weighting to suppress overshoot around sharp transitions) was fitted through the sorted distinct (X, mean Y) knots.",
        ),
        SplineLowessMethod::SmoothingSpline => {
            let param = if let Some(df) = s.smoothing_df {
                format!(" with target effective degrees of freedom df = {df:.3}")
            } else if let Some(knots) = s.smoothing_knots {
                format!(" parameterized by {knots} effective knots")
            } else {
                String::from(" with automatic effective degrees of freedom")
            };
            format!(
                "A penalized cubic smoothing spline (Reinsch 1967; Hutchinson & de Hoog 1985) minimizing weighted RSS plus curvature penalty was fitted{param}."
            )
        }
        SplineLowessMethod::Lowess => {
            let frac = s.effective_lowess_fraction();
            let preset_note = if s.lowess_fraction.is_some() {
                format!("custom span fraction f = {frac:.3}")
            } else {
                format!("{} preset (f = {frac:.2})", s.lowess_preset.label())
            };
            format!(
                "Cleveland (1979) LOWESS tricube-weighted local linear regression was fitted using the {preset_note} and {} Tukey bisquare robustness pass(es), enforcing at least 4*N sampled polyline vertices.",
                s.lowess_robustness_iters
            )
        }
        SplineLowessMethod::PointToPoint => String::from(
            "A dense data-space piecewise-linear point-to-point curve was resampled onto polyline vertices (preserving linear data-space trajectories on logarithmic axes).",
        ),
    };
    let mut extras = Vec::new();
    extras.push(format!("Curve overlay resolution was configured to {} vertices.", s.segment_count));
    if s.emit_curve_coordinates_tab {
        extras.push(String::from("Sampled (X, Y) curve coordinates were emitted as an auxiliary result tab."));
    }
    if s.interpolate_unknowns {
        extras.push(String::from(
            "Nonparametric standard-curve interpolation evaluated S(X) for (X, blank Y) rows and solved S(X) = Y for (blank X, Y) rows.",
        ));
    }
    vec![ReportBlock::Paragraph(vec![plain(format!("{method_text} {}", extras.join(" ")))])]
}

fn describe_linear_regression(s: &LinearRegressionSpec) -> Vec<ReportBlock> {
    let range = s.x_range_filter.filter(|(lo, hi)| lo.is_some() || hi.is_some()).map_or_else(String::new, |(lo, hi)| {
        format!(" The fit included observations within X bounds [{}, {}].", opt_bound_str(lo), opt_bound_str(hi))
    });
    if s.deming_model_ii {
        let error_desc = match s.deming_error {
            curvebook_doc::DemingErrorSpec::EqualError => String::from("assuming equal X and Y error variances (lambda = 1)"),
            curvebook_doc::DemingErrorSpec::StandardDeviations { sd_x, sd_y } => {
                format!("with specified error SDs (SD_X = {sd_x}, SD_Y = {sd_y})")
            }
            curvebook_doc::DemingErrorSpec::VarianceRatio { lambda } => {
                format!("with error variance ratio lambda = {lambda}")
            }
        };
        return vec![ReportBlock::Paragraph(vec![plain(format!(
            "Model II Deming linear regression was fitted {error_desc} to account for measurement error in both X and Y variables, with standard errors and confidence intervals computed by leave-one-out jackknife resampling.{range}"
        ))])];
    }
    let constraint = match s.effective_fixed_point() {
        Some((0.0, 0.0)) => String::from("constrained through the origin (Y = Slope * X)"),
        Some((x0, y0)) => format!("constrained through fixed point ({x0}, {y0}) (Y - {y0} = Slope * (X - {x0}))"),
        None => String::from("with free Y-intercept (Y = Slope * X + YIntercept)"),
    };
    let mut extras = Vec::new();
    if s.confidence_bands {
        extras.push("confidence bands");
    }
    if s.prediction_bands {
        extras.push("prediction bands");
    }
    if s.interpolate_unknowns {
        extras.push("standard-curve inverse/forward unknown interpolation");
    }
    if s.standard_addition {
        extras.push("standard-addition unknown concentration (-X-intercept) with Fieller CI");
    }
    if s.extend_line_to_x_intercept {
        extras.push("fitted line extended to X-intercept");
    }
    let extra_str = if extras.is_empty() { String::new() } else { format!(" Enabled outputs: {}.", extras.join(", ")) };
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Ordinary least-squares simple linear regression was fitted {constraint}, including a runs test for departure from linearity and ANCOVA comparison of slopes and intercepts when multiple datasets are present.{extra_str}{range}"
    ))])]
}

fn describe_simple_logistic(s: &SimpleLogisticSpec) -> Vec<ReportBlock> {
    let mut extras = Vec::new();
    if s.confidence_bands {
        extras.push("sigmoid probability confidence bands");
    }
    if s.emit_row_classification {
        extras.push("row-level classification and automatic prediction for blank-Y rows");
    }
    let extra_str = if extras.is_empty() { String::new() } else { format!(" Enabled outputs: {}.", extras.join(", ")) };
    let range_str = match (s.plot_x_min, s.plot_x_max) {
        (Some(lo), Some(hi)) => format!(" The fitted sigmoid curve was evaluated over X in [{lo}, {hi}]."),
        (Some(lo), None) => format!(" The fitted sigmoid curve started at X = {lo}."),
        (None, Some(hi)) => format!(" The fitted sigmoid curve ended at X = {hi}."),
        (None, None) => String::new(),
    };
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Standalone simple binary logistic regression (logit P(Y=1 | X) = beta_0 + beta_1 * X) was fitted by Newton-Raphson / Fisher scoring IRLS maximum likelihood on single-subcolumn XY data with Y in {{0, 1}}. Parameter inference included asymptotic standard errors, Wald Z-tests, likelihood-ratio G^2 test against the intercept-only null, asymmetrical profile-likelihood confidence intervals (including odds ratios and X at 50%), deviance, AICc, Tjur's R^2, Cox-Snell R^2, DeLong ROC AUC, classification at cutoff {}, and explicit collinearity / separation diagnostics (Hosmer, Lemeshow & Sturdivant 2013; Tjur 2009; Heinze & Schemper 2002).{extra_str}{range_str}",
        s.classification_cutoff
    ))])]
}

struct PartitionedParameterRules<'a> {
    applied: Vec<(&'a str, &'a ParameterRule)>,
    ignored_unknown: usize,
    ignored_duplicate: usize,
}

fn partition_parameter_rules<'a>(rules: &'a [ParameterRule], known_params: &'a [String]) -> PartitionedParameterRules<'a> {
    let mut applied: Vec<(&'a str, &'a ParameterRule)> = Vec::new();
    let mut seen_canonical: Vec<String> = Vec::new();
    let mut ignored_unknown = 0_usize;
    let mut ignored_duplicate = 0_usize;

    if known_params.is_empty() {
        for rule in rules {
            if seen_canonical.iter().any(|prev| prev.eq_ignore_ascii_case(&rule.name)) {
                ignored_duplicate += 1;
            } else {
                seen_canonical.push(rule.name.to_owned());
                applied.push((rule.name.as_str(), rule));
            }
        }
        return PartitionedParameterRules { applied, ignored_unknown, ignored_duplicate };
    }

    for rule in rules {
        let Some(canonical) = known_params.iter().find(|p| p.eq_ignore_ascii_case(&rule.name)) else {
            ignored_unknown += 1;
            continue;
        };
        if seen_canonical.iter().any(|prev| prev.eq_ignore_ascii_case(canonical)) {
            ignored_duplicate += 1;
            continue;
        }
        seen_canonical.push(canonical.clone());
        applied.push((canonical.as_str(), rule));
    }
    PartitionedParameterRules { applied, ignored_unknown, ignored_duplicate }
}

fn describe_nonlinear_regression(s: &NonlinearFitSpec, res: &ResultSheet) -> Vec<ReportBlock> {
    let mut blocks = Vec::new();

    // 1. Model identity and mathematical equation formula
    let (model_name, model_params, formula_plain, formula_latex) = resolve_equation_description(s);
    blocks.push(ReportBlock::Paragraph(bullet_kv(
        "Model and equation",
        [
            plain(format!("{model_name} (")),
            code(&s.equation_id),
            plain("). Equation: "),
            math(formula_plain.clone(), formula_plain, formula_latex),
            plain(if model_params.is_empty() { String::new() } else { format!("; parameters: {}.", model_params.join(", ")) }),
        ],
    )));

    // 2. Objective function, ROUT outlier settings, weighting, and iterations
    let mut obj_runs = Vec::new();
    let outlier_handling = s.effective_outlier_handling();
    match s.method {
        FitMethod::LeastSquares => {
            obj_runs.push(plain(
                "Parameters were estimated by trust-region Levenberg-Marquardt nonlinear regression (with Nelder-Mead simplex warm-start)",
            ));
            if outlier_handling == OutlierHandling::IdentifyAndCountOnly {
                obj_runs.push(plain(
                    "; ROUT outlier detection (Q = 1%) identified and counted outliers without excluding them from the least-squares fit",
                ));
            } else if outlier_handling == OutlierHandling::Eliminate {
                obj_runs.push(plain("; ROUT outlier detection (Q = 1%) identified and excluded outliers before least-squares refitting"));
            }
        }
        FitMethod::PoissonDeviance => {
            obj_runs.push(plain("Parameters were estimated by maximum-likelihood Poisson deviance iteratively reweighted least squares (IRLS)"));
        }
        FitMethod::RobustCauchy => {
            obj_runs.push(plain(
                "Parameters were estimated by robust nonlinear regression using Cauchy-loss IRLS weighting to down-weight extreme residuals",
            ));
        }
        FitMethod::RoutOutlierElimination { fdr_q_percent } => {
            let q_str = format_exact_float(fdr_q_percent);
            if outlier_handling == OutlierHandling::IdentifyAndCountOnly {
                obj_runs.push(plain(
                    "Outliers were identified and counted (retained in the least-squares fit) using the ROUT method (robust Cauchy fit, RSDR scale, Benjamini-Hochberg FDR control at ",
                ));
                obj_runs.push(math(format!("Q = {q_str}%"), format!("Q = {q_str}%"), format!("Q = {q_str}\\%")));
                obj_runs.push(plain(")"));
            } else {
                obj_runs.push(plain(
                    "Outliers were identified and excluded using the ROUT method (robust Cauchy fit, RSDR scale, Benjamini-Hochberg FDR control at ",
                ));
                obj_runs.push(math(format!("Q = {q_str}%"), format!("Q = {q_str}%"), format!("Q = {q_str}\\%")));
                obj_runs.push(plain("), followed by ordinary/weighted least-squares refitting on the retained observations"));
            }
        }
    }
    let strictness_note = match s.convergence_strictness {
        ConvergenceStrictness::Default => String::new(),
        ConvergenceStrictness::Quick => String::from(", convergence strictness = Quick (2 consecutive iterations < 0.01% SS change)"),
        ConvergenceStrictness::Medium => String::from(", convergence strictness = Medium (5 consecutive iterations < 0.0001% SS change)"),
        ConvergenceStrictness::Strict => String::from(", convergence strictness = Strict (5 consecutive iterations < 1e-8% SS change)"),
    };
    obj_runs.push(plain(format!(" (maximum iterations = {}{strictness_note}). ", s.clamped_max_iters())));
    obj_runs.extend(describe_weighting_runs(s.weighting, s.weighting_source));
    if s.replicate_fit_mode == ReplicateFitMode::RowMeansOnly {
        obj_runs.push(plain(" Replicates within each row were collapsed to unweighted row means (1 observation per row)."));
    }
    if s.emit_cleaned_data_tab {
        obj_runs.push(plain(" An analyzable Cleaned data table excluding ROUT-identified outliers was emitted."));
    }

    // Confidence interval & parametric inference availability
    if s.method == FitMethod::RobustCauchy {
        obj_runs.push(plain(
            " In Robust (Cauchy) regression mode, least-squares standard errors, parameter confidence intervals, covariance matrices, confidence/prediction bands, AICc/BIC, and Extra-SS F / AICc / LRT model comparisons are not applicable (NotApplicableInRobustMode); best-fit parameter estimates, residuals, and RSDR are reported.",
        ));
    } else {
        let conf_pct = format_exact_float(res.analysis.confidence_level.percent());
        if s.profile_likelihood_ci {
            obj_runs.push(plain(" Asymmetrical "));
            obj_runs.push(math(format!("{conf_pct}%"), format!("{conf_pct}%"), format!("{conf_pct}\\%")));
            obj_runs.push(plain(
                " confidence intervals were computed by Venzon-Moolgavkar profile-likelihood root finding (supporting open/unbounded bounds on flat likelihood plateaus).",
            ));
        } else {
            obj_runs.push(plain(" Symmetric asymptotic "));
            obj_runs.push(math(format!("{conf_pct}%"), format!("{conf_pct}%"), format!("{conf_pct}\\%")));
            obj_runs.push(plain(" confidence intervals were computed from the Jacobian covariance matrix and Student's t critical value."));
        }
    }
    blocks.push(ReportBlock::Paragraph(obj_runs));

    // 3. Parameter constraints, bounds, and shared global parameters
    if !s.parameter_rules.is_empty() {
        let part = partition_parameter_rules(&s.parameter_rules, &model_params);
        if !part.applied.is_empty() {
            let rule_bullets: Vec<Vec<TextRun>> = part.applied.iter().map(|&(cname, r)| describe_parameter_rule(cname, r)).collect();
            blocks.push(ReportBlock::Paragraph(vec![bold("Parameter constraints and global sharing rules:")]));
            blocks.push(ReportBlock::Bullets(rule_bullets));
        }
        if part.ignored_unknown > 0 || part.ignored_duplicate > 0 {
            let status_note = if matches!(res.status, ResultStatus::Failed { .. }) {
                format!(
                    "Parameter-rule validation rejected {} unrecognized and {} duplicate parameter rule(s).",
                    part.ignored_unknown, part.ignored_duplicate
                )
            } else {
                format!(
                    "Ignored {} unrecognized and {} superseded duplicate parameter rule(s) (case-insensitive first match per model parameter applies).",
                    part.ignored_unknown, part.ignored_duplicate
                )
            };
            blocks.push(ReportBlock::Paragraph(vec![plain(status_note)]));
        }
    }

    // 4. Model comparison and residual / range diagnostics
    let mut extras = Vec::new();
    append_nonlinear_identifiability_and_flags(s, &mut extras);
    if let Some((method, question)) = s.effective_comparison() {
        if s.method == FitMethod::RobustCauchy {
            extras.push(vec![plain(
                "Model comparison was marked Not applicable (NotApplicableInRobustMode) because Robust (Cauchy) regression does not support least-squares F, AICc, or likelihood-ratio tests.",
            )]);
        } else {
            extras.push(describe_model_comparison_question_runs(method, &question, s.prefer_well_defined_fit));
        }
    }
    if let Some((x_lo, x_hi)) = s.x_range_filter {
        extras.push(vec![plain(format!("X range filter: [{} , {}].", opt_bound_str(x_lo), opt_bound_str(x_hi)))]);
    }
    if let Some((y_lo, y_hi)) = s.y_range_filter {
        extras.push(vec![plain(format!("Y range filter: [{} , {}].", opt_bound_str(y_lo), opt_bound_str(y_hi)))]);
    }
    let mut checks = Vec::new();
    if s.replicates_lack_of_fit_test && s.method != FitMethod::RobustCauchy {
        checks.push("replicates lack-of-fit F-test");
    }
    if s.normality_of_residuals {
        checks.push("residual normality battery (D'Agostino-Pearson, Shapiro-Wilk, Anderson-Darling, Lilliefors KS)");
    }
    if s.homoscedasticity_test {
        checks.push("Spearman homoscedasticity test");
    }
    if s.interpolate_unknowns {
        if s.method == FitMethod::RobustCauchy {
            checks.push("unknown point interpolation via Brent root finding (confidence intervals not applicable in Robust mode)");
        } else {
            checks.push("unknown interpolation via Brent root finding");
        }
    }
    if !checks.is_empty() {
        extras.push(vec![plain(format!("Diagnostic checks enabled: {}.", checks.join("; ")))]);
    }
    if !extras.is_empty() {
        blocks.push(ReportBlock::Bullets(extras));
    }

    blocks
}

fn describe_weighting_runs(weighting: WeightingScheme, source: WeightingSource) -> Vec<TextRun> {
    let predicted = source == WeightingSource::PredictedY;
    match weighting {
        WeightingScheme::None => vec![plain("Equal weighting (unweighted, assuming uniform residual SD) was used.")],
        WeightingScheme::RelativeSquare if predicted => vec![
            plain("Observations were weighted by curve-predicted IRLS "),
            math("1 / Y_curve^2", "1 / Y_curve^2", "1 / \\hat{Y}^2"),
            plain(" (Carroll & Ruppert 1988; starting from an unweighted pass)."),
        ],
        WeightingScheme::RelativeSquare => vec![
            plain("Observations were weighted by "),
            math("1 / Y^2", "1 / Y^2", "1 / Y^2"),
            plain(" (minimizing relative squared residuals for constant %CV scatter)."),
        ],
        WeightingScheme::Relative if predicted => vec![
            plain("Observations were weighted by curve-predicted IRLS "),
            math("1 / |Y_curve|", "1 / |Y_curve|", "1 / |\\hat{Y}|"),
            plain(" (Carroll & Ruppert 1988; starting from an unweighted pass)."),
        ],
        WeightingScheme::Relative => vec![
            plain("Observations were weighted by "),
            math("1 / |Y|", "1 / |Y|", "1 / |Y|"),
            plain(" (Poisson-like variance proportional to |Y|)."),
        ],
        WeightingScheme::GeneralPowerY(k) if predicted => {
            let ks = format_exact_float(k);
            vec![
                plain("Observations were weighted by curve-predicted IRLS "),
                math(format!("1 / |Y_curve|^{ks}"), format!("1 / |Y_curve|^{ks}"), format!("1 / |\\hat{{Y}}|^{{{ks}}}")),
                plain(" (Carroll & Ruppert 1988)."),
            ]
        }
        WeightingScheme::GeneralPowerY(k) => {
            let ks = format_exact_float(k);
            vec![
                plain("Observations were weighted by "),
                math(format!("1 / |Y|^{ks}"), format!("1 / |Y|^{ks}"), format!("1 / |Y|^{{{ks}}}")),
                plain("."),
            ]
        }
        WeightingScheme::InverseX => vec![plain("Observations were weighted by "), math("1 / |X|", "1 / |X|", "1 / |X|"), plain(".")],
        WeightingScheme::InverseX2 => vec![plain("Observations were weighted by "), math("1 / X^2", "1 / X^2", "1 / X^2"), plain(".")],
        WeightingScheme::ObservedSd => vec![
            plain("Observations were weighted by "),
            math("1 / SD^2", "1 / SD^2", "1 / \\mathrm{SD}^2"),
            plain(" from observed replicate scatter."),
        ],
    }
}

fn describe_parameter_rule(canonical_name: &str, rule: &ParameterRule) -> Vec<TextRun> {
    let scope_desc = match &rule.scope {
        ParameterScope::Local => String::from("fitted independently per dataset (local)"),
        ParameterScope::SharedGlobal => String::from("shared globally across all datasets"),
        ParameterScope::Constant(v) => format!("constrained to fixed constant {}", format_exact_float(*v)),
        ParameterScope::HookedConstant(hook) => format!("constrained to hooked constant ({})", format_hook_source(hook)),
        ParameterScope::PerDatasetConstant(pairs) => {
            let parts: Vec<String> = pairs.iter().map(|(ds, v)| format!("DS{} = {}", ds + 1, format_exact_float(*v))).collect();
            format!("constrained per dataset ({})", parts.join(", "))
        }
        ParameterScope::ColumnTitleConstant => String::from("constrained to numeric value parsed from dataset column title"),
    };
    let mut runs = vec![code(canonical_name), plain(format!(": {scope_desc}"))];
    if let Some(lo) = rule.lower_bound {
        let ls = format_exact_float(lo);
        runs.push(plain("; lower bound "));
        runs.push(math(format!(">= {ls}"), format!(">= {ls}"), format!("\\ge {ls}")));
    }
    if let Some(hi) = rule.upper_bound {
        let hs = format_exact_float(hi);
        runs.push(plain("; upper bound "));
        runs.push(math(format!("<= {hs}"), format!("<= {hs}"), format!("\\le {hs}")));
    }
    if let Some(init) = rule.initial_override {
        runs.push(plain(format!("; initial estimate override = {}", format_exact_float(init))));
    }
    runs
}

fn append_nonlinear_identifiability_and_flags(s: &NonlinearFitSpec, extras: &mut Vec<Vec<TextRun>>) {
    if let Some(mode) = s.effective_identifiability_mode() {
        let mode_desc = match mode {
            IdentifiabilityMode::IdentifyUnstableParameters => {
                "Parameter identifiability used greedy Hessian condition-number reduction (kappa(H) > 10 / sqrt(eps_mach)), marking unstable parameters while preserving well-conditioned estimates and intervals."
            }
            IdentifiabilityMode::IdentifyAmbiguousFits => {
                "Parameter identifiability used the dependency criterion (Dependency > 0.9999 or rank-deficient Jacobian), flagging ambiguous fits with `~` prefixes."
            }
            IdentifiabilityMode::ReportAll => "Parameter identifiability reporting was set to ReportAll (unconditional raw parameter reporting).",
        };
        extras.push(vec![plain(mode_desc)]);
    }
    if let Some(rules) = &s.fit_flag_rules
        && rules.has_any_rule()
    {
        let mut parts = Vec::new();
        if let Some(v) = rules.min_r_squared {
            parts.push(format!("R^2 < {}", format_exact_float(v)));
        }
        if let Some(v) = rules.min_observations {
            parts.push(format!("N < {v}"));
        }
        if let Some(v) = rules.max_dependency {
            parts.push(format!("max dependency > {}", format_exact_float(v)));
        }
        if let Some(v) = rules.min_normality_p {
            parts.push(format!("normality p < {}", format_exact_float(v)));
        }
        if let Some(v) = rules.min_runs_or_lof_p {
            parts.push(format!("runs/lack-of-fit p < {}", format_exact_float(v)));
        }
        if let Some(v) = rules.max_rout_outliers {
            parts.push(format!("ROUT outliers > {v}"));
        }
        extras.push(vec![plain(format!("Automated fit-quality flag rules: {}.", parts.join(", ")))]);
    }
    if let Some(param) = s.summary_parameter.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        extras.push(vec![plain("Parameter summary table emitted for "), code(param), plain(".")]);
    }
    if let Some(digits) = s.output_significant_digits {
        extras.push(vec![plain(format!("Output formatting precision: {digits} significant digits."))]);
    }
}

fn describe_model_comparison_question_runs(
    method: ModelComparisonMethod,
    question: &ModelComparisonQuestion,
    prefer_well_defined_fit: bool,
) -> Vec<TextRun> {
    let mut runs = match question {
        ModelComparisonQuestion::AlternativeEquation { alternative_equation_id } => {
            let legacy = match method {
                ModelComparisonMethod::ExtraSumOfSquaresF => {
                    ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alternative_equation_id.clone() }
                }
                ModelComparisonMethod::Aicc => ModelComparisonSpec::Aicc { alternative_equation_id: alternative_equation_id.clone() },
                ModelComparisonMethod::LikelihoodRatioTest => {
                    ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: alternative_equation_id.clone() }
                }
            };
            describe_model_comparison_runs(&legacy)
        }
        ModelComparisonQuestion::ParameterInvarianceAcrossDatasets { shared_params } => {
            let m_txt = match method {
                ModelComparisonMethod::ExtraSumOfSquaresF => "the extra-sum-of-squares F-test",
                ModelComparisonMethod::Aicc => "AICc",
                ModelComparisonMethod::LikelihoodRatioTest => "the likelihood-ratio test",
            };
            vec![plain(format!(
                "Model comparison tested whether {} are invariant (shared globally) across datasets versus separate per dataset using {m_txt}.",
                shared_params.join(", ")
            ))]
        }
        ModelComparisonQuestion::SingleGlobalCurveVsSeparate => {
            let m_txt = match method {
                ModelComparisonMethod::ExtraSumOfSquaresF => "the extra-sum-of-squares F-test",
                ModelComparisonMethod::Aicc => "AICc",
                ModelComparisonMethod::LikelihoodRatioTest => "the likelihood-ratio test",
            };
            vec![plain(format!(
                "Model comparison tested whether one global curve fits all datasets versus separate per-dataset curves using {m_txt}."
            ))]
        }
        ModelComparisonQuestion::ParameterEqualsTheoretical { param, theoretical_value } => {
            let m_txt = match method {
                ModelComparisonMethod::ExtraSumOfSquaresF => "the extra-sum-of-squares F-test",
                ModelComparisonMethod::Aicc => "AICc",
                ModelComparisonMethod::LikelihoodRatioTest => "the likelihood-ratio test",
            };
            vec![plain(format!(
                "Model comparison tested whether `{param}` equals theoretical value {} versus an unconstrained estimate using {m_txt}.",
                format_exact_float(*theoretical_value)
            ))]
        }
    };
    if prefer_well_defined_fit {
        runs.push(plain(
            " Well-defined fit pre-check enabled (automatically prefers a clean fit over an Ambiguous, Unstable, or Flagged candidate).",
        ));
    }
    runs
}

fn describe_model_comparison_runs(cmp: &ModelComparisonSpec) -> Vec<TextRun> {
    match cmp {
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id } => {
            let alt_name = get_builtin_model(alternative_equation_id).map_or_else(|| alternative_equation_id.clone(), |m| m.name().to_owned());
            vec![
                plain("Nested model selection compared the primary model against "),
                code(alt_name),
                plain(" ("),
                code(alternative_equation_id),
                plain(") using the extra-sum-of-squares F-test."),
            ]
        }
        ModelComparisonSpec::Aicc { alternative_equation_id } => {
            let alt_name = get_builtin_model(alternative_equation_id).map_or_else(|| alternative_equation_id.clone(), |m| m.name().to_owned());
            vec![
                plain("Model selection compared the primary model against "),
                code(alt_name),
                plain(" ("),
                code(alternative_equation_id),
                plain(") via "),
                math("AICc", "AICc", "\\mathrm{AIC}_c"),
                plain(", Akaike weights, and evidence ratio."),
            ]
        }
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id } => {
            let alt_name = get_builtin_model(alternative_equation_id).map_or_else(|| alternative_equation_id.clone(), |m| m.name().to_owned());
            vec![
                plain("Nested model selection compared the primary model against "),
                code(alt_name),
                plain(" ("),
                code(alternative_equation_id),
                plain(") using the likelihood-ratio test (D = 2 * delta ln L)."),
            ]
        }
    }
}

fn describe_descriptive(s: &DescriptiveSpec) -> Vec<ReportBlock> {
    let agg = match s.aggregation {
        SubcolumnAggregation::RowMeans => "row replicate means",
        SubcolumnAggregation::PerSubcolumn => "individual replicate subcolumns",
        SubcolumnAggregation::PooledGrandSample => "pooled replicate observations",
    };
    let norm = if s.run_normality_battery {
        " Normality and lognormality were evaluated using D'Agostino-Pearson, Shapiro-Wilk, Kolmogorov-Smirnov (Lilliefors), and Anderson-Darling tests."
    } else {
        ""
    };
    let hypo = s.hypothetical_value.map_or_else(String::new, |v| {
        format!(" One-sample t and Wilcoxon tests evaluated departures from hypothetical reference {}.", format_exact_float(v))
    });
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Descriptive statistics (mean, SD, SEM, median, IQR, range, geometric mean, and confidence intervals) summarized {agg}.{norm}{hypo}"
    ))])]
}

fn describe_outlier_detection(s: &OutlierSpec) -> Vec<ReportBlock> {
    let text = match s {
        OutlierSpec::Grubbs { alpha, iterative } => {
            let mode = if *iterative { "iterative ESD / Grubbs" } else { "single-pass Grubbs" };
            format!("Outliers were screened using {mode} extreme studentized deviate testing at alpha = {}.", format_exact_float(*alpha))
        }
        OutlierSpec::Rout { fdr_q_percent } => {
            format!(
                "Outliers were identified via ROUT (robust scale RSDR with Benjamini-Hochberg false discovery control at Q = {}%).",
                format_exact_float(*fdr_q_percent)
            )
        }
    };
    vec![ReportBlock::Paragraph(vec![plain(text)])]
}

fn describe_transform(s: &TransformAnalysisSpec) -> Vec<ReportBlock> {
    let runs = match s {
        TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation } => {
            let x_str = x_transform.map_or_else(|| String::from("none"), |p| format_math_preset(p, "X"));
            let y_str = y_transform.map_or_else(|| String::from("none"), |p| format_math_preset(p, "Y"));
            let err_str = match error_propagation {
                ErrorPropagationMode::ReplicateEndpoints => "replicate endpoint propagation",
                ErrorPropagationMode::FirstOrderDeltaMethod => "first-order Taylor delta method",
            };
            vec![plain(format!("Mathematical table transformation (X: {x_str}; Y: {y_str}) with {err_str} for summary error columns."))]
        }
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set } => {
            let scale = if *output_percent { "0% to 100%" } else { "0.0 to 1.0" };
            let scope = if *per_data_set { "per dataset" } else { "globally across datasets" };
            vec![plain(format!(
                "Data normalization onto [{scale}] {scope} anchored at 0={} and 100={}.",
                format_anchor(zero_anchor),
                format_anchor(hundred_anchor)
            ))]
        }
        TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation } => {
            let op_lbl = match operation {
                BaselineOperation::Difference => "difference",
                BaselineOperation::Ratio => "ratio",
                BaselineOperation::PercentChange => "percent change",
                BaselineOperation::FractionalChange => "fractional change",
            };
            vec![plain(format!("Baseline removal ({op_lbl}) relative to baseline column indices {baseline_columns:?}."))]
        }
        TransformAnalysisSpec::Transpose { swap_row_and_column_titles } => {
            vec![plain(format!("Matrix transposition (swap row/column titles = {swap_row_and_column_titles})."))]
        }
        TransformAnalysisSpec::PruneRows { every_k_rows, average_window } => {
            vec![plain(format!("Periodic row pruning every {every_k_rows} row(s) (average_window = {average_window})."))]
        }
        TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns } => {
            let flt = filter_formula.as_deref().unwrap_or("none");
            if derived_columns.is_empty() {
                vec![plain(format!("Subset selection and archetype reshape into {} (filter: {flt}).", target_archetype.label()))]
            } else {
                let names: Vec<&str> = derived_columns.iter().map(|d| d.title.as_str()).collect();
                vec![plain(format!(
                    "Subset selection, derived column transforms ({}), and archetype reshape into {} (filter: {flt}).",
                    names.join(", "),
                    target_archetype.label()
                ))]
            }
        }
        TransformAnalysisSpec::Simulate { seed, row_count, replicates, x_min, x_max, equation_id, noise, .. } => {
            let noise_str = match noise {
                SimulationNoise::None => String::from("noise-free"),
                SimulationNoise::Gaussian { sd } => format!("Gaussian noise (SD = {})", format_exact_float(*sd)),
                SimulationNoise::RelativePercent { cv_percent } => format!("relative noise (%CV = {})", format_exact_float(*cv_percent)),
                SimulationNoise::Poisson => String::from("Poisson count noise"),
            };
            vec![
                plain("Monte Carlo curve simulation for "),
                code(equation_id),
                plain(format!(
                    " ({row_count} rows x {replicates} replicates, X in [{}, {}], {noise_str}, RNG seed = {seed}).",
                    format_exact_float(*x_min),
                    format_exact_float(*x_max)
                )),
            ]
        }
        TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation } => {
            let strat_desc = match stratify_column {
                Some(col_idx) => {
                    let alloc = if *proportional_allocation { "proportional" } else { "equal-quota" };
                    format!(" stratified by column {} using {alloc} allocation", col_idx + 1)
                }
                None => String::from(" without stratification"),
            };
            vec![plain(format!(
                "Seeded deterministic Fisher-Yates random sampling without replacement selected n = {sample_size} rows{strat_desc} (RNG seed = {seed})."
            ))]
        }
    };
    vec![ReportBlock::Paragraph(runs)]
}

fn result_has_posthoc_ci(res: &ResultSheet) -> bool {
    if !matches!(res.status, ResultStatus::UpToDate) {
        return true;
    }
    let mc_tabs: Vec<&ResultTab> = res.tabs.iter().filter(|t| t.kind == ResultTabKind::MultipleComparisons).collect();
    if mc_tabs.is_empty() {
        return true;
    }
    mc_tabs.iter().any(|tab| tab.rows.iter().any(|row| row.iter().any(|cell| matches!(cell, ResultCell::ConfidenceInterval { .. }))))
}

fn describe_multiplicity_runs(mult: &MultiplicitySpec, ci_available: bool) -> Vec<TextRun> {
    let thresh = format_exact_float(mult.threshold);
    let ci_note = if mult.report_confidence_intervals && ci_available {
        match mult.method {
            MultiplicityMethod::TukeyHsd
            | MultiplicityMethod::Dunnett { .. }
            | MultiplicityMethod::Sidak
            | MultiplicityMethod::Bonferroni
            | MultiplicityMethod::DunnettT3
            | MultiplicityMethod::GamesHowell
            | MultiplicityMethod::TamhaneT2 => " with multiplicity-adjusted confidence intervals",
            MultiplicityMethod::FisherLsd | MultiplicityMethod::None => " with unadjusted confidence intervals",
            MultiplicityMethod::HolmSidak
            | MultiplicityMethod::NewmanKeuls
            | MultiplicityMethod::BenjaminiHochbergFdr
            | MultiplicityMethod::BenjaminiYekutieliFdr
            | MultiplicityMethod::TwoStageBkyFdr
            | MultiplicityMethod::DunnRankSum
            | MultiplicityMethod::LinearTrend => "",
        }
    } else {
        ""
    };
    let (kind_lbl, is_fdr) = match mult.method {
        MultiplicityMethod::None => ("no post-hoc multiplicity correction (unadjusted pairwise comparisons)", false),
        MultiplicityMethod::TukeyHsd => ("Tukey's honestly significant difference (HSD) test across all pairs", false),
        MultiplicityMethod::Dunnett { .. } => ("Dunnett's many-to-one comparison against the designated control group", false),
        MultiplicityMethod::Sidak => ("Sidak FWER correction", false),
        MultiplicityMethod::Bonferroni => ("Bonferroni FWER correction", false),
        MultiplicityMethod::HolmSidak => ("Holm-Sidak step-down FWER correction", false),
        MultiplicityMethod::DunnettT3 => ("Dunnett's T3 unequal-variance pairwise correction", false),
        MultiplicityMethod::GamesHowell => ("Games-Howell unequal-variance pairwise correction", false),
        MultiplicityMethod::TamhaneT2 => ("Tamhane's T2 unequal-variance pairwise correction", false),
        MultiplicityMethod::DunnRankSum => ("Dunn's nonparametric rank-sum post-hoc procedure", false),
        MultiplicityMethod::LinearTrend => ("a post-hoc test for linear trend across ordered group means", false),
        MultiplicityMethod::BenjaminiHochbergFdr => ("Benjamini-Hochberg step-up false discovery rate (FDR) control", true),
        MultiplicityMethod::BenjaminiYekutieliFdr => ("Benjamini-Yekutieli arbitrary-dependence FDR control", true),
        MultiplicityMethod::TwoStageBkyFdr => ("two-stage Benjamini-Krieger-Yekutieli adaptive FDR control", true),
        MultiplicityMethod::NewmanKeuls => ("Newman-Keuls studentized range step-down test", false),
        MultiplicityMethod::FisherLsd => ("Fisher's unprotected least significant difference (LSD)", false),
    };
    if matches!(mult.method, MultiplicityMethod::None) {
        return vec![plain(format!("Multiple comparisons used {kind_lbl}{ci_note}."))];
    }
    multiplicity_adjustment_clause(format!("Multiple comparisons used {kind_lbl} at "), is_fdr, &thresh, format!("{ci_note}.")).to_vec()
}

fn dunn_adjustment_desc(method: MultiplicityMethod) -> (&'static str, bool) {
    match method {
        MultiplicityMethod::TamhaneT2 | MultiplicityMethod::Sidak => ("Sidak FWER adjustment", false),
        MultiplicityMethod::HolmSidak => ("Holm-Sidak step-down FWER adjustment", false),
        MultiplicityMethod::BenjaminiHochbergFdr => ("Benjamini-Hochberg step-up FDR control", true),
        MultiplicityMethod::BenjaminiYekutieliFdr => ("Benjamini-Yekutieli arbitrary-dependence FDR control", true),
        MultiplicityMethod::TwoStageBkyFdr => ("two-stage Benjamini-Krieger-Yekutieli adaptive FDR control", true),
        MultiplicityMethod::DunnRankSum
        | MultiplicityMethod::LinearTrend
        | MultiplicityMethod::TukeyHsd
        | MultiplicityMethod::NewmanKeuls
        | MultiplicityMethod::GamesHowell
        | MultiplicityMethod::DunnettT3
        | MultiplicityMethod::Bonferroni
        | MultiplicityMethod::Dunnett { .. } => ("Bonferroni FWER adjustment", false),
        MultiplicityMethod::None | MultiplicityMethod::FisherLsd => ("no adjustment", false),
    }
}

fn describe_dunn_multiplicity_runs(mult: &MultiplicitySpec, source_ds: Option<&DataSheet>, rank_scope: &str) -> Vec<TextRun> {
    let thresh = format_exact_float(mult.threshold);
    match mult.method {
        MultiplicityMethod::None | MultiplicityMethod::FisherLsd => {
            vec![plain(format!(
                "Multiple comparisons used unadjusted Dunn's nonparametric {rank_scope} pairwise z-tests (no multiplicity correction applied)."
            ))]
        }
        MultiplicityMethod::Dunnett { control_column } => {
            let ctrl_name = column_title_at(source_ds, control_column);
            let mut runs = vec![plain("Control column for post-hoc comparisons: "), code(ctrl_name)];
            runs.extend(multiplicity_adjustment_clause(
                format!(". Multiple comparisons used Dunn's nonparametric {rank_scope} pairwise z-tests against the control group with Bonferroni FWER adjustment at "),
                false,
                &thresh,
                ".",
            ));
            runs
        }
        other => {
            let (adj_desc, is_fdr) = dunn_adjustment_desc(other);
            multiplicity_adjustment_clause(
                format!("Multiple comparisons used Dunn's nonparametric {rank_scope} pairwise z-tests with {adj_desc} at "),
                is_fdr,
                &thresh,
                ".",
            )
            .to_vec()
        }
    }
}

// ---------------------------------------------------------------------------
// Results & Diagnostics Sections (Verbatim Read-Out from ResultSheet)
// ---------------------------------------------------------------------------

fn append_results_section(blocks: &mut Vec<ReportBlock>, doc: &ProjectDocument, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    blocks.push(ReportBlock::Section(vec![plain("Stored Results Summary")]));

    if !res.scalars.is_empty() {
        let mut scalar_bullets = Vec::with_capacity(res.scalars.len());
        for entry in &res.scalars {
            let ds_suffix = entry.data_set_index.map_or_else(String::new, |idx| format!(" [{}]", column_title_at(source_ds, idx)));
            let units_suffix = entry.units.as_ref().map_or_else(String::new, |u| format!(" {u}"));
            scalar_bullets
                .push(bullet_kv(&format!("{}{ds_suffix}", entry.key), [plain(format!("{}{units_suffix}", format_stat_value(entry.value)))]));
        }
        blocks.push(ReportBlock::Paragraph(vec![bold("Key scalar outputs:")]));
        blocks.push(ReportBlock::Bullets(scalar_bullets));
    }

    let (_, default_alpha) = effective_report_alpha(doc, res);
    let mult_threshold = analysis_multiplicity_threshold(&res.analysis.kind);
    let is_p_stack = matches!(res.analysis.kind, AnalysisKind::PValueStack(_));
    for tab in &res.tabs {
        if let Some(tbl) = render_result_tab_block(tab, res.analysis.p_value_format, default_alpha, mult_threshold, is_p_stack) {
            blocks.push(tbl);
        }
    }

    if !res.pairwise_comparisons.is_empty() {
        let sig_count = res.pairwise_comparisons.iter().filter(|p| p.significant).count();
        let total = res.pairwise_comparisons.len();
        blocks.push(ReportBlock::Paragraph(vec![plain(format!(
            "Pairwise comparison records: {sig_count} of {total} comparison(s) met the significance threshold."
        ))]));
    }
}

const fn analysis_multiplicity_threshold(kind: &AnalysisKind) -> Option<f64> {
    match kind {
        AnalysisKind::MultipleRowTTests(s) => Some(s.multiplicity.threshold),
        AnalysisKind::OneWayAnova(s) => Some(s.multiplicity.threshold),
        AnalysisKind::FactorialAnova(s) => Some(s.multiplicity.threshold),
        AnalysisKind::NestedComparison(s) => Some(s.multiplicity.threshold),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => Some(multiplicity.threshold),
        AnalysisKind::PValueStack(s) => Some(s.threshold),
        _ => None,
    }
}

fn column_p_alpha(tab: &ResultTab, header: &str, default_alpha: f64, mult_threshold: Option<f64>, is_p_stack: bool) -> f64 {
    let Some(thresh) = mult_threshold.filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0) else {
        return default_alpha;
    };
    let lower = header.to_ascii_lowercase();
    if tab.kind == ResultTabKind::MultipleComparisons && lower.contains("adjusted") {
        return thresh;
    }
    if is_p_stack && (lower.contains("adjusted") || lower.contains("min adj")) {
        return thresh;
    }
    default_alpha
}

fn render_result_tab_block(
    tab: &ResultTab,
    p_format: PValueFormat,
    default_alpha: f64,
    mult_threshold: Option<f64>,
    is_p_stack: bool,
) -> Option<ReportBlock> {
    if tab.rows.is_empty() {
        return None;
    }
    let is_coordinate_tab = matches!(
        tab.kind,
        ResultTabKind::Residuals
            | ResultTabKind::CurveCoordinates
            | ResultTabKind::NumberAtRisk
            | ResultTabKind::NormalQq
            | ResultTabKind::BaselineSurvival
            | ResultTabKind::LogMinusLog
    );
    let max_rows = if is_coordinate_tab { 12 } else { 30 };
    if is_coordinate_tab && tab.rows.len() > max_rows {
        return Some(ReportBlock::Paragraph(vec![
            bold(format!("{}:", tab.title)),
            plain(" "),
            plain(format!("{} row(s) across {} column(s) stored in result tab ", tab.rows.len(), tab.column_headers.len())),
            code(&tab.id),
            plain("."),
        ]));
    }

    let mut headers = vec![vec![plain("Measure / Row")]];
    for h in &tab.column_headers {
        headers.push(vec![plain(h)]);
    }

    let col_alphas: Vec<f64> = tab.column_headers.iter().map(|h| column_p_alpha(tab, h, default_alpha, mult_threshold, is_p_stack)).collect();

    let shown = tab.rows.len().min(max_rows);
    let mut rows = Vec::with_capacity(shown);
    for (idx, row) in tab.rows.iter().take(shown).enumerate() {
        let label = tab.row_labels.get(idx).map_or_else(|| format!("Row {}", idx + 1), Clone::clone);
        let mut cells = Vec::with_capacity(tab.column_headers.len() + 1);
        cells.push(vec![plain(label)]);
        for c_idx in 0..tab.column_headers.len() {
            let cell = row.get(c_idx).unwrap_or(&ResultCell::Empty);
            let cell_alpha = col_alphas.get(c_idx).copied().unwrap_or(default_alpha);
            cells.push(format_result_cell(cell, p_format, cell_alpha));
        }
        rows.push(cells);
    }

    let mut footnotes: Vec<Vec<TextRun>> = tab.footnotes.iter().map(|f| vec![plain(f)]).collect();
    if tab.rows.len() > shown {
        footnotes.push(vec![plain(format!("Showing first {shown} of {} stored rows.", tab.rows.len()))]);
    }

    Some(ReportBlock::Table(TableBlock { caption: vec![plain(&tab.title)], headers, rows, footnotes }))
}

fn format_result_cell(cell: &ResultCell, p_format: PValueFormat, alpha: f64) -> Vec<TextRun> {
    match cell {
        ResultCell::Empty => vec![plain("-")],
        ResultCell::Number { value, prefix, units } => {
            let pre = prefix.as_deref().unwrap_or("");
            let suf = units.as_deref().map_or_else(String::new, |u| format!(" {u}"));
            vec![plain(format!("{pre}{}{suf}", format_stat_value(*value)))]
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            let lo = match lower {
                CiBound::Finite(v) => plain(format_stat_value(*v)),
                CiBound::Open => math("-inf", "-inf", "-\\infty"),
            };
            let hi = match upper {
                CiBound::Finite(v) => plain(format_stat_value(*v)),
                CiBound::Open => math("+inf", "+inf", "+\\infty"),
            };
            vec![plain("["), lo, plain(", "), hi, plain("]")]
        }
        ResultCell::PValue { raw, formatted, stars } => {
            let base = if formatted.trim().is_empty() { format_stat_value(*raw) } else { formatted.clone() };
            let trimmed_stars = stars.trim();
            if trimmed_stars.is_empty() {
                vec![plain(base)]
            } else if matches!(trimmed_stars, "ns" | "*" | "**" | "***" | "****") && raw.is_finite() && (0.0..=1.0).contains(raw) {
                let eff_stars = p_format.stars_at_alpha(*raw, alpha);
                vec![plain(format!("{base} ({eff_stars})"))]
            } else {
                vec![plain(format!("{base} ({stars})"))]
            }
        }
        ResultCell::Text(s) => vec![plain(s)],
        ResultCell::Boolean(b) => vec![plain(if *b { "Yes" } else { "No" })],
    }
}

fn append_diagnostics_section(blocks: &mut Vec<ReportBlock>, res: &ResultSheet) {
    if res.checklist.is_empty() {
        return;
    }
    blocks.push(ReportBlock::Section(vec![plain("Assumption and Quality Diagnostics")]));
    let bullets: Vec<Vec<TextRun>> = res.checklist.iter().map(format_checklist_item).collect();
    blocks.push(ReportBlock::Bullets(bullets));
}

fn format_checklist_item(item: &ChecklistItem) -> Vec<TextRun> {
    let badge = match item.status {
        ChecklistStatus::Pass => "[PASS]",
        ChecklistStatus::Warn => "[WARN]",
        ChecklistStatus::Violated => "[VIOLATED]",
        ChecklistStatus::Info => "[INFO]",
    };
    let mut runs = bullet_kv(&format!("{badge} {}", item.assumption), []);
    if let Some(metric) = &item.diagnostic_metric {
        runs.push(plain(format!("({metric}) ")));
    }
    runs.push(plain(&item.explanation));
    runs
}

// ---------------------------------------------------------------------------
// Built-in Equation Formula Lookup & Table Metadata Helpers
// ---------------------------------------------------------------------------

fn resolve_equation_description(s: &NonlinearFitSpec) -> (String, Vec<String>, String, String) {
    if let Some(dsl) = &s.custom_dsl {
        let compiled_params = resolve_model_from_spec(s).ok().map(|m| m.param_names().to_vec());
        let builtin = get_builtin_model(&s.equation_id);
        let model_name = builtin.as_ref().map_or_else(|| format!("Custom equation ({})", s.equation_id), |m| m.name().to_owned());
        let params = compiled_params.unwrap_or_else(|| builtin.as_ref().map_or_else(Vec::new, |m| m.param_names().to_vec()));
        return (model_name, params, dsl.clone(), format!("\\mbox{{{}}}", escape_latex_inline_text(dsl)));
    }

    let model = get_builtin_model(&s.equation_id);
    let canonical_id = model.as_ref().map_or(s.equation_id.as_str(), |m| m.id());
    let name = model.as_ref().map_or_else(|| s.equation_id.clone(), |m| m.name().to_owned());
    let params = model.as_ref().map_or_else(Vec::new, |m| m.param_names().to_vec());

    let (plain_eq, tex_eq) = builtin_formula_pair(canonical_id).unwrap_or_else(|| {
        let sig = if params.is_empty() { String::from("Y = f(X; theta)") } else { format!("Y = f(X; {})", params.join(", ")) };
        let tex_sig = escape_latex_math_mode(&sig);
        (sig, format!("\\mathrm{{{tex_sig}}}"))
    });
    (name, params, plain_eq, tex_eq)
}

fn builtin_formula_pair(canonical_id: &str) -> Option<(String, String)> {
    let (plain_s, tex_s) = match canonical_id {
        "log_agonist_vs_response_3pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + 10^{\\mathrm{LogEC50} - X}}",
        ),
        "log_agonist_vs_response_4pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + 10^{(\\mathrm{LogEC50} - X)\\cdot \\mathrm{HillSlope}}}",
        ),
        "log_agonist_vs_response_normalized" => ("Y = 100 / (1 + 10^(LogEC50 - X))", "Y = \\frac{100}{1 + 10^{\\mathrm{LogEC50} - X}}"),
        "log_agonist_vs_response_normalized_var" => {
            ("Y = 100 / (1 + 10^((LogEC50 - X) * HillSlope))", "Y = \\frac{100}{1 + 10^{(\\mathrm{LogEC50} - X)\\cdot \\mathrm{HillSlope}}}")
        }
        "log_inhibitor_vs_response_3pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + 10^(X - LogIC50))",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + 10^{X - \\mathrm{LogIC50}}}",
        ),
        "log_inhibitor_vs_response_4pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + 10^((X - LogIC50) * HillSlope))",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + 10^{(X - \\mathrm{LogIC50})\\cdot \\mathrm{HillSlope}}}",
        ),
        "log_inhibitor_vs_response_normalized" => ("Y = 100 / (1 + 10^(X - LogIC50))", "Y = \\frac{100}{1 + 10^{X - \\mathrm{LogIC50}}}"),
        "log_inhibitor_vs_response_normalized_var" => {
            ("Y = 100 / (1 + 10^((X - LogIC50) * HillSlope))", "Y = \\frac{100}{1 + 10^{(X - \\mathrm{LogIC50})\\cdot \\mathrm{HillSlope}}}")
        }
        "agonist_vs_response_3pl" => (
            "Y = Bottom + (Top - Bottom) * X / (EC50 + X)",
            "Y = \\mathrm{Bottom} + \\frac{(\\mathrm{Top} - \\mathrm{Bottom}) X}{\\mathrm{EC50} + X}",
        ),
        "agonist_vs_response_4pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + (EC50 / X)^HillSlope)",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + (\\mathrm{EC50} / X)^{\\mathrm{HillSlope}}}",
        ),
        "inhibitor_vs_response_3pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + X / IC50)",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + X / \\mathrm{IC50}}",
        ),
        "inhibitor_vs_response_4pl" => (
            "Y = Bottom + (Top - Bottom) / (1 + (X / IC50)^HillSlope)",
            "Y = \\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + (X / \\mathrm{IC50})^{\\mathrm{HillSlope}}}",
        ),
        "one_site_specific_binding" => ("Y = Bmax * X / (Kd + X)", "Y = \\frac{B_{\\max} X}{K_d + X}"),
        "one_site_specific_hill" => ("Y = Bmax * X^h / (Kd^h + X^h)", "Y = \\frac{B_{\\max} X^h}{K_d^h + X^h}"),
        "michaelis_menten" => ("Y = Vmax * X / (Km + X)", "Y = \\frac{V_{\\max} X}{K_m + X}"),
        "one_phase_decay" => ("Y = (Y0 - Plateau) * exp(-K * X) + Plateau", "Y = (Y_0 - \\mathrm{Plateau}) e^{-K X} + \\mathrm{Plateau}"),
        "two_phase_decay" => (
            "Y = Plateau + (Y0 - Plateau) * ((PercentFast/100)*exp(-KFast*X) + (1 - PercentFast/100)*exp(-KSlow*X))",
            "Y = \\mathrm{Plateau} + (Y_0 - \\mathrm{Plateau})\\left(\\frac{\\%\\mathrm{Fast}}{100}e^{-K_{\\mathrm{fast}}X} + \\left(1 - \\frac{\\%\\mathrm{Fast}}{100}\\right)e^{-K_{\\mathrm{slow}}X}\\right)",
        ),
        "one_phase_association" => ("Y = Y0 + (Plateau - Y0) * (1 - exp(-K * X))", "Y = Y_0 + (\\mathrm{Plateau} - Y_0)(1 - e^{-K X})"),
        "exponential_growth" | "malthusian_growth" => ("Y = Y0 * exp(k * X)", "Y = Y_0 e^{k X}"),
        "exponential_plateau" => ("Y = YM - (YM - Y0) * exp(-k * X)", "Y = Y_M - (Y_M - Y_0) e^{-k X}"),
        "logistic_growth" => ("Y = YM * Y0 / (Y0 + (YM - Y0) * exp(-k * X))", "Y = \\frac{Y_M Y_0}{Y_0 + (Y_M - Y_0) e^{-k X}}"),
        "gompertz_growth" => ("Y = YM * (Y0 / YM)^exp(-k * X)", "Y = Y_M (Y_0 / Y_M)^{e^{-k X}}"),
        "gaussian" => (
            "Y = Amplitude * exp(-0.5 * ((X - Mean) / SD)^2)",
            "Y = \\mathrm{Amplitude} \\cdot \\exp\\!\\left(-\\frac{(X - \\mu)^2}{2\\sigma^2}\\right)",
        ),
        "straight_line" => ("Y = Slope * X + YIntercept", "Y = \\mathrm{Slope} \\cdot X + \\mathrm{YIntercept}"),
        "line_through_origin" => ("Y = Slope * X", "Y = \\mathrm{Slope} \\cdot X"),
        _ => return None,
    };
    Some((plain_s.to_owned(), tex_s.to_owned()))
}

fn selected_column_labels(ds: &DataSheet, selected: &[usize]) -> Vec<String> {
    let total = ds.archetype.column_count();
    let indices: Vec<usize> = if selected.is_empty() { (0..total).collect() } else { selected.iter().copied().filter(|&i| i < total).collect() };
    indices.into_iter().map(|idx| column_title_at(Some(ds), idx)).collect()
}

fn column_title_at(source_ds: Option<&DataSheet>, col_idx: usize) -> String {
    let Some(ds) = source_ds else {
        return format!("Column {}", col_idx + 1);
    };
    let raw = match &ds.archetype {
        TableArchetype::Xy(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col_idx).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(col_idx).map(|g| g.title.as_str()),
    };
    match raw {
        Some(t) if !t.trim().is_empty() => t.to_owned(),
        _ => format!("Column {}", col_idx + 1),
    }
}

fn count_active_exclusion_notes(ds: &DataSheet) -> usize {
    ds.exclusion_notes.iter().filter(|note| ds.archetype.data_cell(note.coord).is_some_and(|cell| cell.excluded)).count()
}

fn count_excluded_cells(ds: &DataSheet) -> usize {
    let cols_excluded = |cols: &[curvebook_doc::DataSetColumn]| -> usize {
        cols.iter().flat_map(|c| c.cells.iter()).flat_map(|row| row.iter()).filter(|cell| cell.excluded).count()
    };
    match &ds.archetype {
        TableArchetype::Xy(t) => {
            let x_ex = t.x_cells.iter().flat_map(|r| r.iter()).filter(|c| c.excluded).count();
            x_ex + cols_excluded(&t.data_sets)
        }
        TableArchetype::Column(t) => cols_excluded(&t.data_sets),
        TableArchetype::Grouped(t) => cols_excluded(&t.data_sets),
        TableArchetype::Contingency(t) => cols_excluded(&t.data_sets),
        TableArchetype::Survival(t) => {
            let t_ex = t.times.iter().filter(|c| c.excluded).count();
            t_ex + cols_excluded(&t.cohorts)
        }
        TableArchetype::PartsOfWhole(t) => cols_excluded(&t.data_sets),
        TableArchetype::MultipleVariables(t) => t.variables.iter().flat_map(|v| v.cells.iter()).filter(|c| c.excluded).count(),
        TableArchetype::Nested(t) => t.groups.iter().flat_map(|g| g.cells.iter()).flat_map(|r| r.iter()).filter(|c| c.excluded).count(),
    }
}

fn p_format_label(fmt: PValueFormat) -> String {
    match fmt {
        PValueFormat::FourDecimal => String::from("0.0001 style (4 decimals)"),
        PValueFormat::Apa => String::from("APA style (3 decimals, no leading zero)"),
        PValueFormat::Nejm => String::from("NEJM style (3 decimals with leading zero)"),
        PValueFormat::Custom { decimals, scientific_below } => {
            let clamped_decimals = decimals.clamp(1, 15);
            match scientific_below.filter(|v| v.is_finite() && *v > 0.0) {
                Some(thresh) => format!("custom ({clamped_decimals} decimals, scientific below {})", format_exact_float(thresh)),
                None => format!("custom ({clamped_decimals} decimals)"),
            }
        }
    }
}

fn format_math_preset(preset: MathTransformPreset, axis_symbol: &str) -> String {
    match preset {
        MathTransformPreset::Ln => format!("ln({axis_symbol})"),
        MathTransformPreset::Log10 => format!("log10({axis_symbol})"),
        MathTransformPreset::Log2 => format!("log2({axis_symbol})"),
        MathTransformPreset::Exp10 => format!("10^{axis_symbol}"),
        MathTransformPreset::Exp => format!("exp({axis_symbol})"),
        MathTransformPreset::Reciprocal => format!("1 / {axis_symbol}"),
        MathTransformPreset::Sqrt => format!("sqrt({axis_symbol})"),
        MathTransformPreset::Square => format!("{axis_symbol}^2"),
        MathTransformPreset::Logit => format!("logit({axis_symbol})"),
        MathTransformPreset::Probit => format!("probit({axis_symbol})"),
        MathTransformPreset::ArcsinSqrt => format!("arcsin(sqrt({axis_symbol}))"),
        MathTransformPreset::LinearScale { slope, intercept } => {
            format!("{} * {axis_symbol} + {}", format_exact_float(slope), format_exact_float(intercept))
        }
        MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor } => {
            format!("-log10({axis_symbol} * {})", format_exact_float(molar_unit_factor))
        }
    }
}

fn format_hook_source(hook: &HookSource) -> String {
    match hook {
        HookSource::InfoConstant { info_sheet_id: Some(sheet_id), constant_name } => {
            format!("info constant '{constant_name}' on sheet '{}'", sheet_id.as_str())
        }
        HookSource::InfoConstant { info_sheet_id: None, constant_name } => {
            format!("info constant '{constant_name}' on paired info sheet")
        }
        HookSource::ResultScalar { result_sheet_id, parameter_key, data_set_index: Some(idx) } => {
            format!("fitted scalar '{parameter_key}' (dataset {}) from '{}'", idx + 1, result_sheet_id.as_str())
        }
        HookSource::ResultScalar { result_sheet_id, parameter_key, data_set_index: None } => {
            format!("fitted scalar '{parameter_key}' from '{}'", result_sheet_id.as_str())
        }
        HookSource::ColumnTitleValue { data_sheet_id, data_set_index } => {
            format!("column {} title on sheet '{}'", data_set_index + 1, data_sheet_id.as_str())
        }
    }
}

fn format_anchor(anchor: &NormalizeAnchor) -> String {
    match anchor {
        NormalizeAnchor::ColumnMin => String::from("column minimum"),
        NormalizeAnchor::ColumnMax => String::from("column maximum"),
        NormalizeAnchor::ColumnSum => String::from("column sum"),
        NormalizeAnchor::FirstRow => String::from("first row"),
        NormalizeAnchor::LastRow => String::from("last row"),
        NormalizeAnchor::FixedValue(v) => format_exact_float(*v),
        NormalizeAnchor::ControlColumnMean { column } => format!("mean of control column {}", column + 1),
        NormalizeAnchor::Hooked(h) => format!("hooked scalar ({})", format_hook_source(h)),
    }
}

fn opt_bound_str(bound: Option<f64>) -> String {
    bound.map_or_else(|| String::from("unbounded"), format_exact_float)
}

fn format_exact_float(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let clean = if v == 0.0 { 0.0 } else { v };
    if clean.fract() == 0.0 && clean.abs() < 1e12 {
        return format!("{clean:.0}");
    }
    clean.to_string()
}

fn format_stat_value(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let clean = if v == 0.0 { 0.0 } else { v };
    if clean.fract() == 0.0 && clean.abs() < 1e12 {
        return format!("{clean:.0}");
    }
    let verbatim = clean.to_string();
    if clean != 0.0 && (clean.abs() < 1e-4 || clean.abs() >= 1e7) {
        if verbatim.len() <= 12 { verbatim } else { format!("{clean:.6e}") }
    } else if verbatim.len() <= 10 {
        verbatim
    } else {
        format!("{clean:.6}")
    }
}

// ---------------------------------------------------------------------------
// Format Renderers (PlainText, Markdown, LaTeX)
// ---------------------------------------------------------------------------

fn render_blocks(blocks: &[ReportBlock], format: ReportFormat) -> String {
    let mut out = String::new();
    for (idx, block) in blocks.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        match format {
            ReportFormat::PlainText => render_block_plain(block, &mut out),
            ReportFormat::Markdown => render_block_markdown(block, &mut out),
            ReportFormat::Latex => render_block_latex(block, &mut out),
        }
    }
    out
}

fn render_runs_plain(runs: &[TextRun]) -> String {
    let mut s = String::new();
    for run in runs {
        match run {
            TextRun::Plain(t) | TextRun::Bold(t) | TextRun::Code(t) => s.push_str(t),
            TextRun::Math { plain, .. } => s.push_str(plain),
        }
    }
    s
}

fn collapse_line_breaks_to_space(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_break = false;
    for ch in input.chars() {
        if matches!(ch, '\n' | '\r' | '\t') || ch.is_control() {
            if !in_break {
                out.push(' ');
                in_break = true;
            }
        } else {
            in_break = false;
            out.push(ch);
        }
    }
    out
}

fn render_runs_markdown(runs: &[TextRun], escape_pipes: bool) -> String {
    let mut s = String::new();
    for run in runs {
        match run {
            TextRun::Plain(t) => append_md_text(&mut s, t, escape_pipes),
            TextRun::Bold(t) => {
                let single = collapse_line_breaks_to_space(t);
                let trimmed_start = single.trim_start_matches(' ');
                let leading_spaces = single.len() - trimmed_start.len();
                let core = trimmed_start.trim_end_matches(' ');
                let trailing_spaces = trimmed_start.len() - core.len();
                for _ in 0..leading_spaces {
                    s.push(' ');
                }
                if !core.is_empty() {
                    s.push_str("**");
                    append_md_text(&mut s, core, escape_pipes);
                    s.push_str("**");
                }
                for _ in 0..trailing_spaces {
                    s.push(' ');
                }
            }
            TextRun::Code(t) => append_md_code(&mut s, t, escape_pipes),
            TextRun::Math { markdown, .. } => append_md_text(&mut s, markdown, escape_pipes),
        }
    }
    s
}

fn append_md_text(dst: &mut String, text: &str, escape_pipes: bool) {
    let clean = collapse_line_breaks_to_space(text);
    for ch in clean.chars() {
        match ch {
            '\\' | '<' | '>' | '&' | '*' | '_' | '`' | '~' | '[' | ']' | '#' | '$' => {
                dst.push('\\');
                dst.push(ch);
            }
            '|' if escape_pipes => {
                dst.push_str("\\|");
            }
            _ => dst.push(ch),
        }
    }
}

fn append_md_code(dst: &mut String, text: &str, escape_pipes: bool) {
    let clean = collapse_line_breaks_to_space(text);
    let body = if escape_pipes { clean.replace('|', "\\|") } else { clean };
    let mut max_run = 0_usize;
    let mut cur_run = 0_usize;
    for ch in body.chars() {
        if ch == '`' {
            cur_run += 1;
            max_run = max_run.max(cur_run);
        } else {
            cur_run = 0;
        }
    }
    let fence_len = max_run + 1;
    let needs_pad = body.starts_with('`') || body.ends_with('`');
    for _ in 0..fence_len {
        dst.push('`');
    }
    if needs_pad {
        dst.push(' ');
    }
    dst.push_str(&body);
    if needs_pad {
        dst.push(' ');
    }
    for _ in 0..fence_len {
        dst.push('`');
    }
}

fn render_runs_latex(runs: &[TextRun], single_line: bool) -> String {
    let mut s = String::new();
    for run in runs {
        match run {
            TextRun::Plain(t) => {
                if single_line {
                    s.push_str(&escape_latex_inline_text(t));
                } else {
                    s.push_str(&escape_latex_text(t));
                }
            }
            TextRun::Bold(t) => {
                s.push_str("\\textbf{");
                if single_line {
                    s.push_str(&escape_latex_inline_text(t));
                } else {
                    s.push_str(&escape_latex_text(t));
                }
                s.push('}');
            }
            TextRun::Code(t) => {
                s.push_str("\\texttt{");
                if single_line {
                    s.push_str(&escape_latex_inline_text(t));
                } else {
                    s.push_str(&escape_latex_text(t));
                }
                s.push('}');
            }
            TextRun::Math { latex, .. } => {
                s.push('$');
                if single_line {
                    s.push_str(&collapse_line_breaks_to_space(latex));
                } else {
                    s.push_str(latex);
                }
                s.push('$');
            }
        }
    }
    s
}

fn unicode_latex_math_token(ch: char) -> Option<&'static str> {
    match ch {
        '∞' => Some("\\infty"),
        '≥' => Some("\\ge"),
        '≤' => Some("\\le"),
        '≠' => Some("\\ne"),
        '≈' => Some("\\approx"),
        '±' => Some("\\pm"),
        '×' => Some("\\times"),
        '÷' => Some("\\div"),
        '·' => Some("\\cdot"),
        '−' => Some("-"),
        '°' => Some("^\\circ"),
        '√' => Some("\\surd"),
        'α' => Some("\\alpha"),
        'β' => Some("\\beta"),
        'γ' => Some("\\gamma"),
        'δ' => Some("\\delta"),
        'ε' => Some("\\varepsilon"),
        'ζ' => Some("\\zeta"),
        'η' => Some("\\eta"),
        'θ' => Some("\\theta"),
        'κ' => Some("\\kappa"),
        'λ' => Some("\\lambda"),
        'μ' => Some("\\mu"),
        'ν' => Some("\\nu"),
        'ξ' => Some("\\xi"),
        'π' => Some("\\pi"),
        'ρ' => Some("\\rho"),
        'σ' => Some("\\sigma"),
        'τ' => Some("\\tau"),
        'φ' => Some("\\phi"),
        'χ' => Some("\\chi"),
        'ψ' => Some("\\psi"),
        'ω' => Some("\\omega"),
        'Γ' => Some("\\Gamma"),
        'Δ' => Some("\\Delta"),
        'Θ' => Some("\\Theta"),
        'Λ' => Some("\\Lambda"),
        'Π' => Some("\\Pi"),
        'Σ' => Some("\\Sigma"),
        'Φ' => Some("\\Phi"),
        'Ψ' => Some("\\Psi"),
        'Ω' => Some("\\Omega"),
        _ => None,
    }
}

fn push_escaped_latex_char(ch: char, out: &mut String, math_mode: bool, single_line: bool) {
    if single_line && (matches!(ch, '\n' | '\r' | '\t') || ch.is_control()) {
        out.push(' ');
        return;
    }
    if !single_line && ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
        out.push(' ');
        return;
    }
    if let Some(tok) = unicode_latex_math_token(ch) {
        if math_mode {
            out.push_str(tok);
            out.push(' ');
        } else {
            let _ = write!(out, "${tok}$");
        }
        return;
    }
    match ch {
        '\\' => out.push_str(if math_mode { "\\backslash " } else { "\\textbackslash{}" }),
        '{' => out.push_str("\\{"),
        '}' => out.push_str("\\}"),
        '$' => out.push_str("\\$"),
        '&' => out.push_str("\\&"),
        '%' => out.push_str("\\%"),
        '#' => out.push_str("\\#"),
        '_' => out.push_str("\\_"),
        '~' => out.push_str(if math_mode { "\\sim " } else { "\\textasciitilde{}" }),
        '^' => out.push_str(if math_mode { "\\hat{}" } else { "\\textasciicircum{}" }),
        '<' => out.push_str(if math_mode { "<" } else { "\\textless{}" }),
        '>' => out.push_str(if math_mode { ">" } else { "\\textgreater{}" }),
        '–' => out.push_str(if math_mode { "-" } else { "--" }),
        '—' => out.push_str(if math_mode { "-" } else { "---" }),
        '¹' => out.push_str(if math_mode { "^1" } else { "\\textsuperscript{1}" }),
        '²' => out.push_str(if math_mode { "^2" } else { "\\textsuperscript{2}" }),
        '³' => out.push_str(if math_mode { "^3" } else { "\\textsuperscript{3}" }),
        _ => out.push(ch),
    }
}

fn escape_latex_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        push_escaped_latex_char(ch, &mut out, false, false);
    }
    out
}

fn escape_latex_inline_text(input: &str) -> String {
    let clean = collapse_line_breaks_to_space(input);
    let mut out = String::with_capacity(clean.len() + 8);
    for ch in clean.chars() {
        push_escaped_latex_char(ch, &mut out, false, true);
    }
    out
}

fn escape_latex_math_mode(input: &str) -> String {
    let clean = collapse_line_breaks_to_space(input);
    let mut out = String::with_capacity(clean.len() + 8);
    for ch in clean.chars() {
        push_escaped_latex_char(ch, &mut out, true, true);
    }
    out
}

fn guard_latex_leading_bracket(cell: &str) -> String {
    let trimmed = cell.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        let prefix_len = cell.len().saturating_sub(trimmed.len());
        let prefix = cell.get(..prefix_len).unwrap_or("");
        format!("{prefix}{{[}}{rest}")
    } else {
        cell.to_owned()
    }
}

struct TableRowStyle<'a> {
    prefix: &'a str,
    sep: &'a str,
    suffix: &'a str,
    after_header: Option<&'a str>,
}

fn format_latex_table_cell(col_idx: usize, runs: &[TextRun]) -> String {
    let s = render_runs_latex(runs, true);
    if col_idx == 0 { guard_latex_leading_bracket(&s) } else { s }
}

fn write_formatted_table_rows(
    out: &mut String,
    headers: &[Vec<TextRun>],
    rows: &[Vec<Vec<TextRun>>],
    style: &TableRowStyle<'_>,
    mut format_cell: impl FnMut(usize, &[TextRun]) -> String,
) {
    let mut format_line = |cells: &[Vec<TextRun>]| {
        let formatted: Vec<String> = cells.iter().enumerate().map(|(c_idx, c)| format_cell(c_idx, c)).collect();
        format!("{}{}{}", style.prefix, formatted.join(style.sep), style.suffix)
    };
    let _ = writeln!(out, "{}", format_line(headers));
    if let Some(mid) = style.after_header {
        let _ = writeln!(out, "{mid}");
    }
    for row in rows {
        let _ = writeln!(out, "{}", format_line(row));
    }
}

fn render_block_plain(block: &ReportBlock, out: &mut String) {
    match block {
        ReportBlock::Title(runs) => {
            let text = collapse_line_breaks_to_space(&render_runs_plain(runs));
            let _ = writeln!(out, "{text}");
            let _ = writeln!(out, "{}", "=".repeat(text.chars().count().max(12)));
        }
        ReportBlock::Section(runs) => {
            let text = collapse_line_breaks_to_space(&render_runs_plain(runs));
            let _ = writeln!(out, "{text}");
            let _ = writeln!(out, "{}", "-".repeat(text.chars().count().max(8)));
        }
        ReportBlock::Paragraph(runs) => {
            let _ = writeln!(out, "{}", render_runs_plain(runs));
        }
        ReportBlock::Bullets(items) => {
            for item in items {
                let _ = writeln!(out, "- {}", render_runs_plain(item));
            }
        }
        ReportBlock::Table(tbl) => {
            let _ = writeln!(out, "[{}]", collapse_line_breaks_to_space(&render_runs_plain(&tbl.caption)));
            let style = TableRowStyle { prefix: "  ", sep: " | ", suffix: "", after_header: None };
            write_formatted_table_rows(out, &tbl.headers, &tbl.rows, &style, |_, c| collapse_line_breaks_to_space(&render_runs_plain(c)));
            for note in &tbl.footnotes {
                let _ = writeln!(out, "  Note: {}", render_runs_plain(note));
            }
        }
    }
}

fn render_block_markdown(block: &ReportBlock, out: &mut String) {
    match block {
        ReportBlock::Title(runs) => {
            let _ = writeln!(out, "# {}", render_runs_markdown(runs, false));
        }
        ReportBlock::Section(runs) => {
            let _ = writeln!(out, "## {}", render_runs_markdown(runs, false));
        }
        ReportBlock::Paragraph(runs) => {
            let _ = writeln!(out, "{}", render_runs_markdown(runs, false));
        }
        ReportBlock::Bullets(items) => {
            for item in items {
                let _ = writeln!(out, "- {}", render_runs_markdown(item, false));
            }
        }
        ReportBlock::Table(tbl) => {
            let _ = writeln!(out, "### {}", render_runs_markdown(&tbl.caption, false));
            let sep_row = format!("| {} |", vec!["---"; tbl.headers.len()].join(" | "));
            let style = TableRowStyle { prefix: "| ", sep: " | ", suffix: " |", after_header: Some(&sep_row) };
            write_formatted_table_rows(out, &tbl.headers, &tbl.rows, &style, |_, c| render_runs_markdown(c, true));
            for note in &tbl.footnotes {
                let _ = writeln!(out, "*Note:* {}", render_runs_markdown(note, false));
            }
        }
    }
}

fn render_block_latex(block: &ReportBlock, out: &mut String) {
    match block {
        ReportBlock::Title(runs) => {
            let _ = writeln!(out, "\\section*{{{}}}", render_runs_latex(runs, true));
        }
        ReportBlock::Section(runs) => {
            let _ = writeln!(out, "\\subsection*{{{}}}", render_runs_latex(runs, true));
        }
        ReportBlock::Paragraph(runs) => {
            let _ = writeln!(out, "{}", render_runs_latex(runs, true));
        }
        ReportBlock::Bullets(items) => {
            let _ = writeln!(out, "\\begin{{itemize}}");
            for item in items {
                let _ = writeln!(out, "  \\item {}", render_runs_latex(item, true));
            }
            let _ = writeln!(out, "\\end{{itemize}}");
        }
        ReportBlock::Table(tbl) => {
            let _ = writeln!(out, "\\paragraph*{{{}}}", render_runs_latex(&tbl.caption, true));
            let col_spec = "l".repeat(tbl.headers.len().max(1));
            let _ = writeln!(out, "\\begin{{tabular}}{{{col_spec}}}");
            let _ = writeln!(out, "\\hline");
            let style = TableRowStyle { prefix: "", sep: " & ", suffix: " \\\\", after_header: Some("\\hline") };
            write_formatted_table_rows(out, &tbl.headers, &tbl.rows, &style, format_latex_table_cell);
            let _ = writeln!(out, "\\hline");
            let _ = writeln!(out, "\\end{{tabular}}");
            for note in &tbl.footnotes {
                let _ = writeln!(out, "\\par\\noindent\\textit{{Note:}} {}", render_runs_latex(note, true));
            }
        }
    }
}

fn power_summary_tab_number(res: &ResultSheet, row_label: &str) -> Option<f64> {
    let tab = res.tabs.iter().find(|t| t.id == "power-summary")?;
    let idx = tab.row_labels.iter().position(|lbl| lbl == row_label)?;
    match tab.rows.get(idx)?.first()? {
        ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn resolve_effective_power_spec(doc: &ProjectDocument, res: &ResultSheet, base: &PowerAnalysisSpec) -> PowerAnalysisSpec {
    let mut eff = base.clone();
    if !res.scalars.is_empty() || !res.tabs.is_empty() {
        if let Some(alpha) = res
            .lookup_scalar("Alpha", None)
            .or_else(|| power_summary_tab_number(res, "Significance level (alpha)"))
            .filter(|v| v.is_finite() && *v > 0.0 && *v < 1.0)
        {
            eff.alpha = alpha;
        }
        if base.solve_target != PowerSolveTarget::Power
            && let Some(tp) = res
                .lookup_scalar("TargetPower", None)
                .or_else(|| power_summary_tab_number(res, "Target power (1 - beta)"))
                .filter(|v| v.is_finite() && *v > 0.0 && *v < 1.0)
        {
            eff.target_power = Some(tp);
        }
        if base.solve_target != PowerSolveTarget::DetectableEffect
            && let Some(eff_sz) =
                res.lookup_scalar("EffectSize", None).or_else(|| power_summary_tab_number(res, "Standardized effect size")).filter(|v| v.is_finite())
        {
            let matches_rich_baseline = match &base.effect {
                PowerEffectSpec::Standardized(_) | PowerEffectSpec::Unspecified => false,
                _ => curvebook_stats::power::solve_power(&curvebook_stats::power::power_request_from_doc_spec(base))
                    .ok()
                    .is_some_and(|r| (eff_sz - r.effect_size).abs() <= 1e-12 * r.effect_size.abs().max(1.0)),
            };
            if !matches_rich_baseline {
                eff.effect = PowerEffectSpec::Standardized(eff_sz);
            }
        }
        if base.solve_target != PowerSolveTarget::SampleSize {
            if let Some(n1) = res
                .lookup_scalar("NPerGroup", None)
                .or_else(|| power_summary_tab_number(res, "Sample size (n1 / per group)"))
                .filter(|v| v.is_finite() && *v > 0.0)
            {
                eff.sample_size = Some(n1);
            }
            if let Some(n2) = res
                .lookup_scalar("NGroup2", None)
                .or_else(|| power_summary_tab_number(res, "Sample size (group 2)"))
                .filter(|v| v.is_finite() && *v > 0.0)
            {
                eff.sample_size_group2 = Some(n2);
            }
        }
        return eff;
    }

    if let Some(info_id) = &base.info_sheet_id
        && let Some(info) = doc.info_sheet(info_id)
    {
        let info_f64 =
            |names: &[&str]| -> Option<f64> { names.iter().find_map(|name| info.get_constant(name).and_then(|c| c.value.as_finite_f64())) };
        if let Some(alpha) = info_f64(&["Alpha", "alpha"]).filter(|v| *v > 0.0 && *v < 1.0) {
            eff.alpha = alpha;
        }
        if let Some(pw) = info_f64(&["Target Power", "target_power", "Target power"]).filter(|v| *v > 0.0 && *v < 1.0) {
            eff.target_power = Some(pw);
        }
        if base.solve_target != PowerSolveTarget::DetectableEffect
            && let Some(eff_sz) = info_f64(&["Effect Size", "effect_size", "Effect size"])
        {
            eff.effect = PowerEffectSpec::Standardized(eff_sz);
        }
    }
    eff
}

/// Describe a first-class [`PowerAnalysisSpec`] in neutral ICMJE/ARRIVE methods prose (`R2-S07-10`).
#[must_use]
pub fn describe_power_analysis_spec(spec: &PowerAnalysisSpec) -> String {
    let target_desc = match spec.solve_target {
        PowerSolveTarget::SampleSize => "required sample size",
        PowerSolveTarget::Power => "achievable statistical power (1 - beta)",
        PowerSolveTarget::DetectableEffect => "minimum detectable effect size",
    };
    let tail_desc = match spec.tail_mode {
        PowerTailMode::TwoSided => "two-sided",
        PowerTailMode::OneSided(PowerOneSidedDirection::Greater) => "one-sided (greater alternative)",
        PowerTailMode::OneSided(PowerOneSidedDirection::Less) => "one-sided (less alternative)",
    };
    let family_detail = match spec.family {
        PowerTestFamily::TwoProportions => {
            let mode = match spec.two_proportions_mode {
                TwoProportionsMethod::PooledNullUnpooledAlt => "pooled null / unpooled alternative variance",
                TwoProportionsMethod::Unpooled => "unpooled variance",
                TwoProportionsMethod::CohensH => "arcsine Cohen's h transformation",
            };
            let cc = if spec.continuity_correction { " with continuity correction" } else { "" };
            format!("{} ({mode}{cc})", spec.family.label())
        }
        PowerTestFamily::Correlation => {
            let corr_mode = match spec.correlation_method {
                CorrelationPowerMethod::FisherZ => "Fisher z approximation",
                CorrelationPowerMethod::FisherZWithTCrit => "Fisher z with t critical value",
                CorrelationPowerMethod::ExactTTransform => "exact t transformation",
                CorrelationPowerMethod::ConditionalSlopeT => "conditional slope t formulation",
            };
            format!("{} ({corr_mode})", spec.family.label())
        }
        PowerTestFamily::OneWayAnova => {
            let k_str = spec.num_groups.map_or_else(|| String::from("unspecified"), |k| k.to_string());
            format!("{} (k = {k_str} groups)", spec.family.label())
        }
        PowerTestFamily::ChiSquare => {
            format!("{} (df = {})", spec.family.label(), format_exact_float(spec.chi_square_df))
        }
        PowerTestFamily::TwoSampleUnpairedT
        | PowerTestFamily::TwoSamplePairedT
        | PowerTestFamily::OneSampleT
        | PowerTestFamily::SimpleLinearRegression => String::from(spec.family.label()),
    };
    let effect_desc = match &spec.effect {
        PowerEffectSpec::Unspecified => String::from("effect size solved or left unspecified"),
        PowerEffectSpec::Standardized(d) => format!("standardized {} = {}", spec.family.default_effect_label(), format_exact_float(*d)),
        PowerEffectSpec::MeanDiffAndSd { mean_diff, sd, sd_group2, within_pair_correlation } => {
            let mut extra = format!("mean difference = {}, SD = {}", format_exact_float(*mean_diff), format_exact_float(*sd));
            if let Some(sd2) = sd_group2 {
                let _ = write!(extra, ", group 2 SD = {}", format_exact_float(*sd2));
            }
            if let Some(rho) = within_pair_correlation {
                let _ = write!(extra, ", within-pair correlation r = {}", format_exact_float(*rho));
            }
            extra
        }
        PowerEffectSpec::PairedDiff { mean_diff, sd_diff } => {
            format!("paired mean difference = {}, SD of differences = {}", format_exact_float(*mean_diff), format_exact_float(*sd_diff))
        }
        PowerEffectSpec::GroupMeansAndSd { means, within_sd } => {
            format!("{} group means with within-group SD = {}", means.len(), format_exact_float(*within_sd))
        }
        PowerEffectSpec::TwoProportions { p1, p2 } => {
            format!("proportions p1 = {} vs p2 = {}", format_exact_float(*p1), format_exact_float(*p2))
        }
        PowerEffectSpec::BaselineProportion { p1 } => {
            format!("baseline proportion p1 = {}", format_exact_float(*p1))
        }
        PowerEffectSpec::RSquared(r2) => format!("R^2 = {}", format_exact_float(*r2)),
        PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps } => format!(
            "slope beta1 = {}, sigma_X = {}, residual sigma_eps = {}",
            format_exact_float(*beta1),
            format_exact_float(*sigma_x),
            format_exact_float(*sigma_eps)
        ),
    };
    let mut parts = vec![format!(
        "A priori power and sample-size planning solved for {target_desc} under {family_detail} using {tail_desc} significance level alpha = {}",
        format_exact_float(spec.alpha)
    )];
    if let Some(pw) = spec.target_power {
        parts.push(format!("target power = {}", format_exact_float(pw)));
    }
    if let Some(n1) = spec.sample_size {
        if let Some(n2) = spec.sample_size_group2 {
            parts.push(format!("sample sizes n1 = {}, n2 = {}", format_exact_float(n1), format_exact_float(n2)));
        } else {
            parts.push(format!("sample size n = {} (allocation ratio k = {})", format_exact_float(n1), format_exact_float(spec.allocation_ratio)));
        }
    }
    parts.push(effect_desc);
    format!("{}.", parts.join("; "))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, ResultScalarEntry};

    use super::*;

    fn dual_fnv1a(bytes: &[u8]) -> (usize, u64, u64) {
        let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
        let mut h2: u64 = 0xaf63_bd4c_8601_b7df;
        for &b in bytes {
            h1 ^= u64::from(b);
            h1 = h1.wrapping_mul(0x0100_0000_01b3);
            h2 = h2.wrapping_mul(0x0100_0000_01b3);
            h2 ^= u64::from(b);
        }
        (bytes.len(), h1, h2)
    }

    fn all_multiplicity_methods() -> Vec<MultiplicityMethod> {
        vec![
            MultiplicityMethod::None,
            MultiplicityMethod::FisherLsd,
            MultiplicityMethod::Dunnett { control_column: 0 },
            MultiplicityMethod::Bonferroni,
            MultiplicityMethod::Sidak,
            MultiplicityMethod::HolmSidak,
            MultiplicityMethod::TukeyHsd,
            MultiplicityMethod::DunnettT3,
            MultiplicityMethod::GamesHowell,
            MultiplicityMethod::TamhaneT2,
            MultiplicityMethod::NewmanKeuls,
            MultiplicityMethod::DunnRankSum,
            MultiplicityMethod::LinearTrend,
            MultiplicityMethod::BenjaminiHochbergFdr,
            MultiplicityMethod::BenjaminiYekutieliFdr,
            MultiplicityMethod::TwoStageBkyFdr,
        ]
    }

    fn build_fixture_corpus() -> Result<String, Box<dyn std::error::Error>> {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
        if let [c0, c1] = col.data_sets.as_mut_slice() {
            c0.title = String::from("Vehicle [Ctrl]");
            c1.title = String::from("Drug | High");
            if let Some(cell) = c0.cells.get_mut(0).and_then(|r| r.first_mut()) {
                *cell = Cell::number(1.0);
            }
            if let Some(cell) = c0.cells.get_mut(1).and_then(|r| r.first_mut()) {
                *cell = Cell { excluded: true, ..Cell::number(99.0) };
            }
        }
        let mut ds = DataSheet::new("ds-1", "Cohort & Drug", TableArchetype::Column(col))?;
        ds.frozen = true;
        let mut doc = ProjectDocument::new("Refactored Byte Parity")?;
        doc.add_data_sheet(ds)?;

        let mut anova = AnalysisSpec::new(
            "res-1",
            "ds-1",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::BrownForsytheAndWelch,
                multiplicity: MultiplicitySpec {
                    method: MultiplicityMethod::Dunnett { control_column: 0 },
                    threshold: 0.05,
                    report_confidence_intervals: true,
                },
                ..OneWayAnovaSpec::default()
            }),
        )?;
        anova.selected_data_sets = vec![0, 1];
        let mut res = ResultSheet::new(anova)?;
        res.status = ResultStatus::UpToDate;
        res.scalars.push(ResultScalarEntry { key: String::from("Welch_W"), value: 4.5, units: Some(String::from("ratio")), data_set_index: Some(1) });
        res.tabs.push(ResultTab {
            id: String::from("tab-main"),
            title: String::from("Summary [Bracket] & Pipe | Table"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Est"), String::from("CI | 95%")],
            row_labels: vec![String::from("[Alpha]"), String::from("Row\nTwo")],
            rows: vec![
                vec![
                    ResultCell::Number { value: 1.25, prefix: None, units: None },
                    ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.5) },
                ],
                vec![ResultCell::Text(String::from("a | b\nc")), ResultCell::Empty],
            ],
            footnotes: vec![String::from("Footnote [1].")],
        });
        let mut coord_rows = Vec::new();
        let mut coord_labels = Vec::new();
        for i in 0..14 {
            coord_labels.push(format!("R{i}"));
            coord_rows.push(vec![ResultCell::Number { value: f64::from(i), prefix: None, units: None }]);
        }
        res.tabs.push(ResultTab {
            id: String::from("tab-resid"),
            title: String::from("Residuals"),
            kind: ResultTabKind::Residuals,
            column_headers: vec![String::from("Y")],
            row_labels: coord_labels,
            rows: coord_rows,
            footnotes: Vec::new(),
        });
        doc.add_result_sheet(res)?;

        let missing_spec = AnalysisSpec::new("res-missing", "ds-gone", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?;
        let mut missing_res = ResultSheet::new(missing_spec)?;
        missing_res.status = ResultStatus::Dirty;
        doc.add_result_sheet(missing_res)?;

        let nl_res = ResultSheet::new(AnalysisSpec::new("res-nl", "ds-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear")))?)?;
        doc.add_result_sheet(nl_res)?;

        let mut corpus = String::new();
        for id in ["res-1", "res-missing", "res-nl"] {
            for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
                let report = generate_methods_report(&doc, &SheetId::new(id), fmt)?;
                let _ = writeln!(corpus, "=== {id} / {} ===\n{report}", fmt.as_str());
            }
        }

        let ds_ref = doc.data_sheet(&SheetId::new("ds-1"));
        let res_ref = doc.result_sheet(&SheetId::new("res-1")).ok_or("missing res-1")?;
        for method in all_multiplicity_methods() {
            for ci in [false, true] {
                let spec = MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: ci };
                let blocks = vec![
                    ReportBlock::Paragraph(describe_multiplicity_runs(&spec, ci)),
                    ReportBlock::Paragraph(describe_dunn_multiplicity_runs(&spec, ds_ref, "rank-sum")),
                    ReportBlock::Paragraph(describe_survival_pairwise_runs(&spec, ds_ref)),
                    {
                        let mut welch = Vec::new();
                        append_welch_posthoc_runs(&mut welch, &spec, res_ref, ds_ref, ci);
                        ReportBlock::Paragraph(welch)
                    },
                ];
                for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
                    let rendered = render_blocks(&blocks, fmt);
                    let _ = writeln!(corpus, "=== mult / {method:?} / {ci} / {} ===\n{rendered}", fmt.as_str());
                }
            }
        }

        // Cover 0-column Table edge case across all 3 formats.
        let empty_tbl =
            [ReportBlock::Table(TableBlock { caption: vec![plain("Empty")], headers: Vec::new(), rows: Vec::new(), footnotes: Vec::new() })];
        for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
            let rendered = render_blocks(&empty_tbl, fmt);
            let _ = writeln!(corpus, "=== empty_tbl / {} ===\n{rendered}", fmt.as_str());
        }
        Ok(corpus)
    }

    #[test]
    fn methods_report_outputs_stay_byte_identical() {
        let corpus = match build_fixture_corpus() {
            Ok(c) => c,
            Err(err) => {
                assert_eq!(err.to_string(), "");
                return;
            }
        };
        let digest = dual_fnv1a(corpus.as_bytes());
        assert_eq!(digest, (77_365, 17_018_405_250_278_487_102, 7_477_326_593_824_657_170));
    }
}
