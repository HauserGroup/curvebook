//! Graph/layout canvas interaction controller, scientific analysis/formatting dialog state
//! machines, Command Palette (`Cmd+K`) fuzzy matching, and deterministic software frame
//! compositing (`RasterImage` / PNG) for headless screenshots and visual regression tests.

use curvebook_chart::{ChartScene, PreflightIssue};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, AxisSpec, AxisThreshold, BaselineOperation, BlandAltmanSpec, BracketLabelMode, BracketStyle,
    CanvasResizeMode, ChecklistItem, ChecklistStatus, CiBound, CldOrdering, ClusterDistance, ClusterLinkage, ColorChannelMapping, ColumnDesignHint,
    CompactLetterDisplaySpec, ConnectingLineStyle, ContingencyMethod, ContingencySpec, CoxTieMethod, DataSheet, DescriptiveSpec, DiagnosticPlotKind,
    EquationDisplayMode, ErrorBarMetric, ErrorBarSpec, ErrorPropagationMode, FactorialAnovaSpec, FigureCanvasPreset, FitMethod, FramePreset,
    GlmFamily, GlmSpec, GraphAnnotation, GraphAxisScale, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, HookSource,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LegendSpec, LevelOrderKind, LevelOrderSpec, LineConnectionKind, LinearRegressionSpec,
    MathTransformPreset, MixedModelDfMethod, MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateGraphEncoding,
    MultivariatePlotMode, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OneWayMethod, OutlierSpec,
    PValueFormat, PValueStackMethod, PValueStackSpec, PageOrientation, PairwiseBracketSpec, PairwiseDisplayMode, PanelBadgeStyle, ParameterRule,
    ParameterScope, PartsOfWholeLayout, PassingBablokSpec, ProjectDocument, RectPt, RepeatedMeasuresDesign, ReplicateOverlayStyle,
    ResidualCovariance, ResultCell, ResultSheet, ResultTab, ResultTabKind, RocSpec, SheetId, SizeChannelMapping, SubcolumnAggregation,
    SumOfSquaresType, SummaryMeasure, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec,
    VariableRole, WeightingScheme, WhiskerMethod,
};
use curvebook_engine::{EngineError, Session, cmd};
use curvebook_palette::{ColorRamp, ColormapId, CvdKind, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8, simulate_cvd};
use curvebook_render::TiffCompression;
use std::collections::BTreeMap;

use curvebook_tools::{CanvasToolState, GridCellCoord, PanZoomInertiaState, PhysicalUnit, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use crate::dialogs_r9::*;
use crate::i18n::LocalePreset;
use crate::menus::shortcuts::ShortcutRegistry;
use crate::menus::ui_commands::UI_COMMANDS;
use crate::power_wizard::PowerWizardState;
use crate::theme::UiThemeSpec;

// ============================================================================
// Dialog & Inspector Discriminators
// ============================================================================

/// Active modal dialog open in the front-end shell.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ActiveDialog {
    #[default]
    None,
    FormatAxes {
        sheet_id: SheetId,
    },
    AnalyzeSheet {
        source_sheet_id: SheetId,
    },
    DuplicateAnalysisPrompt {
        source_sheet_id: SheetId,
        existing_result_ids: Vec<SheetId>,
        pending_entry_id: String,
    },
    EditAnalysisParameters {
        result_sheet_id: SheetId,
    },
    ChangeDataAnalyzed {
        result_sheet_id: SheetId,
    },
    ResultViewer {
        result_sheet_id: SheetId,
    },
    ConstantHookPicker {
        target_sheet_id: SheetId,
    },
    ChangeGraphType {
        graph_sheet_id: SheetId,
    },
    FormatGraph {
        graph_sheet_id: SheetId,
    },
    PairwiseComparisons {
        graph_sheet_id: SheetId,
    },
    BubblePlotInspector {
        graph_sheet_id: SheetId,
    },
    ApplyStyleFrom {
        target_graph_id: SheetId,
    },
    PageLayoutComposer {
        layout_sheet_id: SheetId,
    },
    ExportFigure {
        sheet_id: SheetId,
    },
    ConvertArchetype {
        sheet_id: SheetId,
    },
    ImportReshape,
    ExcludeWithReason {
        sheet_id: SheetId,
    },
    FormatSelectedPoints {
        graph_sheet_id: SheetId,
    },
    PowerAndSampleSize,
    Print {
        sheet_id: Option<SheetId>,
    },
    RandomizeGroups,
    RandomSampleRows {
        source_sheet_id: Option<SheetId>,
    },
    SimulateDistributions,
    InsertModelEquation {
        graph_sheet_id: SheetId,
    },
    CommandPalette,
    About,
    Feedback,
}

/// Active non-modal inspector docked on the right side of the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ActiveInspector {
    #[default]
    None,
    FormatGraph,
    FormatAxes,
    PairwiseComparisons,
    BubblePlot,
    PageLayout,
    ResultDetails,
}

impl ActiveInspector {
    /// Human-readable header label for the right inspector panel.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Inspector",
            Self::FormatGraph => "Format Graph",
            Self::FormatAxes => "Format Axes",
            Self::PairwiseComparisons => "Pairwise Comparisons",
            Self::BubblePlot => "Bubble Plot Channels",
            Self::PageLayout => "Page Layout",
            Self::ResultDetails => "Analysis & Checklist",
        }
    }
}

// ============================================================================
// Analyze... Catalog, Search, Variable Roles, Duplicate Prompt, Cmd+T, Retarget
// ============================================================================

/// High-level grouping in the searchable `Analyze...` dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnalysisCategory {
    Transform,
    Column,
    Grouped,
    XyAndRegression,
    Survival,
    Contingency,
    Multivariate,
}

impl AnalysisCategory {
    pub const ALL: [Self; 7] =
        [Self::Transform, Self::Column, Self::Grouped, Self::XyAndRegression, Self::Survival, Self::Contingency, Self::Multivariate];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transform => "Transform & Derive",
            Self::Column => "Column Analyses",
            Self::Grouped => "Grouped Analyses",
            Self::XyAndRegression => "XY & Regression",
            Self::Survival => "Survival Analyses",
            Self::Contingency => "Contingency",
            Self::Multivariate => "Multivariate & GLM",
        }
    }
}

/// Static descriptor for an entry shown inside `Analyze...`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisCatalogItem {
    pub id: &'static str,
    pub label: &'static str,
    pub category: AnalysisCategory,
    pub keywords: &'static str,
    pub summary: &'static str,
}

/// Complete scientific analysis catalog filtered dynamically by active table archetype and query.
pub const ANALYSIS_CATALOG: &[AnalysisCatalogItem] = &[
    AnalysisCatalogItem {
        id: "transform.math",
        label: "Transform Math (Log / Exp / Scale)",
        category: AnalysisCategory::Transform,
        keywords: "transform log ln log10 log2 concentration logit probit",
        summary: "Create a linked derived table via scalar mathematical functions.",
    },
    AnalysisCatalogItem {
        id: "transform.normalize",
        label: "Normalize (0% - 100%)",
        category: AnalysisCategory::Transform,
        keywords: "normalize percent baseline anchor hooked min max",
        summary: "Rescale curves or columns between control anchor endpoints.",
    },
    AnalysisCatalogItem {
        id: "transform.baseline",
        label: "Remove Baseline and Column Math",
        category: AnalysisCategory::Transform,
        keywords: "baseline subtract ratio fractional control column",
        summary: "Compute difference, ratio, or percent change vs baseline columns.",
    },
    AnalysisCatalogItem {
        id: "transform.transpose",
        label: "Transpose Rows and Columns",
        category: AnalysisCategory::Transform,
        keywords: "transpose flip rotate rows columns",
        summary: "Create a live linked transposed table.",
    },
    AnalysisCatalogItem {
        id: "transform.prune",
        label: "Prune Rows",
        category: AnalysisCategory::Transform,
        keywords: "prune decimate downsample average window",
        summary: "Subsample or block-average noisy high-frequency recordings.",
    },
    AnalysisCatalogItem {
        id: "transform.select",
        label: "Select and Transform",
        category: AnalysisCategory::Transform,
        keywords: "select subset filter query table",
        summary: "Filter rows and reshape into another analyzable table archetype.",
    },
    AnalysisCatalogItem {
        id: "transform.simulate",
        label: "Simulate Data",
        category: AnalysisCategory::Transform,
        keywords: "simulate monte carlo synthetic equation noise",
        summary: "Generate deterministic or noisy replicate data from a parametric curve.",
    },
    AnalysisCatalogItem {
        id: "transform.random_sample",
        label: "Random Sample Rows",
        category: AnalysisCategory::Transform,
        keywords: "sample random stratified subset rows seed reproducible",
        summary: "Draw a reproducible simple or stratified random sample of rows.",
    },
    AnalysisCatalogItem {
        id: "stats.descriptive",
        label: "Column Statistics (Descriptive & Normality)",
        category: AnalysisCategory::Column,
        keywords: "descriptive mean sd sem median iqr percentile shapiro d'agostino normality",
        summary: "Quantiles, parametric/nonparametric moments, CIs, and normality battery.",
    },
    AnalysisCatalogItem {
        id: "stats.outliers",
        label: "Identify Outliers (ROUT / Grubbs)",
        category: AnalysisCategory::Column,
        keywords: "outlier grubbs rout fdr anomaly clean",
        summary: "Single-value Grubbs or FDR-controlled ROUT outlier detection.",
    },
    AnalysisCatalogItem {
        id: "stats.two_group",
        label: "t tests (Unpaired / Welch / Paired / Nonparametric)",
        category: AnalysisCategory::Column,
        keywords: "t test welch paired mann-whitney wilcoxon kolmogorov lognormal effect size",
        summary: "Compare two groups with effect sizes, CIs, and assumption checks.",
    },
    AnalysisCatalogItem {
        id: "stats.one_way_anova",
        label: "One-way ANOVA and Nonparametric",
        category: AnalysisCategory::Column,
        keywords: "anova brown-forsythe welch repeated measures kruskal friedman tukey dunnett fdr",
        summary: "Compare three or more groups with multiplicity-adjusted comparisons.",
    },
    AnalysisCatalogItem {
        id: "stats.nested",
        label: "Nested t test / Nested One-way ANOVA",
        category: AnalysisCategory::Column,
        keywords: "nested hierarchical mixed reml biological technical replicates",
        summary: "Account for biological units nested inside experimental groups.",
    },
    AnalysisCatalogItem {
        id: "stats.roc",
        label: "ROC Curve (DeLong CI)",
        category: AnalysisCategory::Column,
        keywords: "roc auc sensitivity specificity youden delong diagnostic",
        summary: "Receiver operating characteristic curve, AUC CI, and Youden threshold.",
    },
    AnalysisCatalogItem {
        id: "stats.factorial_anova",
        label: "Two-way / Three-way / Multifactor ANOVA",
        category: AnalysisCategory::Grouped,
        keywords: "two-way three-way factorial anova mixed reml interaction",
        summary: "Multi-factor variance decomposition with interaction and RM tests.",
    },
    AnalysisCatalogItem {
        id: "fit.linear",
        label: "Simple Linear / Deming Regression",
        category: AnalysisCategory::XyAndRegression,
        keywords: "linear regression slope intercept r2 deming interpolate unknowns",
        summary: "Ordinary or Model II Deming regression with confidence/prediction bands.",
    },
    AnalysisCatalogItem {
        id: "fit.nonlinear",
        label: "Nonlinear Regression (Curve Fitting)",
        category: AnalysisCategory::XyAndRegression,
        keywords: "nonlinear curve fit ec50 ic50 4pl michaelis menten hill exponential aicc",
        summary: "Built-in scientific equation library with profile-likelihood CIs and AICc.",
    },
    AnalysisCatalogItem {
        id: "stats.survival_km",
        label: "Kaplan-Meier Survival Analysis",
        category: AnalysisCategory::Survival,
        keywords: "survival kaplan meier log-rank mantel-cox gehan breslow greenwood median",
        summary: "Product-limit survival curves, log-rank tests, and number-at-risk table.",
    },
    AnalysisCatalogItem {
        id: "stats.survival_cox",
        label: "Cox Proportional Hazards Regression",
        category: AnalysisCategory::Survival,
        keywords: "cox proportional hazards hazard ratio survival efron breslow exact",
        summary: "Multivariable time-to-event hazard ratios from survival or MV tables.",
    },
    AnalysisCatalogItem {
        id: "stats.contingency",
        label: "Contingency Table (Chi-square / Fisher's Exact)",
        category: AnalysisCategory::Contingency,
        keywords: "contingency chi-square fisher exact odds ratio relative risk mcnemar trend",
        summary: "Exact and asymptotic contingency tests plus diagnostic accuracy metrics.",
    },
    AnalysisCatalogItem {
        id: "stats.glm",
        label: "Multiple Regression (Linear / Logistic / Poisson)",
        category: AnalysisCategory::Multivariate,
        keywords: "multiple linear logistic poisson glm vif odds ratio variance inflation",
        summary: "Multivariable modeling over Multiple Variables continuous and categorical predictors.",
    },
    AnalysisCatalogItem {
        id: "stats.pca",
        label: "Principal Component Analysis (PCA)",
        category: AnalysisCategory::Multivariate,
        keywords: "pca principal components scree loadings biplot parallel analysis",
        summary: "Standardized PCA with scores, loadings, and Horn parallel-analysis thresholds.",
    },
    AnalysisCatalogItem {
        id: "stats.pcr",
        label: "Principal Component Regression (PCR)",
        category: AnalysisCategory::Multivariate,
        keywords: "pcr principal component regression pca predictors outcome response",
        summary: "Principal component regression with strict predictor/outcome separation.",
    },
    AnalysisCatalogItem {
        id: "stats.corr_matrix",
        label: "Correlation Matrix (Pearson / Spearman)",
        category: AnalysisCategory::Multivariate,
        keywords: "correlation matrix pearson spearman heatmap",
        summary: "All-pairs correlation coefficients with adjusted p-values.",
    },
    AnalysisCatalogItem {
        id: "stats.clustering",
        label: "Cluster Analysis (K-means / Hierarchical)",
        category: AnalysisCategory::Multivariate,
        keywords: "cluster kmeans hierarchical dendrogram",
        summary: "Unsupervised observation clustering on standardized numeric variables.",
    },
    AnalysisCatalogItem {
        id: "stats.multiple_row_t_tests",
        label: "Multiple Row t Tests / Nonparametric",
        category: AnalysisCategory::Grouped,
        keywords: "multiple row t test mann whitney wilcoxon fdr holm sidak row-wise",
        summary: "Row-wise two-group parametric or rank comparisons with multiplicity control.",
    },
    AnalysisCatalogItem {
        id: "stats.bland_altman",
        label: "Bland-Altman Method Comparison",
        category: AnalysisCategory::Column,
        keywords: "bland altman bias limits of agreement method comparison agreement",
        summary: "Paired bias and 95% limits of agreement across two measurement methods.",
    },
    AnalysisCatalogItem {
        id: "stats.passing_bablok",
        label: "Passing-Bablok Regression",
        category: AnalysisCategory::XyAndRegression,
        keywords: "passing bablok nonparametric method comparison slope intercept cusum",
        summary: "Rank-based Passing-Bablok regression with exact/bootstrap CIs and CUSUM check.",
    },
    AnalysisCatalogItem {
        id: "stats.multiple_imputation",
        label: "Multiple Imputation (PMM)",
        category: AnalysisCategory::Multivariate,
        keywords: "multiple imputation pmm mice missing data rubin rules",
        summary: "Chained predictive mean matching imputations pooled via Rubin's rules.",
    },
    AnalysisCatalogItem {
        id: "stats.pvalue_stack",
        label: "Analyze a Stack of P Values",
        category: AnalysisCategory::Column,
        keywords: "p value p-value stack multiplicity fdr bonferroni holm sidak benjamini hochberg bky yekutieli q value discovery",
        summary: "Adjust a column stack of raw P values across 7 FWER/FDR procedures and flag discoveries.",
    },
];

/// Map a static catalog identifier into its default [`AnalysisKind`].
#[must_use]
pub fn default_analysis_kind_for_id(id: &str) -> Option<AnalysisKind> {
    match id {
        "transform.math" => Some(AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::Log10),
            error_propagation: ErrorPropagationMode::ReplicateEndpoints,
        })),
        "transform.normalize" => Some(AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        })),
        "transform.baseline" => Some(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: vec![0],
            operation: BaselineOperation::Difference,
        })),
        "transform.transpose" => Some(AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true })),
        "transform.prune" => Some(AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: false })),
        "transform.select" => Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
            filter_formula: None,
            target_archetype: TableKind::Column,
            derived_columns: Vec::new(),
        })),
        "transform.simulate" => Some(AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed: 42,
            row_count: 24,
            replicates: 3,
            x_min: -9.0,
            x_max: -3.0,
            equation_id: String::from("log_agonist_vs_response_4pl"),
            parameters: vec![
                (String::from("Bottom"), 0.0),
                (String::from("Top"), 100.0),
                (String::from("LogEC50"), -6.0),
                (String::from("HillSlope"), 1.0),
            ],
            noise: curvebook_doc::SimulationNoise::Gaussian { sd: 4.0 },
        })),
        "transform.random_sample" => Some(AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 1,
            sample_size: 10,
            stratify_column: None,
            proportional_allocation: true,
        })),
        "stats.descriptive" => Some(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())),
        "stats.outliers" => Some(AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })),
        "stats.two_group" => Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())),
        "stats.one_way_anova" => Some(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())),
        "stats.nested" => Some(AnalysisKind::NestedComparison(NestedAnalysisSpec::default())),
        "stats.roc" => Some(AnalysisKind::RocCurve(RocSpec::default())),
        "stats.factorial_anova" => Some(AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())),
        "fit.linear" => Some(AnalysisKind::LinearRegression(LinearRegressionSpec::default())),
        "fit.nonlinear" => Some(AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))),
        "stats.survival_km" => Some(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        })),
        "stats.survival_cox" => Some(AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        })),
        "stats.contingency" => Some(AnalysisKind::Contingency(ContingencySpec::default())),
        "stats.glm" => Some(AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            compute_vif: true,
        })),
        "stats.pca" | "stats.pcr" => Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(7) })),
        "stats.corr_matrix" => Some(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false })),
        "stats.clustering" => Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(3),
            distance: curvebook_doc::ClusterDistance::default(),
            linkage: curvebook_doc::ClusterLinkage::default(),
            k_sweep_max: None,
        })),
        "stats.multiple_row_t_tests" => Some(AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())),
        "stats.bland_altman" => Some(AnalysisKind::BlandAltman(BlandAltmanSpec::default())),
        "stats.passing_bablok" => Some(AnalysisKind::PassingBablok(PassingBablokSpec::default())),
        "stats.multiple_imputation" => Some(AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 10,
            seed: 42,
            response_column: None,
            predictor_columns: Vec::new(),
        })),
        "stats.pvalue_stack" => Some(AnalysisKind::PValueStack(PValueStackSpec::default())),
        "stats.power" => Some(AnalysisKind::PowerAnalysis(curvebook_doc::PowerAnalysisSpec::default())),
        _ => None,
    }
}

/// Filter rule applied to `MultipleVariables` column pickers in analysis and bubble dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VariableRoleFilter {
    #[default]
    All,
    ContinuousOnly,
    CategoricalOnly,
    LabelOnly,
    CalculatedOnly,
    NumericForModeling,
}

impl VariableRoleFilter {
    #[must_use]
    pub const fn matches(self, role: VariableRole) -> bool {
        match self {
            Self::All => true,
            Self::ContinuousOnly => matches!(role, VariableRole::Continuous),
            Self::CategoricalOnly => matches!(role, VariableRole::Categorical),
            Self::LabelOnly => matches!(role, VariableRole::Label),
            Self::CalculatedOnly => matches!(role, VariableRole::Calculated),
            Self::NumericForModeling => matches!(role, VariableRole::Continuous | VariableRole::Calculated),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All Variables",
            Self::ContinuousOnly => "Continuous",
            Self::CategoricalOnly => "Categorical",
            Self::LabelOnly => "Label",
            Self::CalculatedOnly => "Calculated",
            Self::NumericForModeling => "Numeric (Continuous + Calculated)",
        }
    }

    /// Parse a canonical role-filter slug (`all`, `continuous`, `categorical`, `label`, `calculated`, `numeric`).
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "continuous" => Some(Self::ContinuousOnly),
            "categorical" => Some(Self::CategoricalOnly),
            "label" => Some(Self::LabelOnly),
            "calculated" => Some(Self::CalculatedOnly),
            "numeric" | "numeric_for_modeling" => Some(Self::NumericForModeling),
            _ => None,
        }
    }
}

/// Contrasting badge foreground ink (`BLACK` or `WHITE`) for a [`variable_role_color`] fill.
#[must_use]
pub const fn variable_role_badge_fg(bg: Rgba8) -> Rgba8 {
    let luma = (bg.r as u32) * 299 + (bg.g as u32) * 587 + (bg.b as u32) * 114;
    if luma >= 145_000 { Rgba8::BLACK } else { Rgba8::WHITE }
}

/// Short badge text for a typed variable role (`MultipleVariablesTable`).
#[must_use]
pub const fn variable_role_badge(role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => "NUM",
        VariableRole::Categorical => "CAT",
        VariableRole::Label => "LBL",
        VariableRole::Calculated => "FX=",
    }
}

/// Theme-aware badge fill colour for a typed variable role.
#[must_use]
pub const fn variable_role_color(role: VariableRole, theme: &UiThemeSpec) -> Rgba8 {
    match role {
        VariableRole::Continuous => theme.link_blue,
        VariableRole::Categorical => Rgba8::rgb(0xD5, 0x5E, 0x00),
        VariableRole::Label => Rgba8::rgb(0x88, 0x44, 0x99),
        VariableRole::Calculated => theme.link_green,
    }
}

/// Metadata row representing one variable/column selectable in role-filtered pickers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableCandidate {
    pub col_index: usize,
    pub title: String,
    pub role: VariableRole,
    pub role_inferred: bool,
    pub selected: bool,
    pub is_response: bool,
}

/// Resolution chosen by the user when an analysis of the same family already exists on a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalyzeDuplicateChoice {
    ModifyExisting(SheetId),
    CreateNew,
}

/// Interactive state for the searchable `Analyze...` dialog (`docs/evidence/user-interface.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeDialogState {
    pub source_sheet_id: Option<SheetId>,
    pub source_table_kind: Option<TableKind>,
    #[serde(default)]
    pub extracted_result_tab_id: Option<String>,
    pub search_query: String,
    pub category_filter: Option<AnalysisCategory>,
    pub selected_entry_id: Option<String>,
    pub selected_data_sets: Vec<usize>,
    pub variable_candidates: Vec<VariableCandidate>,
    pub role_filter: VariableRoleFilter,
    pub equation_id: String,
    #[serde(default)]
    pub pca_predictors: Vec<usize>,
    #[serde(default)]
    pub pcr_outcome_column: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub correlation_all_pairs: bool,
    #[serde(default)]
    pub correlation_heatmap: bool,
    #[serde(default)]
    pub transform_x: Option<MathTransformPreset>,
    #[serde(default = "default_transform_y")]
    pub transform_y: Option<MathTransformPreset>,
    #[serde(default)]
    pub transform_error_propagation: ErrorPropagationMode,
    #[serde(default)]
    pub transform_formula: Option<String>,
    #[serde(default = "default_transform_target_archetype")]
    pub transform_target_archetype: TableKind,
    #[serde(default = "default_baseline_columns")]
    pub baseline_columns: Vec<usize>,
    #[serde(default = "default_baseline_operation")]
    pub baseline_operation: BaselineOperation,
    #[serde(default)]
    pub two_group_method: Option<TwoGroupMethod>,
    #[serde(default)]
    pub two_group_hypothetical: f64,
    #[serde(default)]
    pub mv_group_column: Option<usize>,
    #[serde(default)]
    pub mv_response_column: Option<usize>,
    #[serde(default)]
    pub contingency_method: Option<ContingencyMethod>,
    #[serde(default)]
    pub contingency_null_proportion: Option<f64>,
    #[serde(default)]
    pub contingency_expected_proportions: Option<Vec<f64>>,
    #[serde(default)]
    pub factorial_three_way: bool,
    #[serde(default)]
    pub factorial_repeated_measures: RepeatedMeasuresDesign,
    #[serde(default)]
    pub factorial_residual_covariance: ResidualCovariance,
    #[serde(default)]
    pub factorial_ss_type: SumOfSquaresType,
    #[serde(default)]
    pub factorial_multiplicity: MultiplicityMethod,
    #[serde(default)]
    pub cluster_distance: ClusterDistance,
    #[serde(default)]
    pub cluster_linkage: ClusterLinkage,
    #[serde(default)]
    pub cluster_k_means_k: Option<usize>,
    #[serde(default)]
    pub cluster_k_sweep_max: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub pca_standardize: bool,
    #[serde(default)]
    pub corr_spearman_rank: bool,
    #[serde(default = "default_imputation_m")]
    pub imputation_m: usize,
    #[serde(default)]
    pub glm_family: GlmFamily,
    #[serde(default)]
    pub glm_reference_levels: Vec<(usize, String)>,
    #[serde(default = "default_true_flag")]
    pub glm_compute_vif: bool,
    #[serde(default)]
    pub cox_time_column: Option<usize>,
    #[serde(default)]
    pub cox_status_column: Option<usize>,
    #[serde(default)]
    pub cox_predictor_columns: Vec<usize>,
    #[serde(default)]
    pub cox_tie_method: CoxTieMethod,
    #[serde(default)]
    pub cox_reference_levels: Vec<(usize, String)>,
    #[serde(default)]
    pub km_mv_time_column: Option<usize>,
    #[serde(default)]
    pub km_mv_status_column: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub km_log_rank_mantel_cox: bool,
    #[serde(default)]
    pub km_gehan_breslow_wilcoxon: bool,
    #[serde(default)]
    pub km_trend_test: bool,
    #[serde(default)]
    pub km_multiplicity_method: MultiplicityMethod,
    #[serde(default)]
    pub custom_dsl: Option<String>,
    #[serde(default)]
    pub fit_method: FitMethod,
    #[serde(default)]
    pub fit_weighting: WeightingScheme,
    #[serde(default)]
    pub parameter_rules: Vec<ParameterRule>,
    #[serde(default)]
    pub mv_x_columns: Vec<usize>,
    #[serde(default)]
    pub mv_y_column: Option<usize>,
    #[serde(default)]
    pub mv_group_columns: Vec<usize>,
    #[serde(default = "default_sample_seed")]
    pub sample_seed: u64,
    #[serde(default = "default_sample_size")]
    pub sample_size: usize,
    #[serde(default)]
    pub sample_stratify_column: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub sample_proportional: bool,
    #[serde(default)]
    pub descriptive_aggregation: SubcolumnAggregation,
    #[serde(default)]
    pub pvalue_stack_method: PValueStackMethod,
    #[serde(default = "default_pvalue_stack_threshold")]
    pub pvalue_stack_threshold: f64,
    #[serde(default)]
    pub pvalue_stack_column: Option<usize>,
    #[serde(default)]
    pub compute_effect_sizes: Option<bool>,
    #[serde(default = "default_outlier_method")]
    pub outlier_method: String,
    #[serde(default = "default_outlier_alpha")]
    pub outlier_alpha: f64,
    #[serde(default = "default_outlier_fdr_q")]
    pub outlier_fdr_q_percent: f64,
    #[serde(default)]
    pub outlier_iterative: bool,
    #[serde(default = "default_tg_param_unpaired")]
    pub saved_two_group_param_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_param_paired")]
    pub saved_two_group_param_paired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_unpaired")]
    pub saved_two_group_nonparam_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_paired")]
    pub saved_two_group_nonparam_paired: TwoGroupMethod,
    #[serde(skip)]
    pub explicit_response_column: Option<usize>,
}

fn default_outlier_method() -> String {
    String::from("rout")
}

const fn default_outlier_alpha() -> f64 {
    0.05
}

const fn default_outlier_fdr_q() -> f64 {
    1.0
}

const fn default_tg_param_unpaired() -> TwoGroupMethod {
    TwoGroupMethod::UnpairedT
}

const fn default_tg_param_paired() -> TwoGroupMethod {
    TwoGroupMethod::PairedT
}

const fn default_tg_nonparam_unpaired() -> TwoGroupMethod {
    TwoGroupMethod::MannWhitneyU
}

const fn default_tg_nonparam_paired() -> TwoGroupMethod {
    TwoGroupMethod::WilcoxonSignedRank
}

const fn default_one_way_param_unpaired() -> OneWayMethod {
    OneWayMethod::OrdinaryAnova
}

const fn default_pvalue_stack_threshold() -> f64 {
    0.05
}

const fn default_transform_y() -> Option<MathTransformPreset> {
    Some(MathTransformPreset::Log10)
}

const fn default_transform_target_archetype() -> TableKind {
    TableKind::Column
}

fn default_baseline_columns() -> Vec<usize> {
    vec![0]
}

const fn default_baseline_operation() -> BaselineOperation {
    BaselineOperation::Difference
}

const fn default_imputation_m() -> usize {
    5
}

const fn default_sample_seed() -> u64 {
    1
}

const fn default_sample_size() -> usize {
    10
}

impl Default for AnalyzeDialogState {
    fn default() -> Self {
        Self {
            source_sheet_id: None,
            source_table_kind: None,
            extracted_result_tab_id: None,
            search_query: String::new(),
            category_filter: None,
            selected_entry_id: None,
            selected_data_sets: Vec::new(),
            variable_candidates: Vec::new(),
            role_filter: VariableRoleFilter::All,
            equation_id: String::new(),
            pca_predictors: Vec::new(),
            pcr_outcome_column: None,
            correlation_all_pairs: true,
            correlation_heatmap: false,
            transform_x: None,
            transform_y: Some(MathTransformPreset::Log10),
            transform_error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            transform_formula: None,
            transform_target_archetype: TableKind::Column,
            baseline_columns: vec![0],
            baseline_operation: BaselineOperation::Difference,
            two_group_method: None,
            two_group_hypothetical: 0.0,
            mv_group_column: None,
            mv_response_column: None,
            contingency_method: None,
            contingency_null_proportion: None,
            contingency_expected_proportions: None,
            factorial_three_way: false,
            factorial_repeated_measures: RepeatedMeasuresDesign::None,
            factorial_residual_covariance: ResidualCovariance::CompoundSymmetry,
            factorial_ss_type: SumOfSquaresType::TypeIII,
            factorial_multiplicity: MultiplicityMethod::None,
            cluster_distance: ClusterDistance::Euclidean,
            cluster_linkage: ClusterLinkage::Ward,
            cluster_k_means_k: Some(3),
            cluster_k_sweep_max: None,
            pca_standardize: true,
            corr_spearman_rank: false,
            imputation_m: 5,
            glm_family: GlmFamily::Linear,
            glm_reference_levels: Vec::new(),
            glm_compute_vif: true,
            cox_time_column: None,
            cox_status_column: None,
            cox_predictor_columns: Vec::new(),
            cox_tie_method: CoxTieMethod::Efron,
            cox_reference_levels: Vec::new(),
            km_mv_time_column: None,
            km_mv_status_column: None,
            km_log_rank_mantel_cox: true,
            km_gehan_breslow_wilcoxon: false,
            km_trend_test: false,
            km_multiplicity_method: MultiplicityMethod::None,
            custom_dsl: None,
            fit_method: FitMethod::LeastSquares,
            fit_weighting: WeightingScheme::None,
            parameter_rules: Vec::new(),
            mv_x_columns: Vec::new(),
            mv_y_column: None,
            mv_group_columns: Vec::new(),
            sample_seed: 1,
            sample_size: 10,
            sample_stratify_column: None,
            sample_proportional: true,
            descriptive_aggregation: SubcolumnAggregation::RowMeans,
            pvalue_stack_method: PValueStackMethod::default(),
            pvalue_stack_threshold: default_pvalue_stack_threshold(),
            pvalue_stack_column: None,
            compute_effect_sizes: None,
            outlier_method: default_outlier_method(),
            outlier_alpha: default_outlier_alpha(),
            outlier_fdr_q_percent: default_outlier_fdr_q(),
            outlier_iterative: false,
            saved_two_group_param_unpaired: default_tg_param_unpaired(),
            saved_two_group_param_paired: default_tg_param_paired(),
            saved_two_group_nonparam_unpaired: default_tg_nonparam_unpaired(),
            saved_two_group_nonparam_paired: default_tg_nonparam_paired(),
            explicit_response_column: None,
        }
    }
}

impl AnalyzeDialogState {
    fn reset_r9_options(&mut self) {
        self.two_group_method = None;
        self.two_group_hypothetical = 0.0;
        self.mv_group_column = None;
        self.mv_response_column = None;
        self.contingency_method = None;
        self.contingency_null_proportion = None;
        self.contingency_expected_proportions = None;
        self.factorial_three_way = false;
        self.factorial_repeated_measures = RepeatedMeasuresDesign::None;
        self.factorial_residual_covariance = ResidualCovariance::CompoundSymmetry;
        self.factorial_ss_type = SumOfSquaresType::TypeIII;
        self.factorial_multiplicity = MultiplicityMethod::None;
        self.cluster_distance = ClusterDistance::Euclidean;
        self.cluster_linkage = ClusterLinkage::Ward;
        self.cluster_k_means_k = Some(3);
        self.cluster_k_sweep_max = None;
        self.pca_standardize = true;
        self.corr_spearman_rank = false;
        self.imputation_m = 5;
        self.glm_family = GlmFamily::Linear;
        self.glm_reference_levels.clear();
        self.glm_compute_vif = true;
        self.cox_time_column = None;
        self.cox_status_column = None;
        self.cox_predictor_columns.clear();
        self.cox_tie_method = CoxTieMethod::Efron;
        self.cox_reference_levels.clear();
        self.km_mv_time_column = None;
        self.km_mv_status_column = None;
        self.km_log_rank_mantel_cox = true;
        self.km_gehan_breslow_wilcoxon = false;
        self.km_trend_test = false;
        self.km_multiplicity_method = MultiplicityMethod::None;
        self.transform_target_archetype = TableKind::Column;
        self.baseline_columns = vec![0];
        self.baseline_operation = BaselineOperation::Difference;
        self.custom_dsl = None;
        self.fit_method = FitMethod::LeastSquares;
        self.fit_weighting = WeightingScheme::None;
        self.parameter_rules.clear();
        self.mv_x_columns.clear();
        self.mv_y_column = None;
        self.mv_group_columns.clear();
        self.sample_seed = 1;
        self.sample_size = 10;
        self.sample_stratify_column = None;
        self.sample_proportional = true;
        self.descriptive_aggregation = SubcolumnAggregation::RowMeans;
        self.pvalue_stack_method = PValueStackMethod::default();
        self.pvalue_stack_threshold = default_pvalue_stack_threshold();
        self.outlier_method = default_outlier_method();
        self.outlier_alpha = default_outlier_alpha();
        self.outlier_fdr_q_percent = default_outlier_fdr_q();
        self.outlier_iterative = false;
        self.saved_two_group_param_unpaired = default_tg_param_unpaired();
        self.saved_two_group_param_paired = default_tg_param_paired();
        self.saved_two_group_nonparam_unpaired = default_tg_nonparam_unpaired();
        self.saved_two_group_nonparam_paired = default_tg_nonparam_paired();
        self.pvalue_stack_column = None;
        self.compute_effect_sizes = None;
        self.explicit_response_column = None;
    }

    /// Populate defaults and variable candidates for `sheet`.
    pub fn open_for_data_sheet(&mut self, sheet: &DataSheet) {
        self.source_sheet_id = Some(sheet.id.clone());
        self.source_table_kind = Some(sheet.kind());
        self.extracted_result_tab_id = None;
        self.search_query.clear();
        self.category_filter = None;
        self.selected_data_sets.clear();
        self.role_filter = VariableRoleFilter::All;
        self.pca_predictors.clear();
        self.pcr_outcome_column = None;
        self.correlation_all_pairs = true;
        self.correlation_heatmap = false;
        self.transform_x = None;
        self.transform_y = Some(MathTransformPreset::Log10);
        self.transform_error_propagation = ErrorPropagationMode::ReplicateEndpoints;
        self.transform_formula = None;
        self.reset_r9_options();
        if self.equation_id.is_empty() {
            self.equation_id = String::from("log_agonist_vs_response_4pl");
        }
        self.variable_candidates = collect_variable_candidates(sheet);
        let first_id = self.compatible_entries().first().map(|item| item.id.to_owned());
        self.selected_entry_id = first_id;
    }

    /// Populate defaults and column candidates from a non-analyzable [`ResultTab`] for chained GUI `Analyze` (`docs/gui-workflows.md` §3.3).
    pub fn open_for_result_tab(&mut self, result: &ResultSheet, tab: &ResultTab) {
        self.source_sheet_id = Some(result.id.clone());
        self.extracted_result_tab_id = Some(tab.id.clone());
        let has_text = tab.rows.iter().flatten().any(|cell| matches!(cell, ResultCell::Text(_) | ResultCell::Boolean(_)));
        self.source_table_kind = Some(if has_text { TableKind::MultipleVariables } else { TableKind::Column });
        self.search_query.clear();
        self.category_filter = None;
        self.selected_data_sets.clear();
        self.role_filter = VariableRoleFilter::All;
        self.pca_predictors = if matches!(result.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
            result.analysis.selected_data_sets.clone()
        } else {
            Vec::new()
        };
        self.pcr_outcome_column = None;
        self.correlation_all_pairs = true;
        self.correlation_heatmap = false;
        self.transform_x = None;
        self.transform_y = Some(MathTransformPreset::Log10);
        self.transform_error_propagation = ErrorPropagationMode::ReplicateEndpoints;
        self.transform_formula = None;
        self.reset_r9_options();
        if self.equation_id.is_empty() {
            self.equation_id = String::from("log_agonist_vs_response_4pl");
        }
        self.variable_candidates = collect_result_tab_candidates(tab);
        let first_id = self.compatible_entries().first().map(|item| item.id.to_owned());
        self.selected_entry_id = first_id;
    }

    /// Switch the selected catalog entry (`analyze.entry.<id>`), resetting PCR selection cleanly when entering `stats.pcr`.
    pub fn select_catalog_entry(&mut self, entry_id: &str) {
        let entering_pcr = entry_id == "stats.pcr" && self.selected_entry_id.as_deref() != Some("stats.pcr");
        self.selected_entry_id = Some(entry_id.to_owned());
        if entering_pcr {
            self.selected_data_sets.clear();
            self.pca_predictors.clear();
            self.pcr_outcome_column = None;
            for cand in &mut self.variable_candidates {
                cand.selected = false;
            }
        }
    }

    /// Toggle a MV predictor column for PCA/PCR while preserving disjoint PCR outcome selection.
    pub fn toggle_pca_predictor(&mut self, col: usize) {
        if let Some(pos) = self.pca_predictors.iter().position(|&c| c == col) {
            self.pca_predictors.remove(pos);
        } else {
            self.pca_predictors.push(col);
            self.pca_predictors.sort_unstable();
        }
        if let Some(pos) = self.selected_data_sets.iter().position(|&c| c == col) {
            self.selected_data_sets.remove(pos);
        } else {
            self.selected_data_sets.push(col);
            self.selected_data_sets.sort_unstable();
            if self.pcr_outcome_column == Some(col) {
                self.pcr_outcome_column = None;
            }
        }
        for cand in &mut self.variable_candidates {
            cand.selected = self.selected_data_sets.contains(&cand.col_index);
        }
    }

    /// Select (or toggle off) the disjoint PCR outcome column with R10 predictor leakage guard.
    pub fn select_pcr_outcome(&mut self, col: usize) -> Result<(), String> {
        if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
            return Err(format!("Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."));
        }
        self.pcr_outcome_column = if self.pcr_outcome_column == Some(col) { None } else { Some(col) };
        if let Some(outcome) = self.pcr_outcome_column {
            self.set_response_column(outcome);
        }
        Ok(())
    }

    /// Apply modal field updates from `ui.answer_dialog` / interactive controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal field updates using regional numeric parsing (`preset`).
    fn remember_two_group_submethod(&mut self, method: TwoGroupMethod) {
        match method {
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT => {
                self.saved_two_group_param_unpaired = method;
            }
            TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT => {
                self.saved_two_group_param_paired = method;
            }
            TwoGroupMethod::MannWhitneyU | TwoGroupMethod::KolmogorovSmirnov => {
                self.saved_two_group_nonparam_unpaired = method;
            }
            TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest => {
                self.saved_two_group_nonparam_paired = method;
            }
            TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
        }
    }

    fn toggle_two_group_design(&mut self, gaussian: bool, paired: bool) {
        if let Some(cur) = self.two_group_method {
            self.remember_two_group_submethod(cur);
            if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = cur {
                self.two_group_method =
                    Some(if gaussian { TwoGroupMethod::OneSampleT { hypothetical } } else { TwoGroupMethod::OneSampleWilcoxon { hypothetical } });
                return;
            }
        }
        self.two_group_method = Some(match (gaussian, paired) {
            (true, false) => self.saved_two_group_param_unpaired,
            (true, true) => self.saved_two_group_param_paired,
            (false, false) => self.saved_two_group_nonparam_unpaired,
            (false, true) => self.saved_two_group_nonparam_paired,
        });
    }

    /// Apply modal field updates using regional numeric parsing (`preset`).
    ///
    /// # Errors
    /// Returns an error if an explicit field fails validation or points to an unknown key.
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        let has_explicit_predictors =
            ["selected_data_sets", "analyze.selected_data_sets", "pca_predictors", "analyze.pca_predictors"].iter().any(|k| fields.contains_key(*k));
        for key in ["entry_id", "selected_entry_id", "analysis", "analyze.entry", "analyze.entries_list"] {
            if let Some(s) = fields.get(key).and_then(Value::as_str) {
                if s == "stats.pcr" && !has_explicit_predictors {
                    self.select_catalog_entry(s);
                } else {
                    self.selected_entry_id = Some(s.to_owned());
                }
            }
        }
        // Apply method selectors before sub-parameters (`alpha`, `fdr_q_percent`, `rout_q`) so JSON
        // object key ordering never resets a caller-provided Q% or alpha (`RV-76`).
        for key in ["outlier_method", "analyze.outlier.method", "fit_method", "analyze.fit.method", "method"] {
            if let Some(val) = fields.get(key) {
                if matches!(key, "outlier_method" | "analyze.outlier.method")
                    || (key == "method" && self.selected_entry_id.as_deref() == Some("stats.outliers"))
                {
                    let raw = val.as_str().ok_or_else(|| String::from("`outlier_method` must be a string"))?;
                    let (m, iter_opt) = parse_outlier_method_str(raw)?;
                    self.outlier_method = m.to_owned();
                    if let Some(it) = iter_opt {
                        self.outlier_iterative = it;
                    }
                } else if matches!(key, "fit_method" | "analyze.fit.method")
                    || (key == "method" && self.selected_entry_id.as_deref() == Some("fit.nonlinear"))
                {
                    self.fit_method = parse_fit_method(val)?;
                }
            }
        }
        for (key, val) in fields {
            match key.as_str() {
                "entry_id"
                | "selected_entry_id"
                | "analysis"
                | "analyze.entry"
                | "analyze.entries_list"
                | "outlier_method"
                | "analyze.outlier.method"
                | "fit_method"
                | "analyze.fit.method" => {}
                "search" | "search_query" | "analyze.search" => {
                    if let Some(s) = val.as_str() {
                        self.search_query = s.to_owned();
                    }
                }
                "equation_id" | "analyze.equation_id" | "analyze.fit.model_id" => {
                    if let Some(s) = val.as_str() {
                        self.equation_id = s.to_owned();
                    }
                }
                "correlation_all_pairs" | "analyze.correlation_all_pairs" => {
                    if let Some(b) = val.as_bool() {
                        self.correlation_all_pairs = b;
                        if !b {
                            self.correlation_heatmap = false;
                        }
                    }
                }
                "correlation_heatmap" | "analyze.correlation_heatmap" | "analyze.correlation.heatmap" => {
                    if let Some(b) = val.as_bool() {
                        if b && !self.correlation_all_pairs {
                            return Err(String::from("Create correlation heatmap is disabled in X vs every Y mode."));
                        }
                        self.correlation_heatmap = b;
                    }
                }
                "selected_data_sets"
                | "analyze.selected_data_sets"
                | "analyze.transform.selected_data_sets"
                | "pca_predictors"
                | "analyze.pca_predictors" => {
                    let cols = parse_usize_list(val, key)?;
                    self.selected_data_sets = cols.clone();
                    if matches!(key.as_str(), "pca_predictors" | "analyze.pca_predictors") {
                        self.pca_predictors = cols;
                    }
                    for cand in &mut self.variable_candidates {
                        cand.selected = self.selected_data_sets.contains(&cand.col_index);
                    }
                }
                "pcr_outcome_column" | "response_column" | "analyze.pcr_outcome_column" => {
                    if let Some(col) = val.as_u64().map(|n| n as usize) {
                        if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
                            return Err(format!(
                                "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                            ));
                        }
                        self.pcr_outcome_column = Some(col);
                        self.set_response_column(col);
                    }
                }
                "formula" | "transform_formula" | "analyze.transform.formula" | "filter_formula" | "analyze.transform.filter_formula" => {
                    if val.is_null() {
                        self.transform_formula = None;
                    } else {
                        let raw = val.as_str().ok_or_else(|| String::from("`formula` must be a string or null"))?.trim();
                        if raw.is_empty() {
                            self.transform_formula = None;
                        } else {
                            self.transform_formula = Some(raw.to_owned());
                            if self.selected_entry_id.as_deref() != Some("transform.select") {
                                let _ = apply_transform_formula_str(raw, &mut self.transform_x, &mut self.transform_y);
                            }
                        }
                    }
                }
                "target_archetype" | "analyze.transform.target_archetype" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`target_archetype` must be a string"))?;
                    self.transform_target_archetype = parse_table_kind_str(raw)?;
                }
                "baseline_columns" | "analyze.transform.baseline_columns" => {
                    self.baseline_columns = parse_usize_list(val, key)?;
                }
                "baseline_operation" | "operation" | "analyze.transform.baseline_operation" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`baseline_operation` must be a string"))?;
                    self.baseline_operation = parse_baseline_operation_str(raw)?;
                }
                "x_transform" | "analyze.transform.x_transform" | "transform_x" => {
                    self.transform_x = parse_math_preset_opt(val)?;
                }
                "y_transform" | "analyze.transform.y_transform" | "transform_y" => {
                    self.transform_y = parse_math_preset_opt(val)?;
                }
                "error_propagation" | "analyze.transform.error_propagation" => {
                    self.transform_error_propagation = parse_error_prop_value(val)?;
                }
                _ => self.apply_r9_analyze_field(key, val, preset)?,
            }
        }
        Ok(())
    }

    fn apply_r9_analyze_field(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        if let Some(col_raw) = key.strip_prefix("analyze.pca.pred.")
            && let Ok(col) = col_raw.parse::<usize>()
        {
            let want_on = val.as_bool().unwrap_or_else(|| !self.selected_data_sets.contains(&col));
            if want_on != self.selected_data_sets.contains(&col) {
                self.toggle_pca_predictor(col);
            }
            return Ok(());
        }
        if let Some(col_raw) = key.strip_prefix("analyze.pcr.outcome.")
            && let Ok(col) = col_raw.parse::<usize>()
        {
            let selected = val.as_bool().unwrap_or(true);
            if selected {
                if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
                    return Err(format!(
                        "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                    ));
                }
                self.pcr_outcome_column = Some(col);
                self.set_response_column(col);
            } else if self.pcr_outcome_column == Some(col) {
                self.pcr_outcome_column = None;
            }
            return Ok(());
        }
        match key {
            "assume_gaussian" | "analyze.assume_gaussian" => {
                let g = val.as_bool().ok_or_else(|| String::from("`assume_gaussian` must be boolean"))?;
                let paired = matches!(
                    self.two_group_method,
                    Some(
                        TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
                    )
                );
                self.toggle_two_group_design(g, paired);
            }
            "paired_or_matched" | "analyze.paired_or_matched" => {
                let paired = val.as_bool().ok_or_else(|| String::from("`paired_or_matched` must be boolean"))?;
                let gaussian = !matches!(
                    self.two_group_method,
                    Some(
                        TwoGroupMethod::MannWhitneyU
                            | TwoGroupMethod::WilcoxonSignedRank
                            | TwoGroupMethod::PairedSignTest
                            | TwoGroupMethod::KolmogorovSmirnov
                            | TwoGroupMethod::OneSampleWilcoxon { .. }
                    )
                );
                self.toggle_two_group_design(gaussian, paired);
            }
            "descriptive_subcolumn_agg" | "subcolumn_agg" | "analyze.descriptive.subcolumn_agg" => {
                self.descriptive_aggregation = parse_subcolumn_aggregation_val(val)?;
            }
            "two_group_method" | "analyze.two_group.method" | "analyze.two_group_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`two_group_method` must be a string"))?;
                let parsed = parse_two_group_method_str(raw, self.two_group_hypothetical)?;
                self.remember_two_group_submethod(parsed);
                self.two_group_method = Some(parsed);
            }
            "compute_effect_sizes" | "analyze.compute_effect_sizes" | "analyze.two_group.compute_effect_sizes" => {
                let b = val.as_bool().ok_or_else(|| String::from("`compute_effect_sizes` must be boolean"))?;
                self.compute_effect_sizes = Some(b);
            }
            "hypothetical" | "two_group_hypothetical" | "analyze.two_group.hypothetical" => {
                self.two_group_hypothetical = parse_finite_f64_with_locale(val, key, preset)?;
            }
            "mv_group_column" | "analyze.mv_group_column" | "analyze.km.mv_group_column" => {
                self.mv_group_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "mv_response_column" | "analyze.mv_response_column" => {
                self.mv_response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "contingency_method" | "analyze.contingency.method" | "analyze.contingency_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`contingency_method` must be a string"))?;
                self.contingency_method = Some(parse_contingency_method_name(raw)?);
            }
            "kappa_weighting" | "analyze.contingency.kappa_weighting" => {
                let w = parse_kappa_weighting(val)?;
                self.contingency_method = Some(ContingencyMethod::CohenKappa { weighting: w });
            }
            "weighting" => {
                if self.selected_entry_id.as_deref() == Some("stats.contingency") {
                    let w = parse_kappa_weighting(val)?;
                    self.contingency_method = Some(ContingencyMethod::CohenKappa { weighting: w });
                } else {
                    self.fit_weighting = parse_weighting_scheme(val)?;
                }
            }
            "fit_weighting" | "analyze.fit.weighting" => {
                self.fit_weighting = parse_weighting_scheme(val)?;
            }
            "null_proportion" | "analyze.contingency.null_proportion" | "analyze.null_proportion" => {
                self.contingency_null_proportion = parse_opt_finite_f64_with_locale(val, key, preset)?;
            }
            "expected_proportions" | "analyze.contingency.expected_proportions" | "analyze.expected_proportions" => {
                self.contingency_expected_proportions = if val.is_null() { None } else { Some(parse_f64_list_with_locale(val, key, preset)?) };
            }
            "three_way" | "analyze.factorial.three_way" => {
                self.factorial_three_way = val.as_bool().ok_or_else(|| String::from("`three_way` must be boolean"))?;
            }
            "repeated_measures" | "analyze.factorial.repeated_measures" => {
                let raw = val.as_str().ok_or_else(|| String::from("`repeated_measures` must be a string"))?;
                self.factorial_repeated_measures = parse_rm_design_str(raw)?;
            }
            "residual_covariance" | "factorial_residual_covariance" | "analyze.factorial.residual_covariance" => {
                let raw = val.as_str().ok_or_else(|| String::from("`residual_covariance` must be a string"))?;
                self.factorial_residual_covariance = parse_residual_covariance_str(raw)?;
            }
            "ss_type" | "sum_of_squares" | "analyze.factorial.ss_type" => {
                let raw = val.as_str().ok_or_else(|| String::from("`ss_type` must be a string"))?;
                self.factorial_ss_type = parse_ss_type_str(raw)?;
            }
            "multiplicity_method" | "analyze.multiplicity_method" | "analyze.factorial.multiplicity" => {
                if self.selected_entry_id.as_deref() == Some("stats.pvalue_stack") {
                    self.pvalue_stack_method = parse_pvalue_stack_method_val(val)?;
                } else {
                    let raw = val.as_str().ok_or_else(|| String::from("`multiplicity_method` must be a string"))?;
                    let parsed = parse_multiplicity_method_name(raw, 0)?;
                    self.factorial_multiplicity = parsed;
                    self.km_multiplicity_method = parsed;
                }
            }
            "distance" | "cluster_distance" | "analyze.cluster.distance" => {
                let raw = val.as_str().ok_or_else(|| String::from("`distance` must be a string"))?;
                self.cluster_distance = parse_cluster_distance_str(raw)?;
            }
            "linkage" | "cluster_linkage" | "analyze.cluster.linkage" => {
                let raw = val.as_str().ok_or_else(|| String::from("`linkage` must be a string"))?;
                self.cluster_linkage = parse_cluster_linkage_str(raw)?;
            }
            "k_means_k" | "k" | "analyze.cluster.k_means_k" => {
                self.cluster_k_means_k = if val.is_null() { None } else { Some(parse_usize_min(val, key, 1)?) };
            }
            "k_sweep_max" | "analyze.cluster.k_sweep_max" => {
                self.cluster_k_sweep_max = if val.is_null() { None } else { Some(parse_usize_min(val, key, 2)?) };
            }
            "standardize" | "pca_standardize" | "analyze.pca.standardize" => {
                self.pca_standardize = val.as_bool().ok_or_else(|| String::from("`standardize` must be boolean"))?;
            }
            "spearman_rank" | "analyze.corr.spearman_rank" | "analyze.correlation.spearman_rank" => {
                self.corr_spearman_rank = val.as_bool().ok_or_else(|| String::from("`spearman_rank` must be boolean"))?;
            }
            "imputations_m" | "m" | "analyze.mi.imputations_m" | "analyze.imputation.m" => {
                self.imputation_m = parse_usize_min(val, key, 1)?;
            }
            "family" | "glm_family" | "analyze.glm.family" => {
                let raw = val.as_str().ok_or_else(|| String::from("`glm_family` must be a string"))?;
                self.glm_family = parse_glm_family_str(raw)?;
            }
            "reference_levels" | "analyze.reference_levels" => {
                let refs = parse_reference_levels(val)?;
                self.glm_reference_levels.clone_from(&refs);
                self.cox_reference_levels = refs;
            }
            "compute_vif" | "analyze.glm.compute_vif" => {
                self.glm_compute_vif = val.as_bool().ok_or_else(|| String::from("`compute_vif` must be boolean"))?;
            }
            "time_column" | "cox_time_column" | "analyze.cox.time_column" => {
                self.cox_time_column = Some(parse_usize_min(val, key, 0)?);
            }
            "status_column" | "cox_status_column" | "analyze.cox.status_column" => {
                self.cox_status_column = Some(parse_usize_min(val, key, 0)?);
            }
            "predictor_columns" | "cox_predictor_columns" | "analyze.cox.predictor_columns" => {
                self.cox_predictor_columns = parse_usize_list(val, key)?;
            }
            "tie_method" | "cox_tie_method" | "analyze.cox.tie_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`tie_method` must be a string"))?;
                self.cox_tie_method = parse_cox_tie_method_str(raw)?;
            }
            "trend_test" | "km_trend_test" | "analyze.km.trend_test" | "log_rank_for_trend" | "analyze.km.log_rank_for_trend" => {
                self.km_trend_test = val.as_bool().ok_or_else(|| String::from("`trend_test` must be boolean"))?;
            }
            "mv_time_column" | "km_mv_time_column" | "analyze.km.mv_time_column" => {
                self.km_mv_time_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "mv_status_column" | "km_mv_status_column" | "analyze.km.mv_status_column" => {
                self.km_mv_status_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "log_rank_mantel_cox" | "km_log_rank_mantel_cox" | "analyze.km.log_rank_mantel_cox" => {
                self.km_log_rank_mantel_cox = val.as_bool().ok_or_else(|| String::from("`log_rank_mantel_cox` must be boolean"))?;
            }
            "gehan_breslow_wilcoxon" | "km_gehan_breslow_wilcoxon" | "analyze.km.gehan_breslow_wilcoxon" => {
                self.km_gehan_breslow_wilcoxon = val.as_bool().ok_or_else(|| String::from("`gehan_breslow_wilcoxon` must be boolean"))?;
            }
            "km_multiplicity_method" | "analyze.km.multiplicity_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`multiplicity_method` must be a string"))?;
                self.km_multiplicity_method = parse_multiplicity_method_name(raw, 0)?;
            }
            "custom_dsl" | "analyze.fit.custom_dsl" => {
                self.custom_dsl = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned);
            }
            "parameter_rules" | "analyze.fit.parameter_rules" => {
                self.parameter_rules = parse_parameter_rules(val)?;
            }
            "mv_x_columns" | "analyze.fit.mv_x_columns" => {
                self.mv_x_columns = parse_usize_list(val, key)?;
            }
            "mv_y_column" | "analyze.fit.mv_y_column" => {
                self.mv_y_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "mv_group_columns" | "analyze.fit.mv_group_columns" => {
                self.mv_group_columns = parse_usize_list(val, key)?;
            }
            "sample_size" | "analyze.sample.sample_size" => {
                self.sample_size = parse_usize_min(val, key, 1)?;
            }
            "seed" | "sample_seed" | "analyze.sample.seed" => {
                self.sample_seed = parse_u64_val(val, key)?;
            }
            "stratify_column" | "analyze.sample.stratify_column" => {
                self.sample_stratify_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "proportional_allocation" | "analyze.sample.proportional_allocation" => {
                self.sample_proportional = val.as_bool().ok_or_else(|| String::from("`proportional_allocation` must be boolean"))?;
            }
            _ => self.apply_pvalue_stack_analyze_field(key, val, preset)?,
        }
        Ok(())
    }

    fn apply_pvalue_stack_analyze_field(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        let is_pstack = self.selected_entry_id.as_deref() == Some("stats.pvalue_stack");
        let is_outliers = self.selected_entry_id.as_deref() == Some("stats.outliers");
        let is_nonlinear = self.selected_entry_id.as_deref() == Some("fit.nonlinear");
        match key {
            "outlier_fdr_q_percent" | "analyze.outlier.fdr_q_percent" | "analyze.fit.rout_q" => {
                let q = parse_outlier_fdr_q_percent(val)?;
                self.outlier_fdr_q_percent = q;
                if let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut self.fit_method {
                    *fdr_q_percent = q;
                }
                return Ok(());
            }
            "outlier_alpha" | "analyze.outlier.alpha" => {
                self.outlier_alpha = parse_outlier_alpha(val)?;
                return Ok(());
            }
            "outlier_iterative" | "analyze.outlier.iterative" => {
                self.outlier_iterative = val.as_bool().ok_or_else(|| String::from("`outlier_iterative` must be boolean"))?;
                return Ok(());
            }
            "fdr_q_percent" | "q_percent" | "rout_q" if is_outliers || is_nonlinear => {
                let q = parse_outlier_fdr_q_percent(val)?;
                if is_outliers {
                    self.outlier_fdr_q_percent = q;
                }
                if is_nonlinear {
                    let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut self.fit_method else {
                        return Err(String::from("`fdr_q_percent` requires ROUT outlier elimination fit method"));
                    };
                    *fdr_q_percent = q;
                }
                return Ok(());
            }
            "alpha" if is_outliers => {
                self.outlier_alpha = parse_outlier_alpha(val)?;
                return Ok(());
            }
            "iterative" if is_outliers => {
                self.outlier_iterative = val.as_bool().ok_or_else(|| String::from("`iterative` must be boolean"))?;
                return Ok(());
            }
            "method" if is_outliers || is_nonlinear => {
                return Ok(());
            }
            "pvalue_stack_method" | "analyze.pvalue_stack.method" | "analyze.pvalue_stack.multiplicity_method" => {
                self.pvalue_stack_method = parse_pvalue_stack_method_val(val)?;
            }
            "pvalue_stack_threshold" | "analyze.pvalue_stack.threshold" => {
                let t = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0 < t && t < 1.0) {
                    return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
                }
                self.pvalue_stack_threshold = t;
            }
            "pvalue_stack_column" | "analyze.pvalue_stack.column" => {
                self.pvalue_stack_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "method" if is_pstack => {
                self.pvalue_stack_method = parse_pvalue_stack_method_val(val)?;
            }
            "threshold" | "alpha" | "q" | "fdr_q" if is_pstack => {
                let t = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0 < t && t < 1.0) {
                    return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
                }
                self.pvalue_stack_threshold = t;
            }
            "column" if is_pstack => {
                self.pvalue_stack_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            other => return Err(format!("unknown Analyze field `{other}`")),
        }
        Ok(())
    }

    /// Filter [`ANALYSIS_CATALOG`] by source table compatibility, category chip, and search query.
    #[must_use]
    pub fn compatible_entries(&self) -> Vec<&'static AnalysisCatalogItem> {
        let query = self.search_query.trim().to_ascii_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        ANALYSIS_CATALOG.iter().filter(|item| self.item_matches(item, &terms)).collect()
    }

    fn item_matches(&self, item: &AnalysisCatalogItem, query_terms: &[&str]) -> bool {
        if let Some(cat) = self.category_filter
            && item.category != cat
        {
            return false;
        }
        if let Some(table_kind) = self.source_table_kind {
            let Some(kind) = default_analysis_kind_for_id(item.id) else {
                return false;
            };
            if !kind.is_compatible_with(table_kind) {
                return false;
            }
        }
        if query_terms.is_empty() {
            return true;
        }
        let haystack = format!("{} {} {} {}", item.id, item.label, item.category.label(), item.keywords).to_ascii_lowercase();
        query_terms.iter().all(|term| haystack.contains(term))
    }

    /// Explicit human-readable diagnostic banner when no catalog items match the active filters.
    #[must_use]
    pub fn empty_feedback(&self) -> Option<String> {
        if !self.compatible_entries().is_empty() {
            return None;
        }
        let table_label = self.source_table_kind.map_or("active sheet", TableKind::label);
        let q = self.search_query.trim();
        if q.is_empty() {
            Some(format!("No analyses in the selected category are compatible with {table_label} tables."))
        } else {
            Some(format!("No analyses compatible with {table_label} match \"{q}\"."))
        }
    }

    /// Role-filtered variables for `MultipleVariablesTable` column pickers.
    #[must_use]
    pub fn filtered_variables(&self) -> Vec<&VariableCandidate> {
        self.variable_candidates.iter().filter(|v| self.role_filter.matches(v.role)).collect()
    }

    /// Toggle inclusion of column `col_index` in `variable_candidates` and `selected_data_sets`.
    pub fn toggle_variable(&mut self, col_index: usize) {
        if let Some(var) = self.variable_candidates.iter_mut().find(|v| v.col_index == col_index) {
            var.selected = !var.selected;
        }
        self.selected_data_sets = self.variable_candidates.iter().filter(|v| v.selected).map(|v| v.col_index).collect();
    }

    /// Assign a primary response column (for GLM / Cox / Factorial modeling on MV tables).
    pub fn set_response_column(&mut self, col_index: usize) {
        self.explicit_response_column = Some(col_index);
        for var in &mut self.variable_candidates {
            var.is_response = var.col_index == col_index;
        }
    }

    /// Synthesize the configured [`AnalysisKind`] from the currently selected catalog entry and
    /// variable/equation choices.
    #[must_use]
    pub fn build_analysis_kind(&self) -> Option<AnalysisKind> {
        let id = self.selected_entry_id.as_deref()?;
        let mut kind = default_analysis_kind_for_id(id)?;
        let response_col = self.variable_candidates.iter().find(|v| v.is_response).map(|v| v.col_index);
        let predictor_cols: Vec<usize> = self
            .variable_candidates
            .iter()
            .filter(|v| v.selected && !v.is_response && !matches!(v.role, VariableRole::Label))
            .map(|v| v.col_index)
            .collect();

        match &mut kind {
            AnalysisKind::OutlierDetection(out) => {
                *out = if self.outlier_method == "grubbs" {
                    OutlierSpec::Grubbs { alpha: self.outlier_alpha, iterative: self.outlier_iterative }
                } else {
                    OutlierSpec::Rout { fdr_q_percent: self.outlier_fdr_q_percent }
                };
            }
            AnalysisKind::DescriptiveStats(desc) => {
                desc.aggregation = self.descriptive_aggregation;
            }
            AnalysisKind::TwoGroupComparison(tg) => {
                if let Some(m) = self.two_group_method {
                    tg.method = m;
                }
                if let Some(ces) = self.compute_effect_sizes {
                    tg.compute_effect_sizes = ces;
                }
                if self.mv_group_column.is_some() {
                    tg.mv_group_column = self.mv_group_column;
                } else if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    tg.mv_group_column = self.variable_candidates.iter().find(|v| v.role == VariableRole::Categorical).map(|v| v.col_index);
                }
                if self.mv_response_column.is_some() {
                    tg.mv_response_column = self.mv_response_column;
                } else if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    tg.mv_response_column = response_col.or_else(|| {
                        self.variable_candidates
                            .iter()
                            .rev()
                            .find(|v| !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                            .map(|v| v.col_index)
                    });
                }
            }
            AnalysisKind::OneWayAnova(anova) => {
                if let Some(ces) = self.compute_effect_sizes {
                    anova.compute_effect_sizes = ces;
                }
            }
            AnalysisKind::Contingency(cont) => {
                if let Some(m) = self.contingency_method {
                    cont.method = m;
                }
                if self.contingency_null_proportion.is_some() {
                    cont.null_proportion = self.contingency_null_proportion;
                }
                if self.contingency_expected_proportions.is_some() {
                    cont.expected_proportions.clone_from(&self.contingency_expected_proportions);
                }
            }
            AnalysisKind::NonlinearRegression(fit) => self.populate_nonlinear_kind(fit),
            AnalysisKind::GeneralizedLinearModel(glm) => self.populate_glm_kind(glm, response_col, &predictor_cols),
            AnalysisKind::Survival(surv) => self.populate_survival_kind(surv, response_col, &predictor_cols),
            AnalysisKind::FactorialAnova(fac) => {
                fac.three_way = self.factorial_three_way;
                if fac.three_way {
                    fac.max_interaction_order = fac.max_interaction_order.max(3);
                }
                fac.repeated_measures = self.factorial_repeated_measures;
                fac.residual_covariance = self.factorial_residual_covariance;
                fac.ss_type = self.factorial_ss_type;
                fac.multiplicity.method = self.factorial_multiplicity;
                if let Some(ces) = self.compute_effect_sizes {
                    fac.compute_effect_sizes = ces;
                }
                if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    let cat_cols: Vec<usize> =
                        self.variable_candidates.iter().filter(|v| v.role == VariableRole::Categorical).map(|v| v.col_index).collect();
                    let factor_limit = if fac.three_way { 3 } else { 2 };
                    if !cat_cols.is_empty() {
                        fac.multifactor_columns = cat_cols.into_iter().take(factor_limit).collect();
                    } else if !predictor_cols.is_empty() {
                        fac.multifactor_columns = predictor_cols;
                    }
                    fac.response_column = response_col;
                } else if !predictor_cols.is_empty() {
                    fac.multifactor_columns = predictor_cols;
                    fac.response_column = response_col;
                }
            }
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max }) => {
                *k_means_k = self.cluster_k_means_k;
                *distance = self.cluster_distance;
                *linkage = self.cluster_linkage;
                *k_sweep_max = self.cluster_k_sweep_max;
            }
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize, .. }) => {
                *standardize = self.pca_standardize;
            }
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank }) => {
                *spearman_rank = self.corr_spearman_rank;
            }
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, response_column, predictor_columns, .. }) => {
                *imputations_m = self.imputation_m.max(1);
                *response_column = response_col;
                if !predictor_cols.is_empty() {
                    *predictor_columns = predictor_cols;
                }
            }
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation }) => {
                *x_transform = self.transform_x;
                *y_transform = self.transform_y;
                *error_propagation = self.transform_error_propagation;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation }) => {
                *baseline_columns = self.baseline_columns.clone();
                *operation = self.baseline_operation;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, .. }) => {
                *filter_formula = self.transform_formula.clone();
                *target_archetype = self.transform_target_archetype;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation }) => {
                *seed = self.sample_seed;
                *sample_size = self.sample_size.max(1);
                *stratify_column = self.sample_stratify_column;
                *proportional_allocation = self.sample_proportional;
            }
            AnalysisKind::PValueStack(ps) => {
                ps.method = self.pvalue_stack_method;
                ps.threshold = self.pvalue_stack_threshold;
                ps.column = self.pvalue_stack_column;
            }
            _ => {}
        }
        Some(kind)
    }

    fn populate_nonlinear_kind(&self, fit: &mut NonlinearFitSpec) {
        if !self.equation_id.trim().is_empty() {
            fit.equation_id = self.equation_id.trim().to_owned();
        }
        if self.custom_dsl.is_some() {
            fit.custom_dsl.clone_from(&self.custom_dsl);
        }
        fit.method = self.fit_method;
        fit.weighting = self.fit_weighting;
        if !self.parameter_rules.is_empty() {
            fit.parameter_rules.clone_from(&self.parameter_rules);
        }
        if !self.mv_x_columns.is_empty() {
            fit.mv_x_columns.clone_from(&self.mv_x_columns);
        }
        if self.mv_y_column.is_some() {
            fit.mv_y_column = self.mv_y_column;
        }
        if !self.mv_group_columns.is_empty() {
            fit.mv_group_columns.clone_from(&self.mv_group_columns);
        }
        if self.source_table_kind == Some(TableKind::MultipleVariables) && fit.mv_x_columns.is_empty() && fit.mv_y_column.is_none() {
            let numeric_cols: Vec<usize> = self
                .variable_candidates
                .iter()
                .filter(|v| !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                .map(|v| v.col_index)
                .collect();
            if numeric_cols.len() >= 2 {
                fit.mv_x_columns = vec![numeric_cols[0]];
                fit.mv_y_column = numeric_cols.last().copied();
            }
        }
    }

    fn populate_glm_kind(&self, glm: &mut curvebook_doc::GlmSpec, response_col: Option<usize>, predictor_cols: &[usize]) {
        glm.family = self.glm_family;
        glm.compute_vif = self.glm_compute_vif;
        if !self.glm_reference_levels.is_empty() {
            glm.reference_levels.clone_from(&self.glm_reference_levels);
        }
        let first_numeric = self
            .variable_candidates
            .iter()
            .find(|v| v.selected && !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
            .map(|v| v.col_index);
        if let Some(resp) = self.explicit_response_column.or(first_numeric).or(response_col) {
            glm.response_column = resp;
            let preds: Vec<usize> = self
                .variable_candidates
                .iter()
                .filter(|v| v.selected && v.col_index != resp && !matches!(v.role, VariableRole::Label))
                .map(|v| v.col_index)
                .collect();
            if !preds.is_empty() {
                glm.predictor_columns = preds;
            }
        } else if !predictor_cols.is_empty() {
            glm.predictor_columns = predictor_cols.to_vec();
        }
    }

    fn populate_survival_kind(&self, surv: &mut SurvivalAnalysisSpec, response_col: Option<usize>, predictor_cols: &[usize]) {
        match surv {
            SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method } => {
                let numeric_cols: Vec<usize> = self
                    .variable_candidates
                    .iter()
                    .filter(|v| v.selected && !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                    .map(|v| v.col_index)
                    .collect();
                if let Some(tc) = self.cox_time_column.or_else(|| numeric_cols.first().copied()).or(response_col) {
                    *time_column = tc;
                }
                if let Some(sc) = self
                    .cox_status_column
                    .or_else(|| numeric_cols.iter().copied().find(|&c| c != *time_column))
                    .or_else(|| predictor_cols.first().copied())
                {
                    *status_column = sc;
                }
                if !self.cox_predictor_columns.is_empty() {
                    predictor_columns.clone_from(&self.cox_predictor_columns);
                } else {
                    let covs: Vec<usize> = self
                        .variable_candidates
                        .iter()
                        .filter(|v| {
                            v.selected && !matches!(v.role, VariableRole::Label) && v.col_index != *time_column && v.col_index != *status_column
                        })
                        .map(|v| v.col_index)
                        .collect();
                    if !covs.is_empty() {
                        *predictor_columns = covs;
                    }
                }
                if !self.cox_reference_levels.is_empty() {
                    reference_levels.clone_from(&self.cox_reference_levels);
                }
                *tie_method = self.cox_tie_method;
            }
            SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox,
                gehan_breslow_wilcoxon,
                trend_test,
                multiplicity,
                mv_time_column,
                mv_status_column,
                mv_group_column,
                ..
            } => {
                *log_rank_mantel_cox = self.km_log_rank_mantel_cox;
                *gehan_breslow_wilcoxon = self.km_gehan_breslow_wilcoxon;
                *trend_test = self.km_trend_test;
                multiplicity.method = self.km_multiplicity_method;
                *mv_time_column = self.km_mv_time_column;
                *mv_status_column = self.km_mv_status_column;
                if self.mv_group_column.is_some() {
                    *mv_group_column = self.mv_group_column;
                }
            }
        }
    }

    /// Find existing result sheets on `doc` that already analyze the same source table using the
    /// same high-level analysis family (for the duplicate-analysis confirmation prompt).
    #[must_use]
    pub fn matching_existing_analyses(&self, doc: &ProjectDocument) -> Vec<SheetId> {
        let Some(src) = &self.source_sheet_id else {
            return Vec::new();
        };
        let Some(pending) = self.build_analysis_kind() else {
            return Vec::new();
        };
        let pending_tag = std::mem::discriminant(&pending);
        doc.result_sheets
            .iter()
            .filter(|rs| &rs.analysis.source_sheet_id == src && std::mem::discriminant(&rs.analysis.kind) == pending_tag)
            .map(|rs| rs.id.clone())
            .collect()
    }

    /// Either return `Err(existing_ids)` to prompt `Modify existing` vs `Create new`, or build an
    /// `analysis.run` [`ToolCommandIntent`].
    pub fn try_build_run_intent(&self, doc: &ProjectDocument) -> Result<ToolCommandIntent, Vec<SheetId>> {
        let matches = self.matching_existing_analyses(doc);
        if !matches.is_empty() {
            return Err(matches);
        }
        Ok(self.build_create_new_intent().unwrap_or_else(|| ToolCommandIntent::new("analyze.run", json!({}))))
    }

    /// Resolve a duplicate-analysis confirmation prompt into `analysis.updateParams` or `analysis.run`.
    #[must_use]
    pub fn resolve_duplicate_choice(&self, choice: &AnalyzeDuplicateChoice) -> Option<ToolCommandIntent> {
        let kind = self.build_analysis_kind()?;
        match choice {
            AnalyzeDuplicateChoice::ModifyExisting(result_sheet_id) => Some(ToolCommandIntent::new(
                "analysis.updateParams",
                json!({
                    "sheet_id": result_sheet_id.as_str(),
                    "kind": kind,
                    "selected_data_sets": self.selected_data_sets,
                }),
            )),
            AnalyzeDuplicateChoice::CreateNew => self.build_create_new_intent(),
        }
    }

    /// Build an unconditional `analysis.run` intent for creating a new result sheet.
    #[must_use]
    pub fn build_create_new_intent(&self) -> Option<ToolCommandIntent> {
        let src = self.source_sheet_id.as_ref()?;
        let kind = self.build_analysis_kind()?;
        Some(ToolCommandIntent::new(
            "analysis.run",
            json!({
                "source_sheet_id": src.as_str(),
                "selected_data_sets": self.selected_data_sets,
                "kind": kind,
            }),
        ))
    }
}

fn collect_variable_candidates(sheet: &DataSheet) -> Vec<VariableCandidate> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let is_numeric_var = |v: &curvebook_doc::VariableColumn| !matches!(v.role, VariableRole::Label | VariableRole::Categorical);
            let default_resp_col = mv
                .variables
                .iter()
                .enumerate()
                .rev()
                .find(|(_, v)| is_numeric_var(v) && v.cells.iter().any(|c| c.active_number().is_some()))
                .map(|(i, _)| i)
                .or_else(|| mv.variables.iter().enumerate().rev().find(|(_, v)| is_numeric_var(v)).map(|(i, _)| i))
                .unwrap_or(0);
            mv.variables
                .iter()
                .enumerate()
                .map(|(col_index, var)| VariableCandidate {
                    col_index,
                    title: var.title.clone(),
                    role: var.role,
                    role_inferred: var.role_inferred,
                    selected: true,
                    is_response: col_index == default_resp_col,
                })
                .collect()
        }
        _ => (0..sheet.archetype.column_count())
            .map(|col_index| VariableCandidate {
                col_index,
                title: format!("Col {}", curvebook_doc::default_column_label(col_index)),
                role: VariableRole::Continuous,
                role_inferred: true,
                selected: true,
                is_response: false,
            })
            .collect(),
    }
}

fn collect_result_tab_candidates(tab: &ResultTab) -> Vec<VariableCandidate> {
    tab.column_headers
        .iter()
        .enumerate()
        .map(|(col_index, header)| {
            let has_text = tab.rows.iter().filter_map(|r| r.get(col_index)).any(|c| matches!(c, ResultCell::Text(_) | ResultCell::Boolean(_)));
            VariableCandidate {
                col_index,
                title: header.clone(),
                role: if has_text { VariableRole::Categorical } else { VariableRole::Continuous },
                role_inferred: true,
                selected: true,
                is_response: col_index == 0,
            }
        })
        .collect()
}

fn default_true_flag() -> bool {
    true
}

fn default_edit_tab() -> String {
    String::from("experimental_design")
}

/// `Cmd+T` parameter re-entry draft editing an existing [`AnalysisSpec`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditAnalysisParametersState {
    pub result_sheet_id: Option<SheetId>,
    pub draft: Option<AnalysisSpec>,
    pub title_was_custom: bool,
    #[serde(default = "default_edit_tab")]
    pub active_tab: String,
    #[serde(default = "default_true_flag")]
    pub assume_gaussian: bool,
    #[serde(default)]
    pub paired_or_matched: bool,
    #[serde(default)]
    pub compare_against_control: bool,
    #[serde(default)]
    pub control_column: usize,
    #[serde(default = "default_true_flag")]
    pub correlation_all_pairs: bool,
    #[serde(default)]
    pub correlation_heatmap: bool,
    #[serde(default)]
    pub contingency_odds_ratio_checked: bool,
    #[serde(default)]
    pub contingency_relative_risk_checked: bool,
    #[serde(default)]
    pub contingency_attributable_risk_checked: bool,
    #[serde(default)]
    pub contingency_or_ci_method: String,
    #[serde(default)]
    pub contingency_rr_ci_method: String,
    #[serde(default)]
    pub contingency_arr_ci_method: String,
    #[serde(default)]
    pub contingency_diag_ci_method: String,
    #[serde(default)]
    pub graph_differences: bool,
    #[serde(default)]
    pub graph_ranks: bool,
    #[serde(default)]
    pub hodges_lehmann_ci: bool,
    #[serde(default)]
    pub welch_correction: bool,
    #[serde(default)]
    pub pca_predictors: Vec<usize>,
    #[serde(default)]
    pub pcr_outcome_column: Option<usize>,
    #[serde(default)]
    pub role_filter: VariableRoleFilter,
    #[serde(default = "default_tg_param_unpaired")]
    pub saved_two_group_param_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_param_paired")]
    pub saved_two_group_param_paired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_unpaired")]
    pub saved_two_group_nonparam_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_paired")]
    pub saved_two_group_nonparam_paired: TwoGroupMethod,
    #[serde(default = "default_one_way_param_unpaired")]
    pub saved_one_way_param_unpaired: OneWayMethod,
    #[serde(default = "default_true_flag")]
    pub saved_rm_geisser_greenhouse: bool,
    #[serde(default = "default_outlier_fdr_q")]
    pub saved_outlier_rout_q: f64,
    #[serde(default = "default_outlier_alpha")]
    pub saved_outlier_grubbs_alpha: f64,
    #[serde(default)]
    pub saved_outlier_grubbs_iterative: bool,
}

impl Default for EditAnalysisParametersState {
    fn default() -> Self {
        Self {
            result_sheet_id: None,
            draft: None,
            title_was_custom: false,
            active_tab: default_edit_tab(),
            assume_gaussian: true,
            paired_or_matched: false,
            compare_against_control: false,
            control_column: 0,
            correlation_all_pairs: true,
            correlation_heatmap: false,
            contingency_odds_ratio_checked: false,
            contingency_relative_risk_checked: false,
            contingency_attributable_risk_checked: false,
            contingency_or_ci_method: String::from("Baptista-Pike"),
            contingency_rr_ci_method: String::from("Koopman"),
            contingency_arr_ci_method: String::from("Newcombe-Wilson"),
            contingency_diag_ci_method: String::from("Wilson-Brown"),
            graph_differences: false,
            graph_ranks: false,
            hodges_lehmann_ci: false,
            welch_correction: false,
            pca_predictors: Vec::new(),
            pcr_outcome_column: None,
            role_filter: VariableRoleFilter::All,
            saved_two_group_param_unpaired: default_tg_param_unpaired(),
            saved_two_group_param_paired: default_tg_param_paired(),
            saved_two_group_nonparam_unpaired: default_tg_nonparam_unpaired(),
            saved_two_group_nonparam_paired: default_tg_nonparam_paired(),
            saved_one_way_param_unpaired: default_one_way_param_unpaired(),
            saved_rm_geisser_greenhouse: true,
            saved_outlier_rout_q: default_outlier_fdr_q(),
            saved_outlier_grubbs_alpha: default_outlier_alpha(),
            saved_outlier_grubbs_iterative: false,
        }
    }
}

impl EditAnalysisParametersState {
    /// Toggle a MV predictor column for PCA/PCR while maintaining disjoint PCR outcome selection.
    pub fn toggle_pca_predictor(&mut self, col: usize) {
        if let Some(pos) = self.pca_predictors.iter().position(|&c| c == col) {
            self.pca_predictors.remove(pos);
        } else {
            self.pca_predictors.push(col);
            self.pca_predictors.sort_unstable();
            if self.pcr_outcome_column == Some(col) {
                self.pcr_outcome_column = None;
            }
        }
        if let Some(spec) = &mut self.draft {
            spec.selected_data_sets.clone_from(&self.pca_predictors);
        }
    }

    /// Select (or toggle off) the disjoint PCR outcome column with R10 predictor leakage guard.
    pub fn select_pcr_outcome(&mut self, col: usize) -> Result<(), String> {
        if self.pca_predictors.contains(&col) {
            return Err(format!("Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."));
        }
        self.pcr_outcome_column = if self.pcr_outcome_column == Some(col) { None } else { Some(col) };
        if let Some(spec) = &mut self.draft {
            spec.selected_data_sets.retain(|&c| Some(c) != self.pcr_outcome_column);
        }
        Ok(())
    }
    /// Load an existing [`ResultSheet`] for parameter re-entry (`Cmd+T`).
    /// Reset dialog state and open Edit Analysis Parameters for an existing `ResultSheet` (`RV-79`).
    pub fn open_for_result(&mut self, result: &ResultSheet) {
        *self = Self::default();
        self.result_sheet_id = Some(result.id.clone());
        self.title_was_custom = result.analysis.custom_title.is_some();
        self.active_tab = default_edit_tab();
        self.draft = Some(result.analysis.clone());
        self.pca_predictors = if matches!(result.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
            result.analysis.selected_data_sets.clone()
        } else {
            Vec::new()
        };
        self.pcr_outcome_column = None;
        self.sync_rule_flags_from_kind();
    }

    /// Capture compatible sub-method selections before a design/Gaussian toggle (`RV-76`, `RV-78`).
    pub fn remember_submethods_from_kind(&mut self, kind: &AnalysisKind) {
        match kind {
            AnalysisKind::OneWayAnova(spec) => match spec.method {
                OneWayMethod::OrdinaryAnova | OneWayMethod::BrownForsytheAndWelch => {
                    self.saved_one_way_param_unpaired = spec.method;
                }
                OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
                    self.saved_rm_geisser_greenhouse = geisser_greenhouse;
                }
                OneWayMethod::KruskalWallis | OneWayMethod::Friedman => {}
            },
            AnalysisKind::TwoGroupComparison(spec) => match spec.method {
                TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT => {
                    self.saved_two_group_param_unpaired = spec.method;
                }
                TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT => {
                    self.saved_two_group_param_paired = spec.method;
                }
                TwoGroupMethod::MannWhitneyU | TwoGroupMethod::KolmogorovSmirnov => {
                    self.saved_two_group_nonparam_unpaired = spec.method;
                }
                TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest => {
                    self.saved_two_group_nonparam_paired = spec.method;
                }
                TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
            },
            AnalysisKind::OutlierDetection(spec) => match spec {
                OutlierSpec::Rout { fdr_q_percent } => self.saved_outlier_rout_q = *fdr_q_percent,
                OutlierSpec::Grubbs { alpha, iterative } => {
                    self.saved_outlier_grubbs_alpha = *alpha;
                    self.saved_outlier_grubbs_iterative = *iterative;
                }
            },
            _ => {}
        }
    }

    fn resolved_two_group_method(&self, assume_gaussian: bool, matched: bool, current: TwoGroupMethod) -> TwoGroupMethod {
        if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = current {
            return if assume_gaussian { TwoGroupMethod::OneSampleT { hypothetical } } else { TwoGroupMethod::OneSampleWilcoxon { hypothetical } };
        }
        match (assume_gaussian, matched) {
            (true, false) => self.saved_two_group_param_unpaired,
            (true, true) => self.saved_two_group_param_paired,
            (false, false) => self.saved_two_group_nonparam_unpaired,
            (false, true) => self.saved_two_group_nonparam_paired,
        }
    }

    /// Synchronize predicate flags (`assume_gaussian`, `paired_or_matched`, etc.) from `draft.kind`.
    pub fn sync_rule_flags_from_kind(&mut self) {
        let Some(kind) = self.draft.as_ref().map(|s| s.kind.clone()) else {
            return;
        };
        self.remember_submethods_from_kind(&kind);
        match &kind {
            AnalysisKind::OneWayAnova(spec) => {
                self.assume_gaussian = !matches!(spec.method, OneWayMethod::KruskalWallis | OneWayMethod::Friedman);
                self.paired_or_matched = matches!(spec.method, OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman);
                if let MultiplicityMethod::Dunnett { control_column } = spec.multiplicity.method {
                    self.compare_against_control = true;
                    self.control_column = control_column;
                } else {
                    self.compare_against_control = false;
                }
            }
            AnalysisKind::FactorialAnova(spec) => {
                self.assume_gaussian = true;
                self.paired_or_matched = spec.repeated_measures != RepeatedMeasuresDesign::None;
                if let MultiplicityMethod::Dunnett { control_column } = spec.multiplicity.method {
                    self.compare_against_control = true;
                    self.control_column = control_column;
                } else {
                    self.compare_against_control = false;
                }
            }
            AnalysisKind::TwoGroupComparison(spec) => {
                self.assume_gaussian = !matches!(
                    spec.method,
                    TwoGroupMethod::MannWhitneyU
                        | TwoGroupMethod::WilcoxonSignedRank
                        | TwoGroupMethod::KolmogorovSmirnov
                        | TwoGroupMethod::OneSampleWilcoxon { .. }
                        | TwoGroupMethod::PairedSignTest
                );
                self.paired_or_matched = matches!(
                    spec.method,
                    TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
                );
                self.welch_correction = spec.method == TwoGroupMethod::WelchT;
            }
            AnalysisKind::Contingency(spec) => {
                self.contingency_odds_ratio_checked = spec.compute_odds_and_risk_ratios;
                self.contingency_relative_risk_checked = spec.compute_odds_and_risk_ratios;
            }
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }) => {
                self.correlation_all_pairs = true;
            }
            _ => {}
        }
    }

    /// Mutate [`AnalysisKind`] in-place, updating `auto_title` while preserving any user-pinned
    /// `custom_title`.
    pub fn set_kind(&mut self, new_kind: AnalysisKind) -> bool {
        let Some(spec) = &mut self.draft else {
            return false;
        };
        spec.update_kind(new_kind);
        self.sync_rule_flags_from_kind();
        true
    }

    /// Update confidence percentage (`0.01..=99.99`).
    pub fn set_confidence_percent(&mut self, percent: f64) -> Result<(), String> {
        let Some(spec) = &mut self.draft else {
            return Err(String::from("no active analysis draft"));
        };
        spec.set_confidence_percent(percent).map_err(|e| e.to_string())
    }

    /// Set display P-value reporting format.
    pub fn set_p_value_format(&mut self, format: PValueFormat) {
        if let Some(spec) = &mut self.draft {
            spec.p_value_format = format;
        }
    }

    /// Pin or clear custom title on the draft [`AnalysisSpec`].
    pub fn set_custom_title(&mut self, custom: Option<String>) -> Result<(), String> {
        let Some(spec) = &mut self.draft else {
            return Err(String::from("no active analysis draft"));
        };
        spec.set_custom_title(custom).map_err(|e| e.to_string())?;
        self.title_was_custom = spec.custom_title.is_some();
        Ok(())
    }

    /// Apply modal field updates from `ui.answer_dialog` / parameter widgets while enforcing R1–R9 predicates.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal field updates using regional numeric parsing (`preset`) while enforcing R1–R9 predicates.
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        let priority_keys = [
            "outlier_method",
            "edit.outlier.method",
            "fit_method",
            "edit.fit.method",
            "method",
            "assume_gaussian",
            "edit.assume_gaussian",
            "paired_or_matched",
            "matched",
            "edit.paired_or_matched",
        ];
        for key in priority_keys {
            if let Some(val) = fields.get(key) {
                self.apply_single_param_patch(key, val, preset)?;
            }
        }
        for (key, val) in fields {
            if priority_keys.contains(&key.as_str()) {
                continue;
            }
            self.apply_single_param_patch(key, val, preset)?;
        }
        Ok(())
    }

    fn apply_single_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        if self.apply_contingency_param_patch(key, val, preset)?
            || self.apply_transform_param_patch(key, val)?
            || self.apply_pvalue_stack_param_patch(key, val, preset)?
            || self.apply_power_analysis_param_patch(key, val, preset)?
            || self.apply_r9_param_patch(key, val)?
        {
            return Ok(());
        }
        match key {
            "active_tab" | "tab" => {
                if let Some(t) = val.as_str() {
                    if t.eq_ignore_ascii_case("repeated_measures") && !self.paired_or_matched {
                        return Err(String::from("Repeated Measures tab is disabled when No matching is selected."));
                    }
                    self.active_tab = t.to_owned();
                }
            }
            "assume_gaussian" | "edit.assume_gaussian" => {
                let g = val.as_bool().ok_or_else(|| String::from("`assume_gaussian` must be boolean"))?;
                self.apply_gaussian_toggle_patch(g);
            }
            "paired_or_matched" | "matched" | "edit.paired_or_matched" => {
                let matched = val.as_bool().ok_or_else(|| String::from("`paired_or_matched` must be boolean"))?;
                self.paired_or_matched = matched;
                self.apply_matched_toggle(matched);
            }
            "compare_against_control" | "edit.compare_against_control" => {
                let cmp = val.as_bool().ok_or_else(|| String::from("`compare_against_control` must be boolean"))?;
                self.compare_against_control = cmp;
                if cmp {
                    let ctrl = self.control_column;
                    self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: ctrl });
                }
            }
            "control_column" | "edit.control_column" => {
                let col = val.as_u64().ok_or_else(|| String::from("`control_column` must be a non-negative integer"))? as usize;
                self.control_column = col;
                if self.compare_against_control {
                    self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: col });
                }
            }
            "welch_correction" | "edit.welch_correction" | "edit.two_group.welch" => {
                let w = val.as_bool().ok_or_else(|| String::from("`welch_correction` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(mut two) = spec.kind.clone()
                {
                    if w && !crate::modal_predicates::r4_two_group_welch_correction(two.method).enabled {
                        return Err(String::from("Welch's correction is disabled for paired or nonparametric tests."));
                    }
                    self.welch_correction = w;
                    two.method = if w { TwoGroupMethod::WelchT } else { TwoGroupMethod::UnpairedT };
                    spec.update_kind(AnalysisKind::TwoGroupComparison(two));
                    self.sync_rule_flags_from_kind();
                }
            }
            "graph_differences" | "edit.two_group.graph_differences" => {
                let b = val.as_bool().ok_or_else(|| String::from("`graph_differences` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
                    && b
                    && !crate::modal_predicates::r4_two_group_graph_differences(two.method).enabled
                {
                    return Err(String::from("Estimation plot of differences is disabled in nonparametric/rank mode."));
                }
                self.graph_differences = b;
            }
            "graph_ranks" | "edit.two_group.graph_ranks" => {
                let b = val.as_bool().ok_or_else(|| String::from("`graph_ranks` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
                    && b
                    && !crate::modal_predicates::r4_two_group_nonparametric_options(two.method).enabled
                {
                    return Err(String::from("Rank-based option is disabled in Gaussian parametric mode."));
                }
                self.graph_ranks = b;
            }
            "hodges_lehmann_ci" | "hodges_lehmann" | "edit.two_group.hodges_lehmann" => {
                let b = val.as_bool().ok_or_else(|| String::from("`hodges_lehmann_ci` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
                    && b
                    && !crate::modal_predicates::r4_two_group_nonparametric_options(two.method).enabled
                {
                    return Err(String::from("Hodges-Lehmann CI is disabled in Gaussian parametric mode."));
                }
                self.hodges_lehmann_ci = b;
            }
            "correlation_all_pairs" | "edit.correlation_all_pairs" => {
                if let Some(b) = val.as_bool() {
                    self.correlation_all_pairs = b;
                    if !b {
                        self.correlation_heatmap = false;
                    }
                }
            }
            "correlation_heatmap" | "edit.correlation_heatmap" | "edit.correlation.heatmap" => {
                if let Some(b) = val.as_bool() {
                    if b && !self.correlation_all_pairs {
                        return Err(String::from("Correlation heatmap is disabled unless all-pairs Correlation matrix mode is active."));
                    }
                    self.correlation_heatmap = b;
                }
            }
            "p_value_format" | "edit.p_value_format" => {
                if let Some(s) = val.as_str() {
                    let next = match s.trim().to_ascii_lowercase().as_str() {
                        "apa" => PValueFormat::Apa,
                        "nejm" => PValueFormat::Nejm,
                        "custom" => PValueFormat::new_custom(4, None),
                        _ => PValueFormat::FourDecimal,
                    };
                    self.set_p_value_format(next);
                }
            }
            "p_value_decimals" | "edit.p_value_digits" | "edit.pvalue.digits" => {
                let d = u8::try_from(parse_usize_min(val, key, 0)?).map_err(|_| String::from("`p_value_decimals` out of range"))?;
                if let Some(spec) = &mut self.draft {
                    if !matches!(spec.p_value_format, PValueFormat::Custom { .. }) {
                        return Err(String::from("Custom decimal precision spinner is disabled unless Custom P-value format is selected."));
                    }
                    let sci = match spec.p_value_format {
                        PValueFormat::Custom { scientific_below, .. } => scientific_below,
                        _ => None,
                    };
                    spec.p_value_format = PValueFormat::new_custom(d, sci);
                }
            }
            "confidence_percent" | "edit.confidence_percent" => {
                if !val.is_null() {
                    let c = parse_finite_f64_with_locale(val, key, preset)?;
                    self.set_confidence_percent(c)?;
                }
            }
            "custom_title" | "edit.custom_title" => {
                let parsed = match val {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => return Err(String::from("`custom_title` must be a string or null")),
                };
                self.set_custom_title(parsed)?;
            }
            "selected_data_sets" | "edit.selected_data_sets" | "edit.transform.selected_data_sets" => {
                let cols = parse_usize_list(val, key)?;
                if let Some(spec) = &mut self.draft {
                    if matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
                        self.pca_predictors.clone_from(&cols);
                    }
                    spec.selected_data_sets = cols;
                }
            }
            "pca_predictors" | "edit.pca.predictors" => {
                let cols = parse_usize_list(val, key)?;
                self.pca_predictors.clone_from(&cols);
                if let Some(spec) = &mut self.draft {
                    spec.selected_data_sets = cols;
                }
            }
            "pcr_outcome_column" | "edit.pcr.outcome_column" => {
                if val.is_null() {
                    self.pcr_outcome_column = None;
                } else {
                    let col = parse_usize_min(val, key, 0)?;
                    if self.pca_predictors.contains(&col) {
                        return Err(format!(
                            "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                        ));
                    }
                    self.pcr_outcome_column = Some(col);
                    if let Some(spec) = &mut self.draft {
                        spec.selected_data_sets.retain(|&c| c != col);
                    }
                }
            }
            "geisser_greenhouse" | "edit.geisser_greenhouse" | "edit.sphericity.geisser_greenhouse" => {
                if !self.paired_or_matched {
                    return Err(String::from("Geisser-Greenhouse sphericity control is disabled when No matching is selected."));
                }
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::OneWayAnova(mut a) = spec.kind.clone()
                {
                    if a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured {
                        return Err(String::from("Sphericity corrections are disabled when Mixed-Effects REML is active."));
                    }
                    if let Some(gg) = val.as_bool() {
                        a.method = OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg };
                        spec.update_kind(AnalysisKind::OneWayAnova(a));
                    }
                }
            }
            "multiplicity_method" => {
                if let Some(raw) = val.as_str() {
                    let method = parse_multiplicity_method_name(raw, self.control_column)?;
                    let draft_reml = self.draft.as_ref().is_some_and(draft_forces_reml_mode);
                    let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(self.assume_gaussian, draft_reml, method);
                    if !gate.enabled {
                        return Err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method disabled.")));
                    }
                    self.set_anova_multiplicity(method);
                }
            }
            key if key.starts_with("edit.post_hoc.") || key.starts_with("edit.posthoc.") => {
                let method = post_hoc_method_for_widget_key(key, self.control_column)
                    .ok_or_else(|| format!("unknown parameter field `{key}` in EditAnalysisParameters"))?;
                if val.as_bool().unwrap_or(true) {
                    self.apply_checked_post_hoc_patch(method)?;
                }
            }
            "anova_method" | "one_way_method" | "edit.anova_method" | "edit.one_way_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`anova_method` must be a string"))?.trim().to_ascii_lowercase();
                let saved_gg = self.saved_rm_geisser_greenhouse;
                let next_method = match raw.as_str() {
                    "ordinary" | "ordinary_anova" | "anova" => OneWayMethod::OrdinaryAnova,
                    "welch" | "brown_forsythe" | "brown_forsythe_welch" | "brown_forsythe_and_welch" => OneWayMethod::BrownForsytheAndWelch,
                    "kruskal" | "kruskal_wallis" => OneWayMethod::KruskalWallis,
                    "friedman" => OneWayMethod::Friedman,
                    "rm" | "repeated_measures" => OneWayMethod::RepeatedMeasures { geisser_greenhouse: saved_gg },
                    _ => return Err(format!("unknown one-way ANOVA method `{raw}`")),
                };
                if let Some(mut spec) = self.draft.take() {
                    self.remember_submethods_from_kind(&spec.kind);
                    if let AnalysisKind::OneWayAnova(mut a) = spec.kind.clone() {
                        a.method = next_method;
                        spec.update_kind(AnalysisKind::OneWayAnova(a));
                    }
                    self.draft = Some(spec);
                    self.sync_rule_flags_from_kind();
                }
            }
            other => return Err(format!("unknown Edit Analysis Parameters field `{other}`")),
        }
        Ok(())
    }

    /// Applies Contingency-specific field patches on `EditAnalysisParametersState`.
    fn apply_contingency_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        let method_flag = match key {
            "fisher" | "edit.contingency.fisher" => Some(ContingencyMethod::FisherExact),
            "barnard" | "edit.contingency.barnard" | "edit.contingency.barnard_exact" => Some(ContingencyMethod::BarnardExact),
            "likelihood_ratio_g2" | "g2" | "edit.contingency.g2" | "edit.contingency.likelihood_g2" => Some(ContingencyMethod::LikelihoodRatioG2),
            "mcnemar" | "edit.contingency.mcnemar" => Some(ContingencyMethod::McNemarPaired),
            "cochran_armitage" | "edit.contingency.cochran_armitage" => Some(ContingencyMethod::CochranArmitageTrend),
            "cohen_kappa" | "edit.contingency.cohen_kappa" => {
                Some(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Unweighted })
            }
            "fleiss_kappa" | "edit.contingency.fleiss_kappa" => Some(ContingencyMethod::FleissKappa),
            "exact_binomial" | "edit.contingency.exact_binomial" => Some(ContingencyMethod::ExactBinomial),
            _ => None,
        };
        if let Some(target_method) = method_flag {
            if let Some(b) = val.as_bool() {
                let next = if b { target_method } else { ContingencyMethod::PearsonChiSquare { yates_continuity: false } };
                self.update_contingency_spec(|c| c.method = next);
            }
            return Ok(true);
        }
        match key {
            "kappa_weighting" | "edit.contingency.kappa_weighting" => {
                let w = parse_kappa_weighting(val)?;
                self.update_contingency_spec(|c| c.method = ContingencyMethod::CohenKappa { weighting: w });
            }
            "weighting" if self.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Contingency(_))) => {
                let w = parse_kappa_weighting(val)?;
                self.update_contingency_spec(|c| c.method = ContingencyMethod::CohenKappa { weighting: w });
            }
            "null_proportion" | "edit.contingency.null_proportion" => {
                let p = parse_opt_finite_f64_with_locale(val, key, preset)?;
                if let Some(v) = p
                    && !(0.0 < v && v < 1.0)
                {
                    return Err(format!("`null_proportion` ({v}) must be strictly between 0 and 1"));
                }
                self.update_contingency_spec(|c| c.null_proportion = p);
            }
            "expected_proportions" | "edit.contingency.expected_proportions" => {
                let props = if val.is_null() { None } else { Some(parse_f64_list_with_locale(val, key, preset)?) };
                self.update_contingency_spec(|c| c.expected_proportions = props);
            }
            "edit.contingency.ci.baptista_pike" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_or_ci_method = s.to_owned();
            }
            "edit.contingency.ci.koopman" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_rr_ci_method = s.to_owned();
            }
            "edit.contingency.ci.newcombe_wilson" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_arr_ci_method = s.to_owned();
            }
            "edit.contingency.ci.wilson_brown" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_diag_ci_method = s.to_owned();
            }
            "compute_odds_and_risk_ratios" | "contingency_odds_ratio_checked" | "edit.contingency.odds_ratio" => {
                if let Some(b) = val.as_bool() {
                    self.contingency_odds_ratio_checked = b;
                    self.contingency_relative_risk_checked = b;
                    self.update_contingency_spec(|c| c.compute_odds_and_risk_ratios = b);
                }
            }
            "contingency_attributable_risk_checked" | "attributable_risk" | "compute_attributable_risk" | "edit.contingency.attributable_risk" => {
                if let Some(b) = val.as_bool() {
                    self.contingency_attributable_risk_checked = b;
                }
            }
            "compute_sensitivity_specificity" | "sensitivity_specificity" | "edit.contingency.sensitivity_specificity" => {
                if let Some(b) = val.as_bool() {
                    self.update_contingency_spec(|c| c.compute_sensitivity_specificity = b);
                }
            }
            "contingency_method" => {
                if let Some(raw) = val.as_str() {
                    let method = parse_contingency_method_name(raw)?;
                    self.update_contingency_spec(|c| c.method = method);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn update_contingency_spec(&mut self, mutate: impl FnOnce(&mut curvebook_doc::ContingencySpec)) {
        if let Some(spec) = &mut self.draft
            && let AnalysisKind::Contingency(mut c) = spec.kind.clone()
        {
            mutate(&mut c);
            spec.update_kind(AnalysisKind::Contingency(c));
        }
    }

    /// Applies `PValueStack`-specific field patches on `EditAnalysisParametersState`.
    fn apply_pvalue_stack_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let AnalysisKind::PValueStack(mut ps) = spec.kind.clone() else {
            return Ok(false);
        };
        let handled = match key {
            "method"
            | "pvalue_stack_method"
            | "multiplicity_method"
            | "multiplicity"
            | "edit.pvalue_stack.method"
            | "edit.pvalue_stack.multiplicity_method" => {
                ps.method = parse_pvalue_stack_method_val(val)?;
                true
            }
            "threshold" | "pvalue_stack_threshold" | "alpha" | "q" | "fdr_q" | "edit.pvalue_stack.threshold" => {
                let t = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0 < t && t < 1.0) {
                    return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
                }
                ps.threshold = t;
                true
            }
            "column" | "pvalue_stack_column" | "edit.pvalue_stack.column" => {
                ps.column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            _ => false,
        };
        if handled {
            spec.update_kind(AnalysisKind::PValueStack(ps));
        }
        Ok(handled)
    }

    /// Applies `PowerAnalysis`-specific field patches on `EditAnalysisParametersState`.
    fn apply_power_analysis_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let AnalysisKind::PowerAnalysis(mut pa) = spec.kind.clone() else {
            return Ok(false);
        };
        let handled = match key {
            "edit.power.family" | "power.family" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
                pa.family = match norm.as_str() {
                    "unpairedt" | "twosampleunpairedt" | "independentt" => curvebook_doc::PowerTestFamily::TwoSampleUnpairedT,
                    "pairedt" | "twosamplepairedt" | "dependentt" => curvebook_doc::PowerTestFamily::TwoSamplePairedT,
                    "onesamplet" | "singlesamplet" => curvebook_doc::PowerTestFamily::OneSampleT,
                    "onewayanova" | "anova" => curvebook_doc::PowerTestFamily::OneWayAnova,
                    "twoproportions" | "proportions" => curvebook_doc::PowerTestFamily::TwoProportions,
                    "correlation" | "pearsoncorrelation" => curvebook_doc::PowerTestFamily::Correlation,
                    "chisquare" | "chi2" => curvebook_doc::PowerTestFamily::ChiSquare,
                    "simplelinearregression" | "linearregression" | "regression" => curvebook_doc::PowerTestFamily::SimpleLinearRegression,
                    _ => return Err(format!("unknown power test family `{raw}`")),
                };
                true
            }
            "edit.power.solve_target" | "power.solve_target" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
                pa.solve_target = match norm.as_str() {
                    "samplesize" | "n" => curvebook_doc::PowerSolveTarget::SampleSize,
                    "power" => curvebook_doc::PowerSolveTarget::Power,
                    "detectableeffect" | "effect" | "mde" => curvebook_doc::PowerSolveTarget::DetectableEffect,
                    _ => return Err(format!("unknown power solve target `{raw}`")),
                };
                true
            }
            "edit.power.alpha" => {
                let a = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0..1.0).contains(&a) || a <= 0.0 {
                    return Err(format!("`{key}` must lie strictly in (0, 1)"));
                }
                pa.alpha = a;
                true
            }
            "edit.power.target_power" => {
                if val.is_null() {
                    pa.target_power = None;
                } else {
                    let pw = parse_finite_f64_with_locale(val, key, preset)?;
                    if !(0.0..1.0).contains(&pw) || pw <= 0.0 {
                        return Err(format!("`{key}` must lie strictly in (0, 1)"));
                    }
                    pa.target_power = Some(pw);
                }
                true
            }
            "edit.power.sample_size" => {
                pa.sample_size = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
                true
            }
            "edit.power.sample_size_group2" => {
                pa.sample_size_group2 = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
                true
            }
            "edit.power.allocation_ratio" => {
                let r = parse_finite_f64_with_locale(val, key, preset)?;
                if r <= 0.0 {
                    return Err(format!("`{key}` must be > 0"));
                }
                pa.allocation_ratio = r;
                true
            }
            "edit.power.two_tailed" => {
                let b = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                pa.tail_mode = if b {
                    curvebook_doc::PowerTailMode::TwoSided
                } else {
                    curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater)
                };
                true
            }
            "edit.power.continuity_correction" => {
                pa.continuity_correction = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                true
            }
            "edit.power.chi_square_df" => {
                let df = parse_finite_f64_with_locale(val, key, preset)?;
                if df <= 0.0 {
                    return Err(format!("`{key}` must be > 0"));
                }
                pa.chi_square_df = df;
                true
            }
            _ => false,
        };
        if handled {
            spec.update_kind(AnalysisKind::PowerAnalysis(pa));
        }
        Ok(handled)
    }

    /// Applies Transform-specific field patches on `EditAnalysisParametersState`.
    fn apply_transform_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        match key {
            "x_transform" | "edit.transform.x_transform" => {
                let parsed = parse_math_preset_opt(val)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, error_propagation, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: parsed, y_transform, error_propagation }));
                }
            }
            "y_transform" | "edit.transform.y_transform" => {
                let parsed = parse_math_preset_opt(val)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, error_propagation, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform: parsed, error_propagation }));
                }
            }
            "error_propagation" | "edit.transform.error_propagation" => {
                let ep_mode = parse_error_prop_value(val)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation: ep_mode }));
                }
            }
            "formula" | "transform_formula" | "edit.transform.formula" | "filter_formula" | "edit.transform.filter_formula" => {
                if let Some(spec) = &mut self.draft {
                    match spec.kind.clone() {
                        AnalysisKind::Transform(TransformAnalysisSpec::Math { mut x_transform, mut y_transform, error_propagation }) => {
                            if let Some(raw) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                apply_transform_formula_str(raw, &mut x_transform, &mut y_transform)?;
                                spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math {
                                    x_transform,
                                    y_transform,
                                    error_propagation,
                                }));
                            }
                        }
                        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, derived_columns, .. }) => {
                            let filter_formula = if val.is_null() {
                                None
                            } else {
                                let s = val.as_str().ok_or_else(|| String::from("`filter_formula` must be a string or null"))?.trim();
                                (!s.is_empty()).then(|| s.to_owned())
                            };
                            spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                                filter_formula,
                                target_archetype,
                                derived_columns,
                            }));
                        }
                        _ => {}
                    }
                }
            }
            "target_archetype" | "edit.transform.target_archetype" => {
                let raw = val.as_str().ok_or_else(|| String::from("`target_archetype` must be a string"))?;
                let parsed = parse_table_kind_str(raw)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, derived_columns, .. }) =
                        spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                        filter_formula,
                        target_archetype: parsed,
                        derived_columns,
                    }));
                }
            }
            "baseline_columns" | "edit.transform.baseline_columns" => {
                let cols = parse_usize_list(val, key)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: cols, operation }));
                }
            }
            "baseline_operation" | "operation" | "edit.transform.baseline_operation" => {
                let raw = val.as_str().ok_or_else(|| String::from("`baseline_operation` must be a string"))?;
                let op = parse_baseline_operation_str(raw)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation: op }));
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn apply_gaussian_toggle_patch(&mut self, assume_gaussian: bool) {
        self.assume_gaussian = assume_gaussian;
        let matched = self.paired_or_matched;
        let ctrl = self.control_column;
        let Some(mut spec) = self.draft.take() else {
            return;
        };
        self.remember_submethods_from_kind(&spec.kind);
        let mut k = spec.kind.clone();
        crate::modal_predicates::apply_r2_gaussian_toggle(&mut k, assume_gaussian, matched, ctrl);
        match &mut k {
            AnalysisKind::OneWayAnova(a) => {
                if assume_gaussian {
                    a.method = if matched {
                        OneWayMethod::RepeatedMeasures { geisser_greenhouse: self.saved_rm_geisser_greenhouse }
                    } else {
                        self.saved_one_way_param_unpaired
                    };
                }
            }
            AnalysisKind::TwoGroupComparison(t) => {
                t.method = self.resolved_two_group_method(assume_gaussian, matched, t.method);
            }
            _ => {}
        }
        spec.update_kind(k);
        self.draft = Some(spec);
        self.sync_rule_flags_from_kind();
    }

    fn apply_matched_toggle(&mut self, matched: bool) {
        let Some(mut spec) = self.draft.take() else {
            return;
        };
        self.remember_submethods_from_kind(&spec.kind);
        let mut updated = spec.kind.clone();
        match &mut updated {
            AnalysisKind::OneWayAnova(a) => {
                a.method = match (self.assume_gaussian, matched) {
                    (true, true) => OneWayMethod::RepeatedMeasures { geisser_greenhouse: self.saved_rm_geisser_greenhouse },
                    (true, false) => self.saved_one_way_param_unpaired,
                    (false, true) => OneWayMethod::Friedman,
                    (false, false) => OneWayMethod::KruskalWallis,
                };
            }
            AnalysisKind::TwoGroupComparison(t) => {
                t.method = self.resolved_two_group_method(self.assume_gaussian, matched, t.method);
            }
            AnalysisKind::FactorialAnova(f) => {
                f.repeated_measures = if matched { RepeatedMeasuresDesign::RowFactorMatched } else { RepeatedMeasuresDesign::None };
            }
            _ => {}
        }
        spec.update_kind(updated);
        self.draft = Some(spec);
        self.sync_rule_flags_from_kind();
    }

    fn apply_checked_post_hoc_patch(&mut self, method: MultiplicityMethod) -> Result<(), String> {
        let draft_reml = self.draft.as_ref().is_some_and(draft_forces_reml_mode);
        let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(self.assume_gaussian, draft_reml, method);
        if !gate.enabled {
            return Err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method is disabled.")));
        }
        self.set_anova_multiplicity(method);
        Ok(())
    }

    fn set_anova_multiplicity(&mut self, method: MultiplicityMethod) {
        let Some(spec) = &mut self.draft else {
            return;
        };
        let mut updated = spec.kind.clone();
        match &mut updated {
            AnalysisKind::OneWayAnova(a) => a.multiplicity.method = method,
            AnalysisKind::FactorialAnova(f) => f.multiplicity.method = method,
            AnalysisKind::NestedComparison(n) => n.multiplicity.method = method,
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method = method,
            _ => {}
        }
        spec.update_kind(updated);
    }

    fn apply_r9_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let mut updated = spec.kind.clone();
        let handled = match (key, &mut updated) {
            ("outlier_method" | "edit.outlier.method" | "method", AnalysisKind::OutlierDetection(out)) => {
                match *out {
                    OutlierSpec::Rout { fdr_q_percent } => self.saved_outlier_rout_q = fdr_q_percent,
                    OutlierSpec::Grubbs { alpha, iterative } => {
                        self.saved_outlier_grubbs_alpha = alpha;
                        self.saved_outlier_grubbs_iterative = iterative;
                    }
                }
                let raw = val.as_str().ok_or_else(|| String::from("`outlier_method` must be a string"))?;
                let (mode, iter_opt) = parse_outlier_method_str(raw)?;
                *out = if mode == "grubbs" {
                    OutlierSpec::Grubbs { alpha: self.saved_outlier_grubbs_alpha, iterative: iter_opt.unwrap_or(self.saved_outlier_grubbs_iterative) }
                } else {
                    OutlierSpec::Rout { fdr_q_percent: self.saved_outlier_rout_q }
                };
                true
            }
            (
                "fdr_q_percent" | "q_percent" | "rout_q" | "outlier_fdr_q_percent" | "edit.outlier.fdr_q_percent" | "edit.fit.rout_q",
                AnalysisKind::OutlierDetection(out),
            ) => {
                let q = parse_outlier_fdr_q_percent(val)?;
                self.saved_outlier_rout_q = q;
                match out {
                    OutlierSpec::Rout { fdr_q_percent } => *fdr_q_percent = q,
                    OutlierSpec::Grubbs { .. } => {
                        return Err(String::from("`fdr_q_percent` requires ROUT outlier detection mode"));
                    }
                }
                true
            }
            ("outlier_alpha" | "edit.outlier.alpha" | "alpha", AnalysisKind::OutlierDetection(out)) => {
                let a = parse_outlier_alpha(val)?;
                self.saved_outlier_grubbs_alpha = a;
                match out {
                    OutlierSpec::Grubbs { alpha, .. } => *alpha = a,
                    OutlierSpec::Rout { .. } => {
                        return Err(String::from("`alpha` for Grubbs requires Grubbs outlier detection mode"));
                    }
                }
                true
            }
            ("iterative" | "outlier_iterative" | "edit.outlier.iterative", AnalysisKind::OutlierDetection(out)) => {
                let it = val.as_bool().ok_or_else(|| String::from("`iterative` must be boolean"))?;
                self.saved_outlier_grubbs_iterative = it;
                match out {
                    OutlierSpec::Grubbs { iterative, .. } => *iterative = it,
                    OutlierSpec::Rout { .. } => {
                        return Err(String::from("`iterative` requires Grubbs outlier detection mode"));
                    }
                }
                true
            }
            ("descriptive_subcolumn_agg" | "subcolumn_agg" | "edit.descriptive.subcolumn_agg", AnalysisKind::DescriptiveStats(desc)) => {
                desc.aggregation = parse_subcolumn_aggregation_val(val)?;
                true
            }
            ("two_group_method" | "edit.two_group.method", AnalysisKind::TwoGroupComparison(t)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`two_group_method` must be a string"))?;
                let hyp = match t.method {
                    TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => hypothetical,
                    _ => 0.0,
                };
                t.method = parse_two_group_method_str(raw, hyp)?;
                true
            }
            ("paired_sign_test" | "edit.two_group.paired_sign_test", AnalysisKind::TwoGroupComparison(t)) => {
                let b = val.as_bool().ok_or_else(|| String::from("`paired_sign_test` must be boolean"))?;
                t.method = if b { TwoGroupMethod::PairedSignTest } else { TwoGroupMethod::WilcoxonSignedRank };
                true
            }
            ("two_tailed" | "edit.two_group.two_tailed", AnalysisKind::TwoGroupComparison(t)) => {
                t.two_tailed = val.as_bool().ok_or_else(|| String::from("`two_tailed` must be boolean"))?;
                true
            }
            ("compute_effect_sizes" | "edit.compute_effect_sizes", AnalysisKind::TwoGroupComparison(t)) => {
                t.compute_effect_sizes = val.as_bool().ok_or_else(|| String::from("`compute_effect_sizes` must be boolean"))?;
                true
            }
            ("compute_effect_sizes" | "edit.compute_effect_sizes", AnalysisKind::FactorialAnova(f)) => {
                f.compute_effect_sizes = val.as_bool().ok_or_else(|| String::from("`compute_effect_sizes` must be boolean"))?;
                true
            }
            ("mv_group_column" | "edit.two_group.mv_group_column", AnalysisKind::TwoGroupComparison(t)) => {
                t.mv_group_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_response_column" | "edit.two_group.mv_response_column", AnalysisKind::TwoGroupComparison(t)) => {
                t.mv_response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("three_way" | "edit.factorial.three_way", AnalysisKind::FactorialAnova(f)) => {
                f.three_way = val.as_bool().ok_or_else(|| String::from("`three_way` must be boolean"))?;
                if f.three_way {
                    f.max_interaction_order = f.max_interaction_order.max(3);
                } else {
                    f.max_interaction_order = f.max_interaction_order.min(2);
                }
                true
            }
            ("repeated_measures" | "edit.factorial.repeated_measures", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`repeated_measures` must be a string"))?;
                f.repeated_measures = parse_rm_design_str(raw)?;
                true
            }
            ("mixed_effects_reml" | "edit.rm.mixed_effects_reml", AnalysisKind::OneWayAnova(a)) => {
                let b = val.as_bool().ok_or_else(|| String::from("`mixed_effects_reml` must be boolean"))?;
                a.mixed_effects_reml = b;
                if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(a.multiplicity.method) {
                    a.multiplicity.method = MultiplicityMethod::Bonferroni;
                }
                true
            }
            ("mixed_effects_reml" | "edit.rm.mixed_effects_reml", AnalysisKind::FactorialAnova(f)) => {
                let b = val.as_bool().ok_or_else(|| String::from("`mixed_effects_reml` must be boolean"))?;
                f.mixed_effects_reml = b;
                if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(f.multiplicity.method) {
                    f.multiplicity.method = MultiplicityMethod::Bonferroni;
                }
                true
            }
            ("df_method" | "edit.rm.df_method", AnalysisKind::OneWayAnova(a)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`df_method` must be a string"))?;
                a.df_method = parse_df_method_str(raw)?;
                true
            }
            ("df_method" | "edit.rm.df_method", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`df_method` must be a string"))?;
                f.df_method = parse_df_method_str(raw)?;
                true
            }
            ("df_method" | "edit.rm.df_method", AnalysisKind::NestedComparison(n)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`df_method` must be a string"))?;
                n.df_method = parse_df_method_str(raw)?;
                true
            }
            ("residual_covariance" | "edit.rm.residual_covariance" | "edit.reml.residual_covariance", AnalysisKind::OneWayAnova(a)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`residual_covariance` must be a string"))?;
                a.residual_covariance = parse_residual_covariance_str(raw)?;
                true
            }
            (
                "residual_covariance" | "factorial_residual_covariance" | "edit.factorial.residual_covariance" | "edit.reml.residual_covariance",
                AnalysisKind::FactorialAnova(f),
            ) => {
                let raw = val.as_str().ok_or_else(|| String::from("`residual_covariance` must be a string"))?;
                f.residual_covariance = parse_residual_covariance_str(raw)?;
                true
            }
            ("ss_type" | "sum_of_squares" | "edit.factorial.ss_type", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`ss_type` must be a string"))?;
                f.ss_type = parse_ss_type_str(raw)?;
                true
            }
            ("max_interaction_order" | "edit.factorial.max_interaction_order", AnalysisKind::FactorialAnova(f)) => {
                f.max_interaction_order = parse_usize_min(val, key, 1)?;
                true
            }
            ("fit_reml_on_missing" | "edit.factorial.fit_reml_on_missing", AnalysisKind::FactorialAnova(f)) => {
                f.fit_reml_on_missing = val.as_bool().ok_or_else(|| String::from("`fit_reml_on_missing` must be boolean"))?;
                true
            }
            ("multifactor_columns" | "edit.factorial.multifactor_columns", AnalysisKind::FactorialAnova(f)) => {
                f.multifactor_columns = parse_usize_list(val, key)?;
                true
            }
            ("factorial_multiplicity" | "edit.factorial.multiplicity", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`factorial.multiplicity` must be a string"))?;
                f.multiplicity.method = parse_multiplicity_method_name(raw, self.control_column)?;
                true
            }
            ("response_column" | "factorial_response_column" | "edit.factorial.response_column", AnalysisKind::FactorialAnova(f)) => {
                f.response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            (
                "distance" | "cluster_distance" | "edit.cluster.distance",
                AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, .. }),
            ) => {
                let raw = val.as_str().ok_or_else(|| String::from("`distance` must be a string"))?;
                *distance = parse_cluster_distance_str(raw)?;
                true
            }
            ("linkage" | "cluster_linkage" | "edit.cluster.linkage", AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage, .. })) => {
                let raw = val.as_str().ok_or_else(|| String::from("`linkage` must be a string"))?;
                *linkage = parse_cluster_linkage_str(raw)?;
                true
            }
            ("k_means_k" | "k" | "edit.cluster.k_means_k", AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, .. })) => {
                *k_means_k = if val.is_null() { None } else { Some(parse_usize_min(val, key, 1)?) };
                true
            }
            ("k_means_auto" | "edit.cluster.k_means_auto", AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, .. })) => {
                let auto = val.as_bool().ok_or_else(|| String::from("`k_means_auto` must be boolean"))?;
                *k_means_k = if auto { None } else { Some(k_means_k.unwrap_or(3)) };
                true
            }
            ("k_sweep_max" | "edit.cluster.k_sweep_max", AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_sweep_max, .. })) => {
                *k_sweep_max = if val.is_null() { None } else { Some(parse_usize_min(val, key, 2)?) };
                true
            }
            ("standardize" | "edit.pca.standardize", AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize, .. })) => {
                *standardize = val.as_bool().ok_or_else(|| String::from("`standardize` must be boolean"))?;
                true
            }
            (
                "parallel_analysis_seed" | "edit.pca.parallel_analysis_seed",
                AnalysisKind::Multivariate(MultivariateSpec::Pca { parallel_analysis_seed, .. }),
            ) => {
                *parallel_analysis_seed = if val.is_null() { None } else { Some(parse_u64_val(val, key)?) };
                true
            }
            (
                "spearman_rank" | "edit.corr.spearman_rank" | "edit.correlation.spearman_rank",
                AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank }),
            ) => {
                *spearman_rank = val.as_bool().ok_or_else(|| String::from("`spearman_rank` must be boolean"))?;
                true
            }
            (
                "imputations_m" | "m" | "edit.mi.imputations_m" | "edit.imputation.m",
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, .. }),
            ) => {
                *imputations_m = parse_usize_min(val, key, 1)?;
                true
            }
            ("max_iters" | "edit.mi.max_iters", AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { max_iters, .. })) => {
                *max_iters = parse_usize_min(val, key, 1)?;
                true
            }
            ("seed" | "edit.mi.seed", AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { seed, .. })) => {
                *seed = parse_u64_val(val, key)?;
                true
            }
            (
                "response_column" | "edit.mi.response_column",
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, .. }),
            ) => {
                *response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            (
                "predictor_columns" | "edit.mi.predictor_columns",
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { predictor_columns, .. }),
            ) => {
                *predictor_columns = parse_usize_list(val, key)?;
                true
            }
            ("family" | "glm_family" | "edit.glm.family", AnalysisKind::GeneralizedLinearModel(glm)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`family` must be a string"))?;
                glm.family = parse_glm_family_str(raw)?;
                true
            }
            ("response_column" | "edit.glm.response_column", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.response_column = parse_usize_min(val, key, 0)?;
                true
            }
            ("predictor_columns" | "edit.glm.predictor_columns", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.predictor_columns = parse_usize_list(val, key)?;
                true
            }
            ("reference_levels" | "edit.glm.reference_levels", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.reference_levels = parse_reference_levels(val)?;
                true
            }
            ("compute_vif" | "edit.glm.compute_vif", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.compute_vif = val.as_bool().ok_or_else(|| String::from("`compute_vif` must be boolean"))?;
                true
            }
            ("time_column" | "edit.cox.time_column", AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, .. })) => {
                *time_column = parse_usize_min(val, key, 0)?;
                true
            }
            (
                "status_column" | "edit.cox.status_column",
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { status_column, .. }),
            ) => {
                *status_column = parse_usize_min(val, key, 0)?;
                true
            }
            (
                "predictor_columns" | "edit.cox.predictor_columns",
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }),
            ) => {
                *predictor_columns = parse_usize_list(val, key)?;
                true
            }
            (
                "reference_levels" | "edit.cox.reference_levels",
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, .. }),
            ) => {
                *reference_levels = parse_reference_levels(val)?;
                true
            }
            ("tie_method" | "edit.cox.tie_method", AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. })) => {
                let raw = val.as_str().ok_or_else(|| String::from("`tie_method` must be a string"))?;
                *tie_method = parse_cox_tie_method_str(raw)?;
                true
            }
            (
                "log_rank_mantel_cox" | "edit.km.log_rank_mantel_cox",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, .. }),
            ) => {
                *log_rank_mantel_cox = val.as_bool().ok_or_else(|| String::from("`log_rank_mantel_cox` must be boolean"))?;
                true
            }
            (
                "gehan_breslow_wilcoxon" | "edit.km.gehan_breslow_wilcoxon",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { gehan_breslow_wilcoxon, .. }),
            ) => {
                *gehan_breslow_wilcoxon = val.as_bool().ok_or_else(|| String::from("`gehan_breslow_wilcoxon` must be boolean"))?;
                true
            }
            (
                "trend_test" | "edit.km.trend_test" | "log_rank_for_trend" | "edit.km.log_rank_for_trend",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { trend_test, .. }),
            ) => {
                *trend_test = val.as_bool().ok_or_else(|| String::from("`trend_test` must be boolean"))?;
                true
            }
            (
                "multiplicity_method" | "edit.km.multiplicity_method",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }),
            ) => {
                let raw = val.as_str().ok_or_else(|| String::from("`multiplicity_method` must be a string"))?;
                multiplicity.method = parse_multiplicity_method_name(raw, self.control_column)?;
                true
            }
            (
                "asymmetrical_greenwood_ci" | "edit.km.asymmetrical_greenwood_ci",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { asymmetrical_greenwood_ci, .. }),
            ) => {
                *asymmetrical_greenwood_ci = val.as_bool().ok_or_else(|| String::from("`asymmetrical_greenwood_ci` must be boolean"))?;
                true
            }
            ("mv_time_column" | "edit.km.mv_time_column", AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, .. })) => {
                *mv_time_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_status_column" | "edit.km.mv_status_column", AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_status_column, .. })) => {
                *mv_status_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_group_column" | "edit.km.mv_group_column", AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_group_column, .. })) => {
                *mv_group_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("equation_id" | "edit.fit.equation_id" | "edit.fit.model_id", AnalysisKind::NonlinearRegression(fit)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`equation_id` must be a string"))?.trim();
                if !raw.is_empty() {
                    fit.equation_id = raw.to_owned();
                }
                true
            }
            ("custom_dsl" | "edit.fit.custom_dsl", AnalysisKind::NonlinearRegression(fit)) => {
                fit.custom_dsl = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned);
                true
            }
            ("weighting" | "fit_weighting" | "edit.fit.weighting", AnalysisKind::NonlinearRegression(fit)) => {
                fit.weighting = parse_weighting_scheme(val)?;
                true
            }
            ("fit_method" | "edit.fit.method" | "method", AnalysisKind::NonlinearRegression(fit)) => {
                fit.method = parse_fit_method(val)?;
                true
            }
            ("rout_q" | "fdr_q_percent" | "q_percent" | "edit.fit.rout_q", AnalysisKind::NonlinearRegression(fit)) => {
                let q = parse_outlier_fdr_q_percent(val)?;
                let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut fit.method else {
                    return Err(String::from("`rout_q` requires ROUT outlier elimination fit method"));
                };
                *fdr_q_percent = q;
                true
            }
            ("parameter_rules" | "edit.fit.parameter_rules", AnalysisKind::NonlinearRegression(fit)) => {
                fit.parameter_rules = parse_parameter_rules(val)?;
                true
            }
            ("profile_likelihood_ci" | "edit.fit.profile_likelihood_ci", AnalysisKind::NonlinearRegression(fit)) => {
                fit.profile_likelihood_ci = val.as_bool().ok_or_else(|| String::from("`profile_likelihood_ci` must be boolean"))?;
                true
            }
            ("confidence_bands" | "edit.fit.confidence_bands", AnalysisKind::NonlinearRegression(fit)) => {
                fit.confidence_bands = val.as_bool().ok_or_else(|| String::from("`confidence_bands` must be boolean"))?;
                true
            }
            ("prediction_bands" | "edit.fit.prediction_bands", AnalysisKind::NonlinearRegression(fit)) => {
                fit.prediction_bands = val.as_bool().ok_or_else(|| String::from("`prediction_bands` must be boolean"))?;
                true
            }
            ("homoscedasticity_test" | "edit.fit.homoscedasticity_test", AnalysisKind::NonlinearRegression(fit)) => {
                fit.homoscedasticity_test = val.as_bool().ok_or_else(|| String::from("`homoscedasticity_test` must be boolean"))?;
                true
            }
            ("emit_residuals_tab" | "edit.fit.emit_residuals_tab", AnalysisKind::NonlinearRegression(fit)) => {
                fit.emit_residuals_tab = val.as_bool().ok_or_else(|| String::from("`emit_residuals_tab` must be boolean"))?;
                true
            }
            ("emit_covariance_tab" | "edit.fit.emit_covariance_tab", AnalysisKind::NonlinearRegression(fit)) => {
                fit.emit_covariance_tab = val.as_bool().ok_or_else(|| String::from("`emit_covariance_tab` must be boolean"))?;
                true
            }
            ("mv_x_columns" | "edit.fit.mv_x_columns", AnalysisKind::NonlinearRegression(fit)) => {
                fit.mv_x_columns = parse_usize_list(val, key)?;
                true
            }
            ("mv_y_column" | "edit.fit.mv_y_column", AnalysisKind::NonlinearRegression(fit)) => {
                fit.mv_y_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_group_columns" | "edit.fit.mv_group_columns", AnalysisKind::NonlinearRegression(fit)) => {
                fit.mv_group_columns = parse_usize_list(val, key)?;
                true
            }
            ("sample_size" | "edit.sample.sample_size", AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { sample_size, .. })) => {
                *sample_size = parse_usize_min(val, key, 1)?;
                true
            }
            ("seed" | "edit.sample.seed", AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, .. })) => {
                *seed = parse_u64_val(val, key)?;
                true
            }
            (
                "stratify_column" | "edit.sample.stratify_column",
                AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { stratify_column, .. }),
            ) => {
                *stratify_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            (
                "proportional_allocation" | "edit.sample.proportional_allocation",
                AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { proportional_allocation, .. }),
            ) => {
                *proportional_allocation = val.as_bool().ok_or_else(|| String::from("`proportional_allocation` must be boolean"))?;
                true
            }
            _ => {
                if let Some(col_raw) = key.strip_prefix("edit.two_group.group.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::TwoGroupComparison(t) = &mut updated
                {
                    t.mv_group_column = Some(col);
                    true
                } else if let Some(col_raw) = key.strip_prefix("edit.two_group.resp.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::TwoGroupComparison(t) = &mut updated
                {
                    t.mv_response_column = Some(col);
                    true
                } else if let Some(col_raw) = key.strip_prefix("edit.fac.resp.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::FactorialAnova(f) = &mut updated
                {
                    f.response_column = Some(col);
                    true
                } else if let Some(col_raw) = key.strip_prefix("edit.fac.col.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::FactorialAnova(f) = &mut updated
                {
                    let want_on = val.as_bool().unwrap_or_else(|| !f.multifactor_columns.contains(&col));
                    if want_on {
                        if !f.multifactor_columns.contains(&col) {
                            f.multifactor_columns.push(col);
                            f.multifactor_columns.sort_unstable();
                        }
                    } else {
                        f.multifactor_columns.retain(|&c| c != col);
                    }
                    true
                } else {
                    false
                }
            }
        };
        if handled {
            spec.update_kind(updated);
            self.sync_rule_flags_from_kind();
        }
        Ok(handled)
    }

    /// Effective displayed title reflecting auto-title vs custom title rules.
    #[must_use]
    pub fn preview_title(&self) -> Option<&str> {
        self.draft.as_ref().map(AnalysisSpec::effective_title)
    }

    /// Emit declarative `analysis.updateParams` intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.result_sheet_id.as_ref()?;
        let spec = self.draft.as_ref()?;
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "kind": spec.kind,
            "confidence_percent": spec.confidence_level.percent(),
            "selected_data_sets": spec.selected_data_sets,
            "p_value_format": spec.p_value_format,
            "title": spec.custom_title,
        });
        if let Some(map) = payload.as_object_mut() {
            if matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. })) {
                map.insert(String::from("correlation_all_pairs"), json!(self.correlation_all_pairs));
                map.insert(String::from("correlation_heatmap"), json!(self.correlation_heatmap));
            }
            if let Some(outcome_col) = self.pcr_outcome_column {
                map.insert(String::from("pcr_outcome_column"), json!(outcome_col));
            }
        }
        Some(ToolCommandIntent::new("analysis.updateParams", payload))
    }

    /// Emit `analysis.updateParams` and, when `correlation_heatmap` is toggled on for a correlation matrix,
    /// spawn a linked `DiagnosticPlotKind::Heatmap` `GraphSheet` if none already points to `result_sheet_id`.
    #[must_use]
    pub fn commit_intents(&self, doc: Option<&ProjectDocument>) -> Vec<ToolCommandIntent> {
        let Some(primary) = self.commit_intent() else {
            return Vec::new();
        };
        let mut intents = vec![primary];
        if let (Some(id), Some(spec)) = (&self.result_sheet_id, &self.draft)
            && self.correlation_heatmap
            && self.correlation_all_pairs
            && matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
        {
            let already_has_heatmap = doc.is_some_and(|d| {
                d.graph_sheets
                    .iter()
                    .any(|g| g.spec.primary_source_sheet_id == *id && g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))
            });
            if !already_has_heatmap {
                intents.push(ToolCommandIntent::new(
                    "graph.create",
                    json!({
                        "data_sheet_id": id.as_str(),
                        "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
                        "initialized": true,
                    }),
                ));
            }
        }
        intents
    }
}

fn draft_forces_reml_mode(spec: &AnalysisSpec) -> bool {
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => {
            matches!(a.method, OneWayMethod::RepeatedMeasures { .. })
                && (a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured)
        }
        AnalysisKind::FactorialAnova(f) => {
            f.repeated_measures != RepeatedMeasuresDesign::None
                && (f.mixed_effects_reml || f.residual_covariance == ResidualCovariance::Unstructured || f.three_way)
        }
        _ => false,
    }
}

fn parse_df_method_str(raw: &str) -> Result<MixedModelDfMethod, String> {
    let norm = norm_key(raw);
    match norm.as_str() {
        "kenwardroger" | "kr" => Ok(MixedModelDfMethod::KenwardRoger),
        "satterthwaite" | "sat" | "faicornelius" => Ok(MixedModelDfMethod::Satterthwaite),
        _ => Err(format!("unknown REML df method `{raw}`; expected `kenward_roger` or `satterthwaite`")),
    }
}

fn parse_multiplicity_method_name(raw: &str, control_col: usize) -> Result<MultiplicityMethod, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "none" => Ok(MultiplicityMethod::None),
        "tukey" | "tukeyhsd" => Ok(MultiplicityMethod::TukeyHsd),
        "dunnett" => Ok(MultiplicityMethod::Dunnett { control_column: control_col }),
        "sidak" => Ok(MultiplicityMethod::Sidak),
        "bonferroni" => Ok(MultiplicityMethod::Bonferroni),
        "holmsidak" => Ok(MultiplicityMethod::HolmSidak),
        "dunnettt3" => Ok(MultiplicityMethod::DunnettT3),
        "gameshowell" => Ok(MultiplicityMethod::GamesHowell),
        "tamhanet2" | "tamhane" => Ok(MultiplicityMethod::TamhaneT2),
        "dunn" | "dunnranksum" => Ok(MultiplicityMethod::DunnRankSum),
        "lineartrend" => Ok(MultiplicityMethod::LinearTrend),
        "benjaminihochbergfdr" | "bh" | "fdr" => Ok(MultiplicityMethod::BenjaminiHochbergFdr),
        "benjaminiyekutielifdr" | "by" => Ok(MultiplicityMethod::BenjaminiYekutieliFdr),
        "twostagebkyfdr" | "bky" => Ok(MultiplicityMethod::TwoStageBkyFdr),
        "newmankeuls" | "snk" => Ok(MultiplicityMethod::NewmanKeuls),
        "fisherlsd" | "lsd" => Ok(MultiplicityMethod::FisherLsd),
        _ => Err(format!("unknown multiplicity method `{raw}`")),
    }
}

fn post_hoc_method_for_widget_key(key: &str, control_col: usize) -> Option<MultiplicityMethod> {
    let suffix = key.strip_prefix("edit.post_hoc.").or_else(|| key.strip_prefix("edit.posthoc."))?;
    parse_multiplicity_method_name(suffix, control_col).ok()
}

fn parse_contingency_method_name(raw: &str) -> Result<ContingencyMethod, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "chisquare" | "pearsonchisquare" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
        "yates" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: true }),
        "likelihoodratiog2" | "g2" | "likelihoodratio" => Ok(ContingencyMethod::LikelihoodRatioG2),
        "fisher" | "fisherexact" => Ok(ContingencyMethod::FisherExact),
        "barnard" | "barnardexact" => Ok(ContingencyMethod::BarnardExact),
        "cochranarmitage" | "cochranarmitagetrend" | "trend" => Ok(ContingencyMethod::CochranArmitageTrend),
        "mcnemar" | "mcnemarpaired" => Ok(ContingencyMethod::McNemarPaired),
        "cohenkappa" | "kappa" | "unweightedkappa" => Ok(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Unweighted }),
        "linearkappa" => Ok(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Linear }),
        "quadratickappa" => Ok(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Quadratic }),
        "fleisskappa" => Ok(ContingencyMethod::FleissKappa),
        "exactbinomial" | "binomial" => Ok(ContingencyMethod::ExactBinomial),
        _ => Err(format!("unknown contingency test method `{raw}`")),
    }
}

fn parse_dialog_num_with_locale(val: &Value, field: &str, preset: &LocalePreset) -> Result<f64, String> {
    parse_finite_f64_with_locale(val, field, preset)
}

pub(crate) fn parse_table_kind_str(raw: &str) -> Result<TableKind, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "column" | "columnar" => Ok(TableKind::Column),
        "grouped" => Ok(TableKind::Grouped),
        "xy" => Ok(TableKind::Xy),
        "multiplevariables" | "multivariate" | "mv" => Ok(TableKind::MultipleVariables),
        "contingency" => Ok(TableKind::Contingency),
        "survival" => Ok(TableKind::Survival),
        "partsofwhole" | "parts" => Ok(TableKind::PartsOfWhole),
        "nested" => Ok(TableKind::Nested),
        _ => Err(format!("unknown table archetype `{raw}`")),
    }
}

fn parse_subcolumn_aggregation_val(val: &Value) -> Result<SubcolumnAggregation, String> {
    if let Some(idx) = val.as_u64() {
        return match idx {
            0 => Ok(SubcolumnAggregation::RowMeans),
            1 => Ok(SubcolumnAggregation::PerSubcolumn),
            2 => Ok(SubcolumnAggregation::PooledGrandSample),
            _ => Err(format!("unknown subcolumn aggregation index `{idx}`")),
        };
    }
    let raw = val.as_str().ok_or_else(|| String::from("`subcolumn_agg` must be a string or index"))?;
    match norm_key(raw).as_str() {
        "rowmeans" | "mean" | "means" => Ok(SubcolumnAggregation::RowMeans),
        "persubcolumn" | "eachreplicate" | "subcolumns" => Ok(SubcolumnAggregation::PerSubcolumn),
        "pooledgrandsample" | "pooled" => Ok(SubcolumnAggregation::PooledGrandSample),
        _ => Err(format!("unknown subcolumn aggregation `{raw}`")),
    }
}

fn parse_baseline_operation_str(raw: &str) -> Result<BaselineOperation, String> {
    let norm = norm_key(raw);
    match norm.as_str() {
        "difference" | "subtract" => Ok(BaselineOperation::Difference),
        "ratio" | "divide" => Ok(BaselineOperation::Ratio),
        "percentchange" | "pctchange" | "percent" => Ok(BaselineOperation::PercentChange),
        "fractionalchange" | "fraction" => Ok(BaselineOperation::FractionalChange),
        _ => Err(format!("unknown baseline operation `{raw}`")),
    }
}

fn parse_grouped_arrangement_str(raw: &str) -> Result<GroupedArrangement, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "interleaved" | "sidebyside" => Ok(GroupedArrangement::Interleaved),
        "stacked" => Ok(GroupedArrangement::Stacked),
        "superimposed" | "overlaid" | "overlay" => Ok(GroupedArrangement::Superimposed),
        _ => Err(format!("unknown grouped_arrangement `{raw}`")),
    }
}

fn parse_graph_kind_str(raw: &str) -> Result<GraphKind, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "xy" | "scatter" | "scatterline" => Ok(GraphKind::Xy),
        "column" | "col" => Ok(GraphKind::Column),
        "grouped" => Ok(GraphKind::Grouped),
        "contingency" => Ok(GraphKind::Contingency),
        "partsofwhole" | "parts" | "donut" | "pie" => Ok(GraphKind::PartsOfWhole),
        "survival" | "km" | "kaplanmeier" => Ok(GraphKind::Survival),
        "multiplevariables" | "multivariate" | "bubble" => Ok(GraphKind::MultipleVariables),
        _ => Err(format!("unknown graph_kind `{raw}`")),
    }
}

fn parse_palette_id_str(raw: &str) -> Result<PaletteId, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "okabeito" | "default" => Ok(PaletteId::OkabeIto),
        "paultolbright" | "bright" => Ok(PaletteId::PaulTolBright),
        "paultolmuted" | "muted" => Ok(PaletteId::PaulTolMuted),
        "paultolhighcontrast" | "highcontrast" => Ok(PaletteId::PaulTolHighContrast),
        "paultolvibrant" | "vibrant" => Ok(PaletteId::PaulTolVibrant),
        "monochromeprint" | "monochrome" | "grayscale" | "greyscale" => Ok(PaletteId::MonochromePrint),
        _ => Err(format!("unknown palette `{raw}`")),
    }
}

fn parse_frame_preset_str(raw: &str) -> Result<FramePreset, String> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "openl" | "lframe" => Ok(FramePreset::OpenL),
        "offsetrangetufte" | "offset" | "tufte" => Ok(FramePreset::OffsetRangeTufte),
        "plainboxed" | "boxed" | "plain" => Ok(FramePreset::PlainBoxed),
        "dualysecondaryright" | "dualy" => Ok(FramePreset::DualYSecondaryRight),
        "origincrossing" | "origin" => Ok(FramePreset::OriginCrossing),
        _ => Err(format!("unknown frame_preset `{raw}`")),
    }
}

pub(crate) fn parse_math_preset_opt(val: &Value) -> Result<Option<MathTransformPreset>, String> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(raw) = val.as_str() {
        return parse_math_preset_str(raw);
    }
    if val.is_object() {
        let preset: MathTransformPreset = serde_json::from_value(val.clone()).map_err(|e| format!("invalid math transform preset: {e}"))?;
        return Ok(Some(preset));
    }
    Err(String::from("math transform preset must be a string, preset object, or null"))
}

fn parse_math_preset_str(raw: &str) -> Result<Option<MathTransformPreset>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    if matches!(compact.as_str(), "none" | "identity" | "null" | "x" | "y") {
        return Ok(None);
    }
    let norm: String = trimmed.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let preset = match (compact.as_str(), norm.as_str()) {
        (_, "log10" | "log10x" | "log10y" | "ylog10y" | "ylog10x" | "xlog10x") => MathTransformPreset::Log10,
        (_, "ln" | "lnx" | "lny" | "loge" | "ylny" | "ylnx" | "xlnx") => MathTransformPreset::Ln,
        (_, "log2" | "log2x" | "log2y" | "ylog2y" | "ylog2x" | "xlog2x") => MathTransformPreset::Log2,
        ("10^x" | "10^y" | "10**x" | "10**y", _) | (_, "exp10" | "exp10x" | "exp10y" | "10powx" | "10powy") => MathTransformPreset::Exp10,
        ("e^x" | "e^y", _) | (_, "exp" | "expx" | "expy" | "epowx" | "epowy") => MathTransformPreset::Exp,
        ("1/x" | "1/y", _) | (_, "reciprocal" | "reciprocalx" | "reciprocaly") => MathTransformPreset::Reciprocal,
        (_, "sqrt" | "sqrtx" | "sqrty") => MathTransformPreset::Sqrt,
        ("x^2" | "y^2" | "x*x" | "y*y" | "x**2" | "y**2", _) | (_, "square" | "squarex" | "squarey") => MathTransformPreset::Square,
        (_, "logit" | "logitx" | "logity") => MathTransformPreset::Logit,
        (_, "probit" | "probitx" | "probity") => MathTransformPreset::Probit,
        (_, "arcsinsqrt" | "arcsinsqrtx" | "arcsinsqrty") => MathTransformPreset::ArcsinSqrt,
        _ => return Err(format!("unknown math transform preset `{raw}`")),
    };
    Ok(Some(preset))
}

fn parse_error_prop_value(val: &Value) -> Result<ErrorPropagationMode, String> {
    let raw = val.as_str().ok_or_else(|| String::from("`error_propagation` must be a string"))?;
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "asymmetric" | "replicateendpoints" | "endpoints" => Ok(ErrorPropagationMode::ReplicateEndpoints),
        "delta" | "deltamethod" | "firstorderdelta" | "firstorderdeltamethod" => Ok(ErrorPropagationMode::FirstOrderDeltaMethod),
        _ => Err(format!("unknown error propagation mode `{raw}`")),
    }
}

fn apply_transform_formula_str(raw: &str, x_slot: &mut Option<MathTransformPreset>, y_slot: &mut Option<MathTransformPreset>) -> Result<(), String> {
    for part in raw.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((lhs, rhs)) = part.split_once('=') {
            let axis = lhs.trim().to_ascii_lowercase();
            let parsed = parse_math_preset_str(rhs)?;
            match axis.as_str() {
                "x" | "x_transform" => *x_slot = parsed,
                "y" | "y_transform" => *y_slot = parsed,
                _ => return Err(format!("unsupported transform formula target `{lhs}`")),
            }
        } else {
            *y_slot = parse_math_preset_str(part)?;
        }
    }
    Ok(())
}

/// Candidate table listed in `Change Data Analyzed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetargetCandidate {
    pub sheet_id: SheetId,
    pub title: String,
    pub table_kind: TableKind,
    pub compatible: bool,
    pub incompatible_reason: Option<String>,
}

/// Cross-table source retargeting state (`analysis.retargetData`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangeDataAnalyzedState {
    pub result_sheet_id: Option<SheetId>,
    pub current_source_id: Option<SheetId>,
    pub selected_target_id: Option<SheetId>,
    pub selected_data_sets: Vec<usize>,
    pub candidates: Vec<RetargetCandidate>,
}

impl ChangeDataAnalyzedState {
    /// Populate compatible and incompatible `DataSheet` candidates for `result`.
    pub fn open_for_result(&mut self, result: &ResultSheet, doc: &ProjectDocument) {
        self.result_sheet_id = Some(result.id.clone());
        self.current_source_id = Some(result.analysis.source_sheet_id.clone());
        self.selected_data_sets = result.analysis.selected_data_sets.clone();
        self.candidates = doc
            .data_sheets
            .iter()
            .map(|ds| {
                let compatible = result.analysis.kind.is_compatible_with(ds.kind());
                let incompatible_reason =
                    (!compatible).then(|| format!("{} cannot run on {} tables", result.analysis.effective_title(), ds.kind().label()));
                RetargetCandidate { sheet_id: ds.id.clone(), title: ds.title.clone(), table_kind: ds.kind(), compatible, incompatible_reason }
            })
            .collect();
        self.selected_target_id = self
            .candidates
            .iter()
            .find(|c| c.compatible && Some(&c.sheet_id) != self.current_source_id.as_ref())
            .or_else(|| self.candidates.iter().find(|c| c.compatible))
            .map(|c| c.sheet_id.clone());
    }

    /// Select a candidate target sheet id. Returns `Err` if incompatible.
    pub fn select_target(&mut self, target_sheet_id: &SheetId) -> Result<(), String> {
        let Some(candidate) = self.candidates.iter().find(|c| &c.sheet_id == target_sheet_id) else {
            return Err(format!("unknown target sheet {}", target_sheet_id.as_str()));
        };
        if !candidate.compatible {
            return Err(candidate.incompatible_reason.clone().unwrap_or_else(|| String::from("incompatible table archetype")));
        }
        self.selected_target_id = Some(candidate.sheet_id.clone());
        Ok(())
    }

    /// Emit `analysis.retargetData` only when the selected target sheet is compatible.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let res_id = self.result_sheet_id.as_ref()?;
        let target = self.selected_target_id.as_ref()?;
        let compatible = self.candidates.iter().any(|c| &c.sheet_id == target && c.compatible);
        if !compatible {
            return None;
        }
        Some(ToolCommandIntent::new(
            "analysis.retargetData",
            json!({
                "sheet_id": res_id.as_str(),
                "source_sheet_id": target.as_str(),
                "selected_data_sets": self.selected_data_sets,
            }),
        ))
    }
}

// ============================================================================
// Multi-tab ResultSheet, Narrative Checklist, and Constant Hook Picker
// ============================================================================

/// Filter applied to plain-language assumption [`ChecklistItem`] diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChecklistSeverityFilter {
    #[default]
    All,
    WarningsAndViolations,
}

/// Interactive view state for a multi-tab [`ResultSheet`] and its narrative [`ChecklistItem`] pane.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResultSheetViewState {
    pub result_sheet_id: Option<SheetId>,
    pub active_tab_index: usize,
    pub checklist_expanded: bool,
    pub checklist_filter: ChecklistSeverityFilter,
    #[serde(default = "default_report_format")]
    pub report_format: String,
    #[serde(default)]
    pub last_methods_report: Option<String>,
}

fn default_report_format() -> String {
    String::from("markdown")
}

impl ResultSheetViewState {
    /// Synchronize active tab bounds against `result`.
    pub fn sync_with_result(&mut self, result: &ResultSheet) {
        if self.result_sheet_id.as_ref() != Some(&result.id) {
            self.result_sheet_id = Some(result.id.clone());
            self.active_tab_index = 0;
            self.checklist_expanded = !result.checklist.is_empty();
        }
        if result.tabs.is_empty() {
            self.active_tab_index = 0;
        } else if self.active_tab_index >= result.tabs.len() {
            self.active_tab_index = result.tabs.len() - 1;
        }
    }

    /// Select tab by zero-based index, clamped within `tab_count`.
    pub fn select_tab(&mut self, index: usize, tab_count: usize) {
        self.active_tab_index = if tab_count == 0 { 0 } else { index.min(tab_count - 1) };
    }

    /// Select a [`ResultTab`] by its stable `id`.
    pub fn select_tab_by_id(&mut self, result: &ResultSheet, tab_id: &str) -> bool {
        if let Some(pos) = result.tabs.iter().position(|t| t.id == tab_id) {
            self.active_tab_index = pos;
            true
        } else {
            false
        }
    }

    /// Currently active [`ResultTab`], if any.
    #[must_use]
    pub fn active_tab<'a>(&self, result: &'a ResultSheet) -> Option<&'a ResultTab> {
        result.tabs.get(self.active_tab_index).or_else(|| result.tabs.first())
    }

    /// Assumption diagnostics filtered by [`Self::checklist_filter`].
    #[must_use]
    pub fn visible_checklist<'a>(&self, result: &'a ResultSheet) -> Vec<&'a ChecklistItem> {
        result
            .checklist
            .iter()
            .filter(|item| match self.checklist_filter {
                ChecklistSeverityFilter::All => true,
                ChecklistSeverityFilter::WarningsAndViolations => {
                    matches!(item.status, ChecklistStatus::Warn | ChecklistStatus::Violated)
                }
            })
            .collect()
    }

    /// Return `(pass, warn, violated, info)` counts across `result.checklist`.
    #[must_use]
    pub fn checklist_summary_counts(result: &ResultSheet) -> (usize, usize, usize, usize) {
        let mut pass = 0usize;
        let mut warn = 0usize;
        let mut violated = 0usize;
        let mut info = 0usize;
        for item in &result.checklist {
            match item.status {
                ChecklistStatus::Pass => pass += 1,
                ChecklistStatus::Warn => warn += 1,
                ChecklistStatus::Violated => violated += 1,
                ChecklistStatus::Info => info += 1,
            }
        }
        (pass, warn, violated, info)
    }

    /// Build a `result.generateReport` intent for `result_sheet_id` (`QOL-6`).
    #[must_use]
    pub fn generate_report_intent(&self, result_sheet_id: &SheetId) -> ToolCommandIntent {
        let fmt = if self.report_format.trim().is_empty() { "markdown" } else { self.report_format.trim() };
        ToolCommandIntent::new("result.generateReport", json!({ "result_sheet_id": result_sheet_id.as_str(), "format": fmt }))
    }

    /// Format a typed [`ResultCell`] cleanly without panicking on `CiBound::Open` or non-finite values.
    #[must_use]
    pub fn format_result_cell(cell: &ResultCell, p_format: PValueFormat) -> String {
        match cell {
            ResultCell::Empty => String::new(),
            ResultCell::Number { value, prefix, units } => {
                let num = if value.is_finite() { format_compact_f64(*value) } else { String::from("—") };
                let pfx = prefix.as_deref().unwrap_or("");
                match units.as_deref() {
                    Some(u) if !u.is_empty() => format!("{pfx}{num} {u}"),
                    _ => format!("{pfx}{num}"),
                }
            }
            ResultCell::ConfidenceInterval { lower, upper } => {
                let lo = format_ci_bound(*lower);
                let hi = format_ci_bound(*upper);
                format!("[{lo}, {hi}]")
            }
            ResultCell::PValue { raw, formatted, stars } => {
                let p_text = if raw.is_finite() { p_format.format_p(*raw) } else { formatted.clone() };
                if stars.is_empty() { p_text } else { format!("{p_text} {stars}") }
            }
            ResultCell::Text(s) => s.clone(),
            ResultCell::Boolean(b) => String::from(if *b { "Yes" } else { "No" }),
        }
    }
}

fn format_ci_bound(bound: CiBound) -> String {
    match bound {
        CiBound::Finite(v) if v.is_finite() => format_compact_f64(v),
        CiBound::Finite(_) | CiBound::Open => String::from("Unbounded"),
    }
}

pub(crate) fn format_compact_f64(v: f64) -> String {
    if !v.is_finite() {
        return String::from("NaN");
    }
    let abs = v.abs();
    if abs != 0.0 && !(1e-3..1e5).contains(&abs) {
        format!("{v:.3e}")
    } else if (v - v.round()).abs() < 1e-9 && abs < 1e12 {
        format!("{:.0}", v.round())
    } else {
        format!("{v:.4}")
    }
}

/// Hookable upstream value surfaced by [`ConstantHookPickerState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableHookOption {
    pub label: String,
    pub preview_value: String,
    pub template_token: String,
    pub source: HookSource,
}

/// Target sheet category for [`ConstantHookPickerState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HookTargetKind {
    #[default]
    AnalysisParameter,
    GraphTextBox,
    LayoutTextCallout,
}

/// Dialog picker inserting live `InfoConstant`, `ResultScalar`, and `ColumnTitleValue` hooks.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConstantHookPickerState {
    pub target_sheet_id: Option<SheetId>,
    pub target_parameter_name: Option<String>,
    #[serde(default)]
    pub target_kind: HookTargetKind,
    #[serde(default)]
    pub existing_parameter_rules: Vec<ParameterRule>,
    pub search_query: String,
    pub options: Vec<AvailableHookOption>,
    pub selected_index: Option<usize>,
}

impl ConstantHookPickerState {
    /// Scan all `InfoSheet`, `ResultSheet`, and numeric data-set column titles in `doc`.
    pub fn open_for_document(&mut self, target_sheet_id: SheetId, parameter_name: Option<String>, doc: &ProjectDocument) {
        self.existing_parameter_rules.clear();
        if let Some(res) = doc.result_sheet(&target_sheet_id) {
            self.target_kind = HookTargetKind::AnalysisParameter;
            if let AnalysisKind::NonlinearRegression(nl) = &res.analysis.kind {
                self.existing_parameter_rules = nl.parameter_rules.clone();
            }
        } else if doc.graph_sheet(&target_sheet_id).is_some() {
            self.target_kind = HookTargetKind::GraphTextBox;
        } else if doc.layout_sheet(&target_sheet_id).is_some() {
            self.target_kind = HookTargetKind::LayoutTextCallout;
        } else {
            self.target_kind = HookTargetKind::AnalysisParameter;
        }
        self.target_sheet_id = Some(target_sheet_id.clone());
        self.target_parameter_name = parameter_name;
        self.search_query.clear();
        self.options.clear();
        let numeric_only = self.target_kind == HookTargetKind::AnalysisParameter;
        collect_info_hooks(doc, numeric_only, &mut self.options);
        collect_result_hooks(doc, Some(&target_sheet_id), &mut self.options);
        collect_column_title_hooks(doc, &mut self.options);
        self.selected_index = (!self.options.is_empty()).then_some(0);
    }

    /// Options matching `search_query`.
    #[must_use]
    pub fn filtered_options(&self) -> Vec<&AvailableHookOption> {
        let q = self.search_query.trim().to_ascii_lowercase();
        self.options
            .iter()
            .filter(|opt| q.is_empty() || opt.label.to_ascii_lowercase().contains(&q) || opt.template_token.to_ascii_lowercase().contains(&q))
            .collect()
    }

    /// Return the currently selected [`AvailableHookOption`].
    #[must_use]
    pub fn selected_option(&self) -> Option<&AvailableHookOption> {
        self.selected_index.and_then(|idx| self.options.get(idx))
    }

    /// Emit `analysis.updateParams`, `graph.addAnnotation`, or `layout.addPanel` binding the chosen
    /// [`HookSource`] or template token onto `target_sheet_id`.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let target = self.target_sheet_id.as_ref()?;
        let opt = self.selected_option()?;
        match self.target_kind {
            HookTargetKind::AnalysisParameter => {
                let param_name = self.target_parameter_name.clone().unwrap_or_else(|| String::from("Bottom"));
                let mut rules: Vec<ParameterRule> = self.existing_parameter_rules.iter().filter(|r| r.name != param_name).cloned().collect();
                rules.push(ParameterRule {
                    name: param_name,
                    scope: ParameterScope::HookedConstant(opt.source.clone()),
                    initial_override: None,
                    lower_bound: None,
                    upper_bound: None,
                });
                Some(ToolCommandIntent::new(
                    "analysis.updateParams",
                    json!({
                        "sheet_id": target.as_str(),
                        "parameter_rules": rules,
                    }),
                ))
            }
            HookTargetKind::GraphTextBox => Some(ToolCommandIntent::new(
                "graph.addAnnotation",
                json!({
                    "sheet_id": target.as_str(),
                    "kind": "text",
                    "text": opt.template_token,
                }),
            )),
            HookTargetKind::LayoutTextCallout => Some(ToolCommandIntent::new(
                "layout.addPanel",
                json!({
                    "sheet_id": target.as_str(),
                    "kind": "text",
                    "markdown": opt.template_token,
                }),
            )),
        }
    }
}

fn collect_info_hooks(doc: &ProjectDocument, numeric_only: bool, out: &mut Vec<AvailableHookOption>) {
    let mut deferred_empty = Vec::new();
    for info in &doc.info_sheets {
        for c in &info.constants {
            if numeric_only && c.value.as_finite_f64().is_none() {
                continue;
            }
            let preview = c.value.display_text();
            let is_empty = preview.is_empty();
            let shown_preview = if is_empty { String::from("<empty>") } else { preview };
            let opt = AvailableHookOption {
                label: format!("Info [{}]: {}", info.title, c.name),
                preview_value: shown_preview,
                template_token: format!("{{{{info:{}:{}}}}}", info.id.as_str(), c.name),
                source: HookSource::InfoConstant { info_sheet_id: Some(info.id.clone()), constant_name: c.name.clone() },
            };
            if is_empty {
                deferred_empty.push(opt);
            } else {
                out.push(opt);
            }
        }
    }
    out.extend(deferred_empty);
}

fn collect_result_hooks(doc: &ProjectDocument, exclude_sheet_id: Option<&SheetId>, out: &mut Vec<AvailableHookOption>) {
    for res in &doc.result_sheets {
        if exclude_sheet_id.is_some_and(|ex| ex == &res.id) {
            continue;
        }
        for scalar in &res.scalars {
            if !scalar.value.is_finite() {
                continue;
            }
            let ds_suffix = scalar.data_set_index.map_or(String::new(), |ds| format!(" [ds {ds}]"));
            let token = match scalar.data_set_index {
                Some(ds) => format!("{{{{result:{}:{ds}:{}}}}}", res.id.as_str(), scalar.key),
                None => format!("{{{{result:{}:{}}}}}", res.id.as_str(), scalar.key),
            };
            out.push(AvailableHookOption {
                label: format!("Result [{}]: {}{ds_suffix}", res.title(), scalar.key),
                preview_value: format_compact_f64(scalar.value),
                template_token: token,
                source: HookSource::ResultScalar {
                    result_sheet_id: res.id.clone(),
                    parameter_key: scalar.key.clone(),
                    data_set_index: scalar.data_set_index,
                },
            });
        }
    }
}

fn collect_column_title_hooks(doc: &ProjectDocument, out: &mut Vec<AvailableHookOption>) {
    for ds in &doc.data_sheets {
        for col_idx in 0..ds.archetype.column_count() {
            let Some(title) = data_column_title(ds, col_idx) else {
                continue;
            };
            if title.trim().parse::<f64>().ok().is_some_and(f64::is_finite) {
                out.push(AvailableHookOption {
                    label: format!("Column Title [{}]: Col {} = {title}", ds.title, curvebook_doc::default_column_label(col_idx)),
                    preview_value: title.to_owned(),
                    template_token: format!("{{{{col:{}:{col_idx}}}}}", ds.id.as_str()),
                    source: HookSource::ColumnTitleValue { data_sheet_id: ds.id.clone(), data_set_index: col_idx },
                });
            }
        }
    }
}

pub(crate) fn data_column_title(sheet: &DataSheet, col_idx: usize) -> Option<&str> {
    match &sheet.archetype {
        TableArchetype::Xy(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col_idx).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(col_idx).map(|g| g.title.as_str()),
    }
}

// ============================================================================
// Change Graph Type Gallery, Format Graph, Format Axes, Brackets, Bubble, Clone Style, Layout
// ============================================================================

/// Thumbnail glyph kind rendered in the first-visit `Change Graph Type` chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThumbnailGlyphKind {
    Scatter,
    ScatterLine,
    BarReplicates,
    BeforeAfter,
    BoxWhisker,
    Violin,
    AlignedDot,
    GroupedInterleaved,
    StackedBar,
    KaplanMeier,
    Donut,
    BubbleScatter,
    Heatmap,
    Volcano,
    BlandAltman,
    RocCurve,
    ForestPlot,
    ResidualPlot,
    NormalQqPlot,
}

/// Entry in the `Change Graph Type` schematic thumbnail chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphTypeOption {
    pub id: &'static str,
    pub label: &'static str,
    pub subtitle: &'static str,
    pub graph_kind: GraphKind,
    pub thumbnail: ThumbnailGlyphKind,
    pub compatible_tables: &'static [TableKind],
}

pub const GRAPH_TYPE_OPTIONS: &[GraphTypeOption] = &[
    GraphTypeOption {
        id: "xy.scatter",
        label: "XY Scatter (Points Only)",
        subtitle: "Individual replicate markers or Mean +/- SD error bars",
        graph_kind: GraphKind::Xy,
        thumbnail: ThumbnailGlyphKind::Scatter,
        compatible_tables: &[TableKind::Xy],
    },
    GraphTypeOption {
        id: "xy.scatter_line",
        label: "XY Points + Connecting Lines",
        subtitle: "Point-to-point or spline curve connecting series points",
        graph_kind: GraphKind::Xy,
        thumbnail: ThumbnailGlyphKind::ScatterLine,
        compatible_tables: &[TableKind::Xy],
    },
    GraphTypeOption {
        id: "col.bar_replicates",
        label: "Bar with Individual Points",
        subtitle: "Mean/Median bar + deterministic beeswarm replicate overlay",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::BarReplicates,
        compatible_tables: &[TableKind::Column, TableKind::Nested],
    },
    GraphTypeOption {
        id: "col.before_after",
        label: "Before-After Paired Lines",
        subtitle: "Connect matched subjects across paired conditions",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::BeforeAfter,
        compatible_tables: &[TableKind::Column],
    },
    GraphTypeOption {
        id: "col.box_whisker",
        label: "Box and Whiskers (Tukey / Min-Max)",
        subtitle: "Median, IQR box, Tukey whiskers, and outlier points",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::BoxWhisker,
        compatible_tables: &[TableKind::Column, TableKind::Grouped],
    },
    GraphTypeOption {
        id: "col.violin",
        label: "Violin Plot",
        subtitle: "Kernel density contour clipped to observed data range",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::Violin,
        compatible_tables: &[TableKind::Column, TableKind::Grouped],
    },
    GraphTypeOption {
        id: "col.raincloud",
        label: "Raincloud Plot (Half-Violin + Points)",
        subtitle: "Half-violin kernel density with aligned individual replicate points",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::Violin,
        compatible_tables: &[TableKind::Column, TableKind::Grouped],
    },
    GraphTypeOption {
        id: "col.aligned_dot",
        label: "Scatter Dot Plot + Crossbar",
        subtitle: "Beeswarm or aligned points with Mean/Median summary line",
        graph_kind: GraphKind::Column,
        thumbnail: ThumbnailGlyphKind::AlignedDot,
        compatible_tables: &[TableKind::Column, TableKind::Nested],
    },
    GraphTypeOption {
        id: "grouped.interleaved",
        label: "Grouped Interleaved Bars",
        subtitle: "Clustered bars across Row Factor levels with error bars",
        graph_kind: GraphKind::Grouped,
        thumbnail: ThumbnailGlyphKind::GroupedInterleaved,
        compatible_tables: &[TableKind::Grouped, TableKind::Contingency],
    },
    GraphTypeOption {
        id: "grouped.stacked",
        label: "Grouped / Contingency Stacked",
        subtitle: "Stacked component bars across categorical rows",
        graph_kind: GraphKind::Grouped,
        thumbnail: ThumbnailGlyphKind::StackedBar,
        compatible_tables: &[TableKind::Grouped, TableKind::Contingency],
    },
    GraphTypeOption {
        id: "grouped.superimposed",
        label: "Grouped Superimposed",
        subtitle: "Overlaid series across Row Factor levels for direct comparison",
        graph_kind: GraphKind::Grouped,
        thumbnail: ThumbnailGlyphKind::GroupedInterleaved,
        compatible_tables: &[TableKind::Grouped],
    },
    GraphTypeOption {
        id: "survival.km",
        label: "Kaplan-Meier Survival Curve",
        subtitle: "Step survival function with censoring ticks & Greenwood CIs",
        graph_kind: GraphKind::Survival,
        thumbnail: ThumbnailGlyphKind::KaplanMeier,
        compatible_tables: &[TableKind::Survival],
    },
    GraphTypeOption {
        id: "parts.donut",
        label: "Donut / Pie Parts of Whole",
        subtitle: "Proportional slices or 10x10 waffle grid with center total",
        graph_kind: GraphKind::PartsOfWhole,
        thumbnail: ThumbnailGlyphKind::Donut,
        compatible_tables: &[TableKind::PartsOfWhole],
    },
    GraphTypeOption {
        id: "mv.bubble",
        label: "Bubble Plot (5-Channel Multivariate)",
        subtitle: "Map X, Y, Color (categorical/continuous), Size, and Label",
        graph_kind: GraphKind::MultipleVariables,
        thumbnail: ThumbnailGlyphKind::BubbleScatter,
        compatible_tables: &[TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.heatmap",
        label: "Heatmap (2D Matrix)",
        subtitle: "Perceptual Viridis / Cividis / Diverging tile grid",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
        thumbnail: ThumbnailGlyphKind::Heatmap,
        compatible_tables: &[TableKind::Grouped, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.volcano",
        label: "Volcano Plot",
        subtitle: "Log2 fold change vs -log10(q) with dual threshold cutoffs",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::Volcano),
        thumbnail: ThumbnailGlyphKind::Volcano,
        compatible_tables: &[TableKind::Xy, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.bland_altman",
        label: "Bland-Altman Method Comparison",
        subtitle: "Mean vs Difference with bias and +/- 1.96 SD limits",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman),
        thumbnail: ThumbnailGlyphKind::BlandAltman,
        compatible_tables: &[TableKind::Column, TableKind::Xy],
    },
    GraphTypeOption {
        id: "diag.roc",
        label: "ROC Curve Diagnostic",
        subtitle: "Sensitivity vs 1-Specificity with diagonal identity line",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve),
        thumbnail: ThumbnailGlyphKind::RocCurve,
        compatible_tables: &[TableKind::Column, TableKind::Xy],
    },
    GraphTypeOption {
        id: "diag.forest",
        label: "Forest Plot (Effects & 95% CIs)",
        subtitle: "Horizontal point-estimate and confidence-interval whiskers",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot),
        thumbnail: ThumbnailGlyphKind::ForestPlot,
        compatible_tables: &[TableKind::Grouped, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.residual",
        label: "Residual Diagnostic Plot",
        subtitle: "Fitted/predicted vs residual scatter diagnostic",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot),
        thumbnail: ThumbnailGlyphKind::ResidualPlot,
        compatible_tables: &[TableKind::Xy, TableKind::Column, TableKind::Grouped, TableKind::MultipleVariables],
    },
    GraphTypeOption {
        id: "diag.normal_qq",
        label: "Normal Q-Q Plot",
        subtitle: "Theoretical normal quantiles vs standardized residuals",
        graph_kind: GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot),
        thumbnail: ThumbnailGlyphKind::NormalQqPlot,
        compatible_tables: &[TableKind::Xy, TableKind::Column, TableKind::Grouped, TableKind::MultipleVariables],
    },
];

/// First-visit `Change Graph Type` chooser state (`docs/implementation/graphing-and-layouts.md` §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeGraphTypeState {
    pub graph_sheet_id: Option<SheetId>,
    pub source_table_kind: Option<TableKind>,
    pub column_design_hint: Option<ColumnDesignHint>,
    pub selected_option_id: String,
    pub summary_measure: SummaryMeasure,
    pub error_bar_metric: ErrorBarMetric,
}

impl Default for ChangeGraphTypeState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            source_table_kind: None,
            column_design_hint: None,
            selected_option_id: String::from("xy.scatter"),
            summary_measure: SummaryMeasure::Mean,
            error_bar_metric: ErrorBarMetric::Sd,
        }
    }
}

impl ChangeGraphTypeState {
    fn resolved_graph_kind(&self, opt: &GraphTypeOption, doc: Option<&ProjectDocument>) -> GraphKind {
        if opt.id == "grouped.stacked" {
            let is_contingency = self.source_table_kind == Some(TableKind::Contingency)
                || self.graph_sheet_id.as_ref().and_then(|gid| doc?.graph_sheet(gid)).is_some_and(|gs| {
                    doc.and_then(|d| d.data_sheet(&gs.spec.primary_source_sheet_id)).is_some_and(|ds| ds.kind() == TableKind::Contingency)
                });
            if is_contingency {
                return GraphKind::Contingency;
            }
            return GraphKind::Grouped;
        }
        opt.graph_kind
    }

    /// Synchronize chooser state when opening or first visiting `graph`.
    pub fn open_for_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        let src = doc.data_sheet(&graph.spec.primary_source_sheet_id);
        self.source_table_kind = src.map(DataSheet::kind);
        self.column_design_hint = src.and_then(|ds| match &ds.archetype {
            TableArchetype::Column(c) => Some(c.design_hint),
            _ => None,
        });

        let recommended = self.recommended_options();
        if let Some(first) = recommended.first() {
            self.selected_option_id = String::from(first.id);
        }
    }

    /// Options tailored to the active `TableArchetype` and `ColumnDesignHint` (`PairedRepeatedMeasures`
    /// prioritizes `Before-After Paired Lines` first).
    #[must_use]
    pub fn recommended_options(&self) -> Vec<&'static GraphTypeOption> {
        let mut out: Vec<&'static GraphTypeOption> = match self.source_table_kind {
            Some(kind) => GRAPH_TYPE_OPTIONS.iter().filter(|opt| opt.compatible_tables.contains(&kind)).collect(),
            None => GRAPH_TYPE_OPTIONS.iter().collect(),
        };
        if self.column_design_hint == Some(ColumnDesignHint::PairedRepeatedMeasures) {
            out.sort_by_key(|opt| usize::from(opt.id != "col.before_after"));
        }
        out
    }

    /// Apply the selected thumbnail choice onto `spec` cleanly.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) -> bool {
        let Some(opt) = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id) else {
            return false;
        };
        spec.kind = self.resolved_graph_kind(opt, None);
        if opt.id == "grouped.stacked" {
            spec.grouped_arrangement = GroupedArrangement::Stacked;
        } else if opt.id == "grouped.superimposed" {
            spec.grouped_arrangement = GroupedArrangement::Superimposed;
        } else if opt.id == "grouped.interleaved" {
            spec.grouped_arrangement = GroupedArrangement::Interleaved;
        }
        for (idx, series) in spec.series.iter_mut().enumerate() {
            let color = spec.palette.color_at(idx);
            let marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: color,
                stroke_color: color,
                stroke_width_pt: 1.0,
            };
            series.gdo = build_gdo_for_option(opt.id, color, marker, self.summary_measure);
            if opt.id == "grouped.stacked" {
                series.y_error_bars = None;
            } else if matches!(series.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::ScatterAndLine { .. }) {
                series.y_error_bars = Some(ErrorBarSpec { metric: self.error_bar_metric, ..ErrorBarSpec::default() });
            } else {
                series.y_error_bars = None;
            }
        }
        true
    }

    /// Emit `graph.setKind` (and mark sheet initialized) command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let opt = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id)?;
        let kind = self.resolved_graph_kind(opt, None);
        Some(ToolCommandIntent::new(
            "graph.setKind",
            json!({
                "sheet_id": id.as_str(),
                "option_id": opt.id,
                "kind": kind,
                "reset_series_style": true,
            }),
        ))
    }

    /// Emit `graph.setSpec` + `graph.setSeries` command intents applying full `option_id` layout,
    /// `grouped_arrangement`, GDO style, and error-bar rules cleanly via `Caller::Ui`.
    #[must_use]
    pub fn commit_intents(&self, doc: &ProjectDocument) -> Vec<ToolCommandIntent> {
        let Some(id) = self.graph_sheet_id.as_ref() else {
            return Vec::new();
        };
        let Some(opt) = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id) else {
            return Vec::new();
        };
        let Some(mut spec) = doc.graph_sheet(id).map(|g| g.spec.clone()) else {
            return self.commit_intent().into_iter().collect();
        };
        let _ = self.apply_to_spec(&mut spec);
        spec.kind = self.resolved_graph_kind(opt, Some(doc));

        let mut intents = vec![ToolCommandIntent::new(
            "graph.setKind",
            json!({
                "sheet_id": id.as_str(),
                "option_id": opt.id,
                "kind": spec.kind,
                "reset_series_style": true,
            }),
        )];
        if spec.kind == GraphKind::Grouped {
            intents.push(ToolCommandIntent::new(
                "graph.setSpec",
                json!({
                    "sheet_id": id.as_str(),
                    "grouped_arrangement": spec.grouped_arrangement,
                }),
            ));
        }
        if !matches!(spec.kind, GraphKind::Diagnostic(_)) && !spec.series.is_empty() {
            let series_patches: Vec<Value> = spec
                .series
                .iter()
                .map(|s| {
                    json!({
                        "gdo": s.gdo,
                        "y_error_bars": s.y_error_bars,
                    })
                })
                .collect();
            intents.push(ToolCommandIntent::new(
                "graph.setSeries",
                json!({
                    "sheet_id": id.as_str(),
                    "series": series_patches,
                }),
            ));
        }
        intents
    }
}

fn build_gdo_for_option(option_id: &str, color: Rgba8, marker: MarkerStyle, summary: SummaryMeasure) -> GraphicalDataObject {
    match option_id {
        "xy.scatter_line" => GraphicalDataObject::ScatterAndLine {
            marker: Some(marker),
            line: Some(ConnectingLineStyle { kind: LineConnectionKind::PointToPoint, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5, color }),
            area_fill: None,
        },
        "col.before_after" => GraphicalDataObject::BeforeAfterLines { marker: Some(marker), line_color: color, stroke_width_pt: 1.2 },
        "col.box_whisker" => GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::Tukey1_5Iqr,
            fill_color: color.with_alpha(190),
            stroke_color: color,
            show_outliers: true,
            show_all_points: false,
        },
        "col.violin" => GraphicalDataObject::Violin {
            fill_color: color.with_alpha(190),
            stroke_color: color,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        },
        "col.raincloud" => GraphicalDataObject::Violin {
            fill_color: color.with_alpha(190),
            stroke_color: color,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: true,
        },
        "col.aligned_dot" => GraphicalDataObject::AlignedDotPlot { summary, marker, show_crossbar: true },
        "col.bar_replicates" | "grouped.interleaved" | "grouped.stacked" | "grouped.superimposed" => GraphicalDataObject::BarWithReplicates {
            summary,
            fill_color: color.with_alpha(210),
            border_color: color,
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        },
        "survival.km" => GraphicalDataObject::KaplanMeierStep {
            line_color: color,
            stroke_width_pt: 1.5,
            show_censored_ticks: true,
            show_greenwood_ci: false,
            ci_dashed: true,
        },
        "parts.donut" => GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Donut, show_center_total: true },
        _ => GraphicalDataObject::ScatterAndLine { marker: Some(marker), line: None, area_fill: None },
    }
}

/// Non-modal `Format Graph` inspector state (`palette`, `CVD` preview, frame, error bars, markers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatGraphState {
    pub graph_sheet_id: Option<SheetId>,
    pub palette: PaletteId,
    pub cvd_preview: Option<CvdKind>,
    pub frame_preset: FramePreset,
    pub grouped_arrangement: GroupedArrangement,
    #[serde(default)]
    pub graph_kind: Option<GraphKind>,
    pub selected_series_index: usize,
    pub error_bar_enabled: bool,
    pub error_bar_spec: ErrorBarSpec,
    pub marker_size_pt: f64,
    pub marker_fill_mode: MarkerFillMode,
    pub line_dash: LineDashStyle,
    pub show_legend: bool,
    pub legend_bordered: bool,
    #[serde(default)]
    pub half_violin_with_points: bool,
    #[serde(default)]
    pub initial_violin_options: Option<(bool, bool, bool)>,
    #[serde(default)]
    pub model_equation_mode: Option<EquationDisplayMode>,
    #[serde(default)]
    pub model_equation_annotations: Vec<GraphAnnotation>,
    #[serde(default)]
    pub annotations_dirty: bool,
    #[serde(default)]
    pub mv_encoding: Option<MultivariateGraphEncoding>,
    #[serde(default)]
    pub mv_levels: Vec<String>,
    #[serde(default)]
    pub mv_level_is_x: bool,
    #[serde(default)]
    pub mv_selected_level: Option<String>,
    #[serde(default)]
    pub mv_levels_dirty: bool,
}

impl Default for FormatGraphState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            palette: PaletteId::OkabeIto,
            cvd_preview: None,
            frame_preset: FramePreset::OpenL,
            grouped_arrangement: GroupedArrangement::Interleaved,
            graph_kind: None,
            selected_series_index: 0,
            error_bar_enabled: true,
            error_bar_spec: ErrorBarSpec::default(),
            marker_size_pt: 5.0,
            marker_fill_mode: MarkerFillMode::Filled,
            line_dash: LineDashStyle::Solid,
            show_legend: true,
            legend_bordered: false,
            half_violin_with_points: false,
            initial_violin_options: None,
            model_equation_mode: None,
            model_equation_annotations: Vec::new(),
            annotations_dirty: false,
            mv_encoding: None,
            mv_levels: Vec::new(),
            mv_level_is_x: true,
            mv_selected_level: None,
            mv_levels_dirty: false,
        }
    }
}

impl FormatGraphState {
    /// Populate draft controls from an existing [`GraphSheet`], clearing MV level state unless populated by [`Self::sync_from_graph_with_doc`].
    pub fn sync_from_graph(&mut self, graph: &GraphSheet) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.palette = graph.spec.palette;
        self.frame_preset = graph.spec.frame_preset;
        self.grouped_arrangement = graph.spec.grouped_arrangement;
        self.graph_kind = Some(graph.spec.kind);
        self.show_legend = graph.spec.legend.visible;
        self.legend_bordered = graph.spec.legend.bordered;
        self.model_equation_annotations.clone_from(&graph.spec.annotations);
        self.model_equation_mode = graph.spec.annotations.iter().find_map(|ann| match ann {
            GraphAnnotation::ModelEquation { mode, .. } => Some(*mode),
            _ => None,
        });
        self.annotations_dirty = false;
        self.mv_encoding.clone_from(&graph.spec.multivariate);
        self.mv_levels.clear();
        self.mv_level_is_x = true;
        self.mv_levels_dirty = false;
        let series_idx = self.selected_series_index.min(graph.spec.series.len().saturating_sub(1));
        self.selected_series_index = series_idx;
        if let Some(series) = graph.spec.series.get(series_idx) {
            self.error_bar_enabled = series.y_error_bars.is_some();
            self.error_bar_spec = series.y_error_bars.unwrap_or_default();
            if let GraphicalDataObject::Violin { clip_to_observed_range, show_median_and_quartiles, half_violin_with_points, .. } = series.gdo {
                self.half_violin_with_points = half_violin_with_points;
                self.initial_violin_options = Some((clip_to_observed_range, show_median_and_quartiles, half_violin_with_points));
            } else {
                self.half_violin_with_points = false;
                self.initial_violin_options = None;
            }
            self.extract_series_marker_and_dash(&series.gdo);
        }
    }

    /// Populate draft controls plus any categorical `MultipleVariables` level ordering from `doc`.
    pub fn sync_from_graph_with_doc(&mut self, graph: &GraphSheet, doc: Option<&ProjectDocument>) {
        self.sync_from_graph(graph);
        let Some(doc) = doc else {
            return;
        };
        let Some(enc) = &graph.spec.multivariate else {
            return;
        };
        let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id) else {
            return;
        };
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
            return;
        };
        let x_cat = mv
            .variables
            .get(enc.x_column)
            .filter(|col| col.role == VariableRole::Categorical || matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        if let Some(col) = x_cat {
            let lvls: Vec<String> =
                col.ordered_levels_with(enc.x_level_order_override.as_ref().unwrap_or(&col.level_order)).into_iter().map(|l| l.level).collect();
            if !lvls.is_empty() {
                self.mv_levels = lvls;
                self.mv_level_is_x = true;
                self.mv_selected_level = crate::views::mv_formula::resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref())
                    .and_then(|i| self.mv_levels.get(i).cloned());
                return;
            }
        }
        if let ColorChannelMapping::Categorical { column, level_order_override, .. } = &enc.color
            && let Some(col) = mv.variables.get(*column)
        {
            let lvls: Vec<String> =
                col.ordered_levels_with(level_order_override.as_ref().unwrap_or(&col.level_order)).into_iter().map(|l| l.level).collect();
            if !lvls.is_empty() {
                self.mv_levels = lvls;
                self.mv_level_is_x = false;
                self.mv_selected_level = crate::views::mv_formula::resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref())
                    .and_then(|i| self.mv_levels.get(i).cloned());
            }
        }
    }

    /// Cycle selected categorical level inside the `Format Graph` dialog.
    pub fn cycle_selected_mv_level(&mut self) {
        if self.mv_levels.is_empty() {
            return;
        }
        let cur = crate::views::mv_formula::resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref()).unwrap_or(0);
        let next = (cur + 1) % self.mv_levels.len();
        self.mv_selected_level = self.mv_levels.get(next).cloned();
    }

    /// Stepwise reorder the selected MV level (`"top"`, `"up"`, `"down"`, `"end"`) into `Explicit` order.
    pub fn reorder_mv_level(&mut self, op_str: &str) -> Result<bool, String> {
        use crate::views::mv_formula::{LevelMoveOp, apply_explicit_level_move};
        if self.mv_levels.is_empty() {
            return Err(String::from("Graph has no categorical levels to reorder."));
        }
        let op = match op_str {
            "top" | "move_to_top" => LevelMoveOp::Top,
            "up" | "move_up" => LevelMoveOp::Up,
            "down" | "move_down" => LevelMoveOp::Down,
            "end" | "move_to_end" => LevelMoveOp::End,
            other => return Err(format!("unknown level move op `{other}`")),
        };
        let moved = apply_explicit_level_move(&mut self.mv_levels, &mut self.mv_selected_level, op);
        let explicit_spec = LevelOrderSpec { order: LevelOrderKind::Explicit(self.mv_levels.clone()), reversed: false };
        if let Some(enc) = &mut self.mv_encoding {
            if self.mv_level_is_x {
                enc.x_level_order_override = Some(explicit_spec);
            } else if let ColorChannelMapping::Categorical { level_order_override, .. } = &mut enc.color {
                *level_order_override = Some(explicit_spec);
            }
            self.mv_levels_dirty = true;
        }
        Ok(moved)
    }

    fn extract_series_marker_and_dash(&mut self, gdo: &GraphicalDataObject) {
        match gdo {
            GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
                if let Some(m) = marker {
                    self.marker_size_pt = m.size_pt;
                    self.marker_fill_mode = m.fill_mode;
                }
                if let Some(l) = line {
                    self.line_dash = l.dash_style;
                }
            }
            GraphicalDataObject::AlignedDotPlot { marker, .. } => {
                self.marker_size_pt = marker.size_pt;
                self.marker_fill_mode = marker.fill_mode;
            }
            GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => {
                self.marker_size_pt = m.size_pt;
                self.marker_fill_mode = m.fill_mode;
            }
            _ => {}
        }
    }

    /// Swatch for series `index`, optionally passed through CVD simulation (`Deuteranopia`, etc.).
    #[must_use]
    pub fn preview_swatch(&self, index: usize) -> Rgba8 {
        let base = self.palette.color_at(index);
        match self.cvd_preview {
            Some(kind) => simulate_cvd(base, kind, 1.0),
            None => base,
        }
    }

    /// Apply current inspector state onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.palette = self.palette;
        spec.frame_preset = self.frame_preset;
        spec.grouped_arrangement = self.grouped_arrangement;
        if let Some(kind) = self.graph_kind {
            spec.kind = kind;
        }
        spec.legend = LegendSpec { visible: self.show_legend, bordered: self.legend_bordered, ..spec.legend };
        if self.annotations_dirty {
            spec.annotations.clone_from(&self.model_equation_annotations);
        }
        if self.mv_levels_dirty
            && let Some(enc) = &self.mv_encoding
        {
            spec.multivariate = Some(enc.clone());
        }
        if let Some(series) = spec.series.get_mut(self.selected_series_index) {
            series.y_error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
            let swatch = self.palette.color_at(self.selected_series_index);
            if let GraphicalDataObject::Violin { half_violin_with_points, .. } = &mut series.gdo {
                *half_violin_with_points = self.half_violin_with_points;
            }
            apply_marker_and_dash(&mut series.gdo, swatch, self.marker_size_pt, self.marker_fill_mode, self.line_dash);
        }
    }

    /// Patch `FormatGraph` fields from scripted GUI actions / inspector text inputs.
    ///
    /// # Errors
    /// Returns an error if a supplied enum variant or numeric value is invalid.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply field patches using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "show_legend" | "format_graph.show_legend" => {
                    self.show_legend = val.as_bool().ok_or_else(|| String::from("`show_legend` must be boolean"))?;
                }
                "legend_bordered" | "format_graph.legend_bordered" => {
                    self.legend_bordered = val.as_bool().ok_or_else(|| String::from("`legend_bordered` must be boolean"))?;
                }
                "error_bar_enabled" | "format_graph.error_bar_enabled" => {
                    self.error_bar_enabled = val.as_bool().ok_or_else(|| String::from("`error_bar_enabled` must be boolean"))?;
                }
                "marker_size_pt" | "format_graph.marker_size_pt" => {
                    let sz = parse_dialog_num_with_locale(val, key, preset)?;
                    if sz <= 0.0 {
                        return Err(String::from("`marker_size_pt` must be a finite positive number"));
                    }
                    self.marker_size_pt = sz.clamp(1.0, 24.0);
                }
                "selected_series_index" | "series_index" | "format_graph.selected_series_index" => {
                    let idx = val
                        .as_u64()
                        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
                        .ok_or_else(|| String::from("`selected_series_index` must be a non-negative integer"))?;
                    self.selected_series_index = usize::try_from(idx).unwrap_or(usize::MAX);
                }
                "grouped_arrangement" | "format_graph.grouped_arrangement" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`grouped_arrangement` must be a string"))?;
                    self.grouped_arrangement = parse_grouped_arrangement_str(raw)?;
                }
                "graph_kind" | "kind" | "format_graph.graph_kind" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`graph_kind` must be a string"))?;
                    self.graph_kind = Some(parse_graph_kind_str(raw)?);
                }
                "palette" | "format_graph.palette" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`palette` must be a string"))?;
                    self.palette = parse_palette_id_str(raw)?;
                }
                "frame_preset" | "format_graph.frame_preset" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`frame_preset` must be a string"))?;
                    self.frame_preset = parse_frame_preset_str(raw)?;
                }
                "half_violin_with_points" | "raincloud" | "format_graph.half_violin_with_points" => {
                    self.half_violin_with_points = val.as_bool().ok_or_else(|| String::from("`half_violin_with_points` must be boolean"))?;
                }
                "model_equation_mode" | "equation_mode" | "format_graph.model_equation_mode" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`model_equation_mode` must be `substituted` or `symbolic`"))?;
                    let mode = parse_equation_display_mode_str(raw)?;
                    self.set_model_equation_mode(mode);
                }
                "toggle_model_equation_mode" | "format_graph.toggle_model_equation_mode" => {
                    let next = match self.model_equation_mode.unwrap_or_default() {
                        EquationDisplayMode::Substituted => EquationDisplayMode::Symbolic,
                        EquationDisplayMode::Symbolic => EquationDisplayMode::Substituted,
                    };
                    self.set_model_equation_mode(next);
                }
                "selected_level" | "format_graph.selected_level" => {
                    let s = val.as_str().ok_or_else(|| String::from("`selected_level` must be a string"))?;
                    self.mv_selected_level = Some(s.to_owned());
                }
                "move_to_top" | "format_graph.move_to_top" => {
                    let _ = self.reorder_mv_level("top")?;
                }
                "move_up" | "format_graph.move_up" => {
                    let _ = self.reorder_mv_level("up")?;
                }
                "move_down" | "format_graph.move_down" => {
                    let _ = self.reorder_mv_level("down")?;
                }
                "move_to_end" | "format_graph.move_to_end" => {
                    let _ = self.reorder_mv_level("end")?;
                }
                other => return Err(format!("unknown Format Graph field `{other}`")),
            }
        }
        Ok(())
    }

    /// Switch all [`GraphAnnotation::ModelEquation`] annotations on this graph to `mode`.
    pub fn set_model_equation_mode(&mut self, next_mode: EquationDisplayMode) {
        self.model_equation_mode = Some(next_mode);
        for ann in &mut self.model_equation_annotations {
            if let GraphAnnotation::ModelEquation { mode, .. } = ann {
                *mode = next_mode;
                self.annotations_dirty = true;
            }
        }
    }

    /// Emit declarative `graph.setSpec` and `graph.setSeries` engine command intents.
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let Some(id) = &self.graph_sheet_id else {
            return Vec::new();
        };
        let error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
        let mut spec_patch = serde_json::Map::new();
        spec_patch.insert(String::from("sheet_id"), json!(id.as_str()));
        spec_patch.insert(String::from("palette"), json!(self.palette));
        spec_patch.insert(String::from("frame_preset"), json!(self.frame_preset));
        spec_patch.insert(String::from("grouped_arrangement"), json!(self.grouped_arrangement));
        if let Some(kind) = self.graph_kind {
            spec_patch.insert(String::from("kind"), json!(kind));
        }
        if self.annotations_dirty {
            spec_patch.insert(String::from("annotations"), json!(self.model_equation_annotations));
        }
        if self.mv_levels_dirty
            && let Some(enc) = &self.mv_encoding
        {
            spec_patch.insert(String::from("multivariate"), json!(enc));
        }
        spec_patch.insert(String::from("legend"), json!({ "visible": self.show_legend, "bordered": self.legend_bordered }));
        let mut series_obj = json!({
            "sheet_id": id.as_str(),
            "series_index": self.selected_series_index,
            "y_error_bars": error_bars,
            "marker": {
                "size_pt": self.marker_size_pt,
                "fill_mode": self.marker_fill_mode,
            },
            "connecting_line": {
                "dash_style": self.line_dash,
            },
        });
        if (self.half_violin_with_points || self.initial_violin_options.is_some_and(|(_, _, was_half)| was_half))
            && let Some(obj) = series_obj.as_object_mut()
        {
            let swatch = self.palette.color_at(self.selected_series_index);
            let (clip_to_observed_range, show_median_and_quartiles, _) = self.initial_violin_options.unwrap_or((true, true, false));
            obj.insert(
                String::from("gdo"),
                json!(GraphicalDataObject::Violin {
                    fill_color: swatch.with_alpha(190),
                    stroke_color: swatch,
                    clip_to_observed_range,
                    show_median_and_quartiles,
                    half_violin_with_points: self.half_violin_with_points,
                }),
            );
        }
        let mut intents = vec![ToolCommandIntent::new("graph.setSpec", Value::Object(spec_patch))];
        if !matches!(self.graph_kind, Some(GraphKind::MultipleVariables)) {
            intents.push(ToolCommandIntent::new("graph.setSeries", series_obj));
        }
        intents
    }
}

fn apply_marker_and_dash(gdo: &mut GraphicalDataObject, swatch: Rgba8, size_pt: f64, fill_mode: MarkerFillMode, dash: LineDashStyle) {
    let clamped_size = if size_pt.is_finite() { size_pt.clamp(1.0, 24.0) } else { 5.0 };
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.size_pt = clamped_size;
                m.fill_mode = fill_mode;
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
            if let Some(l) = line {
                l.dash_style = dash;
                l.color = swatch;
            }
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.size_pt = clamped_size;
            marker.fill_mode = fill_mode;
            marker.fill_color = swatch;
            marker.stroke_color = swatch;
        }
        GraphicalDataObject::BeforeAfterLines { marker: Some(m), line_color, .. } => {
            *line_color = swatch;
            m.size_pt = clamped_size;
            m.fill_mode = fill_mode;
        }
        _ => {}
    }
}

/// Which axis (`X`, `Y`, or optional right `Y2`) is being edited in `Format Axes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AxisTarget {
    X,
    #[default]
    Y,
    Y2,
}

/// Non-modal / dialog state for `Format Axes` (scales, ranges, 1-3 discontinuous `AxisSegment`s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatAxesState {
    pub graph_sheet_id: Option<SheetId>,
    pub target_axis: AxisTarget,
    pub enable_y2: bool,
    pub axis_draft: AxisSpec,
    #[serde(default)]
    pub base_x_thresholds: Vec<AxisThreshold>,
    #[serde(default)]
    pub base_y_thresholds: Vec<AxisThreshold>,
    #[serde(default)]
    pub pending_x_thresholds: Option<Vec<AxisThreshold>>,
    #[serde(default)]
    pub pending_y_thresholds: Option<Vec<AxisThreshold>>,
}

impl Default for FormatAxesState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            target_axis: AxisTarget::Y,
            enable_y2: false,
            axis_draft: AxisSpec::linear("Y"),
            base_x_thresholds: Vec::new(),
            base_y_thresholds: Vec::new(),
            pending_x_thresholds: None,
            pending_y_thresholds: None,
        }
    }
}

impl FormatAxesState {
    /// Load axis `target` (`X`, `Y`, or `Y2`) from `graph`.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, target: AxisTarget) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.target_axis = target;
        self.enable_y2 = graph.spec.y2_axis.is_some();
        self.base_x_thresholds = graph.spec.x_axis.thresholds.clone();
        self.base_y_thresholds = graph.spec.y_axis.thresholds.clone();
        self.pending_x_thresholds = None;
        self.pending_y_thresholds = None;
        self.axis_draft = match target {
            AxisTarget::X => graph.spec.x_axis.clone(),
            AxisTarget::Y => graph.spec.y_axis.clone(),
            AxisTarget::Y2 => graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2")),
        };
    }

    /// Return the effective X-axis reference thresholds including un-committed inspector edits.
    #[must_use]
    pub fn effective_x_thresholds(&self) -> &[AxisThreshold] {
        if self.target_axis == AxisTarget::X {
            &self.axis_draft.thresholds
        } else if let Some(ref xs) = self.pending_x_thresholds {
            xs
        } else {
            &self.base_x_thresholds
        }
    }

    /// Return the effective Y-axis reference thresholds including un-committed inspector edits.
    #[must_use]
    pub fn effective_y_thresholds(&self) -> &[AxisThreshold] {
        if self.target_axis == AxisTarget::Y {
            &self.axis_draft.thresholds
        } else if let Some(ref ys) = self.pending_y_thresholds {
            ys
        } else {
            &self.base_y_thresholds
        }
    }

    fn set_x_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::X {
            self.axis_draft.thresholds = thresholds.clone();
        }
        self.pending_x_thresholds = Some(thresholds);
    }

    fn set_y_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::Y {
            self.axis_draft.thresholds = thresholds.clone();
        }
        self.pending_y_thresholds = Some(thresholds);
    }

    fn set_active_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        self.axis_draft.thresholds = thresholds.clone();
        match self.target_axis {
            AxisTarget::X => self.pending_x_thresholds = Some(thresholds),
            AxisTarget::Y => self.pending_y_thresholds = Some(thresholds),
            AxisTarget::Y2 => {}
        }
    }

    fn append_thresholds_for_target(&mut self, target: AxisTarget, mut items: Vec<AxisThreshold>) {
        match target {
            AxisTarget::X => {
                let mut next = self.effective_x_thresholds().to_vec();
                next.append(&mut items);
                self.set_x_thresholds(next);
            }
            AxisTarget::Y => {
                let mut next = self.effective_y_thresholds().to_vec();
                next.append(&mut items);
                self.set_y_thresholds(next);
            }
            AxisTarget::Y2 => {
                self.axis_draft.thresholds.append(&mut items);
            }
        }
    }

    fn switch_target_axis_preserving_thresholds(&mut self, next: AxisTarget, doc: Option<&ProjectDocument>) {
        let saved_x = (self.target_axis == AxisTarget::X).then(|| self.axis_draft.thresholds.clone()).or_else(|| self.pending_x_thresholds.clone());
        let saved_y = (self.target_axis == AxisTarget::Y).then(|| self.axis_draft.thresholds.clone()).or_else(|| self.pending_y_thresholds.clone());
        if let (Some(d), Some(gid)) = (doc, &self.graph_sheet_id)
            && let Some(gs) = d.graph_sheet(gid)
        {
            self.sync_from_graph(gs, next);
        } else {
            self.target_axis = next;
        }
        if let Some(xs) = saved_x {
            self.set_x_thresholds(xs);
        }
        if let Some(ys) = saved_y {
            self.set_y_thresholds(ys);
        }
    }

    /// Add a discontinuous [`AxisSegment`], enforcing `1..=3` bounds. Returns `false` if already at `3`.
    pub fn add_segment(&mut self) -> bool {
        if self.axis_draft.segments.len() >= 3 {
            return false;
        }
        self.axis_draft.segments.push(AxisSegment::default());
        true
    }

    /// Remove segment `idx`, enforcing at least `1` segment.
    pub fn remove_segment(&mut self, idx: usize) -> bool {
        if self.axis_draft.segments.len() <= 1 || idx >= self.axis_draft.segments.len() {
            return false;
        }
        self.axis_draft.segments.remove(idx);
        true
    }

    /// Update scale bijection (`Linear`, `Log10`, `Log2`, `Ln`, `Logit`, `Probit`, `Reciprocal`) on segment `idx`.
    pub fn set_segment_scale(&mut self, idx: usize, scale: GraphAxisScale) -> bool {
        let Some(seg) = self.axis_draft.segments.get_mut(idx) else {
            return false;
        };
        seg.scale = scale;
        true
    }

    /// Update min/max bounds on segment `idx`, rejecting non-finite or inverted `[min >= max]` ranges.
    pub fn set_segment_bounds(&mut self, idx: usize, min: Option<f64>, max: Option<f64>) -> Result<(), String> {
        if min.is_some_and(|v| !v.is_finite()) || max.is_some_and(|v| !v.is_finite()) {
            return Err(String::from("axis bounds must be finite numbers"));
        }
        if let (Some(lo), Some(hi)) = (min, max)
            && lo >= hi
        {
            return Err(format!("axis minimum ({lo}) must be strictly less than maximum ({hi})"));
        }
        let Some(seg) = self.axis_draft.segments.get_mut(idx) else {
            return Err(format!("segment index {idx} out of range"));
        };
        seg.min = min;
        seg.max = max;
        Ok(())
    }

    /// Add a horizontal/vertical threshold reference line or shaded band.
    pub fn add_threshold(&mut self, value: f64, shade_to: Option<f64>, label: Option<String>, color: Rgba8) -> bool {
        if !value.is_finite() || shade_to.is_some_and(|v| !v.is_finite()) {
            return false;
        }
        self.axis_draft.thresholds.push(AxisThreshold { value, shade_to, color, dash_style: LineDashStyle::Dashed, stroke_width_pt: 1.0, label });
        true
    }

    /// Write the validated axis draft back onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) -> Result<(), String> {
        self.axis_draft.validate().map_err(|e| e.to_string())?;
        match self.target_axis {
            AxisTarget::X => spec.x_axis = self.axis_draft.clone(),
            AxisTarget::Y => spec.y_axis = self.axis_draft.clone(),
            AxisTarget::Y2 => {
                spec.y2_axis = self.enable_y2.then(|| self.axis_draft.clone());
                if self.enable_y2 {
                    spec.frame_preset = FramePreset::DualYSecondaryRight;
                }
            }
        }
        if self.target_axis != AxisTarget::X
            && let Some(ref xs) = self.pending_x_thresholds
        {
            spec.x_axis.thresholds = xs.clone();
        }
        if self.target_axis != AxisTarget::Y
            && let Some(ref ys) = self.pending_y_thresholds
        {
            spec.y_axis.thresholds = ys.clone();
        }
        Ok(())
    }

    /// Emit `graph.setAxes` intent carrying axis target, validated segments, break ornament, and thresholds.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        if self.axis_draft.validate().is_err() {
            return None;
        }
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
        match self.target_axis {
            AxisTarget::X => {
                payload.insert(String::from("x_axis"), serde_json::to_value(&self.axis_draft).ok()?);
            }
            AxisTarget::Y => {
                payload.insert(String::from("y_axis"), serde_json::to_value(&self.axis_draft).ok()?);
            }
            AxisTarget::Y2 => {
                if self.enable_y2 {
                    payload.insert(String::from("frame_preset"), serde_json::to_value(FramePreset::DualYSecondaryRight).ok()?);
                    payload.insert(String::from("y2_axis"), serde_json::to_value(&self.axis_draft).ok()?);
                } else {
                    payload.insert(String::from("y2_axis"), Value::Null);
                }
            }
        }
        if self.target_axis != AxisTarget::X
            && let Some(ref xs) = self.pending_x_thresholds
        {
            payload.insert(String::from("x_axis"), serde_json::json!({ "thresholds": xs }));
        }
        if self.target_axis != AxisTarget::Y
            && let Some(ref ys) = self.pending_y_thresholds
        {
            payload.insert(String::from("y_axis"), serde_json::json!({ "thresholds": ys }));
        }
        Some(ToolCommandIntent::new("graph.setAxes", Value::Object(payload)))
    }

    /// Apply modal/inspector field updates using regional numeric parsing (`preset`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, doc, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal/inspector field updates to the axis draft using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(
        &mut self,
        fields: &serde_json::Map<String, Value>,
        doc: Option<&ProjectDocument>,
        preset: &LocalePreset,
    ) -> Result<(), String> {
        if let Some(target_val) = fields.get("target_axis").or_else(|| fields.get("axes.target_axis"))
            && let Some(raw) = target_val.as_str()
        {
            let next = parse_axis_target_str(raw)?;
            self.switch_target_axis_preserving_thresholds(next, doc);
        }
        let mut pending_min = self.axis_draft.segments.first().and_then(|s| s.min);
        let mut pending_max = self.axis_draft.segments.first().and_then(|s| s.max);
        let mut bounds_changed = false;
        for (key, val) in fields {
            match key.as_str() {
                "enable_y2" | "axes.enable_y2" => {
                    self.enable_y2 = val.as_bool().ok_or_else(|| String::from("`enable_y2` must be boolean"))?;
                }
                "title" | "axis_title" | "axes.title" => {
                    self.axis_draft.title = val.as_str().ok_or_else(|| String::from("`title` must be a string"))?.to_owned();
                }
                "scale" | "axes.scale" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`scale` must be a string"))?;
                    let scale = parse_axis_scale_str(raw)?;
                    let _ = self.set_segment_scale(0, scale);
                }
                "min" | "axis_min" | "axes.min" => {
                    pending_min = parse_opt_finite_f64_with_locale(val, key, preset)?;
                    bounds_changed = true;
                }
                "max" | "axis_max" | "axes.max" => {
                    pending_max = parse_opt_finite_f64_with_locale(val, key, preset)?;
                    bounds_changed = true;
                }
                "break_ornament" | "axes.break_ornament" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`break_ornament` must be a string"))?;
                    self.axis_draft.break_ornament = parse_break_ornament_str(raw)?;
                }
                "gap_width_pt" | "axes.gap_width_pt" => {
                    self.axis_draft.gap_width_pt = parse_finite_f64_with_locale(val, key, preset)?.clamp(1.0, 48.0);
                }
                "stroke_width_pt" | "axes.stroke_width_pt" => {
                    self.axis_draft.stroke_width_pt = parse_finite_f64_with_locale(val, key, preset)?.clamp(0.2, 8.0);
                }
                "add_segment" | "axes.add_segment" if val.as_bool() == Some(true) => {
                    let _ = self.add_segment();
                }
                "remove_segment" | "axes.remove_segment" => {
                    if let Some(idx) = val.as_u64().map(|n| n as usize) {
                        let _ = self.remove_segment(idx);
                    }
                }
                "thresholds" | "axes.thresholds" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.set_active_thresholds(items);
                }
                "x_thresholds" | "axes.x_thresholds" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.set_x_thresholds(items);
                }
                "y_thresholds" | "axes.y_thresholds" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.set_y_thresholds(items);
                }
                "add_threshold" | "axes.add_threshold" => {
                    let axis = match val.get("axis") {
                        None | Some(Value::Null) => self.target_axis,
                        Some(other) => {
                            let s = other.as_str().ok_or_else(|| String::from("`axis` must be a string"))?;
                            parse_axis_target_str(s)?
                        }
                    };
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.append_thresholds_for_target(axis, items);
                }
                "add_x_threshold" | "axes.add_x_threshold" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.append_thresholds_for_target(AxisTarget::X, items);
                }
                "add_y_threshold" | "axes.add_y_threshold" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.append_thresholds_for_target(AxisTarget::Y, items);
                }
                "clear_thresholds" | "axes.clear_thresholds" if val.as_bool() == Some(true) => {
                    self.set_active_thresholds(Vec::new());
                }
                "target_axis" | "axes.target_axis" | "add_segment" | "axes.add_segment" | "clear_thresholds" | "axes.clear_thresholds" => {}
                other => return Err(format!("unknown Format Axes field `{other}`")),
            }
        }
        if bounds_changed {
            self.set_segment_bounds(0, pending_min, pending_max)?;
        }
        Ok(())
    }
}

fn parse_axis_target_str(raw: &str) -> Result<AxisTarget, String> {
    match norm_key(raw).as_str() {
        "x" => Ok(AxisTarget::X),
        "y" => Ok(AxisTarget::Y),
        "y2" => Ok(AxisTarget::Y2),
        _ => Err(format!("unknown target_axis `{raw}`")),
    }
}

fn parse_axis_thresholds_value(val: &Value, key: &str, preset: &LocalePreset) -> Result<Vec<AxisThreshold>, String> {
    match val {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => arr.iter().map(|item| parse_single_axis_threshold(item, key, preset)).collect(),
        _ => Ok(vec![parse_single_axis_threshold(val, key, preset)?]),
    }
}

fn parse_single_axis_threshold(val: &Value, key: &str, preset: &LocalePreset) -> Result<AxisThreshold, String> {
    if val.is_number() || val.is_string() {
        let num = parse_finite_f64_with_locale(val, key, preset)?;
        return Ok(AxisThreshold {
            value: num,
            shade_to: None,
            color: Rgba8::rgb(120, 120, 120),
            dash_style: LineDashStyle::Dashed,
            stroke_width_pt: 1.0,
            label: None,
        });
    }
    if let Ok(mut parsed) = serde_json::from_value::<AxisThreshold>(val.clone()) {
        if !parsed.value.is_finite() || parsed.shade_to.is_some_and(|v| !v.is_finite()) {
            return Err(format!("`{key}` threshold bounds must be finite"));
        }
        parsed.stroke_width_pt = parsed.stroke_width_pt.clamp(0.2, 8.0);
        return Ok(parsed);
    }
    let obj = val.as_object().ok_or_else(|| format!("`{key}` entries must be objects or numbers"))?;
    let val_num = obj.get("value").ok_or_else(|| format!("`{key}` threshold missing `value`"))?;
    let value = parse_finite_f64_with_locale(val_num, key, preset)?;
    let shade_to = obj.get("shade_to").map(|v| parse_opt_finite_f64_with_locale(v, "shade_to", preset)).transpose()?.flatten();
    let label = match obj.get("label") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| format!("`{key}` `label` must be a string"))?.to_owned()),
    };
    let color = match obj.get("color") {
        None | Some(Value::Null) => Rgba8::rgb(120, 120, 120),
        Some(v) => parse_threshold_color(v, key)?,
    };
    let dash_style = match obj.get("dash_style") {
        None | Some(Value::Null) => LineDashStyle::Dashed,
        Some(v) => parse_threshold_dash_style(v, key)?,
    };
    let stroke_width_pt = match obj.get("stroke_width_pt") {
        None | Some(Value::Null) => 1.0,
        Some(v) => parse_finite_f64_with_locale(v, "stroke_width_pt", preset)?.clamp(0.2, 8.0),
    };
    Ok(AxisThreshold { value, shade_to, color, dash_style, stroke_width_pt, label })
}

fn parse_threshold_color(val: &Value, key: &str) -> Result<Rgba8, String> {
    if let Ok(c) = serde_json::from_value::<Rgba8>(val.clone()) {
        return Ok(c);
    }
    if let Some(hex) = val.as_str() {
        let s = hex.trim().trim_start_matches('#');
        if s.is_ascii() && (s.len() == 6 || s.len() == 8) {
            let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))?;
            let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))?;
            let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))?;
            let a = if s.len() == 8 { u8::from_str_radix(&s[6..8], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))? } else { 255 };
            return Ok(Rgba8::rgba(r, g, b, a));
        }
    }
    Err(format!("invalid `{key}` color value"))
}

fn parse_threshold_dash_style(val: &Value, key: &str) -> Result<LineDashStyle, String> {
    if let Ok(d) = serde_json::from_value::<LineDashStyle>(val.clone()) {
        return Ok(d);
    }
    if let Some(raw) = val.as_str() {
        return match norm_key(raw).as_str() {
            "solid" => Ok(LineDashStyle::Solid),
            "dashed" | "dash" => Ok(LineDashStyle::Dashed),
            "dotted" | "dot" => Ok(LineDashStyle::Dotted),
            "dashdot" | "dash_dot" => Ok(LineDashStyle::DashDot),
            _ => Err(format!("unknown `{key}` dash_style `{raw}`")),
        };
    }
    Err(format!("`{key}` dash_style must be a string"))
}

/// Live Pairwise Comparison Bracket & Compact Letter Display inspector (`Auto-add from analysis`, `R10.11`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseComparisonsState {
    pub graph_sheet_id: Option<SheetId>,
    pub selected_result_sheet_id: Option<SheetId>,
    pub available_result_ids: Vec<SheetId>,
    pub bracket_style: BracketStyle,
    pub label_mode: BracketLabelMode,
    pub hide_non_significant: bool,
    pub brackets: Vec<PairwiseBracketSpec>,
    #[serde(default)]
    pub brackets_dirty: bool,
    #[serde(default)]
    pub had_cld_on_graph: bool,
    #[serde(default)]
    pub display_mode: PairwiseDisplayMode,
    #[serde(default)]
    pub cld_enabled: bool,
    #[serde(default)]
    pub cld_ordering: CldOrdering,
    #[serde(default)]
    pub cld_uppercase: bool,
    #[serde(default)]
    pub cld_alpha_override: Option<f64>,
    #[serde(default)]
    pub cld_font_size_pt: f64,
    #[serde(default)]
    pub cld_offset_pt: f64,
    #[serde(default)]
    pub cld_color: Rgba8,
}

impl Default for PairwiseComparisonsState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            selected_result_sheet_id: None,
            available_result_ids: Vec::new(),
            bracket_style: BracketStyle::Square,
            label_mode: BracketLabelMode::SummaryStars,
            hide_non_significant: true,
            brackets: Vec::new(),
            brackets_dirty: false,
            had_cld_on_graph: false,
            display_mode: PairwiseDisplayMode::BracketsOnly,
            cld_enabled: false,
            cld_ordering: CldOrdering::DescendingMean,
            cld_uppercase: false,
            cld_alpha_override: None,
            cld_font_size_pt: 9.5,
            cld_offset_pt: 6.0,
            cld_color: Rgba8::BLACK,
        }
    }
}

impl PairwiseComparisonsState {
    /// Synchronize brackets, CLD settings, and discover upstream [`ResultSheet`]s carrying pairwise comparisons.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.brackets = graph.spec.comparisons.clone();
        self.brackets_dirty = false;
        self.available_result_ids = doc.result_sheets.iter().filter(|rs| !rs.pairwise_comparisons.is_empty()).map(|rs| rs.id.clone()).collect();
        self.display_mode = graph.spec.pairwise_display_mode;
        if let Some(cld) = &graph.spec.compact_letter_display {
            self.had_cld_on_graph = true;
            self.cld_enabled = true;
            self.cld_ordering = cld.ordering;
            self.cld_uppercase = cld.uppercase;
            self.cld_alpha_override = cld.alpha_override;
            self.cld_font_size_pt = cld.font_size_pt;
            self.cld_offset_pt = cld.offset_pt;
            self.cld_color = cld.color;
            if self.available_result_ids.contains(&cld.result_sheet_id) {
                self.selected_result_sheet_id = Some(cld.result_sheet_id.clone());
            }
        } else {
            self.had_cld_on_graph = false;
            self.cld_enabled = false;
            self.cld_ordering = CldOrdering::DescendingMean;
            self.cld_uppercase = false;
            self.cld_alpha_override = None;
            self.cld_font_size_pt = 9.5;
            self.cld_offset_pt = 6.0;
            self.cld_color = Rgba8::BLACK;
            if let Some(first_bracket_res) = self.brackets.first().map(|b| &b.result_sheet_id)
                && self.available_result_ids.contains(first_bracket_res)
            {
                self.selected_result_sheet_id = Some(first_bracket_res.clone());
            }
        }
        if self.selected_result_sheet_id.as_ref().is_none_or(|id| !self.available_result_ids.contains(id)) {
            self.selected_result_sheet_id = self.available_result_ids.first().cloned();
        }
    }

    /// Populate brackets automatically from the selected [`ResultSheet`]'s `pairwise_comparisons`.
    pub fn auto_add_from_analysis(&mut self, doc: &ProjectDocument) -> usize {
        let Some(res_id) = &self.selected_result_sheet_id else {
            return 0;
        };
        let Some(res) = doc.result_sheet(res_id) else {
            return 0;
        };
        let mut added = 0usize;
        for cmp in &res.pairwise_comparisons {
            if self.hide_non_significant && !cmp.significant {
                continue;
            }
            let duplicate = self.brackets.iter().any(|b| {
                &b.result_sheet_id == res_id
                    && ((b.group_a == cmp.group_a && b.group_b == cmp.group_b) || (b.group_a == cmp.group_b && b.group_b == cmp.group_a))
            });
            if duplicate {
                continue;
            }
            self.brackets.push(PairwiseBracketSpec {
                result_sheet_id: res_id.clone(),
                group_a: cmp.group_a,
                group_b: cmp.group_b,
                style: self.bracket_style,
                label_mode: self.label_mode.clone(),
                hide_if_non_significant: self.hide_non_significant,
                stroke_width_pt: 1.0,
                font_size_pt: 9.0,
                bracket_step_pt: 12.0,
            });
            added += 1;
        }
        if added > 0 {
            self.brackets_dirty = true;
        }
        added
    }

    /// Toggle `cld_enabled` and synchronize `display_mode` cleanly when still in default mode.
    pub fn toggle_cld_enabled(&mut self) {
        self.cld_enabled = !self.cld_enabled;
        if self.cld_enabled {
            if self.display_mode == PairwiseDisplayMode::BracketsOnly {
                self.display_mode = if self.brackets.is_empty() {
                    PairwiseDisplayMode::CompactLetterDisplayOnly
                } else {
                    PairwiseDisplayMode::BracketsAndCompactLetterDisplay
                };
            }
        } else {
            self.display_mode = PairwiseDisplayMode::BracketsOnly;
        }
    }

    /// Cycle `pairwise_display_mode` across brackets-only, CLD-only, and both.
    pub fn cycle_display_mode(&mut self) {
        self.display_mode = match self.display_mode {
            PairwiseDisplayMode::BracketsOnly => PairwiseDisplayMode::CompactLetterDisplayOnly,
            PairwiseDisplayMode::CompactLetterDisplayOnly => PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
            PairwiseDisplayMode::BracketsAndCompactLetterDisplay => PairwiseDisplayMode::BracketsOnly,
        };
        self.cld_enabled = self.display_mode != PairwiseDisplayMode::BracketsOnly;
    }

    /// Apply current bracket and CLD settings back onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.comparisons = self.brackets.clone();
        spec.pairwise_display_mode = self.display_mode;
        spec.compact_letter_display = if self.cld_enabled {
            self.selected_result_sheet_id.as_ref().map(|res_id| CompactLetterDisplaySpec {
                result_sheet_id: res_id.clone(),
                alpha_override: self.cld_alpha_override,
                ordering: self.cld_ordering,
                uppercase: self.cld_uppercase,
                font_size_pt: self.cld_font_size_pt,
                offset_pt: self.cld_offset_pt,
                color: self.cld_color,
            })
        } else {
            None
        };
    }

    #[must_use]
    fn brackets_commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let res_id = self.selected_result_sheet_id.as_ref()?;
        if self.brackets.is_empty() {
            return None;
        }
        let style_str = match self.bracket_style {
            BracketStyle::Square => "square",
            BracketStyle::HalfTick => "half_tick",
            BracketStyle::FlatLine => "flat_line",
            BracketStyle::CurlyBrace => "curly_brace",
        };
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "result_sheet_id": res_id.as_str(),
            "clear_existing": true,
            "style": style_str,
            "label_mode": self.label_mode,
            "hide_if_non_significant": self.hide_non_significant,
            "significant_only": self.hide_non_significant,
            "stroke_width_pt": 1.0,
            "font_size_pt": 9.0,
            "bracket_step_pt": 12.0,
        });
        if let ([first], Some(map)) = (self.brackets.as_slice(), payload.as_object_mut()) {
            map.insert(String::from("group_a"), json!(first.group_a));
            map.insert(String::from("group_b"), json!(first.group_b));
        }
        Some(ToolCommandIntent::new("graph.addComparisonBrackets", payload))
    }

    /// Emit `graph.setCompactLetterDisplay` intent when CLD or pairwise display mode should be updated.
    #[must_use]
    pub fn cld_commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        if !self.cld_enabled && !self.had_cld_on_graph && self.display_mode == PairwiseDisplayMode::BracketsOnly {
            return None;
        }
        let mode_str = pairwise_display_mode_str(self.display_mode);
        if !self.cld_enabled {
            return Some(ToolCommandIntent::new(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": id.as_str(),
                    "enabled": false,
                    "pairwise_display_mode": mode_str,
                }),
            ));
        }
        let res_id = self.selected_result_sheet_id.as_ref()?;
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "enabled": true,
            "result_sheet_id": res_id.as_str(),
            "pairwise_display_mode": mode_str,
            "ordering": cld_ordering_str(self.cld_ordering),
            "uppercase": self.cld_uppercase,
            "alpha_override": self.cld_alpha_override,
            "font_size_pt": self.cld_font_size_pt,
            "offset_pt": self.cld_offset_pt,
        });
        if self.cld_color != Rgba8::BLACK
            && let Some(map) = payload.as_object_mut()
        {
            map.insert(String::from("color"), json!([self.cld_color.r, self.cld_color.g, self.cld_color.b, self.cld_color.a]));
        }
        Some(ToolCommandIntent::new("graph.setCompactLetterDisplay", payload))
    }

    /// Emit primary intent (`graph.addComparisonBrackets` when brackets exist, else `graph.setCompactLetterDisplay`).
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        self.brackets_commit_intent().or_else(|| self.cld_commit_intent())
    }

    /// Emit all applicable pairwise intents (`graph.addComparisonBrackets` and/or `graph.setCompactLetterDisplay`).
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let mut intents = Vec::new();
        if self.brackets_dirty
            && !self.brackets.is_empty()
            && let Some(brackets_intent) = self.brackets_commit_intent()
        {
            intents.push(brackets_intent);
        }
        if let Some(cld_intent) = self.cld_commit_intent() {
            intents.push(cld_intent);
        }
        intents
    }

    /// Apply field patches for `Pairwise Comparisons` brackets and Compact Letter Display (`WF-46`, `WF-50`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, doc, &LocalePreset::period_comma("en-US"))
    }

    /// Apply field patches using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(
        &mut self,
        fields: &serde_json::Map<String, Value>,
        doc: Option<&ProjectDocument>,
        preset: &LocalePreset,
    ) -> Result<(), String> {
        let mut auto_add = false;
        let mut explicit_mode = None;
        let mut explicit_enabled = None;
        for (key, val) in fields {
            match key.as_str() {
                "result_sheet_id" | "selected_result_sheet_id" | "pairwise.result_sheet_id" => {
                    let s = val.as_str().ok_or_else(|| String::from("`result_sheet_id` must be a string"))?.trim();
                    if !s.is_empty() {
                        let sid = SheetId::try_new(s).map_err(|e| e.to_string())?;
                        self.selected_result_sheet_id = Some(sid);
                    }
                }
                "bracket_style" | "style" | "pairwise.bracket_style" | "pairwise.style" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`bracket_style` must be a string"))?;
                    self.bracket_style = parse_bracket_style_str(raw)?;
                    self.brackets_dirty = true;
                }
                "label_mode" | "pairwise.label_mode" => {
                    self.label_mode = parse_bracket_label_mode(val)?;
                    self.brackets_dirty = true;
                }
                "hide_non_significant" | "hide_if_non_significant" | "pairwise.hide_non_significant" => {
                    self.hide_non_significant = val.as_bool().ok_or_else(|| String::from("`hide_non_significant` must be boolean"))?;
                    self.brackets_dirty = true;
                }
                "auto_add" | "pairwise.auto_add" if val.as_bool() == Some(true) => {
                    auto_add = true;
                }
                "display_mode" | "pairwise_display_mode" | "pairwise.display_mode" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`display_mode` must be a string"))?;
                    let mode = parse_pairwise_display_mode_str(raw)?;
                    self.display_mode = mode;
                    explicit_mode = Some(mode);
                }
                "cld_enabled" | "enabled" | "compact_letter_display" | "pairwise.cld_enabled" => {
                    let en = val.as_bool().ok_or_else(|| String::from("`cld_enabled` must be boolean"))?;
                    self.cld_enabled = en;
                    explicit_enabled = Some(en);
                }
                "cld_ordering" | "ordering" | "pairwise.cld_ordering" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`cld_ordering` must be a string"))?;
                    self.cld_ordering = parse_cld_ordering_str(raw)?;
                }
                "cld_uppercase" | "uppercase" | "pairwise.cld_uppercase" => {
                    self.cld_uppercase = val.as_bool().ok_or_else(|| String::from("`cld_uppercase` must be boolean"))?;
                }
                "cld_alpha_override" | "alpha_override" | "pairwise.cld_alpha_override" => {
                    let alpha = parse_opt_finite_f64_with_locale(val, "cld_alpha_override", preset)?;
                    if let Some(a) = alpha
                        && (a <= 0.0 || a >= 1.0)
                    {
                        return Err(format!("`cld_alpha_override` must lie in (0, 1), got {a}"));
                    }
                    self.cld_alpha_override = alpha;
                }
                "cld_font_size_pt" | "font_size_pt" | "pairwise.cld_font_size_pt" => {
                    let fs = parse_finite_f64_with_locale(val, "cld_font_size_pt", preset)?;
                    if fs <= 0.0 || fs > 144.0 {
                        return Err(format!("`cld_font_size_pt` must lie in (0, 144], got {fs}"));
                    }
                    self.cld_font_size_pt = fs;
                }
                "cld_offset_pt" | "offset_pt" | "pairwise.cld_offset_pt" => {
                    let off = parse_dialog_num_with_locale(val, "cld_offset_pt", preset)?;
                    if !(0.0..=200.0).contains(&off) {
                        return Err(format!("`cld_offset_pt` must lie in [0, 200], got {off}"));
                    }
                    self.cld_offset_pt = off;
                }
                "auto_add" | "pairwise.auto_add" => {}
                other => return Err(format!("unknown Pairwise Comparisons field `{other}`")),
            }
        }
        if let Some(en) = explicit_enabled
            && explicit_mode.is_none()
        {
            if en {
                if self.display_mode == PairwiseDisplayMode::BracketsOnly {
                    self.display_mode = if self.brackets.is_empty() {
                        PairwiseDisplayMode::CompactLetterDisplayOnly
                    } else {
                        PairwiseDisplayMode::BracketsAndCompactLetterDisplay
                    };
                }
            } else {
                self.display_mode = PairwiseDisplayMode::BracketsOnly;
            }
        } else if let Some(mode) = explicit_mode
            && explicit_enabled.is_none()
        {
            self.cld_enabled = mode != PairwiseDisplayMode::BracketsOnly;
        }
        for b in &mut self.brackets {
            b.style = self.bracket_style;
            b.label_mode = self.label_mode.clone();
            b.hide_if_non_significant = self.hide_non_significant;
        }
        if auto_add && let Some(d) = doc {
            let _ = self.auto_add_from_analysis(d);
            self.brackets_dirty = true;
        }
        Ok(())
    }
}

/// 5-channel `MultipleVariables` Bubble Plot inspector (`X`, `Y`, `Color`, `Size`, `Label`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BubblePlotInspectorState {
    pub graph_sheet_id: Option<SheetId>,
    pub plot_mode: MultivariatePlotMode,
    pub x_column: usize,
    pub y_column: usize,
    pub color_channel: ColorChannelMapping,
    pub size_channel: SizeChannelMapping,
    pub label_column: Option<usize>,
    pub show_convex_hulls: bool,
    pub show_confidence_ellipses: bool,
    pub available_columns: Vec<VariableCandidate>,
}

impl Default for BubblePlotInspectorState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            plot_mode: MultivariatePlotMode::BubbleScatter,
            x_column: 0,
            y_column: 1,
            color_channel: ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0)),
            size_channel: SizeChannelMapping::Constant { radius_pt: 5.0 },
            label_column: None,
            show_convex_hulls: false,
            show_confidence_ellipses: false,
            available_columns: Vec::new(),
        }
    }
}

impl BubblePlotInspectorState {
    /// Load 5-channel encodings and typed source columns for `graph`.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.available_columns = doc.data_sheet(&graph.spec.primary_source_sheet_id).map_or_else(Vec::new, collect_variable_candidates);
        if let Some(mv) = &graph.spec.multivariate {
            self.plot_mode = mv.plot_mode.clone();
            self.x_column = mv.x_column;
            self.y_column = mv.y_column;
            self.color_channel = mv.color.clone();
            self.size_channel = mv.size;
            self.label_column = mv.label_column;
            self.show_convex_hulls = mv.show_convex_hulls;
            self.show_confidence_ellipses = mv.show_confidence_ellipses;
        }
    }

    /// Assign categorical column color channel.
    pub fn set_color_categorical(&mut self, column: usize, palette: PaletteId) {
        self.color_channel = ColorChannelMapping::Categorical { column, palette, level_order_override: None };
    }

    /// Assign continuous colormap ramp color channel.
    pub fn set_color_continuous(&mut self, column: usize, colormap: ColormapId, domain_clamp: Option<(f64, f64)>) {
        self.color_channel = ColorChannelMapping::Continuous { column, ramp: ColorRamp::Named { id: colormap, reversed: false }, domain_clamp };
    }

    /// Assign variable bubble radius channel.
    pub fn set_size_variable(&mut self, column: usize, min_radius_pt: f64, max_radius_pt: f64) {
        let min_r = if min_radius_pt.is_finite() { min_radius_pt.clamp(1.0, 24.0) } else { 2.5 };
        let max_r = if max_radius_pt.is_finite() { max_radius_pt.clamp(min_r, 36.0) } else { 14.0 };
        self.size_channel = SizeChannelMapping::Variable { column, min_radius_pt: min_r, max_radius_pt: max_r };
    }

    /// Materialize the 5-channel [`MultivariateGraphEncoding`].
    #[must_use]
    pub fn to_encoding(&self) -> MultivariateGraphEncoding {
        MultivariateGraphEncoding {
            plot_mode: self.plot_mode.clone(),
            x_column: self.x_column,
            y_column: self.y_column,
            color: self.color_channel.clone(),
            size: self.size_channel,
            label_column: self.label_column,
            x_level_order_override: None,
            show_convex_hulls: self.show_convex_hulls,
            show_confidence_ellipses: self.show_confidence_ellipses,
        }
    }

    /// Apply 5-channel encodings to `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.kind = GraphKind::MultipleVariables;
        spec.multivariate = Some(self.to_encoding());
    }

    /// Emit `graph.setSpec` updating multivariate 5-channel encodings.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        Some(ToolCommandIntent::new(
            "graph.setSpec",
            json!({
                "sheet_id": id.as_str(),
                "kind": GraphKind::MultipleVariables,
                "multivariate": self.to_encoding(),
            }),
        ))
    }

    /// Apply field patches for 5-channel `MultipleVariables` bubble & categorical summary plots (`WF-44`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        let mut color_mode: Option<String> = None;
        let mut color_col: Option<usize> = None;
        let mut palette_override: Option<PaletteId> = None;
        let mut colormap_override: Option<ColormapId> = None;
        for (key, val) in fields {
            match key.as_str() {
                "x_column" | "bubble.x_column" => self.x_column = parse_usize_min(val, key, 0)?,
                "y_column" | "bubble.y_column" => self.y_column = parse_usize_min(val, key, 0)?,
                "size_column" | "bubble.size_column" => {
                    if val.is_null() {
                        self.size_channel = SizeChannelMapping::Constant { radius_pt: 5.0 };
                    } else {
                        self.set_size_variable(parse_usize_min(val, key, 0)?, 2.5, 14.0);
                    }
                }
                "label_column" | "group_column" | "bubble.label_column" => {
                    self.label_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                }
                "color_mode" | "bubble.color_mode" => {
                    color_mode = val.as_str().map(norm_key);
                }
                "color_column" | "bubble.color_column" => {
                    color_col = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                }
                "palette" | "bubble.palette" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`palette` must be a string"))?;
                    palette_override = Some(parse_palette_str(raw)?);
                }
                "colormap" | "bubble.colormap" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`colormap` must be a string"))?;
                    colormap_override = Some(parse_colormap_id_str(raw)?);
                }
                "show_convex_hulls" | "bubble.show_convex_hulls" => {
                    self.show_convex_hulls = val.as_bool().ok_or_else(|| String::from("`show_convex_hulls` must be boolean"))?;
                }
                "show_confidence_ellipses" | "bubble.show_confidence_ellipses" => {
                    self.show_confidence_ellipses = val.as_bool().ok_or_else(|| String::from("`show_confidence_ellipses` must be boolean"))?;
                }
                "plot_mode" | "categorical_summary" | "bubble.plot_mode" => {
                    let is_cat = val.as_bool().unwrap_or_else(|| val.as_str().is_some_and(|s| norm_key(s).contains("categorical")));
                    self.plot_mode = if is_cat {
                        MultivariatePlotMode::CategoricalSummary {
                            gdo: build_gdo_for_option("col.box_whisker", self.palette_fallback(), MarkerStyle::default(), SummaryMeasure::Mean),
                            y_error_bars: None,
                        }
                    } else {
                        MultivariatePlotMode::BubbleScatter
                    };
                }
                "bubble.color_channel" | "color_channel" => {
                    if let Ok(ch) = serde_json::from_value::<ColorChannelMapping>(val.clone()) {
                        self.color_channel = ch;
                    } else if val.is_null() {
                        self.color_channel = ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0));
                    } else if let Some(s) = val.as_str() {
                        let norm = norm_key(s);
                        if norm.contains("constant") {
                            color_mode = Some(String::from("constant"));
                        } else if norm.contains("continuous") {
                            color_mode = Some(String::from("continuous"));
                        } else if norm.contains("categorical") {
                            color_mode = Some(String::from("categorical"));
                        } else {
                            return Err(format!("unsupported `{key}` value `{s}`"));
                        }
                    } else {
                        return Err(format!("`{key}` must be a ColorChannelMapping object, mode string, or null"));
                    }
                }
                "bubble.size_channel" | "size_channel" => {
                    if let Ok(sz) = serde_json::from_value::<SizeChannelMapping>(val.clone()) {
                        self.size_channel = sz;
                    } else if val.is_null() {
                        self.size_channel = SizeChannelMapping::Constant { radius_pt: 5.0 };
                    } else if let Ok(col) = parse_usize_min(val, key, 0) {
                        self.set_size_variable(col, 2.5, 14.0);
                    } else {
                        return Err(format!("`{key}` must be a SizeChannelMapping object, column index, or null"));
                    }
                }
                other => return Err(format!("unknown Bubble Plot field `{other}`")),
            }
        }
        if color_mode.is_some() || color_col.is_some() || palette_override.is_some() || colormap_override.is_some() {
            match color_mode.as_deref() {
                Some("constant") => self.color_channel = ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0)),
                Some("continuous") => {
                    let col = color_col.unwrap_or(self.y_column);
                    self.set_color_continuous(col, colormap_override.unwrap_or(ColormapId::Viridis), None);
                }
                Some("categorical") => {
                    let col = color_col.unwrap_or(self.label_column.unwrap_or(0));
                    self.set_color_categorical(col, palette_override.unwrap_or(PaletteId::OkabeIto));
                }
                _ if colormap_override.is_some() => {
                    let col = color_col.unwrap_or(self.y_column);
                    self.set_color_continuous(col, colormap_override.unwrap_or(ColormapId::Viridis), None);
                }
                _ if color_col.is_some() => {
                    self.set_color_categorical(color_col.unwrap_or(0), palette_override.unwrap_or(PaletteId::OkabeIto));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn palette_fallback(&self) -> Rgba8 {
        PaletteId::OkabeIto.color_at(0)
    }
}

/// `Apply Style From...` cross-graph style cloning dialog (`graph.applyStyleFrom`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApplyStyleFromState {
    pub target_graph_id: Option<SheetId>,
    pub donor_graph_id: Option<SheetId>,
    pub donor_candidates: Vec<(SheetId, String)>,
    pub clone_palette: bool,
    pub clone_frame_and_axes: bool,
    pub clone_error_bars_and_legend: bool,
}

impl ApplyStyleFromState {
    /// Discover candidate donor graph sheets in `doc` distinct from `target_graph_id`.
    pub fn open_for_graph(&mut self, target_graph_id: &SheetId, doc: &ProjectDocument) {
        self.target_graph_id = Some(target_graph_id.clone());
        self.clone_palette = true;
        self.clone_frame_and_axes = true;
        self.clone_error_bars_and_legend = true;
        self.donor_candidates = doc.graph_sheets.iter().filter(|g| &g.id != target_graph_id).map(|g| (g.id.clone(), g.title.clone())).collect();
        self.donor_graph_id = self.donor_candidates.first().map(|(id, _)| id.clone());
    }

    /// Preview the styled [`GraphSpec`] without mutating target data bindings.
    #[must_use]
    pub fn preview_applied_spec(&self, doc: &ProjectDocument) -> Option<GraphSpec> {
        let target_id = self.target_graph_id.as_ref()?;
        let donor_id = self.donor_graph_id.as_ref()?;
        let target = doc.graph_sheet(target_id)?;
        let donor = doc.graph_sheet(donor_id)?;
        let mut preview = target.spec.clone();
        preview.apply_style_from(&donor.spec);
        if !self.clone_palette {
            preview.palette = target.spec.palette;
        }
        if !self.clone_frame_and_axes {
            preview.frame_preset = target.spec.frame_preset;
            preview.x_axis.stroke_width_pt = target.spec.x_axis.stroke_width_pt;
            preview.y_axis.stroke_width_pt = target.spec.y_axis.stroke_width_pt;
        }
        if !self.clone_error_bars_and_legend {
            preview.legend = target.spec.legend;
        }
        Some(preview)
    }

    /// Emit `graph.applyStyleFrom` command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let target = self.target_graph_id.as_ref()?;
        let donor = self.donor_graph_id.as_ref()?;
        Some(ToolCommandIntent::new(
            "graph.applyStyleFrom",
            json!({
                "sheet_id": target.as_str(),
                "donor_sheet_id": donor.as_str(),
                "clone_palette": self.clone_palette,
                "clone_frame_and_axes": self.clone_frame_and_axes,
                "clone_error_bars_and_legend": self.clone_error_bars_and_legend,
            }),
        ))
    }

    /// Apply modal field patches for `Apply Style From...` (`WF-48`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "donor_graph_id" | "donor_sheet_id" | "style.donor_graph_id" | "apply_style.donor_graph_id" => {
                    let s = val.as_str().ok_or_else(|| String::from("`donor_graph_id` must be a string"))?.trim();
                    if !s.is_empty() {
                        self.donor_graph_id = Some(SheetId::from(s));
                    }
                }
                "clone_palette" | "style.clone_palette" | "apply_style.clone_palette" => {
                    self.clone_palette = val.as_bool().ok_or_else(|| String::from("`clone_palette` must be boolean"))?;
                }
                "clone_frame_and_axes" | "style.clone_frame_and_axes" | "apply_style.clone_frame_and_axes" => {
                    self.clone_frame_and_axes = val.as_bool().ok_or_else(|| String::from("`clone_frame_and_axes` must be boolean"))?;
                }
                "clone_error_bars_and_legend" | "style.clone_error_bars_and_legend" | "apply_style.clone_error_bars_and_legend" => {
                    self.clone_error_bars_and_legend = val.as_bool().ok_or_else(|| String::from("`clone_error_bars_and_legend` must be boolean"))?;
                }
                other => return Err(format!("unknown Apply Style From field `{other}`")),
            }
        }
        Ok(())
    }
}

/// Multi-panel publication figure composer (`LayoutSpec`, journal presets, panel badges, alignment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageLayoutComposerState {
    pub layout_sheet_id: Option<SheetId>,
    pub preset: JournalWidthPreset,
    pub orientation: PageOrientation,
    pub rows: u8,
    pub cols: u8,
    pub align_plot_origins: bool,
    pub panel_badges: PanelBadgeStyle,
    pub items: Vec<LayoutItem>,
    pub new_callout_markdown: String,
}

impl Default for PageLayoutComposerState {
    fn default() -> Self {
        Self {
            layout_sheet_id: None,
            preset: JournalWidthPreset::DoubleColumn174Mm,
            orientation: PageOrientation::Landscape,
            rows: 1,
            cols: 2,
            align_plot_origins: true,
            panel_badges: PanelBadgeStyle::UpperCaseLetters,
            items: Vec::new(),
            new_callout_markdown: String::new(),
        }
    }
}

impl PageLayoutComposerState {
    /// Load layout properties and items from `layout`.
    pub fn sync_from_layout(&mut self, layout: &LayoutSheet) {
        self.layout_sheet_id = Some(layout.id.clone());
        self.orientation = layout.spec.orientation;
        let (rows, cols) = layout.spec.grid_template.unwrap_or((1, 2));
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.align_plot_origins = layout.spec.align_plot_origins;
        self.panel_badges = layout.spec.panel_badges;
        self.items = layout.spec.items.clone();
    }

    /// Switch physical journal/page size preset.
    pub fn apply_preset(&mut self, preset: JournalWidthPreset) {
        self.preset = preset;
        let derived = LayoutSpec::from_preset(preset);
        self.orientation = derived.orientation;
    }

    /// Place `graph_ids` neatly across `rows x cols` slots inside `LayoutSpec::from_preset(preset)`.
    pub fn arrange_grid(&mut self, graph_ids: &[SheetId], margin_pt: f64, gutter_pt: f64) {
        let page = LayoutSpec::from_preset(self.preset).page_size_pt();
        let rows = usize::from(self.rows.max(1));
        let cols = usize::from(self.cols.max(1));
        let margin = if margin_pt.is_finite() { margin_pt.max(0.0) } else { 14.0 };
        let gutter = if gutter_pt.is_finite() { gutter_pt.max(0.0) } else { 10.0 };
        let usable_w = (page.width - 2.0 * margin - gutter * ((cols.saturating_sub(1)) as f64)).max(24.0);
        let usable_h = (page.height - 2.0 * margin - gutter * ((rows.saturating_sub(1)) as f64)).max(24.0);
        let cell_w = usable_w / (cols as f64);
        let cell_h = usable_h / (rows as f64);

        self.items.retain(|item| !matches!(item, LayoutItem::LiveGraph { .. }));
        for (idx, gid) in graph_ids.iter().take(rows * cols).enumerate() {
            let r = idx / cols;
            let c = idx % cols;
            let x = margin + (c as f64) * (cell_w + gutter);
            let y = margin + (r as f64) * (cell_h + gutter);
            let badge = self.panel_badges.badge_for(idx);
            self.items.push(LayoutItem::LiveGraph {
                id: format!("panel-{}", idx + 1),
                graph_sheet_id: gid.clone(),
                bounds_pt: RectPt::new(x, y, cell_w, cell_h),
                scale_percent: 100.0,
                badge_override: badge,
            });
        }
    }

    /// Append a live embedded table item.
    pub fn add_live_table(&mut self, source_sheet_id: SheetId, tab_id: Option<String>, bounds_pt: RectPt) {
        let next = self.items.len() + 1;
        self.items.push(LayoutItem::LiveTable { id: format!("table-{next}"), source_sheet_id, tab_id, bounds_pt });
    }

    /// Append a Markdown / hook-interpolated callout item.
    pub fn add_text_callout(&mut self, markdown: String, bounds_pt: RectPt, font_size_pt: f64) {
        let next = self.items.len() + 1;
        let size = if font_size_pt.is_finite() { font_size_pt.clamp(4.0, 72.0) } else { 10.0 };
        self.items.push(LayoutItem::TextCallout { id: format!("callout-{next}"), markdown, bounds_pt, font_size_pt: size });
    }

    /// Materialize a full [`LayoutSpec`].
    #[must_use]
    pub fn to_layout_spec(&self) -> LayoutSpec {
        let mut spec = LayoutSpec::from_preset(self.preset);
        spec.orientation = self.orientation;
        spec.grid_template = Some((self.rows.max(1), self.cols.max(1)));
        spec.align_plot_origins = self.align_plot_origins;
        spec.panel_badges = self.panel_badges;
        spec.items = self.items.clone();
        spec
    }

    /// Emit `layout.setSpec` and `layout.alignOrigins` intents.
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let Some(id) = &self.layout_sheet_id else {
            return Vec::new();
        };
        let mut out = vec![ToolCommandIntent::new(
            "layout.setSpec",
            json!({
                "sheet_id": id.as_str(),
                "spec": self.to_layout_spec(),
            }),
        )];
        if self.align_plot_origins {
            out.push(ToolCommandIntent::new("layout.alignOrigins", json!({ "sheet_id": id.as_str() })));
        }
        out
    }

    /// Apply modal/inspector field patches for `Page Layout` (`WF-48`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
        let mut grid_changed = false;
        let mut explicit_graph_ids: Option<Vec<SheetId>> = None;
        for (key, val) in fields {
            match key.as_str() {
                "preset" | "layout.preset" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`preset` must be a string"))?;
                    self.apply_preset(parse_journal_preset_str(raw)?);
                }
                "orientation" | "layout.orientation" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`orientation` must be a string"))?;
                    self.orientation = parse_page_orientation_str(raw)?;
                }
                "rows" | "layout.rows" => {
                    self.rows = u8::try_from(parse_usize_min(val, key, 1)?.min(12)).unwrap_or(12);
                    grid_changed = true;
                }
                "cols" | "columns" | "layout.cols" => {
                    self.cols = u8::try_from(parse_usize_min(val, key, 1)?.min(12)).unwrap_or(12);
                    grid_changed = true;
                }
                "align_plot_origins" | "layout.align_plot_origins" => {
                    self.align_plot_origins = val.as_bool().ok_or_else(|| String::from("`align_plot_origins` must be boolean"))?;
                }
                "panel_badges" | "layout.panel_badges" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`panel_badges` must be a string"))?;
                    self.panel_badges = parse_panel_badge_style_str(raw)?;
                }
                "new_callout_markdown" | "callout" | "layout.new_callout_markdown" => {
                    if let Some(s) = val.as_str() {
                        self.new_callout_markdown = s.to_owned();
                    }
                }
                "arrange_graph_ids" | "graph_ids" | "layout.arrange_graph_ids" => {
                    let arr = val.as_array().ok_or_else(|| format!("`{key}` must be an array of sheet IDs"))?;
                    let mut ids = Vec::with_capacity(arr.len());
                    for (idx, item) in arr.iter().enumerate() {
                        let raw = item.as_str().ok_or_else(|| format!("`{key}[{idx}]` must be a string"))?;
                        ids.push(SheetId::try_new(raw).map_err(|e| e.to_string())?);
                    }
                    explicit_graph_ids = Some(ids);
                }
                other => return Err(format!("unknown Page Layout field `{other}`")),
            }
        }
        if let Some(ids) = explicit_graph_ids {
            self.arrange_grid(&ids, 18.0, 12.0);
        } else if grid_changed && let Some(d) = doc {
            let graph_ids: Vec<SheetId> = d.graph_sheets.iter().map(|g| g.id.clone()).collect();
            self.arrange_grid(&graph_ids, 18.0, 12.0);
        }
        Ok(())
    }
}

/// Output format for [`ExportFigureState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportFigureFormat {
    #[default]
    Svg,
    Pdf,
    Png,
    Tiff(TiffCompression),
}

/// Publication figure export modal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFigureState {
    pub format: ExportFigureFormat,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
}

impl Default for ExportFigureState {
    fn default() -> Self {
        Self { format: ExportFigureFormat::Png, width_px: 1800, height_px: 1200, dpi: 300 }
    }
}

impl ExportFigureState {
    /// Emit engine export intent (`graph.exportSvg`, `graph.exportPdf`, `graph.exportPng`, or `export.tiff`).
    #[must_use]
    pub fn commit_intent(&self, sheet_id: &SheetId) -> ToolCommandIntent {
        match self.format {
            ExportFigureFormat::Svg => ToolCommandIntent::new("graph.exportSvg", json!({ "sheet_id": sheet_id.as_str() })),
            ExportFigureFormat::Pdf => ToolCommandIntent::new("graph.exportPdf", json!({ "sheet_id": sheet_id.as_str() })),
            ExportFigureFormat::Png => ToolCommandIntent::new(
                "graph.exportPng",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": self.width_px.max(16),
                    "height_px": self.height_px.max(16),
                    "dpi": self.dpi.max(72),
                }),
            ),
            ExportFigureFormat::Tiff(compression) => ToolCommandIntent::new(
                "export.tiff",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": self.width_px.max(16),
                    "height_px": self.height_px.max(16),
                    "dpi": self.dpi.max(72),
                    "compression": format!("{compression:?}"),
                }),
            ),
        }
    }

    /// Apply modal field patches for `Export Figure...` (`WF-48`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "format" | "export.format" => self.format = parse_export_format(val)?,
                "width_px" | "export.width_px" => {
                    let w = parse_usize_min(val, key, 16)?;
                    self.width_px = u32::try_from(w).unwrap_or(u32::MAX);
                }
                "height_px" | "export.height_px" => {
                    let h = parse_usize_min(val, key, 16)?;
                    self.height_px = u32::try_from(h).unwrap_or(u32::MAX);
                }
                "dpi" | "export.dpi" => {
                    let d = parse_usize_min(val, key, 72)?;
                    self.dpi = u16::try_from(d.clamp(72, 2400)).unwrap_or(600);
                }
                other => return Err(format!("unknown Export Figure field `{other}`")),
            }
        }
        Ok(())
    }
}

// ============================================================================
// Command Palette (Cmd+K) Fuzzy Filter Over Engine + UI Commands
// ============================================================================

/// Source layer of a Command Palette entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommandOrigin {
    Engine,
    Ui,
}

/// Scored command match displayed inside the Command Palette (`Cmd+K`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub menu_path: &'static str,
    pub shortcut: String,
    pub params_doc: &'static str,
    pub summary: &'static str,
    pub beginner: &'static str,
    pub enabled: bool,
    pub origin: CommandOrigin,
    pub score: i32,
}

impl CommandPaletteEntry {
    /// Resolve the localized label for this command in `locale`, falling back to English [`Self::label`].
    #[must_use]
    pub fn localized_label(&self, locale: &str) -> &'static str {
        crate::i18n::command_label(locale, self.id, self.label)
    }

    fn format_row_header(&self, title: &str) -> String {
        let status = if self.enabled { "" } else { " [disabled]" };
        if self.shortcut.is_empty() {
            format!("{title} ({}){status} — {}", self.id, self.summary)
        } else {
            format!("{title} ({}) [{}]{status} — {}", self.id, self.shortcut, self.summary)
        }
    }

    /// Format compact command palette row copy surfacing `label`, `id`, `shortcut`, and `summary`.
    #[must_use]
    pub fn format_compact_row(&self) -> String {
        self.format_row_header(self.label)
    }

    /// Format compact command palette row copy using the localized label for `locale`.
    #[must_use]
    pub fn format_compact_row_in_locale(&self, locale: &str) -> String {
        self.format_row_header(self.localized_label(locale))
    }

    /// Format rich command palette row copy surfacing `label`, `id`, `shortcut`, `summary`, and `beginner`.
    #[must_use]
    pub fn format_palette_row(&self) -> String {
        self.format_palette_row_in_locale("en")
    }

    /// Format rich localized command palette row copy surfacing `summary` and `beginner`.
    #[must_use]
    pub fn format_palette_row_in_locale(&self, locale: &str) -> String {
        format!("{} | Tip: {}", self.format_row_header(self.localized_label(locale)), self.beginner)
    }
}

struct PaletteCandidate<'a> {
    id: &'static str,
    label: &'static str,
    menu_path: &'static str,
    default_shortcut: &'a str,
    params_doc: &'static str,
    summary: &'static str,
    beginner: &'static str,
    enabled: bool,
    origin: CommandOrigin,
}

/// Command Palette (`Cmd+K`) fuzzy search state spanning Engine + UI commands.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandPaletteState {
    pub query: String,
    pub selected_index: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shortcut_overrides: BTreeMap<String, String>,
}

impl CommandPaletteState {
    /// Reset search text and selection upon opening `Cmd+K`.
    pub fn open(&mut self) {
        self.query.clear();
        self.selected_index = 0;
    }

    /// Synchronize customized or unbound bindings from `shortcuts` so queries without a registry
    /// reference still surface live active shortcuts.
    pub fn sync_shortcuts(&mut self, shortcuts: &ShortcutRegistry) {
        self.shortcut_overrides.clear();
        for binding in shortcuts.bindings() {
            if binding.is_customized() {
                self.shortcut_overrides.insert(binding.command_id.clone(), binding.active_combo.clone());
            }
        }
    }

    /// Return fuzzy-filtered and relevance-sorted commands from both Engine and UI catalogs.
    #[must_use]
    pub fn matches(&self) -> Vec<CommandPaletteEntry> {
        self.collect_matches(None, None)
    }

    /// Return fuzzy-filtered and relevance-sorted commands with `enabled` evaluated for `session`.
    #[must_use]
    pub fn matches_for_session(&self, session: &Session) -> Vec<CommandPaletteEntry> {
        self.collect_matches(Some(session), None)
    }

    /// Return fuzzy-filtered and relevance-sorted commands using live shortcuts from `shortcuts`.
    #[must_use]
    pub fn matches_with_shortcuts(&self, session: Option<&Session>, shortcuts: &ShortcutRegistry) -> Vec<CommandPaletteEntry> {
        self.collect_matches(session, Some(shortcuts))
    }

    /// Format visible command palette rows surfacing `summary` and active `beginner` guidance.
    #[must_use]
    pub fn preview_rows(&self, session: Option<&Session>, max_rows: usize) -> Vec<String> {
        self.preview_rows_for_locale(session, "en", max_rows)
    }

    /// Format localized visible command palette rows surfacing `summary` and active `beginner` guidance.
    #[must_use]
    pub fn preview_rows_for_locale(&self, session: Option<&Session>, locale: &str, max_rows: usize) -> Vec<String> {
        self.format_preview_rows(&self.collect_matches(session, None), locale, max_rows)
    }

    /// Format localized visible command palette rows using live shortcuts from `shortcuts`.
    #[must_use]
    pub fn preview_rows_with_shortcuts(&self, session: Option<&Session>, shortcuts: &ShortcutRegistry, locale: &str, max_rows: usize) -> Vec<String> {
        self.format_preview_rows(&self.collect_matches(session, Some(shortcuts)), locale, max_rows)
    }

    fn format_preview_rows(&self, hits: &[CommandPaletteEntry], locale: &str, max_rows: usize) -> Vec<String> {
        let mut rows = Vec::with_capacity(hits.len().min(max_rows) + 2);
        rows.push(format!("> {}", self.query));
        for (idx, hit) in hits.iter().take(max_rows).enumerate() {
            let marker = if idx == self.selected_index { "* " } else { "  " };
            rows.push(format!("{marker}{}", hit.format_compact_row_in_locale(locale)));
        }
        if let Some(selected) = hits.get(self.selected_index).or_else(|| hits.first()) {
            rows.push(format!("Tip: {}", selected.beginner));
        }
        rows
    }

    fn resolve_shortcut(&self, shortcuts: Option<&ShortcutRegistry>, command_id: &str, default_shortcut: &str) -> String {
        if let Some(reg) = shortcuts
            && let Some(active) = reg.active_combo(command_id)
        {
            return active.to_owned();
        }
        if let Some(override_combo) = self.shortcut_overrides.get(command_id) {
            return override_combo.clone();
        }
        default_shortcut.to_owned()
    }

    fn try_build_entry(&self, shortcuts: Option<&ShortcutRegistry>, spec: PaletteCandidate<'_>, search_menu: &str) -> Option<CommandPaletteEntry> {
        let q = self.query.trim();
        let shortcut = self.resolve_shortcut(shortcuts, spec.id, spec.default_shortcut);
        let base = score_command(q, spec.id, spec.label, search_menu, &shortcut);
        let score = score_palette_entry(q, base, spec.summary, spec.beginner)?;
        Some(CommandPaletteEntry {
            id: spec.id,
            label: spec.label,
            menu_path: spec.menu_path,
            shortcut,
            params_doc: spec.params_doc,
            summary: spec.summary,
            beginner: spec.beginner,
            enabled: spec.enabled,
            origin: spec.origin,
            score,
        })
    }

    fn collect_matches(&self, session: Option<&Session>, shortcuts: Option<&ShortcutRegistry>) -> Vec<CommandPaletteEntry> {
        let mut results = Vec::new();
        for spec in cmd::registry().specs() {
            let candidate = PaletteCandidate {
                id: spec.id,
                label: spec.label,
                menu_path: spec.menu_path,
                default_shortcut: spec.shortcut,
                params_doc: spec.summary,
                summary: spec.summary,
                beginner: spec.beginner,
                enabled: session.is_none_or(|s| (spec.enabled)(s)),
                origin: CommandOrigin::Engine,
            };
            if let Some(entry) = self.try_build_entry(shortcuts, candidate, spec.menu_path) {
                results.push(entry);
            }
        }
        for spec in UI_COMMANDS {
            let candidate = PaletteCandidate {
                id: spec.id,
                label: spec.label,
                menu_path: "UI",
                default_shortcut: spec.shortcut,
                params_doc: spec.params_doc,
                summary: spec.summary,
                beginner: spec.beginner,
                enabled: true,
                origin: CommandOrigin::Ui,
            };
            if let Some(entry) = self.try_build_entry(shortcuts, candidate, "View") {
                results.push(entry);
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(b.id)));
        results
    }
}

fn score_palette_entry(query: &str, base_score: Option<i32>, summary: &str, beginner: &str) -> Option<i32> {
    if let Some(score) = base_score {
        return Some(score);
    }
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Some(0);
    }
    if summary.to_ascii_lowercase().contains(&needle) {
        return Some(40);
    }
    if beginner.to_ascii_lowercase().contains(&needle) {
        return Some(30);
    }
    None
}

impl CommandPaletteState {
    /// Move selection cursor down in the filtered list.
    pub fn select_next(&mut self) {
        let count = self.matches().len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    /// Move selection cursor up in the filtered list.
    pub fn select_prev(&mut self) {
        let count = self.matches().len();
        if count > 0 {
            self.selected_index = if self.selected_index == 0 { count - 1 } else { self.selected_index - 1 };
        }
    }

    /// Emit [`ToolCommandIntent`] for the highlighted entry.
    #[must_use]
    pub fn activate_selected(&self) -> Option<ToolCommandIntent> {
        let list = self.matches();
        let entry = list.get(self.selected_index).or_else(|| list.first())?;
        Some(ToolCommandIntent::new(entry.id, json!({})))
    }
}

fn score_command(query: &str, id: &str, label: &str, menu_path: &str, shortcut: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q = query.to_ascii_lowercase();
    let best_id = fuzzy_score(&q, &id.to_ascii_lowercase());
    let best_label = fuzzy_score(&q, &label.to_ascii_lowercase());
    let best_menu = fuzzy_score(&q, &menu_path.to_ascii_lowercase());
    let best_shortcut = fuzzy_score(&q, &shortcut.to_ascii_lowercase());
    [best_label.map(|s| s + 25), best_id.map(|s| s + 20), best_menu, best_shortcut].into_iter().flatten().max()
}

fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    if haystack == needle {
        return Some(10_000);
    }
    if haystack.starts_with(needle) {
        return Some(5_000);
    }
    if let Some(pos) = haystack.find(needle) {
        let bonus =
            if pos == 0 || haystack.as_bytes().get(pos.saturating_sub(1)).is_some_and(|b| !b.is_ascii_alphanumeric()) { 2_500 } else { 1_500 };
        return Some(bonus - (pos as i32));
    }
    let mut score = 0i32;
    let mut prev_matched = false;
    let mut n_iter = needle.chars().peekable();
    for (idx, ch) in haystack.chars().enumerate() {
        let Some(&target) = n_iter.peek() else {
            break;
        };
        if ch == target {
            score += if prev_matched { 45 } else { 15 };
            if idx == 0 {
                score += 30;
            }
            prev_matched = true;
            let _ = n_iter.next();
        } else {
            prev_matched = false;
            score -= 1;
        }
    }
    n_iter.peek().is_none().then_some(score)
}

// ============================================================================
// QOL-1 / QOL-3 / QOL-4 / QOL-7 / QOL-8 Dialog & Workspace Models
// ============================================================================

/// Non-destructive table archetype conversion modal state (`QOL-1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertArchetypeDialogState {
    pub sheet_id: Option<SheetId>,
    pub source_kind: Option<TableKind>,
    pub target_kind: TableKind,
    pub allow_lossy: bool,
    #[serde(default)]
    pub preview_only: bool,
    pub preview_ready: bool,
    pub preview_lossy: bool,
    pub preserved_cells: usize,
    pub dropped_cells: usize,
    pub preserved_exclusions: usize,
    pub preview_warnings: Vec<String>,
}

impl Default for ConvertArchetypeDialogState {
    fn default() -> Self {
        Self {
            sheet_id: None,
            source_kind: None,
            target_kind: TableKind::Column,
            allow_lossy: false,
            preview_only: false,
            preview_ready: false,
            preview_lossy: false,
            preserved_cells: 0,
            dropped_cells: 0,
            preserved_exclusions: 0,
            preview_warnings: Vec::new(),
        }
    }
}

impl ConvertArchetypeDialogState {
    /// Supported conversion destination archetypes for `table.convertArchetype`.
    #[must_use]
    pub const fn supported_targets() -> &'static [TableKind] {
        &[TableKind::Column, TableKind::Grouped, TableKind::Xy, TableKind::MultipleVariables, TableKind::Contingency]
    }

    /// Initialize conversion dialog state for `sheet`.
    pub fn open_for_sheet(&mut self, sheet: &DataSheet) {
        let current = sheet.kind();
        self.sheet_id = Some(sheet.id.clone());
        self.source_kind = Some(current);
        self.target_kind = match current {
            TableKind::Column => TableKind::Grouped,
            TableKind::Grouped => TableKind::Column,
            TableKind::Xy => TableKind::MultipleVariables,
            _ => TableKind::Column,
        };
        self.allow_lossy = false;
        self.preview_only = false;
        self.preview_ready = false;
        self.preview_lossy = false;
        self.preserved_cells = 0;
        self.dropped_cells = 0;
        self.preserved_exclusions = 0;
        self.preview_warnings.clear();
    }

    /// Patch conversion dialog fields from scripted GUI actions / inspector text inputs.
    ///
    /// # Errors
    /// Returns an error if `target_kind` is unrecognized or flags have wrong types.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "target_kind" | "kind" | "convert.target_kind" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`target_kind` must be a string"))?;
                    let parsed = parse_table_kind_str(raw)?;
                    if !Self::supported_targets().contains(&parsed) {
                        return Err(format!("unsupported conversion target `{raw}`"));
                    }
                    self.set_target_kind(parsed);
                }
                "allow_lossy" | "convert.allow_lossy" => {
                    self.allow_lossy = val.as_bool().ok_or_else(|| String::from("`allow_lossy` must be boolean"))?;
                }
                "preview_only" | "convert.preview_only" => {
                    self.preview_only = val.as_bool().ok_or_else(|| String::from("`preview_only` must be boolean"))?;
                }
                other => return Err(format!("unknown Convert Table Archetype field `{other}`")),
            }
        }
        Ok(())
    }

    /// Set a new target archetype, resetting stale preview state.
    pub fn set_target_kind(&mut self, target: TableKind) {
        if self.target_kind != target {
            self.target_kind = target;
            self.preview_ready = false;
        }
    }

    /// Build a read-only `table.convertArchetype` (`preview_only: true`) intent.
    #[must_use]
    pub fn preview_intent(&self) -> Option<ToolCommandIntent> {
        let sheet_id = self.sheet_id.as_ref()?;
        Some(ToolCommandIntent::new(
            "table.convertArchetype",
            json!({
                "sheet_id": sheet_id.as_str(),
                "target_kind": convert_target_wire_name(self.target_kind),
                "preview_only": true,
            }),
        ))
    }

    /// Populate preview impact fields from engine `table.convertArchetype` preview JSON output.
    pub fn apply_preview_payload(&mut self, payload: &Value) {
        let p = payload.get("preview").unwrap_or(payload);
        self.preview_lossy = p.get("lossy").and_then(Value::as_bool).unwrap_or(false);
        self.preserved_cells = p.get("preserved_cells").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
        self.dropped_cells = p.get("dropped_cells").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
        self.preserved_exclusions = p.get("preserved_exclusions").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
        self.preview_warnings = p
            .get("warnings")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|w| w.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        self.preview_ready = true;
    }

    /// Build the mutating `table.convertArchetype` intent, enforcing explicit `allow_lossy` when required.
    ///
    /// # Errors
    /// Returns an error when no sheet is bound or when a lossy conversion awaits explicit confirmation.
    pub fn commit_intent(&self) -> Result<ToolCommandIntent, String> {
        let sheet_id = self.sheet_id.as_ref().ok_or_else(|| String::from("no data sheet selected for archetype conversion"))?;
        if self.preview_lossy && !self.allow_lossy {
            return Err(format!(
                "lossy conversion to {} drops {} cell(s); enable allow_lossy to proceed",
                self.target_kind.label(),
                self.dropped_cells
            ));
        }
        Ok(ToolCommandIntent::new(
            "table.convertArchetype",
            json!({
                "sheet_id": sheet_id.as_str(),
                "target_kind": convert_target_wire_name(self.target_kind),
                "allow_lossy": self.allow_lossy,
                "preview_only": false,
            }),
        ))
    }
}

fn convert_target_wire_name(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Column => "Column",
        TableKind::Grouped => "Grouped",
        TableKind::Xy => "Xy",
        TableKind::MultipleVariables => "MultipleVariables",
        TableKind::Contingency => "Contingency",
        _ => "Column",
    }
}

/// Modal dialog state for auditable exclusion-with-reason (`QOL-3`, `Cmd+Shift+E`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeWithReasonDialogState {
    pub sheet_id: Option<SheetId>,
    pub coords: Vec<GridCellCoord>,
    pub reason_tag: String,
    pub note: String,
    pub timestamp_utc: Option<String>,
}

impl Default for ExcludeWithReasonDialogState {
    fn default() -> Self {
        Self { sheet_id: None, coords: Vec::new(), reason_tag: String::from("Outlier"), note: String::new(), timestamp_utc: None }
    }
}

impl ExcludeWithReasonDialogState {
    /// Preset reason tags offered in the exclusion audit dialog.
    #[must_use]
    pub const fn preset_reason_tags() -> &'static [&'static str] {
        &["Outlier", "Contamination", "Instrument Fault", "Bubble", "Missing Replicate"]
    }

    /// Initialize dialog state targeting `coords` on `sheet_id`.
    pub fn open_for_coords(&mut self, sheet_id: SheetId, coords: Vec<GridCellCoord>) {
        self.sheet_id = Some(sheet_id);
        self.coords = coords;
        if self.reason_tag.trim().is_empty() {
            self.reason_tag = String::from("Outlier");
        }
    }

    /// Build the `table.excludeCellsWithReason` command intent.
    ///
    /// # Errors
    /// Returns an error if no cells are selected or `reason_tag` is empty.
    pub fn commit_intent(&self) -> Result<ToolCommandIntent, String> {
        let sheet_id = self.sheet_id.as_ref().ok_or_else(|| String::from("no data sheet selected for exclusion"))?;
        if self.coords.is_empty() {
            return Err(String::from("at least one grid cell must be selected before excluding with reason"));
        }
        let tag = self.reason_tag.trim();
        if tag.is_empty() {
            return Err(String::from("exclusion reason_tag cannot be empty"));
        }
        let coords_json: Vec<Value> = self.coords.iter().map(|c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol })).collect();
        let mut params = json!({
            "sheet_id": sheet_id.as_str(),
            "coords": coords_json,
            "reason_tag": tag,
            "note": self.note,
        });
        if let Some(ts) = &self.timestamp_utc
            && !ts.trim().is_empty()
            && let Some(obj) = params.as_object_mut()
        {
            obj.insert(String::from("timestamp_utc"), Value::String(ts.trim().to_owned()));
        }
        Ok(ToolCommandIntent::new("table.excludeCellsWithReason", params))
    }
}

/// Simultaneous side-by-side Data/Result + live Graph split-pane workspace configuration (`QOL-4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitWorkspaceState {
    pub enabled: bool,
    /// Fraction of center canvas width allocated to the primary pane in `[0.2, 0.8]`.
    pub ratio: f64,
    pub primary_sheet_id: Option<SheetId>,
    pub secondary_sheet_id: Option<SheetId>,
    #[serde(default)]
    pub dragging_divider: bool,
}

impl Default for SplitWorkspaceState {
    fn default() -> Self {
        Self { enabled: false, ratio: 0.5, primary_sheet_id: None, secondary_sheet_id: None, dragging_divider: false }
    }
}

impl SplitWorkspaceState {
    /// Clamp split divider fraction into `[0.2, 0.8]`.
    pub fn set_ratio(&mut self, ratio: f64) {
        if ratio.is_finite() {
            self.ratio = ratio.clamp(0.2, 0.8);
        }
    }

    /// Resolve concrete `(primary_sheet_id, secondary_sheet_id)` pair against `doc`.
    #[must_use]
    pub fn resolve_sheets(&self, doc: &ProjectDocument) -> (Option<SheetId>, Option<SheetId>) {
        let active = doc.active_sheet_id.clone();
        let primary = self
            .primary_sheet_id
            .clone()
            .filter(|id| doc.sheet_section(id).is_some())
            .or_else(|| {
                active.as_ref().and_then(|id| {
                    if doc.graph_sheet(id).is_some() {
                        doc.graph_sheet(id)
                            .map(|gs| gs.spec.primary_source_sheet_id.clone())
                            .filter(|src| doc.data_sheet(src).is_some())
                            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()))
                    } else {
                        Some(id.clone())
                    }
                })
            })
            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()));

        let secondary = self.secondary_sheet_id.clone().filter(|id| doc.sheet_section(id).is_some()).or_else(|| {
            if let Some(act) = &active
                && (doc.graph_sheet(act).is_some() || doc.layout_sheet(act).is_some())
            {
                return Some(act.clone());
            }
            if let Some(prim) = &primary {
                if let Some(gs) = doc.graph_sheets.iter().find(|g| &g.spec.primary_source_sheet_id == prim) {
                    return Some(gs.id.clone());
                }
                if let Some(rs) = doc.result_sheet(prim)
                    && let Some(gs) = doc.graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == rs.analysis.source_sheet_id)
                {
                    return Some(gs.id.clone());
                }
            }
            doc.graph_sheets.first().map(|g| g.id.clone()).or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
        });

        (primary, secondary)
    }
}

pub use curvebook_doc::RemediationKind;

/// Non-prescriptive assumption diagnostic strip displayed above a [`ResultSheet`] (`QOL-7`).
///
/// Surfaces checked assumptions, numeric diagnostic metrics, statistical/sample-size limitations,
/// and detail-tab navigation links on the *same* [`ResultSheet`] without prescribing alternative
/// tests, data transforms, or outlier exclusion, and never creates sibling sheets automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssumptionDiagnosticBanner {
    pub source_result_id: SheetId,
    pub severity: ChecklistStatus,
    pub summary: String,
    pub diagnostic_metric: Option<String>,
    pub limitation_note: String,
    pub detail_tab_ids: Vec<String>,
}

impl AssumptionDiagnosticBanner {
    /// Derive a strictly diagnostics-only banner from `rs.checklist` (`Violated`, `Warn`, or
    /// inconclusive/small-sample `Info`) and collect same-sheet detail tab IDs from `rs.tabs`.
    #[must_use]
    pub fn from_result(rs: &ResultSheet) -> Option<Self> {
        let rank = |status: ChecklistStatus| -> u8 {
            match status {
                ChecklistStatus::Violated => 3,
                ChecklistStatus::Warn => 2,
                ChecklistStatus::Info => 1,
                ChecklistStatus::Pass => 0,
            }
        };
        let flagged = rs
            .checklist
            .iter()
            .filter(|c| matches!(c.status, ChecklistStatus::Violated | ChecklistStatus::Warn) || is_inconclusive_info_item(c))
            .max_by_key(|c| rank(c.status))?;

        let mut detail_tab_ids: Vec<String> = rs
            .tabs
            .iter()
            .filter(|tab| {
                matches!(
                    tab.kind,
                    ResultTabKind::Diagnostics
                        | ResultTabKind::NormalQq
                        | ResultTabKind::Residuals
                        | ResultTabKind::CovarianceMatrix
                        | ResultTabKind::LogMinusLog
                )
            })
            .map(|tab| tab.id.clone())
            .collect();
        if detail_tab_ids.is_empty() {
            detail_tab_ids = rs.tabs.iter().map(|tab| tab.id.clone()).collect();
        }

        let status_label = match flagged.status {
            ChecklistStatus::Violated => "assumption violated",
            ChecklistStatus::Warn => "assumption warning",
            ChecklistStatus::Info => "inconclusive / sample-size limitation",
            ChecklistStatus::Pass => "checked",
        };
        let checked_count = rs
            .checklist
            .iter()
            .filter(|c| matches!(c.status, ChecklistStatus::Violated | ChecklistStatus::Warn) || is_inconclusive_info_item(c))
            .count();
        let summary = if checked_count > 1 {
            format!("{} ({status_label}; {checked_count} checks flagged)", flagged.assumption)
        } else {
            format!("{} ({status_label})", flagged.assumption)
        };

        Some(Self {
            source_result_id: rs.id.clone(),
            severity: flagged.status,
            summary,
            diagnostic_metric: flagged.diagnostic_metric.clone(),
            limitation_note: sanitize_diagnostics_only_text(flagged),
            detail_tab_ids,
        })
    }

    /// Preferred same-sheet diagnostic detail tab ID (`Diagnostics`, `NormalQq`, `Residuals`, ...).
    #[must_use]
    pub fn preferred_diagnostic_tab_id(&self) -> Option<&str> {
        self.detail_tab_ids.first().map(String::as_str)
    }

    /// Single-line diagnostics-only banner text rendered above the result sheet grid.
    #[must_use]
    pub fn format_banner_line(&self) -> String {
        match &self.diagnostic_metric {
            Some(metric) => format!("! {} [{}]: {} — Inspect Diagnostics", self.summary, metric, self.limitation_note),
            None => format!("! {}: {} — Inspect Diagnostics", self.summary, self.limitation_note),
        }
    }
}

// ============================================================================
// Unified CanvasDialogState
// ============================================================================

/// Modal dialog state for styling selected graph points (`Format > Format Selected Points...`, `WF-22`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FormatSelectedPointsState {
    pub graph_sheet_id: Option<SheetId>,
    pub selected_points: Vec<(usize, usize)>,
    #[serde(default)]
    pub selected_subcols: Vec<Option<usize>>,
    pub color: Option<String>,
    pub fill_color: Option<String>,
    pub symbol: Option<String>,
    pub size_pt: Option<f64>,
    pub border_width_pt: Option<f64>,
    pub fill_mode: Option<String>,
    pub hidden: Option<bool>,
    pub show_row_labels: bool,
    pub clear_overrides: bool,
}

impl FormatSelectedPointsState {
    /// Open dialog state targeting `graph_sheet_id` and `points` `(series_index, point_index)`.
    pub fn open_for_graph(&mut self, graph_sheet_id: SheetId, points: Vec<(usize, usize)>) {
        self.graph_sheet_id = Some(graph_sheet_id);
        self.selected_points = points;
        self.selected_subcols.clear();
        self.clear_overrides = false;
    }

    /// Open dialog state targeting `graph_sheet_id` and `(series_index, point_index, subcol)` tuples.
    pub fn open_for_graph_with_subcols(&mut self, graph_sheet_id: SheetId, targets: Vec<(usize, usize, Option<usize>)>) {
        self.graph_sheet_id = Some(graph_sheet_id);
        self.set_selected_targets(targets);
        self.clear_overrides = false;
    }

    /// Replace selected targets with explicit `(series_index, point_index, subcol)` tuples.
    pub fn set_selected_targets(&mut self, targets: Vec<(usize, usize, Option<usize>)>) {
        self.selected_points = targets.iter().map(|&(s, p, _)| (s, p)).collect();
        self.selected_subcols = targets.into_iter().map(|(_, _, sc)| sc).collect();
    }

    /// Apply field patch payload from UI widgets or `ui.answer_dialog`.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply field patch payload using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        let opt_patch_str = |val: &Value, key: &str| -> Result<Option<String>, String> {
            match val {
                Value::Null => Ok(None),
                Value::String(s) => Ok(Some(s.clone())),
                _ => Err(format!("`{key}` must be a string or null")),
            }
        };
        for (key, val) in fields {
            match key.as_str() {
                "points" | "selected_points" => {
                    if !val.is_null() {
                        let targets = crate::control_io::parse_selected_points_with_subcols(val, key)?;
                        self.set_selected_targets(targets);
                    }
                }
                "color" | "symbol_color" | "format_points.color" => self.color = opt_patch_str(val, key)?,
                "fill_color" | "format_points.fill_color" => self.fill_color = opt_patch_str(val, key)?,
                "symbol" | "shape" | "symbol_shape" | "format_points.symbol" => self.symbol = opt_patch_str(val, key)?,
                "size_pt" | "format_points.size_pt" => self.size_pt = parse_opt_finite_f64_with_locale(val, key, preset)?,
                "border_width_pt" | "stroke_width_pt" | "format_points.border_width_pt" => {
                    self.border_width_pt = parse_opt_finite_f64_with_locale(val, key, preset)?;
                }
                "fill_mode" | "format_points.fill_mode" => self.fill_mode = opt_patch_str(val, key)?,
                "hidden" | "format_points.hidden" => {
                    self.hidden = match val {
                        Value::Null => None,
                        Value::Bool(b) => Some(*b),
                        _ => return Err(format!("`{key}` must be a boolean or null")),
                    };
                }
                "show_row_labels" | "format_points.show_row_labels" => {
                    self.show_row_labels = val.as_bool().ok_or_else(|| format!("`{key}` must be a boolean"))?;
                }
                "clear" | "clear_overrides" | "format_points.clear" => {
                    self.clear_overrides = val.as_bool().ok_or_else(|| format!("`{key}` must be a boolean"))?;
                }
                other => return Err(format!("unknown Format Selected Points field `{other}`")),
            }
        }
        Ok(())
    }

    /// Build `graph.formatSelectedPoints` command intent across all selected points.
    #[must_use]
    pub fn build_commit_intent(&self) -> Option<ToolCommandIntent> {
        let sheet_id = self.graph_sheet_id.as_ref()?;
        if self.selected_points.is_empty() {
            return None;
        }
        let points_json: Vec<Value> = self
            .selected_points
            .iter()
            .enumerate()
            .map(|(idx, &(series_index, point_index))| {
                let mut entry = json!({ "series_index": series_index, "point_index": point_index });
                if let Some(Some(sc)) = self.selected_subcols.get(idx)
                    && let Some(obj) = entry.as_object_mut()
                {
                    obj.insert(String::from("subcol"), json!(sc));
                }
                entry
            })
            .collect();
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("sheet_id"), json!(sheet_id.as_str()));
        payload.insert(String::from("points"), Value::Array(points_json));
        if self.clear_overrides {
            payload.insert(String::from("mode"), json!("clear"));
            return Some(ToolCommandIntent::new("graph.formatSelectedPoints", Value::Object(payload)));
        }

        let mut has_patch = false;
        if let Some(ref c) = self.color {
            payload.insert(String::from("color"), json!(c));
            has_patch = true;
        }
        if let Some(ref fc) = self.fill_color {
            payload.insert(String::from("fill_color"), json!(fc));
            has_patch = true;
        }
        if let Some(ref sym) = self.symbol {
            payload.insert(String::from("symbol"), json!(sym));
            has_patch = true;
        }
        if let Some(sz) = self.size_pt {
            payload.insert(String::from("size_pt"), json!(sz));
            has_patch = true;
        }
        if let Some(bw) = self.border_width_pt {
            payload.insert(String::from("border_width_pt"), json!(bw));
            has_patch = true;
        }
        if let Some(ref fm) = self.fill_mode {
            payload.insert(String::from("fill_mode"), json!(fm));
            has_patch = true;
        }
        if let Some(h) = self.hidden {
            payload.insert(String::from("hidden"), json!(h));
            has_patch = true;
        }
        if !has_patch && !self.show_row_labels {
            return None;
        }
        if has_patch {
            payload.insert(String::from("mode"), json!("merge"));
            Some(ToolCommandIntent::new("graph.formatSelectedPoints", Value::Object(payload)))
        } else {
            None
        }
    }

    /// Build optional `graph.addAnnotation` intents displaying row labels when `show_row_labels` is true.
    #[must_use]
    pub fn build_row_label_intents(&self, doc: &ProjectDocument) -> Vec<ToolCommandIntent> {
        self.build_row_label_intents_for_scene(doc, None)
    }

    /// Build `graph.addAnnotation` row-label intents placed near each point's plotted coordinates
    /// (with deterministic collision-staggered fallback offsets when a point is off-scene).
    #[must_use]
    pub fn build_row_label_intents_for_scene(&self, doc: &ProjectDocument, scene: Option<&ChartScene>) -> Vec<ToolCommandIntent> {
        if !self.show_row_labels {
            return Vec::new();
        }
        let Some(ref graph_id) = self.graph_sheet_id else {
            return Vec::new();
        };
        let Some(gs) = doc.graph_sheet(graph_id) else {
            return Vec::new();
        };
        let ds = doc
            .data_sheet(&gs.spec.primary_source_sheet_id)
            .or_else(|| doc.result_sheet(&gs.spec.primary_source_sheet_id).and_then(|rs| rs.analyzable_data.as_ref()));
        let mut intents = Vec::new();
        let mut placed_positions: Vec<(f64, f64)> = Vec::new();
        for (idx, &(series_idx, point_idx)) in self.selected_points.iter().enumerate() {
            let subcol_filter = self.selected_subcols.get(idx).copied().flatten();
            let label =
                ds.and_then(|d| row_title_at(&d.archetype, point_idx)).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Row {}", point_idx + 1));
            let plotted = scene.and_then(|sc| {
                sc.plotted_points
                    .iter()
                    .find(|pt| {
                        pt.source_cell.row == point_idx
                            && pt.source_cell.col == series_idx
                            && subcol_filter.is_none_or(|s| pt.source_cell.subcol == s)
                    })
                    .or_else(|| sc.plotted_points.iter().find(|pt| pt.source_cell.row == point_idx))
            });
            let (mut x_pt, mut y_pt) = if let Some(pt) = plotted {
                (pt.center_pt.x + 8.0, (pt.center_pt.y - 18.0).max(4.0))
            } else {
                let col_slot = (idx % 3) as f64;
                let row_slot = (idx / 3) as f64;
                (72.0 + col_slot * 124.0, 48.0 + row_slot * 30.0)
            };
            while placed_positions.iter().any(|&(px, py)| (px - x_pt).abs() < 48.0 && (py - y_pt).abs() < 18.0) {
                y_pt += 20.0;
                if y_pt > 260.0 {
                    y_pt = 24.0;
                    x_pt += 88.0;
                }
            }
            placed_positions.push((x_pt, y_pt));
            intents.push(ToolCommandIntent::new(
                "graph.addAnnotation",
                json!({
                    "sheet_id": graph_id.as_str(),
                    "kind": "text",
                    "text": label,
                    "x_pt": x_pt,
                    "y_pt": y_pt,
                }),
            ));
        }
        intents
    }
}

fn row_title_at(arch: &TableArchetype, row: usize) -> Option<String> {
    match arch {
        TableArchetype::Xy(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Column(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Grouped(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Contingency(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Survival(t) => t.row_titles.get(row).cloned(),
        TableArchetype::PartsOfWhole(t) => t.slice_labels.get(row).cloned(),
        TableArchetype::MultipleVariables(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Nested(t) => t.row_titles.get(row).cloned(),
    }
}

/// Interactive state for the graph/layout canvas, non-modal inspectors, and parameter dialogs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CanvasDialogState {
    pub canvas: CanvasToolState,
    pub active_dialog: ActiveDialog,
    pub status_banner: Option<String>,
    pub active_inspector: ActiveInspector,
    pub analyze: AnalyzeDialogState,
    pub edit_params: EditAnalysisParametersState,
    pub change_data: ChangeDataAnalyzedState,
    pub result_view: ResultSheetViewState,
    pub hook_picker: ConstantHookPickerState,
    pub change_graph_type: ChangeGraphTypeState,
    pub format_graph: FormatGraphState,
    pub format_axes: FormatAxesState,
    pub pairwise: PairwiseComparisonsState,
    pub bubble_plot: BubblePlotInspectorState,
    pub apply_style: ApplyStyleFromState,
    pub page_layout: PageLayoutComposerState,
    pub export_figure: ExportFigureState,
    pub command_palette: CommandPaletteState,
    #[serde(default)]
    pub convert_archetype: ConvertArchetypeDialogState,
    #[serde(default)]
    pub exclude_with_reason: ExcludeWithReasonDialogState,
    #[serde(default)]
    pub active_canvas_preset: Option<FigureCanvasPreset>,
    #[serde(default)]
    pub canvas_resize_mode: CanvasResizeMode,
    #[serde(default)]
    pub preflight_issues: Vec<PreflightIssue>,
    #[serde(default)]
    pub rulers_visible: bool,
    #[serde(default)]
    pub ruler_unit: PhysicalUnit,
    #[serde(default)]
    pub journal_preset: Option<curvebook_tools::JournalWidthPreset>,
    #[serde(default)]
    pub pan_zoom: PanZoomInertiaState,
    #[serde(default)]
    pub print: PrintDialogState,
    #[serde(default)]
    pub power_wizard: PowerWizardState,
    #[serde(default)]
    pub format_points: FormatSelectedPointsState,
    #[serde(default)]
    pub randomize_groups: RandomizeGroupsDialogState,
    #[serde(default)]
    pub random_sample_rows: RandomSampleRowsDialogState,
    #[serde(default)]
    pub simulate_distributions: SimulateDistributionsDialogState,
    #[serde(default)]
    pub insert_model_equation: InsertModelEquationDialogState,
    #[serde(default)]
    pub feedback: crate::feedback::FeedbackDraft,
}

/// Interactive state for the `Print...` setup and OS print-spool dialog (`R5.4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintDialogState {
    pub job: crate::PrintJobSpec,
    pub copies: u16,
    pub selected_printer: Option<String>,
    pub discovered_printers: Vec<String>,
    pub last_spool_bytes: usize,
    pub last_status: Option<String>,
    #[serde(default)]
    pub submit_to_os: bool,
}

impl Default for PrintDialogState {
    fn default() -> Self {
        Self {
            job: crate::PrintJobSpec::default(),
            copies: 1,
            selected_printer: None,
            discovered_printers: Vec::new(),
            last_spool_bytes: 0,
            last_status: None,
            submit_to_os: false,
        }
    }
}

impl PrintDialogState {
    /// Initialize dialog settings targeting `active_sheet` (falling back to the first graph or
    /// layout sheet in `doc`) and refresh discovered OS print queues.
    pub fn open_for_document(&mut self, doc: &ProjectDocument, active_sheet: Option<&SheetId>, discovered_printers: Vec<String>) {
        let resolved_id = active_sheet
            .filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some())
            .cloned()
            .or_else(|| doc.layout_sheets.first().map(|ls| ls.id.clone()))
            .or_else(|| doc.graph_sheets.first().map(|gs| gs.id.clone()));

        if let Some(id) = &resolved_id {
            self.job.sheet_ids = vec![id.clone()];
            if let Some(ls) = doc.layout_sheet(id) {
                self.job.title = Some(ls.title.clone());
            } else if let Some(gs) = doc.graph_sheet(id) {
                self.job.title = Some(gs.title.clone());
            }
        } else {
            self.job.sheet_ids.clear();
            self.job.title = Some(doc.title.clone());
        }

        self.copies = self.copies.clamp(1, 999);
        self.submit_to_os = false;
        self.discovered_printers = discovered_printers;
        if let Some(sel) = &self.selected_printer
            && !self.discovered_printers.is_empty()
            && !self.discovered_printers.iter().any(|p| p == sel)
        {
            self.selected_printer = self.discovered_printers.first().cloned();
        }
        self.last_status = Some(match self.discovered_printers.len() {
            0 => String::from("Ready to spool PDF at 100% physical scale"),
            n => format!("{n} OS printer queue(s) discovered"),
        });
    }

    /// Update desired print copy count, clamping into `1..=999`.
    pub fn update_copies(&mut self, copies: u16) -> u16 {
        self.copies = copies.clamp(1, 999);
        self.copies
    }

    /// Select an OS printer queue name (or `None` for the OS default queue).
    pub fn select_printer(&mut self, printer: Option<String>) {
        self.selected_printer = printer.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
    }

    /// Update print scale percentage, clamping finite values into `10.0..=400.0`.
    pub fn set_scale_percent(&mut self, scale_percent: f64) {
        if scale_percent.is_finite() {
            self.job.scale_percent = scale_percent.clamp(10.0, 400.0);
        }
    }

    /// Patch `Print` modal fields (`crop_marks`, `bleed_pt`, `copies`, `scale_percent`, `printer`, `title`).
    ///
    /// # Errors
    /// Returns an error when any numeric field is out of valid range or has an invalid type.
    /// Apply modal field updates from `ui.answer_dialog` / widget controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal field updates using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "crop_marks" | "print.crop_marks" => {
                    self.job.crop_marks = val.as_bool().ok_or_else(|| String::from("`crop_marks` must be boolean"))?;
                }
                "bleed_pt" | "print.bleed_pt" => {
                    let bleed = parse_dialog_num_with_locale(val, key, preset)?;
                    if !(0.0..=144.0).contains(&bleed) {
                        return Err(format!("`bleed_pt` ({bleed}) must be within 0.0..=144.0"));
                    }
                    self.job.bleed_pt = bleed;
                }
                "copies" | "print.copies" => {
                    let raw = parse_dialog_num_with_locale(val, key, preset)?;
                    if raw < 1.0 || raw > 999.0 || (raw - raw.round()).abs() > 1e-9 {
                        return Err(format!("`copies` ({raw}) must be an integer within 1..=999"));
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let count = raw.round() as u16;
                    self.update_copies(count);
                }
                "scale_percent" | "print.scale_percent" => {
                    let scale = parse_dialog_num_with_locale(val, key, preset)?;
                    if !(10.0..=400.0).contains(&scale) {
                        return Err(format!("`scale_percent` ({scale}) must be within 10.0..=400.0"));
                    }
                    self.set_scale_percent(scale);
                }
                "selected_printer" | "printer" | "print.printer" | "print.selected_printer" => {
                    if val.is_null() {
                        self.select_printer(None);
                    } else {
                        let s = val.as_str().ok_or_else(|| String::from("`printer` must be a string or null"))?;
                        self.select_printer(Some(s.to_owned()));
                    }
                }
                "title" | "print.title" => {
                    if val.is_null() {
                        self.job.title = None;
                    } else {
                        let s = val.as_str().ok_or_else(|| String::from("`title` must be a string or null"))?;
                        self.job.title = (!s.trim().is_empty()).then(|| s.trim().to_owned());
                    }
                }
                "submit_to_os" | "print.submit_to_os" => {
                    self.submit_to_os = val.as_bool().ok_or_else(|| String::from("`submit_to_os` must be boolean"))?;
                }
                other => return Err(format!("unknown Print field `{other}`")),
            }
        }
        Ok(())
    }
}

impl CanvasDialogState {
    /// Compile the currently active graph or layout sheet into a [`ChartScene`], if applicable.
    pub fn active_scene(&self, session: &Session) -> Result<Option<ChartScene>, EngineError> {
        let Some(active_id) = &session.document().active_sheet_id else {
            return Ok(None);
        };
        if session.document().graph_sheet(active_id).is_none() && session.document().layout_sheet(active_id).is_none() {
            return Ok(None);
        }
        Ok(Some(session.render_sheet_scene(active_id)?))
    }

    /// Reset canvas zoom to fit the sheet cleanly (`ui.zoom.fitSheet`).
    pub fn reset_zoom_to_fit(&mut self) {
        self.canvas.set_zoom(1.0);
        self.canvas.pan_pt.x = 0.0;
        self.canvas.pan_pt.y = 0.0;
        self.pan_zoom = PanZoomInertiaState::default();
    }

    pub(crate) fn effective_zoom_and_pan(&self) -> (f64, [f64; 2]) {
        let zoom = if (self.pan_zoom.zoom - 1.0).abs() > 1e-9 { self.pan_zoom.zoom } else { self.canvas.zoom };
        let pan_x = if self.pan_zoom.pan_pt[0].abs() > 1e-9 { self.pan_zoom.pan_pt[0] } else { self.canvas.pan_pt.x };
        let pan_y = if self.pan_zoom.pan_pt[1].abs() > 1e-9 { self.pan_zoom.pan_pt[1] } else { self.canvas.pan_pt.y };
        (zoom.max(0.05), [pan_x, pan_y])
    }

    /// Open `Convert Table Archetype...` modal dialog (`QOL-1`).
    pub fn open_convert_archetype(&mut self, doc: &ProjectDocument, sheet_id: &SheetId) -> bool {
        let Some(ds) = doc.data_sheet(sheet_id) else {
            return false;
        };
        self.convert_archetype.open_for_sheet(ds);
        self.active_dialog = ActiveDialog::ConvertArchetype { sheet_id: sheet_id.clone() };
        true
    }

    /// Open `Import Reshape (Plate Map / Tidy Long)` modal dialog (`QOL-2`).
    pub fn open_import_reshape(&mut self) {
        self.active_dialog = ActiveDialog::ImportReshape;
    }

    /// Open `Exclude Selection with Reason...` (`Cmd+Shift+E`) modal dialog (`QOL-3`).
    pub fn open_exclude_with_reason(&mut self, sheet_id: SheetId, coords: Vec<GridCellCoord>) {
        self.exclude_with_reason.open_for_coords(sheet_id.clone(), coords);
        self.active_dialog = ActiveDialog::ExcludeWithReason { sheet_id };
    }

    /// Build `graph.applyCanvasPreset` or `layout.applyCanvasPreset` intent (`QOL-8`).
    #[must_use]
    pub fn apply_canvas_preset_intent(&mut self, sheet_id: &SheetId, is_layout: bool, preset: FigureCanvasPreset) -> ToolCommandIntent {
        self.active_canvas_preset = Some(preset);
        let cmd = if is_layout { "layout.applyCanvasPreset" } else { "graph.applyCanvasPreset" };
        ToolCommandIntent::new(
            cmd,
            json!({
                "sheet_id": sheet_id.as_str(),
                "preset": format!("{preset:?}"),
                "resize_mode": format!("{:?}", self.canvas_resize_mode),
            }),
        )
    }

    /// Refresh [`Self::preflight_issues`] directly from a compiled [`ChartScene`] (`QOL-8`).
    pub fn refresh_preflight(&mut self, scene: &ChartScene) {
        self.preflight_issues = curvebook_chart::preflight_check_scene(scene);
    }

    /// Synchronize inspector/dialog state when navigating to `sheet_id`.
    ///
    /// First-time visits to an uninitialized [`GraphSheet`] automatically present
    /// [`ActiveDialog::ChangeGraphType`].
    pub fn on_sheet_selected(&mut self, doc: &ProjectDocument, sheet_id: &SheetId) {
        if let Some(gs) = doc.graph_sheet(sheet_id) {
            self.format_graph.sync_from_graph(gs);
            self.format_axes.sync_from_graph(gs, AxisTarget::Y);
            self.pairwise.sync_from_graph(gs, doc);
            self.bubble_plot.sync_from_graph(gs, doc);
            if !gs.initialized {
                self.change_graph_type.open_for_graph(gs, doc);
                self.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: gs.id.clone() };
            }
            self.active_inspector =
                if matches!(gs.spec.kind, GraphKind::MultipleVariables) { ActiveInspector::BubblePlot } else { ActiveInspector::FormatGraph };
        } else if let Some(ls) = doc.layout_sheet(sheet_id) {
            self.page_layout.sync_from_layout(ls);
            self.active_inspector = ActiveInspector::PageLayout;
        } else if let Some(rs) = doc.result_sheet(sheet_id) {
            self.result_view.sync_with_result(rs);
            self.active_inspector = ActiveInspector::ResultDetails;
        } else if let Some(ds) = doc.data_sheet(sheet_id) {
            self.analyze.open_for_data_sheet(ds);
            self.active_inspector = ActiveInspector::None;
        }
    }

    /// Open standalone 4-stage Power & Sample Size Wizard (`WF-15`, `R8.7`).
    pub fn open_power_wizard(&mut self) {
        let _ = self.power_wizard.recalculate();
        self.active_dialog = ActiveDialog::PowerAndSampleSize;
    }

    /// Open `Format Selected Points...` dialog targeting `graph_sheet_id` and `points` (`WF-22`).
    pub fn open_format_selected_points(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId, points: Vec<(usize, usize)>) -> bool {
        if doc.graph_sheet(graph_sheet_id).is_none() {
            return false;
        }
        self.format_points.open_for_graph(graph_sheet_id.clone(), points);
        self.active_dialog = ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_sheet_id.clone() };
        true
    }

    /// Open `Format Selected Points...` dialog preserving optional replicate `subcol` per point (`R2-S08-03`).
    pub fn open_format_selected_points_with_subcols(
        &mut self,
        doc: &ProjectDocument,
        graph_sheet_id: &SheetId,
        targets: Vec<(usize, usize, Option<usize>)>,
    ) -> bool {
        if doc.graph_sheet(graph_sheet_id).is_none() {
            return false;
        }
        self.format_points.open_for_graph_with_subcols(graph_sheet_id.clone(), targets);
        self.active_dialog = ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_sheet_id.clone() };
        true
    }

    /// Open `Randomize Subjects to Groups...` dialog (`R9.14`, `table.randomizeGroups`).
    pub fn open_randomize_groups(&mut self, doc: &ProjectDocument) {
        self.randomize_groups.open_for_document(doc);
        self.active_dialog = ActiveDialog::RandomizeGroups;
    }

    /// Open `Random Sample Rows...` dialog (`R9.14`, `table.randomSampleRows`).
    pub fn open_random_sample_rows(&mut self, doc: &ProjectDocument, source_sheet_id: Option<&SheetId>) -> bool {
        let target_id = source_sheet_id.cloned().or_else(|| doc.data_sheets.first().map(|d| d.id.clone()));
        let Some(id) = target_id else {
            return false;
        };
        if !self.random_sample_rows.open_for_sheet(doc, &id) {
            return false;
        }
        self.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(id) };
        true
    }

    /// Open `Simulate Distributions...` dialog (`R9.14`, `table.simulateDistributions`).
    pub fn open_simulate_distributions(&mut self) {
        self.active_dialog = ActiveDialog::SimulateDistributions;
    }

    /// Open `Insert Model Equation...` dialog targeting `graph_sheet_id` (`R9.16`, strictly equations only).
    pub fn open_insert_model_equation(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.insert_model_equation.open_for_graph(gs, doc);
        self.active_dialog = ActiveDialog::InsertModelEquation { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Bubble Plot Channels` dialog & inspector targeting `graph_sheet_id` (`WF-44`).
    pub fn open_bubble_plot(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.bubble_plot.sync_from_graph(gs, doc);
        self.active_inspector = ActiveInspector::BubblePlot;
        self.active_dialog = ActiveDialog::BubblePlotInspector { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Format Graph` dialog & inspector targeting `graph_sheet_id` (`WF-45`).
    pub fn open_format_graph(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.format_graph.sync_from_graph_with_doc(gs, Some(doc));
        self.active_inspector = ActiveInspector::FormatGraph;
        self.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Pairwise Comparisons` dialog & inspector targeting `graph_sheet_id` (`WF-46`).
    pub fn open_pairwise_comparisons(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.pairwise.sync_from_graph(gs, doc);
        self.active_inspector = ActiveInspector::PairwiseComparisons;
        self.active_dialog = ActiveDialog::PairwiseComparisons { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Page Layout Composer` dialog & inspector targeting `layout_sheet_id` (`WF-48`).
    pub fn open_page_layout_composer(&mut self, doc: &ProjectDocument, layout_sheet_id: &SheetId) -> bool {
        let Some(ls) = doc.layout_sheet(layout_sheet_id) else {
            return false;
        };
        self.page_layout.sync_from_layout(ls);
        self.active_inspector = ActiveInspector::PageLayout;
        self.active_dialog = ActiveDialog::PageLayoutComposer { layout_sheet_id: ls.id.clone() };
        true
    }

    /// Open `Export Figure...` dialog targeting `sheet_id` (`WF-48`).
    pub fn open_export_figure(&mut self, sheet_id: &SheetId) {
        self.active_dialog = ActiveDialog::ExportFigure { sheet_id: sheet_id.clone() };
    }

    /// Open `Analyze...` for `source_sheet_id` (`DataSheet`, analyzable `ResultSheet`, or tabular `ResultSheet`).
    pub fn open_analyze_dialog(&mut self, doc: &ProjectDocument, source_sheet_id: &SheetId) -> bool {
        if let Some(ds) = doc.data_sheet(source_sheet_id) {
            self.analyze.open_for_data_sheet(ds);
            self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: ds.id.clone() };
            return true;
        }
        if let Some(rs) = doc.result_sheet(source_sheet_id) {
            if let Some(analyzable) = &rs.analyzable_data {
                self.analyze.open_for_data_sheet(analyzable);
                self.analyze.source_sheet_id = Some(rs.id.clone());
                self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: rs.id.clone() };
                return true;
            }
            let active_tab = self
                .result_view
                .active_tab(rs)
                .filter(|t| !t.column_headers.is_empty() && !t.rows.is_empty())
                .or_else(|| rs.tabs.iter().find(|t| !t.column_headers.is_empty() && !t.rows.is_empty()));
            if let Some(tab) = active_tab {
                self.analyze.open_for_result_tab(rs, tab);
                self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: rs.id.clone() };
                return true;
            }
        }
        false
    }

    /// Open parameter re-entry (`Cmd+T`) for `result_sheet_id`.
    pub fn open_edit_parameters(&mut self, doc: &ProjectDocument, result_sheet_id: &SheetId) -> bool {
        let Some(rs) = doc.result_sheet(result_sheet_id) else {
            return false;
        };
        self.edit_params.open_for_result(rs);
        self.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: rs.id.clone() };
        true
    }

    /// Open `Change Data Analyzed` cross-table retargeting for `result_sheet_id`.
    pub fn open_change_data_analyzed(&mut self, doc: &ProjectDocument, result_sheet_id: &SheetId) -> bool {
        let Some(rs) = doc.result_sheet(result_sheet_id) else {
            return false;
        };
        self.change_data.open_for_result(rs, doc);
        self.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id: rs.id.clone() };
        true
    }

    /// Open the constant hook picker targeting `target_sheet_id`.
    pub fn open_hook_picker(&mut self, doc: &ProjectDocument, target_sheet_id: &SheetId, param_name: Option<String>) {
        self.hook_picker.open_for_document(target_sheet_id.clone(), param_name, doc);
        self.active_dialog = ActiveDialog::ConstantHookPicker { target_sheet_id: target_sheet_id.clone() };
    }

    /// Open `Apply Style From...` for `target_graph_id`.
    pub fn open_apply_style_from(&mut self, doc: &ProjectDocument, target_graph_id: &SheetId) -> bool {
        if doc.graph_sheet(target_graph_id).is_none() {
            return false;
        }
        self.apply_style.open_for_graph(target_graph_id, doc);
        self.active_dialog = ActiveDialog::ApplyStyleFrom { target_graph_id: target_graph_id.clone() };
        true
    }

    /// Open `Cmd+K` Command Palette.
    pub fn open_command_palette(&mut self) {
        self.command_palette.open();
        self.active_dialog = ActiveDialog::CommandPalette;
    }

    /// Open `Print...` dialog for `sheet_id` (or first printable sheet in `doc`).
    pub fn open_print_dialog(&mut self, doc: &ProjectDocument, sheet_id: Option<&SheetId>, discovered_printers: Vec<String>) {
        self.print.open_for_document(doc, sheet_id, discovered_printers);
        let target = self.print.job.sheet_ids.first().cloned().or_else(|| sheet_id.cloned());
        self.active_dialog = ActiveDialog::Print { sheet_id: target };
    }

    /// Confirm `Analyze...`, opening [`ActiveDialog::DuplicateAnalysisPrompt`] if a matching
    /// analysis already exists on the source sheet.
    pub fn submit_analyze_or_prompt(&mut self, doc: &ProjectDocument) -> Option<ToolCommandIntent> {
        match self.analyze.try_build_run_intent(doc) {
            Ok(intent) => {
                self.active_dialog = ActiveDialog::None;
                Some(intent)
            }
            Err(existing_result_ids) => {
                if let (Some(src), Some(entry_id)) = (&self.analyze.source_sheet_id, &self.analyze.selected_entry_id) {
                    self.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
                        source_sheet_id: src.clone(),
                        existing_result_ids,
                        pending_entry_id: entry_id.clone(),
                    };
                }
                None
            }
        }
    }

    /// Dismiss any open modal dialog.
    pub fn close_dialog(&mut self) {
        self.active_dialog = ActiveDialog::None;
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, ConstantValue, DataSheet, GraphSheet, InfoConstant, InfoSheet,
        JournalWidthPreset, LayoutSheet, LayoutSpec, NonlinearFitSpec, OneWayAnovaSpec, PValueFormat, ProjectDocument, RemediationKind, ResultCell,
        ResultSheet, ResultStatus, TableArchetype, TwoGroupSpec, XyTable,
    };

    use super::{
        ActiveDialog, AssumptionDiagnosticBanner, AxisTarget, CanvasDialogState, CommandPaletteState, ConstantHookPickerState, FormatAxesState,
        HookTargetKind, ResultSheetViewState,
    };
    use crate::theme::{UiThemeSpec, wcag_contrast_ratio};

    #[test]
    fn command_palette_includes_both_engine_and_ui_commands() {
        let mut palette = CommandPaletteState { query: String::from("nav"), ..Default::default() };
        let hits = palette.matches();
        assert!(hits.iter().any(|h| h.id == "ui.navigator.toggle"));

        palette.query = String::from("exportPng");
        let hits = palette.matches();
        assert_eq!(hits.first().map(|h| h.id), Some("graph.exportPng"));
    }

    #[test]
    fn result_cell_formatting_handles_open_ci_bounds_and_pvalues() {
        let ci = ResultSheetViewState::format_result_cell(
            &ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.5), upper: CiBound::Open },
            PValueFormat::FourDecimal,
        );
        assert!(ci.contains("Unbounded"));

        let p = ResultSheetViewState::format_result_cell(
            &ResultCell::PValue { raw: 0.0021, formatted: String::from("0.0021"), stars: String::from("**") },
            PValueFormat::Apa,
        );
        assert!(p.contains(".002"));
        assert!(p.contains("**"));
    }

    #[test]
    fn format_axes_enforces_one_to_three_segments() {
        let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4).expect("xy");
        let ds = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).expect("ds");
        let gs = GraphSheet::new_uninitialized_for_data("g1", &ds).expect("gs");
        let mut axes = FormatAxesState::default();
        axes.sync_from_graph(&gs, AxisTarget::Y);
        assert!(!axes.remove_segment(0));
        assert!(axes.add_segment());
        assert!(axes.add_segment());
        assert!(!axes.add_segment());
        assert_eq!(axes.axis_draft.segments.len(), 3);
    }

    #[test]
    fn duplicate_prompt_and_checklist_counts_work() {
        let mut doc = ProjectDocument::default();
        let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4).expect("xy");
        let ds = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).expect("ds");
        doc.add_data_sheet(ds).expect("add data");

        let spec = AnalysisSpec::new("r1", "d1", AnalysisKind::LinearRegression(Default::default())).expect("spec");
        let mut res = ResultSheet::new(spec).expect("result");
        res.status = ResultStatus::UpToDate;
        res.checklist.push(ChecklistItem {
            assumption: String::from("Normality"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(String::from("p=0.04")),
            explanation: String::from("Mild skew"),
            remediation: None,
        });
        assert_eq!(ResultSheetViewState::checklist_summary_counts(&res), (0, 1, 0, 0));
        doc.add_result_sheet(res).expect("add result");

        let mut dialogs = CanvasDialogState::default();
        assert!(dialogs.open_analyze_dialog(&doc, &"d1".into()));
        dialogs.analyze.selected_entry_id = Some(String::from("fit.linear"));
        assert!(dialogs.submit_analyze_or_prompt(&doc).is_none());
        assert!(matches!(dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }));
    }

    #[test]
    fn theme_specs_meet_wcag_aa_across_all_themes() {
        for theme in [UiThemeSpec::light(), UiThemeSpec::dark(), UiThemeSpec::high_contrast()] {
            assert!(wcag_contrast_ratio(theme.border, theme.panel_bg) >= 3.0, "border vs panel_bg failed for {:?}", theme.mode);
            assert!(wcag_contrast_ratio(theme.border, theme.surface) >= 3.0, "border vs surface failed for {:?}", theme.mode);
            assert!(wcag_contrast_ratio(theme.text_primary, theme.surface) >= 4.5, "text_primary vs surface failed for {:?}", theme.mode);
        }
    }

    #[test]
    fn constant_hook_picker_emits_engine_compatible_commands_by_target() {
        let mut doc = ProjectDocument::default();
        let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 4).expect("xy");
        let ds = DataSheet::new("d1", "Assay", TableArchetype::Xy(xy)).expect("ds");
        doc.add_data_sheet(ds).expect("add data");
        let mut info = InfoSheet::new_preseeded("info-1", "Protocol").expect("info");
        info.constants.push(InfoConstant::new("Baseline_EC50", ConstantValue::Number(12.5), Some(String::from("nM"))).expect("const"));
        doc.add_info_sheet(info).expect("add info");

        let fit_spec =
            AnalysisSpec::new("r1", "d1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).expect("fit spec");
        doc.add_result_sheet(ResultSheet::new(fit_spec).expect("fit res")).expect("add fit");
        let gs = GraphSheet::new_uninitialized_for_data("g1", doc.data_sheet(&"d1".into()).expect("d1")).expect("gs");
        doc.add_graph_sheet(gs).expect("add gs");
        let layout = LayoutSheet::new("l1", "Fig 1", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm)).expect("layout");
        doc.add_layout_sheet(layout).expect("add layout");

        let mut picker = ConstantHookPickerState::default();
        picker.open_for_document("r1".into(), Some(String::from("Bottom")), &doc);
        assert_eq!(picker.target_kind, HookTargetKind::AnalysisParameter);
        let res_intent = picker.commit_intent().expect("res hook");
        assert_eq!(res_intent.command_id, "analysis.updateParams");
        assert_eq!(res_intent.params["sheet_id"], "r1");
        assert!(res_intent.params["parameter_rules"].is_array());

        picker.open_for_document("g1".into(), None, &doc);
        assert_eq!(picker.target_kind, HookTargetKind::GraphTextBox);
        let graph_intent = picker.commit_intent().expect("graph hook");
        assert_eq!(graph_intent.command_id, "graph.addAnnotation");
        assert_eq!(graph_intent.params["sheet_id"], "g1");

        picker.open_for_document("l1".into(), None, &doc);
        assert_eq!(picker.target_kind, HookTargetKind::LayoutTextCallout);
        let layout_intent = picker.commit_intent().expect("layout hook");
        assert_eq!(layout_intent.command_id, "layout.addPanel");
        assert_eq!(layout_intent.params["sheet_id"], "l1");
    }

    #[test]
    fn assumption_banner_is_diagnostics_only_without_remediation_or_prescriptions() {
        let tg_spec = AnalysisSpec::new("res-t", "d1", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).expect("spec");
        let mut res = ResultSheet::new(tg_spec).expect("result");
        res.status = ResultStatus::UpToDate;
        res.tabs.push(curvebook_doc::ResultTab {
            id: String::from("diagnostics"),
            title: String::from("Diagnostics"),
            kind: curvebook_doc::ResultTabKind::Diagnostics,
            column_headers: vec![String::from("Check")],
            row_labels: Vec::new(),
            rows: Vec::new(),
            footnotes: Vec::new(),
        });
        res.checklist = vec![
            ChecklistItem {
                assumption: String::from("Variance homogeneity"),
                status: ChecklistStatus::Violated,
                diagnostic_metric: Some(String::from("F = 8.42, p = 0.008")),
                explanation: String::from("Sample variances differ materially across groups."),
                remediation: Some(RemediationKind::WelchT),
            },
            ChecklistItem {
                assumption: String::from("Normality"),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some(String::from("p = 0.03")),
                explanation: String::from("Moderate right skew observed."),
                remediation: Some(RemediationKind::MannWhitneyU),
            },
        ];
        let banner = AssumptionDiagnosticBanner::from_result(&res).expect("banner from Violated check");
        assert_eq!(banner.severity, ChecklistStatus::Violated);
        assert_eq!(banner.diagnostic_metric.as_deref(), Some("F = 8.42, p = 0.008"));
        assert!(banner.limitation_note.contains("Sample variances differ"));
        assert_eq!(banner.detail_tab_ids, vec![String::from("diagnostics")]);

        // Prescriptive phrases inside raw item explanations are neutralized (QOL-7).
        res.checklist = vec![ChecklistItem {
            assumption: String::from("Normality"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(String::from("W = 0.81, p = 0.02")),
            explanation: String::from("Switch to Mann-Whitney U or transform data or exclude outlier."),
            remediation: Some(RemediationKind::MannWhitneyU),
        }];
        let neutralized = AssumptionDiagnosticBanner::from_result(&res).expect("neutralized banner");
        assert!(!neutralized.limitation_note.to_ascii_lowercase().contains("mann-whitney"));
        assert!(!neutralized.limitation_note.to_ascii_lowercase().contains("exclude outlier"));

        // Inconclusive / small-N `ChecklistStatus::Info` items also surface diagnostics-only banners,
        // whereas routine `Pass` items yield `None`.
        let anova_spec = AnalysisSpec::new("res-a", "d1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("anova spec");
        let mut anova_res = ResultSheet::new(anova_spec).expect("anova res");
        anova_res.checklist = vec![ChecklistItem {
            assumption: String::from("Normality test power"),
            status: ChecklistStatus::Pass,
            diagnostic_metric: Some(String::from("p = 0.42")),
            explanation: String::from("Passed Shapiro-Wilk."),
            remediation: None,
        }];
        assert!(AssumptionDiagnosticBanner::from_result(&anova_res).is_none());

        anova_res.checklist = vec![ChecklistItem {
            assumption: String::from("Normality check"),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(String::from("n = 4")),
            explanation: String::from("Sample size is too small (n < 5) for reliable normality testing."),
            remediation: None,
        }];
        let info_banner = AssumptionDiagnosticBanner::from_result(&anova_res).expect("small-N Info banner");
        assert_eq!(info_banner.severity, ChecklistStatus::Info);
        assert!(info_banner.limitation_note.contains("too small"));
    }

    #[test]
    fn command_palette_surfaces_summary_beginner_and_session_enabled() {
        let palette = CommandPaletteState { query: String::from("analyze.run"), ..CommandPaletteState::default() };
        let session = super::Session::new();
        let rows = palette.preview_rows(Some(&session), 5);
        assert!(rows.len() >= 3);
        let hits = palette.matches_for_session(&session);
        let top = hits.first().expect("expected analyze.run match");
        assert_eq!(top.id, "analyze.run");
        assert!(!top.summary.is_empty());
        assert!(!top.beginner.is_empty());
        assert!(top.format_palette_row().contains(top.summary));
        assert!(top.format_palette_row().contains(top.beginner));
        assert!(rows.iter().any(|row| row.contains(top.summary)));
        assert!(rows.iter().any(|row| row.contains(top.beginner)));

        let axes_disabled = {
            let axes_palette = CommandPaletteState { query: String::from("graph.setAxes"), ..CommandPaletteState::default() };
            axes_palette.matches_for_session(&session)
        };
        assert!(!axes_disabled.first().expect("graph.setAxes entry").enabled);
        assert!(axes_disabled.first().expect("graph.setAxes entry").format_palette_row().contains("[disabled]"));
    }

    #[test]
    fn w6_pcr_catalog_r10_leakage_and_mv_response_default() -> Result<(), String> {
        use super::{ANALYSIS_CATALOG, AnalyzeDialogState, collect_variable_candidates, default_analysis_kind_for_id};
        use curvebook_doc::{MultivariateSpec, VariableRole};

        let pcr = ANALYSIS_CATALOG.iter().find(|c| c.id == "stats.pcr").ok_or("missing stats.pcr in ANALYSIS_CATALOG")?;
        assert!(pcr.label.contains("Principal Component Regression"));
        assert!(matches!(default_analysis_kind_for_id("stats.pcr"), Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { .. }))));

        let mut mv = curvebook_doc::MultipleVariablesTable::new(5, 4).map_err(|e| e.to_string())?;
        mv.variables[0].role = VariableRole::Label;
        mv.variables[1].role = VariableRole::Categorical;
        mv.variables[2].role = VariableRole::Continuous;
        mv.variables[3].role = VariableRole::Continuous;
        mv.variables[4].role = VariableRole::Categorical;
        for r in 0..4 {
            mv.variables[2].cells[r] = curvebook_doc::Cell::number((r + 1) as f64);
            mv.variables[3].cells[r] = curvebook_doc::Cell::number((r + 1) as f64 * 2.0);
        }
        let ds = DataSheet::new("mv-pcr", "MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
        let cands = collect_variable_candidates(&ds);
        assert_eq!(cands.iter().find(|c| c.is_response).map(|c| c.col_index), Some(3));

        let mut an = AnalyzeDialogState::default();
        an.open_for_data_sheet(&ds);
        an.select_catalog_entry("stats.pcr");
        assert!(an.selected_data_sets.is_empty());
        an.toggle_pca_predictor(2);
        assert!(an.select_pcr_outcome(2).is_err());
        an.select_pcr_outcome(3)?;
        assert_eq!(an.pcr_outcome_column, Some(3));
        Ok(())
    }

    #[test]
    fn w6_heatmap_commit_intents_and_grouped_superimposed() -> Result<(), String> {
        use super::{ChangeGraphTypeState, DiagnosticPlotKind, EditAnalysisParametersState, GRAPH_TYPE_OPTIONS, GraphKind};
        use curvebook_doc::{GraphSpec, GroupedArrangement, GroupedTable, MultivariateSpec, SheetId};
        use serde_json::json;

        assert!(GRAPH_TYPE_OPTIONS.iter().any(|o| o.id == "grouped.superimposed"));
        let mut cgt = ChangeGraphTypeState { selected_option_id: String::from("grouped.superimposed"), ..ChangeGraphTypeState::default() };
        cgt.graph_sheet_id = Some(SheetId::from("g-1"));
        let grp = GroupedTable::new(curvebook_doc::SubcolumnMode::RawReplicates(2), 2, 3).map_err(|e| e.to_string())?;
        let ds_g = DataSheet::new("ds-g", "Grouped", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?;
        let mut spec = GraphSpec::default_for_data_sheet(&ds_g);
        assert!(cgt.apply_to_spec(&mut spec));
        assert_eq!(spec.grouped_arrangement, GroupedArrangement::Superimposed);

        let mut ep = EditAnalysisParametersState::default();
        let analysis =
            AnalysisSpec::new("res-corr", "mv-1", AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }))
                .map_err(|e| e.to_string())?;
        ep.result_sheet_id = Some(SheetId::from("res-corr"));
        ep.draft = Some(analysis);
        ep.correlation_all_pairs = true;
        ep.correlation_heatmap = true;

        let primary = ep.commit_intent().ok_or("missing commit_intent")?;
        assert_eq!(primary.params["correlation_all_pairs"], json!(true));
        assert_eq!(primary.params["correlation_heatmap"], json!(true));

        let intents = ep.commit_intents(None);
        assert_eq!(intents.len(), 2);
        assert_eq!(intents[1].command_id, "graph.create");
        assert_eq!(intents[1].params["kind"], json!(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)));
        Ok(())
    }
}
