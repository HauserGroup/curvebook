//! Clean-room ICMJE/ARRIVE statistical methods and results report generator (`QOL-6`).
//!
//! Produces deterministic [`ReportFormat::PlainText`], [`ReportFormat::Markdown`], and
//! [`ReportFormat::Latex`] narratives directly from stored [`AnalysisSpec`], [`DataSheet`],
//! and [`ResultSheet`] records without mutating any stored numbers.

use std::fmt::Write as _;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, BlandAltmanSpec, ChecklistItem, ChecklistStatus, CiBound, ClusterDistance, ClusterLinkage,
    ContingencyMethod, ContingencySpec, CorrelationPowerMethod, CoxTieMethod, DataSheet, DescriptiveSpec, ErrorPropagationMode, FactorialAnovaSpec,
    FitMethod, GlmFamily, GlmSpec, KappaWeighting, LinearRegressionSpec, MathTransformPreset, MixedModelDfMethod, ModelComparisonSpec,
    MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, OneWayMethod, OutlierSpec, PValueStackMethod, PValueStackSpec, ParameterRule, ParameterScope, PassingBablokSpec,
    PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection, PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument,
    RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, RocCiMethod, RocSpec, SheetId,
    SimulationNoise, SubcolumnAggregation, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, TwoGroupMethod,
    TwoGroupSpec, TwoProportionsMethod, WeightingScheme,
};
use curvebook_fit::get_builtin_model;

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

// ---------------------------------------------------------------------------
// Report Assembly
// ---------------------------------------------------------------------------

fn build_report_blocks(doc: &ProjectDocument, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let mut blocks = Vec::new();
    blocks.push(ReportBlock::Title(vec![plain(format!("Statistical Methods and Results Report: {}", res.title()))]));

    append_study_context_section(&mut blocks, doc, res, source_ds);
    append_methods_section(&mut blocks, res, source_ds);
    append_results_section(&mut blocks, res, source_ds);
    append_diagnostics_section(&mut blocks, res);

    blocks
}

fn append_study_context_section(blocks: &mut Vec<ReportBlock>, doc: &ProjectDocument, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    blocks.push(ReportBlock::Section(vec![plain("Study Context and Data Provenance (ARRIVE / ICMJE)")]));

    let mut bullets = vec![
        vec![bold("Project: "), plain(&doc.title)],
        vec![bold("Analysis sheet: "), plain(res.title()), plain(" ("), code(res.id.as_str()), plain(")")],
        vec![bold("Procedure: "), plain(res.analysis.kind.derive_auto_title())],
        vec![bold("Recalculation status: "), status_run(&res.status)],
    ];

    if let Some(ds) = source_ds {
        let arch_label = ds.kind().label();
        let rows = ds.archetype.row_count();
        let cols = ds.archetype.column_count();
        let mut src_line = vec![
            bold("Source table: "),
            plain(&ds.title),
            plain(" ("),
            code(ds.id.as_str()),
            plain(format!("; {arch_label}, {rows} rows x {cols} columns")),
        ];
        if ds.frozen {
            src_line.push(plain("; frozen snapshot"));
        }
        src_line.push(plain(")"));
        bullets.push(src_line);

        let selected_labels = selected_column_labels(ds, &res.analysis.selected_data_sets);
        if !selected_labels.is_empty() {
            let scope = if res.analysis.selected_data_sets.is_empty() { "All columns analyzed: " } else { "Selected columns analyzed: " };
            bullets.push(vec![bold(scope), plain(selected_labels.join(", "))]);
        }

        let excluded_count = count_excluded_cells(ds);
        let documented_count = count_active_exclusion_notes(ds);
        bullets.push(exclusion_audit_bullet(excluded_count, documented_count));
    } else {
        bullets.push(vec![bold("Source table ID: "), code(res.analysis.source_sheet_id.as_str())]);
    }

    blocks.push(ReportBlock::Bullets(bullets));
}

fn status_run(status: &ResultStatus) -> TextRun {
    match status {
        ResultStatus::UpToDate => plain("Up to date"),
        ResultStatus::Dirty => plain("Pending recalculation (Dirty)"),
        ResultStatus::Failed { code: err_code, message } => plain(format!("Failed ({err_code}): {message}")),
    }
}

fn exclusion_audit_bullet(excluded_count: usize, documented_count: usize) -> Vec<TextRun> {
    if excluded_count == 0 {
        vec![bold("Observation exclusions: "), plain("0 cells excluded; all finite active observations were included.")]
    } else {
        vec![
            bold("Observation exclusions: "),
            plain(format!(
                "{excluded_count} cell(s) masked as excluded prior to analysis ({documented_count} with recorded audit justification note(s))."
            )),
        ]
    }
}

fn append_methods_section(blocks: &mut Vec<ReportBlock>, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    blocks.push(ReportBlock::Section(vec![plain("Statistical and Mathematical Methods")]));

    let spec = &res.analysis;
    let conf_pct = format_exact_float(spec.confidence_level.percent());
    let alpha = format_exact_float(spec.confidence_level.alpha());

    blocks.push(ReportBlock::Paragraph(vec![
        plain("Analyses were conducted in CurveBook with "),
        math(format!("{conf_pct}%"), format!("{conf_pct}%"), format!("{conf_pct}\\%")),
        plain(" confidence intervals and nominal significance threshold "),
        math(format!("alpha = {alpha}"), format!("alpha = {alpha}"), format!("\\alpha = {alpha}")),
        plain(" ("),
        math("p < alpha", "p < alpha", "p < \\alpha"),
        plain(" vs. "),
        math("p >= alpha", "p >= alpha", "p \\ge \\alpha"),
        plain("), using stored P-value formatting rule "),
        plain(p_format_label(spec.p_value_format)),
        plain(". Stored summary estimates use exact integer or short verbatim decimal formatting, or compact 6-decimal fixed/scientific formatting when representations exceed 10-12 characters."),
    ]));

    let mut method_paragraphs = describe_analysis_kind(spec, res, source_ds);
    blocks.append(&mut method_paragraphs);
}

fn describe_analysis_kind(spec: &AnalysisSpec, res: &ResultSheet, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(s) => describe_two_group(s, source_ds),
        AnalysisKind::MultipleRowTTests(s) => describe_multiple_row_tests(s),
        AnalysisKind::OneWayAnova(s) => describe_one_way_anova(s, source_ds),
        AnalysisKind::FactorialAnova(s) => describe_factorial_anova(s, source_ds),
        AnalysisKind::NestedComparison(s) => describe_nested(s),
        AnalysisKind::BlandAltman(s) => describe_bland_altman(s, source_ds, spec.confidence_level.percent()),
        AnalysisKind::PassingBablok(s) => describe_passing_bablok(s, source_ds),
        AnalysisKind::PValueStack(s) => describe_pvalue_stack(s, source_ds),
        AnalysisKind::PowerAnalysis(s) => vec![ReportBlock::Paragraph(vec![plain(describe_power_analysis_spec(s))])],
        AnalysisKind::Contingency(s) => describe_contingency(s, spec, source_ds, res),
        AnalysisKind::Survival(s) => describe_survival(s, source_ds),
        AnalysisKind::RocCurve(s) => describe_roc(s),
        AnalysisKind::GeneralizedLinearModel(s) => describe_glm(s, source_ds),
        AnalysisKind::Multivariate(s) => describe_multivariate(s, source_ds),
        AnalysisKind::LinearRegression(s) => describe_linear_regression(s),
        AnalysisKind::NonlinearRegression(s) => describe_nonlinear_regression(s, res),
        AnalysisKind::DescriptiveStats(s) => describe_descriptive(s),
        AnalysisKind::OutlierDetection(s) => describe_outlier_detection(s),
        AnalysisKind::Transform(s) => describe_transform(s),
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
        runs.push(plain(format!(" Grouping variable `{g_lbl}` and response variable `{r_lbl}` were extracted from the multivariable table.")));
    }
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_multiple_row_tests(s: &MultipleRowTestsSpec) -> Vec<ReportBlock> {
    let family = match (s.nonparametric, s.paired, s.assume_equal_sd) {
        (true, true, _) => "Wilcoxon matched-pairs signed-rank tests",
        (true, false, _) => "Mann-Whitney U rank-sum tests",
        (false, true, _) => "paired Student's t-tests",
        (false, false, true) => "unpaired Student's t-tests assuming equal SD",
        (false, false, false) => "Welch's unpaired t-tests without assuming equal SD",
    };
    let mut runs = vec![plain(format!("Row-wise two-group comparisons were evaluated using {family}. "))];
    runs.extend(describe_multiplicity_runs(&s.multiplicity));
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_one_way_anova(s: &OneWayAnovaSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let mut runs = match s.method {
        OneWayMethod::OrdinaryAnova => vec![plain(
            "Group means across three or more independent conditions were compared using ordinary one-way ANOVA assuming homogeneous variances (checked via Brown-Forsythe test).",
        )],
        OneWayMethod::BrownForsytheAndWelch => vec![plain(
            "Groups with potentially unequal variances were compared using Welch's ANOVA (W) and the Brown-Forsythe F* ANOVA without assuming variance homogeneity.",
        )],
        OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
            let df_lbl = match s.df_method {
                MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
                MixedModelDfMethod::Satterthwaite => "Satterthwaite",
            };
            if s.residual_covariance == ResidualCovariance::Unstructured {
                vec![plain(format!(
                    "Matched subject blocks were analyzed via a mixed-effects REML model with unstructured residual covariance and {df_lbl} degrees of freedom."
                ))]
            } else if s.mixed_effects_reml {
                vec![plain(format!(
                    "Matched subject blocks were analyzed via a compound-symmetry mixed-effects REML model with {df_lbl} degrees of freedom."
                ))]
            } else if geisser_greenhouse {
                vec![plain(format!(
                    "Matched subject blocks were analyzed via repeated-measures one-way ANOVA with the Geisser-Greenhouse sphericity correction applied to treatment degrees of freedom (reporting epsilon_GG, epsilon_HF, and Mauchly's sphericity test; falling back to mixed-effects REML with {df_lbl} degrees of freedom if observations are missing)."
                ))]
            } else {
                vec![plain(format!(
                    "Matched subject blocks were analyzed via repeated-measures one-way ANOVA assuming compound symmetry / sphericity (with Geisser-Greenhouse and Huynh-Feldt epsilon diagnostics reported; falling back to mixed-effects REML with {df_lbl} degrees of freedom if observations are missing)."
                ))]
            }
        }
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
            runs.push(plain(" Variance-explained effect size (partial eta-squared) was computed."));
        }
        OneWayMethod::OrdinaryAnova | OneWayMethod::BrownForsytheAndWelch | OneWayMethod::KruskalWallis | OneWayMethod::Friedman => {}
    }
    match s.method {
        OneWayMethod::OrdinaryAnova | OneWayMethod::RepeatedMeasures { .. } => {
            if let MultiplicityMethod::Dunnett { control_column } = s.multiplicity.method {
                let ctrl_name = column_title_at(source_ds, control_column);
                runs.push(plain(format!(" Control column for post-hoc comparisons: {ctrl_name}.")));
            }
            runs.push(plain(" "));
            runs.extend(describe_multiplicity_runs(&s.multiplicity));
        }
        OneWayMethod::BrownForsytheAndWelch => {
            let eff_method = match s.multiplicity.method {
                m @ (MultiplicityMethod::GamesHowell | MultiplicityMethod::TamhaneT2 | MultiplicityMethod::DunnettT3) => m,
                _ => MultiplicityMethod::GamesHowell,
            };
            let eff_mult = MultiplicitySpec { method: eff_method, ..s.multiplicity };
            runs.push(plain(" "));
            runs.extend(describe_multiplicity_runs(&eff_mult));
        }
        OneWayMethod::KruskalWallis => {
            runs.push(plain(" "));
            runs.extend(describe_dunn_multiplicity_runs(&s.multiplicity, source_ds));
        }
        OneWayMethod::Friedman => {
            runs.push(plain(" Post-hoc pairwise comparisons were not computed for the Friedman rank test."));
        }
    }
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_factorial_anova(s: &FactorialAnovaSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
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
    let df_lbl = match s.df_method {
        MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite",
    };
    let reml_text = match (s.repeated_measures, s.residual_covariance, s.fit_reml_on_missing || s.mixed_effects_reml || s.three_way) {
        (RepeatedMeasuresDesign::None, _, _) => String::new(),
        (_, ResidualCovariance::Unstructured, _) => format!(
            " Repeated-measures correlations were modeled with an unstructured residual covariance matrix estimated by Cholesky restricted maximum likelihood (REML) with {df_lbl} degrees of freedom."
        ),
        (_, ResidualCovariance::CompoundSymmetry, true) => format!(
            " Repeated-measures mixed-effects restricted maximum likelihood (REML) with compound-symmetry covariance and {df_lbl} degrees of freedom was fitted (retaining incomplete or multi-way repeated-measures blocks without listwise deletion)."
        ),
        (_, ResidualCovariance::CompoundSymmetry, false) => String::new(),
    };
    let mut runs = vec![plain(format!("Factorial comparisons used {design_label} with {ss_label} sums of squares and {rm_label}.{reml_text}"))];
    if let Some(resp) = s.response_column {
        let resp_lbl = column_title_at(source_ds, resp);
        runs.push(plain(format!(" Response variable: {resp_lbl}.")));
    }
    if s.compute_effect_sizes {
        runs.push(plain(" Partial eta-squared and omega-squared effect sizes were calculated."));
    }
    runs.push(plain(" "));
    runs.extend(describe_multiplicity_runs(&s.multiplicity));
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_nested(s: &NestedAnalysisSpec) -> Vec<ReportBlock> {
    let test_name = if s.two_group_t_test { "nested hierarchical t-test" } else { "nested hierarchical one-way ANOVA" };
    let df_lbl = match s.df_method {
        MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite",
    };
    let mut runs = vec![plain(format!(
        "Subcolumn technical/biological replicates nested within clusters were analyzed via a {test_name} (mixed-effects REML variance decomposition of cluster vs. residual variance with {df_lbl} denominator degrees of freedom). "
    ))];
    runs.extend(describe_multiplicity_runs(&s.multiplicity));
    vec![ReportBlock::Paragraph(runs)]
}

fn describe_bland_altman(s: &BlandAltmanSpec, source_ds: Option<&DataSheet>, conf_percent: f64) -> Vec<ReportBlock> {
    let col_a = column_title_at(source_ds, s.column_a);
    let col_b = column_title_at(source_ds, s.column_b);
    let scale = if s.percent_difference { "percentage differences relative to the pair mean" } else { "raw paired differences (A - B)" };
    let conf_pct = format_exact_float(conf_percent);
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Method agreement between `{col_a}` and `{col_b}` was evaluated via Bland-Altman analysis of {scale} against pair averages, reporting mean bias, standard deviation of differences, and {conf_pct}% limits of agreement with confidence intervals."
    ))])]
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
    let col_clause = match s.column {
        Some(col) => format!("on `{}`", column_title_at(source_ds, col)),
        None => String::from("across the selected P-value column(s)"),
    };
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Raw P values {col_clause} were evaluated for statistical discovery using the {} procedure ({citation}) to control the {control} at {threshold_sym} = {}. Adjusted P values (or FDR q-values) and boolean discovery indicators preserve the original input row order and rank mapping.",
        s.method.label(),
        s.threshold
    ))])]
}

fn describe_passing_bablok(s: &PassingBablokSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    let col_x = column_title_at(source_ds, s.x_column);
    let col_y = column_title_at(source_ds, s.y_column);
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Nonparametric Passing-Bablok method-comparison regression of `{col_y}` against `{col_x}` computed the shifted median of valid pairwise slopes and median intercept, with rank-based confidence intervals and CUSUM linearity testing against the identity line (slope = 1, intercept = 0)."
    ))])]
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

fn describe_survival(s: &SurvivalAnalysisSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
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
                runs.push(plain(format!(" Extracted from multivariable columns: time=`{t_lbl}`, status=`{s_lbl}`, group=`{g_lbl}`.")));
            }
            if !matches!(multiplicity.method, MultiplicityMethod::None) {
                runs.push(plain(" For "));
                runs.push(math(">= 3", ">= 3", "\\ge 3"));
                runs.push(plain(" cohorts, pairwise log-rank comparisons used "));
                runs.extend(describe_multiplicity_runs(multiplicity));
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
            let pred_lbls: Vec<String> = predictor_columns.iter().map(|&c| column_title_at(source_ds, c)).collect();
            vec![ReportBlock::Paragraph(vec![plain(format!(
                "Semi-parametric Cox proportional hazards regression modeled time `{time_lbl}` and event status `{status_lbl}` across predictors ({}) using {tie_lbl} tie handling. Reported outputs include regression coefficients (beta), Wald z-statistics, hazard ratios with confidence intervals, Harrell's concordance index (C-index), likelihood-ratio omnibus test, and Schoenfeld residual proportional-hazards tests.",
                pred_lbls.join(", ")
            ))])]
        }
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
    let pred_lbls: Vec<String> = s.predictor_columns.iter().map(|&c| column_title_at(source_ds, c)).collect();
    let vif_note = if s.compute_vif { " Predictor multicollinearity was evaluated using Variance Inflation Factors (VIF)." } else { "" };
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Multivariable modeling of response `{resp_lbl}` on predictors ({}) used {family_desc}.{vif_note}",
        pred_lbls.join(", ")
    ))])]
}

fn describe_multivariate(s: &MultivariateSpec, source_ds: Option<&DataSheet>) -> Vec<ReportBlock> {
    match s {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => {
            let matrix_kind =
                if *standardize { "standardized correlation matrix (z-scored variables)" } else { "unstandardized sample covariance matrix" };
            let pa_note = match parallel_analysis_seed {
                Some(seed) => format!(
                    " Component retention combined the Kaiser criterion (eigenvalue >= 1) with Horn's parallel analysis at the 95th percentile (RNG seed {seed})."
                ),
                None => String::from(" Component retention was evaluated using the Kaiser eigenvalue criterion."),
            };
            vec![ReportBlock::Paragraph(vec![plain(format!(
                "Principal component analysis (PCA) decomposed the {matrix_kind} across complete multivariate rows.{pa_note}"
            ))])]
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
            let mut extra = String::new();
            if let Some(resp) = response_column {
                let r_lbl = column_title_at(source_ds, *resp);
                let p_lbls: Vec<String> = predictor_columns.iter().map(|&c| column_title_at(source_ds, c)).collect();
                let _ = write!(extra, " Downstream pooled regression modeled `{r_lbl}` on ({}).", p_lbls.join(", "));
            }
            vec![ReportBlock::Paragraph(vec![plain(format!(
                "Missing data were handled by Multiple Imputation by Chained Equations (m = {imputations_m} imputed datasets, max_iters = {max_iters}, RNG seed = {seed}). Estimates were combined using Rubin's (1987) pooling rules (within-imputation variance U, between-imputation variance B, total variance T, relative increase in variance RIV, fraction of missing information FMI, and Barnard-Rubin adjusted degrees of freedom).{extra}"
            ))])]
        }
    }
}

fn describe_linear_regression(s: &LinearRegressionSpec) -> Vec<ReportBlock> {
    if s.deming_model_ii {
        return vec![ReportBlock::Paragraph(vec![plain(
            "Model II Deming orthogonal linear regression was fitted to account for measurement error in both X and Y variables, with standard errors and confidence intervals computed by leave-one-out jackknife resampling.",
        )])];
    }
    let origin =
        if s.force_through_origin { "constrained through the origin (Y = Slope * X)" } else { "with free Y-intercept (Y = Slope * X + YIntercept)" };
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
    let extra_str = if extras.is_empty() { String::new() } else { format!(" Enabled outputs: {}.", extras.join(", ")) };
    vec![ReportBlock::Paragraph(vec![plain(format!(
        "Ordinary least-squares simple linear regression was fitted {origin}, including a runs test for departure from linearity and ANCOVA comparison of slopes and intercepts when multiple datasets are present.{extra_str}"
    ))])]
}

fn describe_nonlinear_regression(s: &NonlinearFitSpec, res: &ResultSheet) -> Vec<ReportBlock> {
    let mut blocks = Vec::new();

    // 1. Model identity and mathematical equation formula
    let (model_name, model_params, formula_plain, formula_latex) = resolve_equation_description(s);
    blocks.push(ReportBlock::Paragraph(vec![
        bold("Model and equation: "),
        plain(format!("{model_name} (")),
        code(&s.equation_id),
        plain("). Equation: "),
        math(formula_plain.clone(), formula_plain, formula_latex),
        plain(if model_params.is_empty() { String::new() } else { format!("; parameters: {}.", model_params.join(", ")) }),
    ]));

    // 2. Objective function, ROUT outlier settings, weighting, and iterations
    let mut obj_runs = Vec::new();
    match s.method {
        FitMethod::LeastSquares => {
            obj_runs.push(plain(
                "Parameters were estimated by trust-region Levenberg-Marquardt nonlinear regression (with Nelder-Mead simplex warm-start)",
            ));
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
            obj_runs.push(plain(
                "Outliers were identified and excluded using the ROUT method (robust Cauchy fit, RSDR scale, Benjamini-Hochberg FDR control at ",
            ));
            obj_runs.push(math(format!("Q = {q_str}%"), format!("Q = {q_str}%"), format!("Q = {q_str}\\%")));
            obj_runs.push(plain("), followed by ordinary/weighted least-squares refitting on the retained observations"));
        }
    }
    obj_runs.push(plain(format!(" (maximum iterations = {}). ", s.clamped_max_iters())));
    obj_runs.extend(describe_weighting_runs(s.weighting));

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
        let mut rule_bullets = Vec::with_capacity(s.parameter_rules.len());
        for rule in &s.parameter_rules {
            rule_bullets.push(describe_parameter_rule(rule));
        }
        blocks.push(ReportBlock::Paragraph(vec![bold("Parameter constraints and global sharing rules:")]));
        blocks.push(ReportBlock::Bullets(rule_bullets));
    }

    // 4. Model comparison and residual / range diagnostics
    let mut extras = Vec::new();
    if let Some(cmp) = &s.comparison {
        if s.method == FitMethod::RobustCauchy {
            extras.push(vec![plain(
                "Model comparison was marked Not applicable (NotApplicableInRobustMode) because Robust (Cauchy) regression does not support least-squares F, AICc, or likelihood-ratio tests.",
            )]);
        } else {
            extras.push(describe_model_comparison_runs(cmp));
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

fn describe_weighting_runs(weighting: WeightingScheme) -> Vec<TextRun> {
    match weighting {
        WeightingScheme::None => vec![plain("Equal weighting (unweighted, assuming uniform residual SD) was used.")],
        WeightingScheme::RelativeSquare => vec![
            plain("Observations were weighted by "),
            math("1 / Y^2", "1 / Y^2", "1 / Y^2"),
            plain(" (minimizing relative squared residuals for constant %CV scatter)."),
        ],
        WeightingScheme::Relative => vec![
            plain("Observations were weighted by "),
            math("1 / |Y|", "1 / |Y|", "1 / |Y|"),
            plain(" (Poisson-like variance proportional to |Y|)."),
        ],
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

fn describe_parameter_rule(rule: &ParameterRule) -> Vec<TextRun> {
    let scope_desc = match &rule.scope {
        ParameterScope::Local => String::from("fitted independently per dataset (local)"),
        ParameterScope::SharedGlobal => String::from("shared globally across all datasets"),
        ParameterScope::Constant(v) => format!("constrained to fixed constant {}", format_exact_float(*v)),
        ParameterScope::HookedConstant(hook) => format!("constrained to hooked constant ({hook:?})"),
        ParameterScope::PerDatasetConstant(pairs) => {
            let parts: Vec<String> = pairs.iter().map(|(ds, v)| format!("DS{} = {}", ds + 1, format_exact_float(*v))).collect();
            format!("constrained per dataset ({})", parts.join(", "))
        }
        ParameterScope::ColumnTitleConstant => String::from("constrained to numeric value parsed from dataset column title"),
    };
    let mut runs = vec![code(&rule.name), plain(format!(": {scope_desc}"))];
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

fn describe_model_comparison_runs(cmp: &ModelComparisonSpec) -> Vec<TextRun> {
    match cmp {
        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id } => {
            let alt_name = get_builtin_model(alternative_equation_id).map_or_else(|| alternative_equation_id.clone(), |m| m.name().to_owned());
            vec![plain(format!(
                "Nested model selection compared the primary model against `{alt_name}` (`{alternative_equation_id}`) using the extra-sum-of-squares F-test."
            ))]
        }
        ModelComparisonSpec::Aicc { alternative_equation_id } => {
            let alt_name = get_builtin_model(alternative_equation_id).map_or_else(|| alternative_equation_id.clone(), |m| m.name().to_owned());
            vec![
                plain("Model selection compared the primary model against "),
                plain(format!("`{alt_name}` (`{alternative_equation_id}`) via ")),
                math("AICc", "AICc", "\\mathrm{AIC}_c"),
                plain(", Akaike weights, and evidence ratio."),
            ]
        }
        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id } => {
            let alt_name = get_builtin_model(alternative_equation_id).map_or_else(|| alternative_equation_id.clone(), |m| m.name().to_owned());
            vec![plain(format!(
                "Nested model selection compared the primary model against `{alt_name}` (`{alternative_equation_id}`) using the likelihood-ratio test (D = 2 * delta ln L)."
            ))]
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
    let detail = match s {
        TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation } => {
            let x_str = x_transform.map_or_else(|| String::from("none"), |p| format_math_preset(p, "X"));
            let y_str = y_transform.map_or_else(|| String::from("none"), |p| format_math_preset(p, "Y"));
            let err_str = match error_propagation {
                ErrorPropagationMode::ReplicateEndpoints => "replicate endpoint propagation",
                ErrorPropagationMode::FirstOrderDeltaMethod => "first-order Taylor delta method",
            };
            format!("Mathematical table transformation (X: {x_str}; Y: {y_str}) with {err_str} for summary error columns.")
        }
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set } => {
            let scale = if *output_percent { "0% to 100%" } else { "0.0 to 1.0" };
            let scope = if *per_data_set { "per dataset" } else { "globally across datasets" };
            format!(
                "Data normalization onto [{scale}] {scope} anchored at 0={} and 100={}.",
                format_anchor(zero_anchor),
                format_anchor(hundred_anchor)
            )
        }
        TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation } => {
            let op_lbl = match operation {
                BaselineOperation::Difference => "difference",
                BaselineOperation::Ratio => "ratio",
                BaselineOperation::PercentChange => "percent change",
                BaselineOperation::FractionalChange => "fractional change",
            };
            format!("Baseline removal ({op_lbl}) relative to baseline column indices {baseline_columns:?}.")
        }
        TransformAnalysisSpec::Transpose { swap_row_and_column_titles } => {
            format!("Matrix transposition (swap row/column titles = {swap_row_and_column_titles}).")
        }
        TransformAnalysisSpec::PruneRows { every_k_rows, average_window } => {
            format!("Periodic row pruning every {every_k_rows} row(s) (average_window = {average_window}).")
        }
        TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype } => {
            let flt = filter_formula.as_deref().unwrap_or("none");
            format!("Subset selection and archetype reshape into {} (filter: {flt}).", target_archetype.label())
        }
        TransformAnalysisSpec::Simulate { seed, row_count, replicates, x_min, x_max, equation_id, noise, .. } => {
            let noise_str = match noise {
                SimulationNoise::None => String::from("noise-free"),
                SimulationNoise::Gaussian { sd } => format!("Gaussian noise (SD = {})", format_exact_float(*sd)),
                SimulationNoise::RelativePercent { cv_percent } => format!("relative noise (%CV = {})", format_exact_float(*cv_percent)),
                SimulationNoise::Poisson => String::from("Poisson count noise"),
            };
            format!(
                "Monte Carlo curve simulation for `{equation_id}` ({row_count} rows x {replicates} replicates, X in [{}, {}], {noise_str}, RNG seed = {seed}).",
                format_exact_float(*x_min),
                format_exact_float(*x_max)
            )
        }
        TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation } => {
            let strat_desc = match stratify_column {
                Some(col_idx) => {
                    let alloc = if *proportional_allocation { "proportional" } else { "equal-quota" };
                    format!(" stratified by column {} using {alloc} allocation", col_idx + 1)
                }
                None => String::from(" without stratification"),
            };
            format!(
                "Seeded deterministic Fisher-Yates random sampling without replacement selected n = {sample_size} rows{strat_desc} (RNG seed = {seed})."
            )
        }
    };
    vec![ReportBlock::Paragraph(vec![plain(detail)])]
}

fn describe_multiplicity_runs(mult: &MultiplicitySpec) -> Vec<TextRun> {
    let thresh = format_exact_float(mult.threshold);
    let ci_note = if mult.report_confidence_intervals { " with multiplicity-adjusted confidence intervals" } else { "" };
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
        return vec![plain(format!("Multiple comparisons used {kind_lbl}."))];
    }
    let target_math = if is_fdr {
        math(format!("Q = {thresh}"), format!("Q = {thresh}"), format!("Q = {thresh}"))
    } else {
        math(format!("familywise alpha = {thresh}"), format!("familywise alpha = {thresh}"), format!("\\text{{familywise }}\\alpha = {thresh}"))
    };
    vec![plain(format!("Multiple comparisons used {kind_lbl} at ")), target_math, plain(format!("{ci_note}."))]
}

fn describe_dunn_multiplicity_runs(mult: &MultiplicitySpec, source_ds: Option<&DataSheet>) -> Vec<TextRun> {
    let thresh = format_exact_float(mult.threshold);
    match mult.method {
        MultiplicityMethod::None | MultiplicityMethod::FisherLsd => {
            vec![plain("Multiple comparisons used unadjusted Dunn's nonparametric rank-sum pairwise z-tests (no multiplicity correction applied).")]
        }
        MultiplicityMethod::Dunnett { control_column } => {
            let ctrl_name = column_title_at(source_ds, control_column);
            vec![
                plain(format!(
                    "Control column for post-hoc comparisons: {ctrl_name}. Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests against the control group with Bonferroni FWER adjustment at "
                )),
                math(
                    format!("familywise alpha = {thresh}"),
                    format!("familywise alpha = {thresh}"),
                    format!("\\text{{familywise }}\\alpha = {thresh}"),
                ),
                plain("."),
            ]
        }
        MultiplicityMethod::DunnRankSum
        | MultiplicityMethod::LinearTrend
        | MultiplicityMethod::TukeyHsd
        | MultiplicityMethod::NewmanKeuls
        | MultiplicityMethod::GamesHowell
        | MultiplicityMethod::DunnettT3
        | MultiplicityMethod::Bonferroni => vec![
            plain("Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests with Bonferroni FWER adjustment at "),
            math(format!("familywise alpha = {thresh}"), format!("familywise alpha = {thresh}"), format!("\\text{{familywise }}\\alpha = {thresh}")),
            plain("."),
        ],
        MultiplicityMethod::TamhaneT2 | MultiplicityMethod::Sidak => vec![
            plain("Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests with Sidak FWER adjustment at "),
            math(format!("familywise alpha = {thresh}"), format!("familywise alpha = {thresh}"), format!("\\text{{familywise }}\\alpha = {thresh}")),
            plain("."),
        ],
        MultiplicityMethod::HolmSidak => vec![
            plain("Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests with Holm-Sidak step-down FWER adjustment at "),
            math(format!("familywise alpha = {thresh}"), format!("familywise alpha = {thresh}"), format!("\\text{{familywise }}\\alpha = {thresh}")),
            plain("."),
        ],
        MultiplicityMethod::BenjaminiHochbergFdr => vec![
            plain("Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests with Benjamini-Hochberg step-up FDR control at "),
            math(format!("Q = {thresh}"), format!("Q = {thresh}"), format!("Q = {thresh}")),
            plain("."),
        ],
        MultiplicityMethod::BenjaminiYekutieliFdr => vec![
            plain(
                "Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests with Benjamini-Yekutieli arbitrary-dependence FDR control at ",
            ),
            math(format!("Q = {thresh}"), format!("Q = {thresh}"), format!("Q = {thresh}")),
            plain("."),
        ],
        MultiplicityMethod::TwoStageBkyFdr => vec![
            plain(
                "Multiple comparisons used Dunn's nonparametric rank-sum pairwise z-tests with two-stage Benjamini-Krieger-Yekutieli adaptive FDR control at ",
            ),
            math(format!("Q = {thresh}"), format!("Q = {thresh}"), format!("Q = {thresh}")),
            plain("."),
        ],
    }
}

// ---------------------------------------------------------------------------
// Results & Diagnostics Sections (Verbatim Read-Out from ResultSheet)
// ---------------------------------------------------------------------------

fn append_results_section(blocks: &mut Vec<ReportBlock>, res: &ResultSheet, source_ds: Option<&DataSheet>) {
    blocks.push(ReportBlock::Section(vec![plain("Stored Results Summary")]));

    if !res.scalars.is_empty() {
        let mut scalar_bullets = Vec::with_capacity(res.scalars.len());
        for entry in &res.scalars {
            let ds_suffix = entry.data_set_index.map_or_else(String::new, |idx| format!(" [{}]", column_title_at(source_ds, idx)));
            let units_suffix = entry.units.as_ref().map_or_else(String::new, |u| format!(" {u}"));
            scalar_bullets
                .push(vec![bold(format!("{}{ds_suffix}: ", entry.key)), plain(format!("{}{units_suffix}", format_stat_value(entry.value)))]);
        }
        blocks.push(ReportBlock::Paragraph(vec![bold("Key scalar outputs:")]));
        blocks.push(ReportBlock::Bullets(scalar_bullets));
    }

    for tab in &res.tabs {
        if let Some(tbl) = render_result_tab_block(tab) {
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

fn render_result_tab_block(tab: &ResultTab) -> Option<ReportBlock> {
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
            bold(format!("{}: ", tab.title)),
            plain(format!("{} row(s) across {} column(s) stored in result tab `{}`.", tab.rows.len(), tab.column_headers.len(), tab.id)),
        ]));
    }

    let mut headers = vec![vec![plain("Measure / Row")]];
    for h in &tab.column_headers {
        headers.push(vec![plain(h)]);
    }

    let shown = tab.rows.len().min(max_rows);
    let mut rows = Vec::with_capacity(shown);
    for (idx, row) in tab.rows.iter().take(shown).enumerate() {
        let label = tab.row_labels.get(idx).map_or_else(|| format!("Row {}", idx + 1), Clone::clone);
        let mut cells = Vec::with_capacity(tab.column_headers.len() + 1);
        cells.push(vec![plain(label)]);
        for c_idx in 0..tab.column_headers.len() {
            let cell = row.get(c_idx).unwrap_or(&ResultCell::Empty);
            cells.push(format_result_cell(cell));
        }
        rows.push(cells);
    }

    let mut footnotes: Vec<Vec<TextRun>> = tab.footnotes.iter().map(|f| vec![plain(f)]).collect();
    if tab.rows.len() > shown {
        footnotes.push(vec![plain(format!("Showing first {shown} of {} stored rows.", tab.rows.len()))]);
    }

    Some(ReportBlock::Table(TableBlock { caption: vec![plain(&tab.title)], headers, rows, footnotes }))
}

fn format_result_cell(cell: &ResultCell) -> Vec<TextRun> {
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
            if stars.trim().is_empty() { vec![plain(base)] } else { vec![plain(format!("{base} ({stars})"))] }
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
    let mut runs = vec![bold(format!("{badge} {}: ", item.assumption))];
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
        let model_name = get_builtin_model(&s.equation_id).map_or_else(|| format!("Custom equation ({})", s.equation_id), |m| m.name().to_owned());
        let params = get_builtin_model(&s.equation_id).map_or_else(Vec::new, |m| m.param_names().to_vec());
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

fn p_format_label(fmt: curvebook_doc::PValueFormat) -> String {
    match fmt {
        curvebook_doc::PValueFormat::FourDecimal => String::from("0.0001 style (4 decimals)"),
        curvebook_doc::PValueFormat::Apa => String::from("APA style (3 decimals, no leading zero)"),
        curvebook_doc::PValueFormat::Nejm => String::from("NEJM style (3 decimals with leading zero)"),
        curvebook_doc::PValueFormat::Custom { decimals, scientific_below } => match scientific_below {
            Some(thresh) => format!("custom ({decimals} decimals, scientific below {})", format_exact_float(thresh)),
            None => format!("custom ({decimals} decimals)"),
        },
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

fn format_anchor(anchor: &NormalizeAnchor) -> String {
    match anchor {
        NormalizeAnchor::ColumnMin => String::from("column minimum"),
        NormalizeAnchor::ColumnMax => String::from("column maximum"),
        NormalizeAnchor::ColumnSum => String::from("column sum"),
        NormalizeAnchor::FirstRow => String::from("first row"),
        NormalizeAnchor::LastRow => String::from("last row"),
        NormalizeAnchor::FixedValue(v) => format_exact_float(*v),
        NormalizeAnchor::ControlColumnMean { column } => format!("mean of control column {}", column + 1),
        NormalizeAnchor::Hooked(h) => format!("hooked scalar ({h:?})"),
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

fn render_runs_markdown(runs: &[TextRun], escape_pipes: bool) -> String {
    let mut s = String::new();
    for run in runs {
        match run {
            TextRun::Plain(t) => append_md_text(&mut s, t, escape_pipes),
            TextRun::Bold(t) => {
                s.push_str("**");
                append_md_text(&mut s, t, escape_pipes);
                s.push_str("**");
            }
            TextRun::Code(t) => {
                s.push('`');
                append_md_text(&mut s, t, escape_pipes);
                s.push('`');
            }
            TextRun::Math { markdown, .. } => append_md_text(&mut s, markdown, escape_pipes),
        }
    }
    s
}

fn append_md_text(dst: &mut String, text: &str, escape_pipes: bool) {
    if !escape_pipes {
        dst.push_str(text);
        return;
    }
    for ch in text.chars() {
        if ch == '|' {
            dst.push_str("\\|");
        } else if ch == '\n' {
            dst.push(' ');
        } else {
            dst.push(ch);
        }
    }
}

fn render_runs_latex(runs: &[TextRun]) -> String {
    let mut s = String::new();
    for run in runs {
        match run {
            TextRun::Plain(t) => s.push_str(&escape_latex_text(t)),
            TextRun::Bold(t) => {
                s.push_str("\\textbf{");
                s.push_str(&escape_latex_text(t));
                s.push('}');
            }
            TextRun::Code(t) => {
                s.push_str("\\texttt{");
                s.push_str(&escape_latex_text(t));
                s.push('}');
            }
            TextRun::Math { latex, .. } => {
                s.push('$');
                s.push_str(latex);
                s.push('$');
            }
        }
    }
    s
}

fn unicode_latex_math_token(ch: char) -> Option<&'static str> {
    match ch {
        '≥' => Some("\\ge"),
        '≤' => Some("\\le"),
        '±' => Some("\\pm"),
        '×' => Some("\\times"),
        'α' => Some("\\alpha"),
        'β' => Some("\\beta"),
        'γ' => Some("\\gamma"),
        'δ' => Some("\\delta"),
        'Δ' => Some("\\Delta"),
        'ε' => Some("\\varepsilon"),
        'η' => Some("\\eta"),
        'θ' => Some("\\theta"),
        'λ' => Some("\\lambda"),
        'μ' => Some("\\mu"),
        'π' => Some("\\pi"),
        'ρ' => Some("\\rho"),
        'σ' => Some("\\sigma"),
        'τ' => Some("\\tau"),
        'φ' => Some("\\phi"),
        'χ' => Some("\\chi"),
        'ω' => Some("\\omega"),
        _ => None,
    }
}

fn push_escaped_latex_char(ch: char, out: &mut String, math_mode: bool, single_line: bool) {
    if single_line && matches!(ch, '\n' | '\r' | '\t') {
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
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        push_escaped_latex_char(ch, &mut out, false, true);
    }
    out
}

fn escape_latex_math_mode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        push_escaped_latex_char(ch, &mut out, true, true);
    }
    out
}

fn render_block_plain(block: &ReportBlock, out: &mut String) {
    match block {
        ReportBlock::Title(runs) => {
            let text = render_runs_plain(runs);
            let _ = writeln!(out, "{text}");
            let _ = writeln!(out, "{}", "=".repeat(text.chars().count().max(12)));
        }
        ReportBlock::Section(runs) => {
            let text = render_runs_plain(runs);
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
            let _ = writeln!(out, "[{}]", render_runs_plain(&tbl.caption));
            let header_line: Vec<String> = tbl.headers.iter().map(|h| render_runs_plain(h)).collect();
            let _ = writeln!(out, "  {}", header_line.join(" | "));
            for row in &tbl.rows {
                let row_cells: Vec<String> = row.iter().map(|c| render_runs_plain(c)).collect();
                let _ = writeln!(out, "  {}", row_cells.join(" | "));
            }
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
            let headers: Vec<String> = tbl.headers.iter().map(|h| render_runs_markdown(h, true)).collect();
            let _ = writeln!(out, "| {} |", headers.join(" | "));
            let seps = vec!["---"; headers.len()];
            let _ = writeln!(out, "| {} |", seps.join(" | "));
            for row in &tbl.rows {
                let cells: Vec<String> = row.iter().map(|c| render_runs_markdown(c, true)).collect();
                let _ = writeln!(out, "| {} |", cells.join(" | "));
            }
            for note in &tbl.footnotes {
                let _ = writeln!(out, "*Note:* {}", render_runs_markdown(note, false));
            }
        }
    }
}

fn render_block_latex(block: &ReportBlock, out: &mut String) {
    match block {
        ReportBlock::Title(runs) => {
            let _ = writeln!(out, "\\section*{{{}}}", render_runs_latex(runs));
        }
        ReportBlock::Section(runs) => {
            let _ = writeln!(out, "\\subsection*{{{}}}", render_runs_latex(runs));
        }
        ReportBlock::Paragraph(runs) => {
            let _ = writeln!(out, "{}", render_runs_latex(runs));
        }
        ReportBlock::Bullets(items) => {
            let _ = writeln!(out, "\\begin{{itemize}}");
            for item in items {
                let _ = writeln!(out, "  \\item {}", render_runs_latex(item));
            }
            let _ = writeln!(out, "\\end{{itemize}}");
        }
        ReportBlock::Table(tbl) => {
            let _ = writeln!(out, "\\paragraph*{{{}}}", render_runs_latex(&tbl.caption));
            let col_spec = "l".repeat(tbl.headers.len().max(1));
            let _ = writeln!(out, "\\begin{{tabular}}{{{col_spec}}}");
            let _ = writeln!(out, "\\hline");
            let headers: Vec<String> = tbl.headers.iter().map(|h| render_runs_latex(h).replace(['\n', '\r'], " ")).collect();
            let _ = writeln!(out, "{} \\\\\\relax", headers.join(" & "));
            let _ = writeln!(out, "\\hline");
            for row in &tbl.rows {
                let cells: Vec<String> = row.iter().map(|c| render_runs_latex(c).replace(['\n', '\r'], " ")).collect();
                let _ = writeln!(out, "{} \\\\\\relax", cells.join(" & "));
            }
            let _ = writeln!(out, "\\hline");
            let _ = writeln!(out, "\\end{{tabular}}");
            for note in &tbl.footnotes {
                let _ = writeln!(out, "\\par\\noindent\\textit{{Note:}} {}", render_runs_latex(note));
            }
        }
    }
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
