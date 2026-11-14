//! Graph/layout canvas interaction controller, scientific analysis/formatting dialog state
//! machines, Command Palette (`Cmd+K`) fuzzy matching, and deterministic software frame
//! compositing (`RasterImage` / PNG) for headless screenshots and visual regression tests.

use lumencraft_chart::{ChartScene, PreflightIssue};
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, AxisSpec, AxisThreshold, BaselineOperation, BlandAltmanSpec, BracketLabelMode, BracketStyle,
    CanvasResizeMode, ChecklistItem, ChecklistStatus, CiBound, ColorChannelMapping, ColumnDesignHint, ConnectingLineStyle, ContingencySpec,
    CoxTieMethod, DataSheet, DescriptiveSpec, DiagnosticPlotKind, ErrorBarMetric, ErrorBarSpec, ErrorPropagationMode, FactorialAnovaSpec,
    FigureCanvasPreset, FramePreset, GlmFamily, GlmSpec, GraphAxisScale, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement,
    HookSource, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LegendSpec, LineConnectionKind, LinearRegressionSpec, MathTransformPreset,
    MultipleRowTestsSpec, MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec,
    NormalizeAnchor, OneWayAnovaSpec, OutlierSpec, PValueFormat, PageOrientation, PairwiseBracketSpec, PanelBadgeStyle, ParameterRule,
    ParameterScope, PartsOfWholeLayout, PassingBablokSpec, ProjectDocument, RectPt, ReplicateOverlayStyle, ResultCell, ResultSheet, ResultTab,
    RocSpec, SheetId, SizeChannelMapping, SummaryMeasure, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupSpec,
    VariableRole, WhiskerMethod,
};
use lumencraft_engine::{EngineError, Session, cmd};
use lumencraft_palette::{ColorRamp, ColormapId, CvdKind, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8, simulate_cvd};
use lumencraft_render::TiffCompression;
use lumencraft_tools::{CanvasToolState, GridCellCoord, PanZoomInertiaState, PhysicalUnit, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::menus::ui_commands::UI_COMMANDS;
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
    CommandPalette,
    About,
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
        keywords: "cox proportional hazards hazard ratio survival efron breslow",
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
        "transform.select" => {
            Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula: None, target_archetype: TableKind::Column }))
        }
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
            noise: lumencraft_doc::SimulationNoise::Gaussian { sd: 4.0 },
        })),
        "stats.descriptive" => Some(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())),
        "stats.outliers" => Some(AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })),
        "stats.two_group" => Some(AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())),
        "stats.one_way_anova" => Some(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())),
        "stats.nested" => {
            Some(AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: false, multiplicity: MultiplicitySpec::default() }))
        }
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
        "stats.pca" => Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(7) })),
        "stats.corr_matrix" => Some(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false })),
        "stats.clustering" => Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: Some(3) })),
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

/// Interactive state for the searchable `Analyze...` dialog (`docs/ui-parity.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnalyzeDialogState {
    pub source_sheet_id: Option<SheetId>,
    pub source_table_kind: Option<TableKind>,
    pub search_query: String,
    pub category_filter: Option<AnalysisCategory>,
    pub selected_entry_id: Option<String>,
    pub selected_data_sets: Vec<usize>,
    pub variable_candidates: Vec<VariableCandidate>,
    pub role_filter: VariableRoleFilter,
    pub equation_id: String,
}

impl AnalyzeDialogState {
    /// Populate defaults and variable candidates for `sheet`.
    pub fn open_for_data_sheet(&mut self, sheet: &DataSheet) {
        self.source_sheet_id = Some(sheet.id.clone());
        self.source_table_kind = Some(sheet.kind());
        self.search_query.clear();
        self.category_filter = None;
        self.selected_data_sets.clear();
        self.role_filter = VariableRoleFilter::All;
        if self.equation_id.is_empty() {
            self.equation_id = String::from("log_agonist_vs_response_4pl");
        }
        self.variable_candidates = collect_variable_candidates(sheet);
        let first_id = self.compatible_entries().first().map(|item| item.id.to_owned());
        self.selected_entry_id = first_id;
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
            AnalysisKind::NonlinearRegression(fit) => {
                if !self.equation_id.trim().is_empty() {
                    fit.equation_id = self.equation_id.trim().to_owned();
                }
            }
            AnalysisKind::GeneralizedLinearModel(glm) => {
                if let Some(resp) = response_col {
                    glm.response_column = resp;
                }
                if !predictor_cols.is_empty() {
                    glm.predictor_columns = predictor_cols;
                }
            }
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. }) => {
                if let Some(resp) = response_col {
                    *time_column = resp;
                }
                if let Some(&first_pred) = predictor_cols.first() {
                    *status_column = first_pred;
                }
                if predictor_cols.len() > 1 {
                    *predictor_columns = predictor_cols.get(1..).map_or_else(Vec::new, |s| s.to_vec());
                }
            }
            AnalysisKind::FactorialAnova(fac) if !predictor_cols.is_empty() => {
                fac.multifactor_columns = predictor_cols;
                fac.response_column = response_col;
            }
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, predictor_columns, .. }) => {
                *response_column = response_col;
                if !predictor_cols.is_empty() {
                    *predictor_columns = predictor_cols;
                }
            }
            _ => {}
        }
        Some(kind)
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
        TableArchetype::MultipleVariables(mv) => mv
            .variables
            .iter()
            .enumerate()
            .map(|(col_index, var)| VariableCandidate {
                col_index,
                title: var.title.clone(),
                role: var.role,
                role_inferred: var.role_inferred,
                selected: true,
                is_response: col_index == 0,
            })
            .collect(),
        _ => (0..sheet.archetype.column_count())
            .map(|col_index| VariableCandidate {
                col_index,
                title: format!("Col {}", lumencraft_doc::default_column_label(col_index)),
                role: VariableRole::Continuous,
                role_inferred: true,
                selected: true,
                is_response: false,
            })
            .collect(),
    }
}

/// `Cmd+T` parameter re-entry draft editing an existing [`AnalysisSpec`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EditAnalysisParametersState {
    pub result_sheet_id: Option<SheetId>,
    pub draft: Option<AnalysisSpec>,
    pub title_was_custom: bool,
}

impl EditAnalysisParametersState {
    /// Load an existing [`ResultSheet`] for parameter re-entry (`Cmd+T`).
    pub fn open_for_result(&mut self, result: &ResultSheet) {
        self.result_sheet_id = Some(result.id.clone());
        self.title_was_custom = result.analysis.custom_title.is_some();
        self.draft = Some(result.analysis.clone());
    }

    /// Mutate [`AnalysisKind`] in-place, updating `auto_title` while preserving any user-pinned
    /// `custom_title`.
    pub fn set_kind(&mut self, new_kind: AnalysisKind) -> bool {
        let Some(spec) = &mut self.draft else {
            return false;
        };
        spec.update_kind(new_kind);
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
        Some(ToolCommandIntent::new(
            "analysis.updateParams",
            json!({
                "sheet_id": id.as_str(),
                "kind": spec.kind,
                "confidence_percent": spec.confidence_level.percent(),
                "selected_data_sets": spec.selected_data_sets,
                "p_value_format": spec.p_value_format,
                "custom_title": spec.custom_title,
            }),
        ))
    }
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
                    "text": opt.template_token,
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
                    label: format!("Column Title [{}]: Col {} = {title}", ds.title, lumencraft_doc::default_column_label(col_idx)),
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
        graph_kind: GraphKind::Contingency,
        thumbnail: ThumbnailGlyphKind::StackedBar,
        compatible_tables: &[TableKind::Grouped, TableKind::Contingency],
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
];

/// First-visit `Change Graph Type` chooser state (`docs/graphing-and-layouts.md` §2).
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
        spec.kind = opt.graph_kind;
        if opt.id == "grouped.stacked" {
            spec.grouped_arrangement = GroupedArrangement::Stacked;
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
            if matches!(series.gdo, GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::ScatterAndLine { .. }) {
                series.y_error_bars = Some(ErrorBarSpec { metric: self.error_bar_metric, ..ErrorBarSpec::default() });
            }
        }
        true
    }

    /// Emit `graph.setKind` (and mark sheet initialized) command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let opt = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == self.selected_option_id)?;
        Some(ToolCommandIntent::new(
            "graph.setKind",
            json!({
                "sheet_id": id.as_str(),
                "option_id": opt.id,
                "kind": opt.graph_kind,
                "reset_series_style": true,
            }),
        ))
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
        "col.aligned_dot" => GraphicalDataObject::AlignedDotPlot { summary, marker, show_crossbar: true },
        "col.bar_replicates" | "grouped.interleaved" | "grouped.stacked" => GraphicalDataObject::BarWithReplicates {
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
    pub selected_series_index: usize,
    pub error_bar_enabled: bool,
    pub error_bar_spec: ErrorBarSpec,
    pub marker_size_pt: f64,
    pub marker_fill_mode: MarkerFillMode,
    pub line_dash: LineDashStyle,
    pub show_legend: bool,
    pub legend_bordered: bool,
}

impl Default for FormatGraphState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            palette: PaletteId::OkabeIto,
            cvd_preview: None,
            frame_preset: FramePreset::OpenL,
            grouped_arrangement: GroupedArrangement::Interleaved,
            selected_series_index: 0,
            error_bar_enabled: true,
            error_bar_spec: ErrorBarSpec::default(),
            marker_size_pt: 5.0,
            marker_fill_mode: MarkerFillMode::Filled,
            line_dash: LineDashStyle::Solid,
            show_legend: true,
            legend_bordered: false,
        }
    }
}

impl FormatGraphState {
    /// Populate draft controls from an existing [`GraphSheet`].
    pub fn sync_from_graph(&mut self, graph: &GraphSheet) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.palette = graph.spec.palette;
        self.frame_preset = graph.spec.frame_preset;
        self.grouped_arrangement = graph.spec.grouped_arrangement;
        self.show_legend = graph.spec.legend.visible;
        self.legend_bordered = graph.spec.legend.bordered;
        let series_idx = self.selected_series_index.min(graph.spec.series.len().saturating_sub(1));
        self.selected_series_index = series_idx;
        if let Some(series) = graph.spec.series.get(series_idx) {
            self.error_bar_enabled = series.y_error_bars.is_some();
            self.error_bar_spec = series.y_error_bars.unwrap_or_default();
            self.extract_series_marker_and_dash(&series.gdo);
        }
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
        spec.legend = LegendSpec { visible: self.show_legend, bordered: self.legend_bordered, ..spec.legend };
        if let Some(series) = spec.series.get_mut(self.selected_series_index) {
            series.y_error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
            let swatch = self.palette.color_at(self.selected_series_index);
            apply_marker_and_dash(&mut series.gdo, swatch, self.marker_size_pt, self.marker_fill_mode, self.line_dash);
        }
    }

    /// Emit declarative `graph.setSpec` and `graph.setSeries` engine command intents.
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let Some(id) = &self.graph_sheet_id else {
            return Vec::new();
        };
        let error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
        vec![
            ToolCommandIntent::new(
                "graph.setSpec",
                json!({
                    "sheet_id": id.as_str(),
                    "palette": self.palette,
                    "frame_preset": self.frame_preset,
                    "grouped_arrangement": self.grouped_arrangement,
                    "legend": { "visible": self.show_legend, "bordered": self.legend_bordered },
                }),
            ),
            ToolCommandIntent::new(
                "graph.setSeries",
                json!({
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
                }),
            ),
        ]
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
}

impl Default for FormatAxesState {
    fn default() -> Self {
        Self { graph_sheet_id: None, target_axis: AxisTarget::Y, enable_y2: false, axis_draft: AxisSpec::linear("Y") }
    }
}

impl FormatAxesState {
    /// Load axis `target` (`X`, `Y`, or `Y2`) from `graph`.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, target: AxisTarget) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.target_axis = target;
        self.enable_y2 = graph.spec.y2_axis.is_some();
        self.axis_draft = match target {
            AxisTarget::X => graph.spec.x_axis.clone(),
            AxisTarget::Y => graph.spec.y_axis.clone(),
            AxisTarget::Y2 => graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2")),
        };
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
        Ok(())
    }

    /// Emit `graph.setAxes` intent carrying axis target, validated segments, and break ornament.
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
        Some(ToolCommandIntent::new("graph.setAxes", Value::Object(payload)))
    }
}

/// Live Pairwise Comparison Bracket inspector (`Auto-add from analysis`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseComparisonsState {
    pub graph_sheet_id: Option<SheetId>,
    pub selected_result_sheet_id: Option<SheetId>,
    pub available_result_ids: Vec<SheetId>,
    pub bracket_style: BracketStyle,
    pub label_mode: BracketLabelMode,
    pub hide_non_significant: bool,
    pub brackets: Vec<PairwiseBracketSpec>,
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
        }
    }
}

impl PairwiseComparisonsState {
    /// Synchronize brackets and discover upstream [`ResultSheet`]s carrying pairwise comparisons.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.brackets = graph.spec.comparisons.clone();
        self.available_result_ids = doc.result_sheets.iter().filter(|rs| !rs.pairwise_comparisons.is_empty()).map(|rs| rs.id.clone()).collect();
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
        added
    }

    /// Apply current bracket settings and items back onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.comparisons = self.brackets.clone();
    }

    /// Emit `graph.addComparisonBrackets` intent matching `cmd_graph_add_comparison_brackets`.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
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
    pub shortcut: &'static str,
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

    /// Format compact command palette row copy surfacing `label`, `id`, and `summary`.
    #[must_use]
    pub fn format_compact_row(&self) -> String {
        let status = if self.enabled { "" } else { " [disabled]" };
        format!("{} ({}){} — {}", self.label, self.id, status, self.summary)
    }

    /// Format compact command palette row copy using the localized label for `locale`.
    #[must_use]
    pub fn format_compact_row_in_locale(&self, locale: &str) -> String {
        let status = if self.enabled { "" } else { " [disabled]" };
        format!("{} ({}){} — {}", self.localized_label(locale), self.id, status, self.summary)
    }

    /// Format rich command palette row copy surfacing `label`, `id`, `shortcut`, `summary`, and `beginner`.
    #[must_use]
    pub fn format_palette_row(&self) -> String {
        self.format_palette_row_in_locale("en")
    }

    /// Format rich localized command palette row copy surfacing `summary` and `beginner`.
    #[must_use]
    pub fn format_palette_row_in_locale(&self, locale: &str) -> String {
        let status = if self.enabled { "" } else { " [disabled]" };
        let title = self.localized_label(locale);
        if self.shortcut.is_empty() {
            format!("{title} ({}){} — {} | Tip: {}", self.id, status, self.summary, self.beginner)
        } else {
            format!("{title} ({}) [{}]{} — {} | Tip: {}", self.id, self.shortcut, status, self.summary, self.beginner)
        }
    }
}

/// Command Palette (`Cmd+K`) fuzzy search state spanning Engine + UI commands.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandPaletteState {
    pub query: String,
    pub selected_index: usize,
}

impl CommandPaletteState {
    /// Reset search text and selection upon opening `Cmd+K`.
    pub fn open(&mut self) {
        self.query.clear();
        self.selected_index = 0;
    }

    /// Return fuzzy-filtered and relevance-sorted commands from both Engine and UI catalogs.
    #[must_use]
    pub fn matches(&self) -> Vec<CommandPaletteEntry> {
        self.collect_matches(None)
    }

    /// Return fuzzy-filtered and relevance-sorted commands with `enabled` evaluated for `session`.
    #[must_use]
    pub fn matches_for_session(&self, session: &Session) -> Vec<CommandPaletteEntry> {
        self.collect_matches(Some(session))
    }

    /// Format visible command palette rows surfacing `summary` and active `beginner` guidance.
    #[must_use]
    pub fn preview_rows(&self, session: Option<&Session>, max_rows: usize) -> Vec<String> {
        self.preview_rows_for_locale(session, "en", max_rows)
    }

    /// Format localized visible command palette rows surfacing `summary` and active `beginner` guidance.
    #[must_use]
    pub fn preview_rows_for_locale(&self, session: Option<&Session>, locale: &str, max_rows: usize) -> Vec<String> {
        let hits = match session {
            Some(s) => self.matches_for_session(s),
            None => self.matches(),
        };
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

    fn collect_matches(&self, session: Option<&Session>) -> Vec<CommandPaletteEntry> {
        let q = self.query.trim();
        let mut results = Vec::new();
        for spec in cmd::registry().specs() {
            let base = score_command(q, spec.id, spec.label, spec.menu_path, spec.shortcut);
            if let Some(score) = score_palette_entry(q, base, spec.summary, spec.beginner) {
                let enabled = session.is_none_or(|s| (spec.enabled)(s));
                results.push(CommandPaletteEntry {
                    id: spec.id,
                    label: spec.label,
                    menu_path: spec.menu_path,
                    shortcut: spec.shortcut,
                    params_doc: spec.summary,
                    summary: spec.summary,
                    beginner: spec.beginner,
                    enabled,
                    origin: CommandOrigin::Engine,
                    score,
                });
            }
        }
        for spec in UI_COMMANDS {
            let base = score_command(q, spec.id, spec.label, "View", spec.shortcut);
            if let Some(score) = score_palette_entry(q, base, spec.summary, spec.beginner) {
                results.push(CommandPaletteEntry {
                    id: spec.id,
                    label: spec.label,
                    menu_path: "UI",
                    shortcut: spec.shortcut,
                    params_doc: spec.params_doc,
                    summary: spec.summary,
                    beginner: spec.beginner,
                    enabled: true,
                    origin: CommandOrigin::Ui,
                    score,
                });
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
        &[TableKind::Column, TableKind::Grouped, TableKind::Xy, TableKind::MultipleVariables]
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
        self.preview_ready = false;
        self.preview_lossy = false;
        self.preserved_cells = 0;
        self.dropped_cells = 0;
        self.preserved_exclusions = 0;
        self.preview_warnings.clear();
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

        let secondary = self.secondary_sheet_id.clone().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).or_else(|| {
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

pub use lumencraft_doc::RemediationKind;

/// Interactive assumption warning strip displayed above a [`ResultSheet`] when diagnostics flag
/// unequal variance or non-normality (`QOL-7`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssumptionDiagnosticBanner {
    pub source_result_id: SheetId,
    pub severity: ChecklistStatus,
    pub summary: String,
    pub recommendation_label: String,
    pub remediation_kind: RemediationKind,
}

impl AssumptionDiagnosticBanner {
    /// Derive an actionable remediation banner from structured `ChecklistItem::remediation` diagnostics.
    ///
    /// Never modifies `rs`; remediation creates a fresh non-destructive sibling analysis sheet.
    #[must_use]
    pub fn from_result(rs: &ResultSheet) -> Option<Self> {
        let supports_sibling = matches!(&rs.analysis.kind, AnalysisKind::TwoGroupComparison(_) | AnalysisKind::OneWayAnova(_));
        if !supports_sibling {
            return None;
        }
        let flagged = rs
            .checklist
            .iter()
            .filter(|c| matches!(c.status, ChecklistStatus::Violated | ChecklistStatus::Warn) && c.remediation.is_some())
            .max_by_key(|c| if c.status == ChecklistStatus::Violated { 2u8 } else { 1u8 })?;
        let remediation_kind = flagged.remediation?;

        Some(Self {
            source_result_id: rs.id.clone(),
            severity: flagged.status,
            summary: format!("{}: {}", flagged.assumption, flagged.explanation),
            recommendation_label: format!("Run {} (sibling)", remediation_kind.label()),
            remediation_kind,
        })
    }

    /// Build a non-destructive `analysis.run` intent creating a new sibling [`ResultSheet`] (`sibling_id`).
    #[must_use]
    pub fn build_sibling_analysis_intent(&self, original: &ResultSheet, sibling_id: &SheetId) -> ToolCommandIntent {
        let title = format!("{} ({})", original.title(), self.remediation_kind.label());
        let mut params = json!({
            "sheet_id": sibling_id.as_str(),
            "source_sheet": original.analysis.source_sheet_id.as_str(),
            "kind": self.remediation_kind.engine_kind(),
            "title": title,
            "confidence_percent": original.analysis.confidence_level.percent(),
            "overwrite": false,
        });
        if !original.analysis.selected_data_sets.is_empty()
            && let Some(obj) = params.as_object_mut()
        {
            obj.insert(String::from("selected_data_sets"), json!(original.analysis.selected_data_sets));
        }
        ToolCommandIntent::new("analysis.run", params)
    }
}

// ============================================================================
// Unified CanvasDialogState
// ============================================================================

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
    pub journal_preset: Option<lumencraft_tools::JournalWidthPreset>,
    #[serde(default)]
    pub pan_zoom: PanZoomInertiaState,
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
        self.preflight_issues = lumencraft_chart::preflight_check_scene(scene);
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

    /// Open `Analyze...` for `source_sheet_id` if it is a `DataSheet` (or analyzable `ResultSheet`).
    pub fn open_analyze_dialog(&mut self, doc: &ProjectDocument, source_sheet_id: &SheetId) -> bool {
        if let Some(ds) = doc.data_sheet(source_sheet_id) {
            self.analyze.open_for_data_sheet(ds);
            self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: ds.id.clone() };
            return true;
        }
        if let Some(rs) = doc.result_sheet(source_sheet_id)
            && let Some(analyzable) = &rs.analyzable_data
        {
            self.analyze.open_for_data_sheet(analyzable);
            self.analyze.source_sheet_id = Some(rs.id.clone());
            self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: rs.id.clone() };
            return true;
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

pub use crate::compositor::{WorkspaceFrameLayout, WorkspacePixelRect, compute_workspace_layout, render_workspace_frame_png, render_workspace_rgba};

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, ConstantValue, DataSheet, GraphSheet, InfoConstant, InfoSheet,
        JournalWidthPreset, LayoutSheet, LayoutSpec, NonlinearFitSpec, OneWayAnovaSpec, PValueFormat, ProjectDocument, RemediationKind, ResultCell,
        ResultSheet, ResultStatus, TableArchetype, TwoGroupSpec, XyTable,
    };

    use super::{
        ActiveDialog, AssumptionDiagnosticBanner, AxisTarget, CanvasDialogState, CommandPaletteState, ConstantHookPickerState, FormatAxesState,
        HookTargetKind, ResultSheetViewState,
    };
    use crate::compositor::FramePalette;
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
        let xy = XyTable::new(lumencraft_doc::SubcolumnMode::Single, 1, 4).expect("xy");
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
        let xy = XyTable::new(lumencraft_doc::SubcolumnMode::Single, 1, 4).expect("xy");
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
    fn frame_palette_meets_wcag_aa_across_all_themes() {
        for theme in [UiThemeSpec::light(), UiThemeSpec::dark(), UiThemeSpec::high_contrast()] {
            let pal = FramePalette::from_theme(&theme);
            assert!(wcag_contrast_ratio(pal.border, pal.panel) >= 3.0, "border vs panel failed for {:?}", theme.mode);
            assert!(wcag_contrast_ratio(pal.border, pal.surface) >= 3.0, "border vs surface failed for {:?}", theme.mode);
            assert!(wcag_contrast_ratio(pal.text, pal.surface) >= 4.5, "text vs surface failed for {:?}", theme.mode);
        }
    }

    #[test]
    fn constant_hook_picker_emits_engine_compatible_commands_by_target() {
        let mut doc = ProjectDocument::default();
        let xy = XyTable::new(lumencraft_doc::SubcolumnMode::Single, 1, 4).expect("xy");
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
    fn assumption_banner_reads_structured_remediation_ignoring_checklist_wording() {
        let tg_spec = AnalysisSpec::new("res-t", "d1", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).expect("spec");
        let mut res = ResultSheet::new(tg_spec).expect("result");
        res.status = ResultStatus::UpToDate;
        // Reworded non-English text without any English keywords ("variance", "levene", "welch")
        // should still resolve directly from `ChecklistItem::remediation`.
        res.checklist = vec![
            ChecklistItem {
                assumption: String::from("Streuung der Stichproben"),
                status: ChecklistStatus::Violated,
                diagnostic_metric: Some(String::from("F = 8.42, p = 0.008")),
                explanation: String::from("Die Gruppen zeigen stark unterschiedliche Streuungen."),
                remediation: Some(RemediationKind::WelchT),
            },
            ChecklistItem {
                assumption: String::from("Normalverteilung"),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some(String::from("p = 0.03")),
                explanation: String::from("Schiefe Verteilung."),
                remediation: Some(RemediationKind::MannWhitneyU),
            },
        ];
        let banner = AssumptionDiagnosticBanner::from_result(&res).expect("banner from Violated remediation");
        assert_eq!(banner.remediation_kind, RemediationKind::WelchT);

        // Switching the checklist wording while carrying `MannWhitneyU` selects `MannWhitneyU`.
        res.checklist = vec![ChecklistItem {
            assumption: String::from("Even if text mentions variance or Levene or Brown-Forsythe"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: None,
            explanation: String::from("Text wording must not override structured remediation."),
            remediation: Some(RemediationKind::MannWhitneyU),
        }];
        let banner_mw = AssumptionDiagnosticBanner::from_result(&res).expect("banner from structured field");
        assert_eq!(banner_mw.remediation_kind, RemediationKind::MannWhitneyU);

        // Conversely, an ANOVA checklist item with keywords but `remediation: None` yields no banner.
        let anova_spec = AnalysisSpec::new("res-a", "d1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("anova spec");
        let mut anova_res = ResultSheet::new(anova_spec).expect("anova res");
        anova_res.checklist = vec![ChecklistItem {
            assumption: String::from("Equal variance check with Welch keyword"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: None,
            explanation: String::from("Informational only when remediation is None."),
            remediation: None,
        }];
        assert!(AssumptionDiagnosticBanner::from_result(&anova_res).is_none());

        anova_res.checklist[0].remediation = Some(RemediationKind::KruskalWallis);
        assert_eq!(AssumptionDiagnosticBanner::from_result(&anova_res).expect("kruskal wallis").remediation_kind, RemediationKind::KruskalWallis);
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
}
