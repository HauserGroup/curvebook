//! Graph/layout canvas interaction controller, scientific analysis/formatting dialog state
//! machines, Command Palette (`Cmd+K`) fuzzy matching, and deterministic software frame
//! compositing (`RasterImage` / PNG) for headless screenshots and visual regression tests.

use lumencraft_chart::ChartScene;
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, AxisSpec, AxisThreshold, BaselineOperation, BlandAltmanSpec, BracketLabelMode, BracketStyle,
    ChecklistItem, ChecklistStatus, CiBound, ColorChannelMapping, ColumnDesignHint, ConnectingLineStyle, ContingencySpec, CoxTieMethod, DataSheet,
    DescriptiveSpec, DiagnosticPlotKind, ErrorBarMetric, ErrorBarSpec, ErrorPropagationMode, FactorialAnovaSpec, FramePreset, GlmFamily, GlmSpec,
    GraphAxisScale, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, HookSource, InfoSheet, JournalWidthPreset, LayoutItem,
    LayoutSheet, LayoutSpec, LegendSpec, LineConnectionKind, LinearRegressionSpec, LinkBorderColor, MathTransformPreset, MultipleRowTestsSpec,
    MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, OutlierSpec, PValueFormat, PageOrientation, PairwiseBracketSpec, PanelBadgeStyle, ParameterRule, ParameterScope,
    PartsOfWholeLayout, PassingBablokSpec, ProjectDocument, RectPt, ReplicateOverlayStyle, ResultCell, ResultSheet, ResultTab, RocSpec, SheetId,
    SheetSection, SizeChannelMapping, SummaryMeasure, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupSpec,
    VariableRole, WhiskerMethod,
};
use lumencraft_engine::{EngineError, Session, catalog};
use lumencraft_palette::{ColorRamp, ColormapId, CvdKind, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8, simulate_cvd};
use lumencraft_render::{RasterImage, RenderError, RenderTargetSpec, TiffCompression, encode_png, render_scene};
use lumencraft_text::{TextDirection, reorder_visual_bidi};
use lumencraft_tools::{CanvasToolState, PanZoomInertiaState, PhysicalUnit, ToolCommandIntent, build_ruler_ticks};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::menus::ui_commands::UI_COMMANDS;
use crate::theme::UiThemeSpec;
use crate::workspace_nav::{ActiveWorkspacePane, PortfolioCard};

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

fn format_compact_f64(v: f64) -> String {
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

fn data_column_title(sheet: &DataSheet, col_idx: usize) -> Option<&str> {
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
    pub origin: CommandOrigin,
    pub score: i32,
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
        let q = self.query.trim();
        let mut results = Vec::new();
        for cmd in catalog::COMMANDS {
            if let Some(score) = score_command(q, cmd.id, cmd.label, cmd.menu_path, cmd.shortcut) {
                results.push(CommandPaletteEntry {
                    id: cmd.id,
                    label: cmd.label,
                    menu_path: cmd.menu_path,
                    shortcut: cmd.shortcut,
                    params_doc: cmd.params_doc,
                    origin: CommandOrigin::Engine,
                    score,
                });
            }
        }
        for &(id, label, shortcut, params_doc) in UI_COMMANDS {
            if let Some(score) = score_command(q, id, label, "View", shortcut) {
                results.push(CommandPaletteEntry { id, label, menu_path: "UI", shortcut, params_doc, origin: CommandOrigin::Ui, score });
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(b.id)));
        results
    }

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

    fn effective_zoom_and_pan(&self) -> (f64, [f64; 2]) {
        let zoom = if (self.pan_zoom.zoom - 1.0).abs() > 1e-9 { self.pan_zoom.zoom } else { self.canvas.zoom };
        let pan_x = if self.pan_zoom.pan_pt[0].abs() > 1e-9 { self.pan_zoom.pan_pt[0] } else { self.canvas.pan_pt.x };
        let pan_y = if self.pan_zoom.pan_pt[1].abs() > 1e-9 { self.pan_zoom.pan_pt[1] } else { self.canvas.pan_pt.y };
        (zoom.max(0.05), [pan_x, pan_y])
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

// ============================================================================
// Deterministic Software Frame Compositor (RasterImage + PNG for ui.screenshot)
// ============================================================================

/// Integer pixel rectangle inside a composited workspace frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct WorkspacePixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl WorkspacePixelRect {
    #[must_use]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    #[must_use]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }
}

/// Partitioned pixel geometry of every chrome and canvas region in a workspace screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkspaceFrameLayout {
    pub total_width: u32,
    pub total_height: u32,
    pub toolbar: WorkspacePixelRect,
    pub navigator: WorkspacePixelRect,
    pub canvas: WorkspacePixelRect,
    pub inspector: WorkspacePixelRect,
    pub status_bar: WorkspacePixelRect,
    pub modal_overlay: Option<WorkspacePixelRect>,
}

/// Compute deterministic UI region bounds for a frame of `(width_px, height_px)`.
///
/// Mirrors horizontal Navigator / Canvas / Inspector panes via [`crate::i18n::mirror_x_for_rtl`]
/// when `app.locale` resolves to a right-to-left script (`ar`, `he`, `fa`, `ur`).
#[must_use]
pub fn compute_workspace_layout(app: &UiApp, width_px: u32, height_px: u32) -> WorkspaceFrameLayout {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let toolbar_h = (height_px / 14).clamp(20, 38).min(height_px);
    let status_h = (height_px / 22).clamp(16, 26).min(height_px.saturating_sub(toolbar_h));
    let mid_y = toolbar_h;
    let mid_h = height_px.saturating_sub(toolbar_h).saturating_sub(status_h);

    let nav_w = if app.nav.navigator_open && width_px >= 320 { (width_px / 5).clamp(136, 240) } else { 0 };
    let insp_w = if app.nav.inspector_open && width_px >= 420 { (width_px / 4).clamp(176, 280) } else { 0 };
    let canvas_w = width_px.saturating_sub(nav_w).saturating_sub(insp_w);

    let nav_x = crate::i18n::mirror_x_for_rtl(0, nav_w, width_px, is_rtl);
    let canvas_x = crate::i18n::mirror_x_for_rtl(nav_w, canvas_w, width_px, is_rtl);
    let insp_x = crate::i18n::mirror_x_for_rtl(nav_w.saturating_add(canvas_w), insp_w, width_px, is_rtl);

    let modal_overlay = if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        let mw = (width_px * 3 / 5).clamp(180, width_px.saturating_sub(24).max(1));
        let mh = (height_px * 3 / 5).clamp(120, height_px.saturating_sub(24).max(1));
        let mx = crate::i18n::mirror_x_for_rtl((width_px.saturating_sub(mw)) / 2, mw, width_px, is_rtl);
        Some(WorkspacePixelRect::new(mx, (height_px.saturating_sub(mh)) / 2, mw, mh))
    } else {
        None
    };

    WorkspaceFrameLayout {
        total_width: width_px,
        total_height: height_px,
        toolbar: WorkspacePixelRect::new(0, 0, width_px, toolbar_h),
        navigator: WorkspacePixelRect::new(nav_x, mid_y, nav_w, mid_h),
        canvas: WorkspacePixelRect::new(canvas_x, mid_y, canvas_w, mid_h),
        inspector: WorkspacePixelRect::new(insp_x, mid_y, insp_w, mid_h),
        status_bar: WorkspacePixelRect::new(0, toolbar_h.saturating_add(mid_h), width_px, status_h),
        modal_overlay,
    }
}

/// Render the full application frame (chrome, active graph/layout/grid/result view, inspector,
/// status bar, and open modal overlay) deterministically into an RGBA [`RasterImage`].
pub fn render_workspace_rgba(app: &UiApp, target: RenderTargetSpec) -> Result<RasterImage, RenderError> {
    let mut img = RasterImage::new(target.width_px, target.height_px, target.dpi, app.theme.surface_bg)?;
    let layout = compute_workspace_layout(app, target.width_px, target.height_px);
    let palette = FramePalette::from_theme(&app.theme);

    draw_toolbar(&mut img, layout.toolbar, app, &palette);
    if layout.navigator.width > 0 {
        draw_navigator(&mut img, layout.navigator, app, &palette);
    }
    let viewport_rect = if app.canvas_dialogs.rulers_visible && layout.canvas.width >= 48 && layout.canvas.height >= 48 {
        draw_physical_rulers(&mut img, layout.canvas, app, &palette)
    } else {
        layout.canvas
    };
    draw_center_viewport(&mut img, viewport_rect, app, target.dpi, &palette);
    if layout.inspector.width > 0 {
        draw_inspector_pane(&mut img, layout.inspector, app, &palette);
    }
    draw_status_bar(&mut img, layout.status_bar, app, &palette);
    draw_active_pane_focus(&mut img, &layout, app, &palette);
    if let Some(modal_rect) = layout.modal_overlay {
        draw_modal_overlay(&mut img, layout.canvas, modal_rect, app, &palette);
    }
    Ok(img)
}

/// Render the full application frame deterministically and encode it as PNG bytes (`ui.screenshot`).
pub fn render_workspace_frame_png(app: &UiApp, target: RenderTargetSpec) -> Result<Vec<u8>, RenderError> {
    let image = render_workspace_rgba(app, target)?;
    encode_png(&image)
}

#[derive(Clone, Copy)]
struct FramePalette {
    panel: Rgba8,
    surface: Rgba8,
    text: Rgba8,
    muted: Rgba8,
    accent: Rgba8,
    header: Rgba8,
    border: Rgba8,
    canvas_mat: Rgba8,
    excluded: Rgba8,
    link_blue: Rgba8,
    link_green: Rgba8,
    link_red: Rgba8,
}

impl FramePalette {
    fn from_theme(theme: &UiThemeSpec) -> Self {
        let is_dark = theme.surface_bg.relative_luminance() < 0.25;
        let delta = if is_dark { 14 } else { -12 };
        let mat_delta = if is_dark { -8 } else { -18 };
        Self {
            panel: theme.panel_bg,
            surface: theme.surface_bg,
            text: theme.text_primary,
            muted: theme.text_muted,
            accent: theme.accent,
            header: shift_luma(theme.panel_bg, delta),
            border: theme.border,
            canvas_mat: shift_luma(theme.surface_bg, mat_delta),
            excluded: theme.excluded_cell_text,
            link_blue: theme.link_blue,
            link_green: theme.link_green,
            link_red: theme.link_red,
        }
    }
}

fn shift_luma(color: Rgba8, delta: i16) -> Rgba8 {
    let shift = |ch: u8| -> u8 { i16::from(ch).saturating_add(delta).clamp(0, 255) as u8 };
    Rgba8::rgba(shift(color.r), shift(color.g), shift(color.b), color.a)
}

fn draw_toolbar(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.header);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.bottom().saturating_sub(1), rect.width, 1), pal.border);

    let brand_w = 56u32.min(rect.width.saturating_sub(8));
    let brand_local_x = crate::i18n::mirror_x_for_rtl(8, brand_w, rect.width, is_rtl);
    let brand_rect = WorkspacePixelRect::new(rect.x.saturating_add(brand_local_x), rect.y + 5, brand_w, rect.height.saturating_sub(10).max(1));
    fill_rect(img, brand_rect, pal.accent);
    draw_text_5x7_dir(img, brand_rect.x + 6, brand_rect.y + 4, "LUMEN", Rgba8::WHITE, is_rtl);

    let reserved_buttons_px = draw_toolbar_buttons(img, rect, app, pal, is_rtl);
    let title = app.document().active_sheet_id.as_ref().map_or(app.document().title.as_str(), |id| active_sheet_title(app.document(), id));
    let (title_x, title_max_w) = if is_rtl {
        let left_bound = rect.x.saturating_add(reserved_buttons_px).saturating_add(8);
        let right_bound = brand_rect.x.saturating_sub(12);
        (left_bound, right_bound.saturating_sub(left_bound))
    } else {
        let left_bound = brand_rect.right().saturating_add(12);
        let right_bound = rect.right().saturating_sub(reserved_buttons_px).saturating_sub(8);
        (left_bound, right_bound.saturating_sub(left_bound))
    };
    if title_max_w >= 24 {
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: title_x, y: rect.y + 10, text: title, color: pal.text, max_width_px: title_max_w, rtl: is_rtl },
        );
    }
}

fn draw_toolbar_buttons(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette, is_rtl: bool) -> u32 {
    let buttons = [
        ("Analyze", matches!(app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. })),
        ("Axes", matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. })),
        ("Cmd+K", matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette)),
    ];
    let min_gap_guard = 80u32;
    let mut rx = rect.width.saturating_sub(12);
    for (label, active) in buttons.into_iter().rev() {
        let bw = ((label.len() as u32) * 6 + 14).min(80);
        if rx < min_gap_guard.saturating_add(bw) {
            break;
        }
        rx = rx.saturating_sub(bw);
        let btn_local_x = crate::i18n::mirror_x_for_rtl(rx, bw, rect.width, is_rtl);
        let btn = WorkspacePixelRect::new(rect.x.saturating_add(btn_local_x), rect.y + 5, bw, rect.height.saturating_sub(10).max(1));
        fill_rect(img, btn, if active { pal.accent } else { pal.surface });
        stroke_rect(img, btn, pal.border);
        draw_text_5x7_dir(img, btn.x + 6, btn.y + 4, label, if active { Rgba8::WHITE } else { pal.text }, is_rtl);
        rx = rx.saturating_sub(6);
    }
    rect.width.saturating_sub(rx)
}

fn active_sheet_title<'a>(doc: &'a ProjectDocument, id: &'a SheetId) -> &'a str {
    doc.data_sheet(id)
        .map(|s| s.title.as_str())
        .or_else(|| doc.info_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.result_sheet(id).map(|s| s.title()))
        .or_else(|| doc.graph_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.layout_sheet(id).map(|s| s.title.as_str()))
        .unwrap_or(id.as_str())
}

fn draw_navigator(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.panel);
    let border_x = if is_rtl { rect.x } else { rect.right().saturating_sub(1) };
    fill_rect(img, WorkspacePixelRect::new(border_x, rect.y, 1, rect.height), pal.border);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + 8,
            text: "PROJECT NAVIGATOR",
            color: pal.muted,
            max_width_px: rect.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );

    let list_bottom = rect.y + (rect.height * 2 / 3);
    let mut y = rect.y + 24;
    for entry in &app.nav.entries {
        if y + 18 >= list_bottom {
            break;
        }
        let row = WorkspacePixelRect::new(rect.x + 6, y, rect.width.saturating_sub(12), 16);
        if entry.is_active {
            fill_rect(img, row, pal.accent);
        }
        let tag = match entry.section {
            SheetSection::Data => "D",
            SheetSection::Info => "I",
            SheetSection::Analyses => "A",
            SheetSection::Graphs => "G",
            SheetSection::Layouts => "L",
        };
        let fg = if entry.is_active { Rgba8::WHITE } else { pal.text };
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: row.x + 4,
                y: row.y + 4,
                text: &format!("[{tag}] {}", entry.title),
                color: fg,
                max_width_px: row.width.saturating_sub(8),
                rtl: is_rtl,
            },
        );
        y = y.saturating_add(18);
    }

    let fam_y = list_bottom.saturating_add(4);
    fill_rect(img, WorkspacePixelRect::new(rect.x + 6, fam_y, rect.width.saturating_sub(12), 1), pal.border);
    if fam_y + 18 < rect.bottom() {
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: rect.x + 10,
                y: fam_y + 6,
                text: "FAMILY",
                color: pal.muted,
                max_width_px: rect.width.saturating_sub(20),
                rtl: is_rtl,
            },
        );
        let mut fy = fam_y + 20;
        for rel in &app.nav.family_related_ids {
            if fy + 12 >= rect.bottom() {
                break;
            }
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 12,
                    y: fy,
                    text: rel.as_str(),
                    color: pal.link_blue,
                    max_width_px: rect.width.saturating_sub(20),
                    rtl: is_rtl,
                },
            );
            fy = fy.saturating_add(13);
        }
    }
}

fn draw_physical_rulers(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) -> WorkspacePixelRect {
    let ruler_h = 16u32;
    let ruler_w = 24u32;
    let corner = WorkspacePixelRect::new(rect.x, rect.y, ruler_w, ruler_h);
    let top_bar = WorkspacePixelRect::new(rect.x.saturating_add(ruler_w), rect.y, rect.width.saturating_sub(ruler_w), ruler_h);
    let left_bar = WorkspacePixelRect::new(rect.x, rect.y.saturating_add(ruler_h), ruler_w, rect.height.saturating_sub(ruler_h));
    let inner = WorkspacePixelRect::new(
        rect.x.saturating_add(ruler_w),
        rect.y.saturating_add(ruler_h),
        rect.width.saturating_sub(ruler_w),
        rect.height.saturating_sub(ruler_h),
    );

    fill_rect(img, corner, pal.header);
    fill_rect(img, top_bar, pal.header);
    fill_rect(img, left_bar, pal.header);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y.saturating_add(ruler_h).saturating_sub(1), rect.width, 1), pal.border);
    fill_rect(img, WorkspacePixelRect::new(rect.x.saturating_add(ruler_w).saturating_sub(1), rect.y, 1, rect.height), pal.border);

    let unit_tag = match app.canvas_dialogs.ruler_unit {
        PhysicalUnit::Millimeters => "MM",
        PhysicalUnit::Inches => "IN",
        PhysicalUnit::Points => "PT",
    };
    draw_text_5x7(img, corner.x + 4, corner.y + 4, unit_tag, pal.accent);

    let (doc_w_pt, doc_h_pt) = active_ruler_doc_span_pt(app);
    let (zoom, pan_pt) = app.canvas_dialogs.effective_zoom_and_pan();
    draw_horizontal_ruler_ticks(img, top_bar, app.canvas_dialogs.ruler_unit, doc_w_pt, zoom, pan_pt[0], pal);
    draw_vertical_ruler_ticks(img, left_bar, app.canvas_dialogs.ruler_unit, doc_h_pt, zoom, pan_pt[1], pal);
    inner
}

fn active_ruler_doc_span_pt(app: &UiApp) -> (f64, f64) {
    if let Some(preset) = app.canvas_dialogs.journal_preset {
        return preset.size_pt();
    }
    let doc = app.document();
    if let Some(active_id) = &doc.active_sheet_id
        && let Some(ls) = doc.layout_sheet(active_id)
    {
        let page = ls.spec.page_size_pt();
        if page.width > 0.0 && page.height > 0.0 {
            return (page.width, page.height);
        }
    }
    lumencraft_tools::JournalWidthPreset::DoubleColumn174Mm.size_pt()
}

fn ruler_px_offset(view_px: f64, span_px: u32) -> Option<u32> {
    if !view_px.is_finite() || view_px < 0.0 || span_px == 0 {
        return None;
    }
    let max_off = f64::from(span_px.saturating_sub(1));
    if view_px > max_off {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(view_px.round() as u32)
}

fn draw_horizontal_ruler_ticks(
    img: &mut RasterImage,
    bar: WorkspacePixelRect,
    unit: PhysicalUnit,
    doc_span_pt: f64,
    zoom: f64,
    pan_pt: f64,
    pal: &FramePalette,
) {
    if bar.width == 0 {
        return;
    }
    let px_per_pt = (f64::from(bar.width) / doc_span_pt.max(1.0)).max(0.05);
    let ticks = build_ruler_ticks(unit, doc_span_pt, zoom, pan_pt, px_per_pt);
    for tick in ticks {
        let Some(off_x) = ruler_px_offset(tick.view_px, bar.width) else {
            continue;
        };
        let tick_h = if tick.major { 7 } else { 3 };
        let color = if tick.major { pal.text } else { pal.muted };
        fill_rect(img, WorkspacePixelRect::new(bar.x.saturating_add(off_x), bar.bottom().saturating_sub(tick_h + 1), 1, tick_h), color);
        if let Some(label) = tick.label.as_deref()
            && off_x.saturating_add(10) < bar.width
        {
            draw_text_clipped(
                img,
                bar.x.saturating_add(off_x).saturating_add(2),
                bar.y + 2,
                label,
                pal.text,
                bar.width.saturating_sub(off_x + 3).min(24),
            );
        }
    }
}

fn draw_vertical_ruler_ticks(
    img: &mut RasterImage,
    bar: WorkspacePixelRect,
    unit: PhysicalUnit,
    doc_span_pt: f64,
    zoom: f64,
    pan_pt: f64,
    pal: &FramePalette,
) {
    if bar.height == 0 {
        return;
    }
    let px_per_pt = (f64::from(bar.height) / doc_span_pt.max(1.0)).max(0.05);
    let ticks = build_ruler_ticks(unit, doc_span_pt, zoom, pan_pt, px_per_pt);
    for tick in ticks {
        let Some(off_y) = ruler_px_offset(tick.view_px, bar.height) else {
            continue;
        };
        let tick_w = if tick.major { 7 } else { 3 };
        let color = if tick.major { pal.text } else { pal.muted };
        fill_rect(img, WorkspacePixelRect::new(bar.right().saturating_sub(tick_w + 1), bar.y.saturating_add(off_y), tick_w, 1), color);
        if let Some(label) = tick.label.as_deref()
            && off_y.saturating_add(8) < bar.height
        {
            draw_text_clipped(img, bar.x + 2, bar.y.saturating_add(off_y).saturating_add(2), label, pal.text, bar.width.saturating_sub(10));
        }
    }
}

fn draw_center_viewport(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, dpi: u16, pal: &FramePalette) {
    fill_rect(img, rect, pal.canvas_mat);
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    if app.nav.gallery.open {
        draw_portfolio_gallery(img, rect, app, pal, is_rtl);
        return;
    }
    let doc = app.document();
    let Some(active_id) = &doc.active_sheet_id else {
        draw_empty_studio(img, rect, pal, is_rtl);
        return;
    };

    if doc.graph_sheet(active_id).is_some() || doc.layout_sheet(active_id).is_some() {
        draw_scene_viewport(img, rect, app, active_id, dpi, pal);
    } else if let Some(ds) = doc.data_sheet(active_id) {
        draw_data_grid(img, rect, ds, app, pal);
    } else if let Some(rs) = doc.result_sheet(active_id) {
        draw_result_view(img, rect, rs, &app.canvas_dialogs.result_view, pal);
    } else if let Some(info) = doc.info_sheet(active_id) {
        draw_info_view(img, rect, info, pal);
    } else {
        draw_empty_studio(img, rect, pal, is_rtl);
    }
}

fn draw_portfolio_gallery(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette, is_rtl: bool) {
    let header_title = format!("PORTFOLIO GALLERY ({})", app.nav.gallery.cards.len());
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 12,
            y: rect.y + 8,
            text: &header_title,
            color: pal.text,
            max_width_px: rect.width.saturating_sub(24),
            rtl: is_rtl,
        },
    );
    fill_rect(img, WorkspacePixelRect::new(rect.x + 10, rect.y + 22, rect.width.saturating_sub(20), 1), pal.border);

    let body = WorkspacePixelRect::new(rect.x + 12, rect.y + 28, rect.width.saturating_sub(24), rect.height.saturating_sub(36));
    if body.width < 40 || body.height < 32 {
        return;
    }
    if app.nav.gallery.cards.is_empty() {
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: body.x, y: body.y + 12, text: "NO SHEETS MATCH FILTER", color: pal.muted, max_width_px: body.width, rtl: is_rtl },
        );
        return;
    }

    let gap = 10u32;
    let cols = (body.width / 132).clamp(1, 4);
    let card_w = body.width.saturating_sub(gap.saturating_mul(cols.saturating_sub(1))) / cols;
    let card_h = 74u32.min(body.height);
    for (idx, card) in app.nav.gallery.cards.iter().enumerate() {
        let col = (idx as u32) % cols;
        let row = (idx as u32) / cols;
        let cy = body.y.saturating_add(row.saturating_mul(card_h.saturating_add(gap)));
        if cy.saturating_add(card_h) > body.bottom() {
            break;
        }
        let local_cx = col.saturating_mul(card_w.saturating_add(gap));
        let cx = body.x.saturating_add(crate::i18n::mirror_x_for_rtl(local_cx, card_w, body.width, is_rtl));
        let card_rect = WorkspacePixelRect::new(cx, cy, card_w, card_h);
        draw_gallery_card(img, card_rect, card, app.nav.gallery.selected_ids.contains(&card.sheet_id), pal, is_rtl);
    }
}

fn draw_gallery_card(img: &mut RasterImage, rect: WorkspacePixelRect, card: &PortfolioCard, is_selected: bool, pal: &FramePalette, is_rtl: bool) {
    fill_rect(img, rect, pal.surface);
    let accent_band = card.color_tag.unwrap_or(pal.accent);
    let stripe_x = rect.x.saturating_add(crate::i18n::mirror_x_for_rtl(0, 4, rect.width, is_rtl));
    fill_rect(img, WorkspacePixelRect::new(stripe_x, rect.y + 1, 4, rect.height.saturating_sub(2)), accent_band);

    let thumb = WorkspacePixelRect::new(rect.x + 8, rect.y + 6, rect.width.saturating_sub(16), rect.height.saturating_sub(36).max(8));
    fill_rect(img, thumb, pal.panel);
    stroke_rect(img, thumb, pal.border);
    if thumb.width >= 20 && thumb.height >= 12 {
        fill_rect(img, WorkspacePixelRect::new(thumb.x + 4, thumb.y + 4, thumb.width.saturating_sub(8) / 2, 4), accent_band);
        fill_rect(img, WorkspacePixelRect::new(thumb.x + 4, thumb.y + 11, thumb.width.saturating_sub(12), 2), pal.muted);
    }

    let text_y = thumb.bottom().saturating_add(4);
    let label = format!("[{}] {}", card.badge_label, card.title);
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: rect.x + 8, y: text_y, text: &label, color: pal.text, max_width_px: rect.width.saturating_sub(16), rtl: is_rtl },
    );
    if text_y + 18 <= rect.bottom() {
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: rect.x + 8,
                y: text_y + 10,
                text: &card.subtitle,
                color: pal.muted,
                max_width_px: rect.width.saturating_sub(16),
                rtl: is_rtl,
            },
        );
    }

    let selected = is_selected || card.selected;
    let border_color = if selected {
        pal.accent
    } else if card.active {
        pal.link_blue
    } else {
        pal.border
    };
    stroke_rect(img, rect, border_color);
    if selected && rect.width > 4 && rect.height > 4 {
        stroke_rect(img, WorkspacePixelRect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2), pal.accent);
    }
}

fn draw_active_pane_focus(img: &mut RasterImage, layout: &WorkspaceFrameLayout, app: &UiApp, pal: &FramePalette) {
    let focus_color = Rgba8::rgba(pal.accent.r, pal.accent.g, pal.accent.b, 190);
    let target = match app.nav.active_pane {
        ActiveWorkspacePane::Navigator if layout.navigator.width > 4 => Some(layout.navigator),
        ActiveWorkspacePane::FamilyStrip if layout.navigator.width > 4 && layout.navigator.height > 24 => {
            let fam_y = layout.navigator.y + (layout.navigator.height * 2 / 3).saturating_add(2);
            Some(WorkspacePixelRect::new(layout.navigator.x, fam_y, layout.navigator.width, layout.navigator.bottom().saturating_sub(fam_y)))
        }
        ActiveWorkspacePane::MainCanvasOrGrid if layout.canvas.width > 4 => Some(layout.canvas),
        ActiveWorkspacePane::Inspector if layout.inspector.width > 4 => Some(layout.inspector),
        ActiveWorkspacePane::FloatingNotes if layout.canvas.width >= 96 && layout.canvas.height >= 40 => {
            let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
            let badge_w = 88u32;
            let local_x = crate::i18n::mirror_x_for_rtl(layout.canvas.width.saturating_sub(badge_w + 8), badge_w, layout.canvas.width, is_rtl);
            let notes_rect = WorkspacePixelRect::new(layout.canvas.x.saturating_add(local_x), layout.canvas.bottom().saturating_sub(24), badge_w, 18);
            fill_rect(img, notes_rect, pal.panel);
            stroke_rect(img, notes_rect, pal.accent);
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: notes_rect.x + 6,
                    y: notes_rect.y + 5,
                    text: "NOTES PANE",
                    color: pal.accent,
                    max_width_px: badge_w - 12,
                    rtl: is_rtl,
                },
            );
            None
        }
        _ => None,
    };

    if let Some(pane) = target
        && pane.width >= 4
        && pane.height >= 4
    {
        fill_rect(img, WorkspacePixelRect::new(pane.x, pane.y, pane.width, 2), focus_color);
        stroke_rect(img, WorkspacePixelRect::new(pane.x + 1, pane.y + 1, pane.width - 2, pane.height - 2), focus_color);
    }
}

fn draw_empty_studio(img: &mut RasterImage, rect: WorkspacePixelRect, pal: &FramePalette, is_rtl: bool) {
    let cw = rect.width.saturating_sub(48).min(260);
    let ch = rect.height.saturating_sub(48).min(96);
    let card = WorkspacePixelRect::new(rect.x + (rect.width.saturating_sub(cw)) / 2, rect.y + (rect.height.saturating_sub(ch)) / 2, cw, ch);
    fill_rect(img, card, pal.panel);
    stroke_rect(img, card, pal.border);
    let max_w = card.width.saturating_sub(32);
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: card.x + 16, y: card.y + 18, text: "LUMEN SCIENTIFIC STUDIO", color: pal.text, max_width_px: max_w, rtl: is_rtl },
    );
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: card.x + 16, y: card.y + 38, text: "NEW DATA TABLE: CMD+T", color: pal.muted, max_width_px: max_w, rtl: is_rtl },
    );
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: card.x + 16, y: card.y + 56, text: "COMMAND PALETTE: CMD+K", color: pal.accent, max_width_px: max_w, rtl: is_rtl },
    );
}

fn draw_scene_viewport(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, sheet_id: &SheetId, dpi: u16, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let pad = 16u32.min(rect.width / 8).min(rect.height / 8);
    let inner_w = rect.width.saturating_sub(pad * 2).max(1);
    let inner_h = rect.height.saturating_sub(pad * 2 + 20).max(1);
    let paper = WorkspacePixelRect::new(rect.x + pad, rect.y + pad + 18, inner_w, inner_h);

    let zoom_pct = (app.canvas_dialogs.effective_zoom_and_pan().0 * 100.0).round() as i64;
    let header_label = format!("CANVAS: {} ({zoom_pct}%)", active_sheet_title(app.document(), sheet_id));
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + pad,
            y: rect.y + 8,
            text: &header_label,
            color: pal.text,
            max_width_px: rect.width.saturating_sub(pad * 2),
            rtl: is_rtl,
        },
    );

    if let Ok(Some(scene)) = app.canvas_dialogs.active_scene(app.session()) {
        if let Ok(rendered) = render_scene(&scene, RenderTargetSpec { width_px: paper.width, height_px: paper.height, dpi: dpi.max(72) }) {
            blit_image(img, paper.x, paper.y, &rendered);
        }
    } else {
        fill_rect(img, paper, Rgba8::WHITE);
    }
    stroke_rect(img, paper, pal.border);

    if let Some(gs) = app.document().graph_sheet(sheet_id) {
        draw_graph_summary_chips(img, paper, gs, pal);
    } else if let Some(ls) = app.document().layout_sheet(sheet_id) {
        draw_layout_summary_badges(img, paper, ls, pal);
    }
}

fn draw_graph_summary_chips(img: &mut RasterImage, paper: WorkspacePixelRect, gs: &GraphSheet, pal: &FramePalette) {
    let mut sx = paper.x + 10;
    for (idx, _s) in gs.spec.series.iter().take(4).enumerate() {
        let chip = WorkspacePixelRect::new(sx, paper.y + 8, 12, 8);
        fill_rect(img, chip, gs.spec.palette.color_at(idx));
        stroke_rect(img, chip, Rgba8::BLACK);
        sx = sx.saturating_add(16);
    }
    if !gs.initialized && paper.width > 120 && paper.height > 40 {
        let banner = WorkspacePixelRect::new(paper.x + 10, paper.bottom().saturating_sub(24), paper.width.saturating_sub(20).min(180), 16);
        fill_rect(img, banner, pal.accent);
        draw_text_clipped(img, banner.x + 6, banner.y + 4, "SELECT GRAPH TYPE", Rgba8::WHITE, banner.width.saturating_sub(12));
    }
}

fn draw_layout_summary_badges(img: &mut RasterImage, paper: WorkspacePixelRect, ls: &LayoutSheet, pal: &FramePalette) {
    let page = ls.spec.page_size_pt();
    if page.width <= 0.0 || page.height <= 0.0 {
        return;
    }
    let sx = f64::from(paper.width) / page.width;
    let sy = f64::from(paper.height) / page.height;
    for (idx, item) in ls.spec.items.iter().enumerate() {
        let bounds = match item {
            LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
                *bounds_pt
            }
        };
        let px = paper.x + ((bounds.x * sx).round().max(0.0) as u32);
        let py = paper.y + ((bounds.y * sy).round().max(0.0) as u32);
        let pw = ((bounds.width * sx).round().max(8.0) as u32).min(paper.right().saturating_sub(px));
        let ph = ((bounds.height * sy).round().max(8.0) as u32).min(paper.bottom().saturating_sub(py));
        if pw >= 4 && ph >= 4 {
            let box_rect = WorkspacePixelRect::new(px, py, pw, ph);
            stroke_rect(img, box_rect, pal.accent);
            if let Some(badge) = ls.spec.panel_badges.badge_for(idx) {
                fill_rect(img, WorkspacePixelRect::new(px + 2, py + 2, 14, 12), pal.accent);
                draw_text_clipped(img, px + 5, py + 4, &badge, Rgba8::WHITE, 10);
            }
        }
    }
}

fn draw_data_grid(img: &mut RasterImage, rect: WorkspacePixelRect, ds: &DataSheet, app: &UiApp, pal: &FramePalette) {
    let sheet_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    fill_rect(img, sheet_rect, pal.surface);
    stroke_rect(img, sheet_rect, pal.border);

    let title = format!("[{}] {}", ds.kind().label(), ds.title);
    draw_text_clipped(img, sheet_rect.x + 8, sheet_rect.y + 6, &title, pal.text, sheet_rect.width.saturating_sub(16));

    let frame = app.grid.tool.format_visible_window(ds);
    let row_h = 18u32;
    let hdr_y = sheet_rect.y + 22;
    let body_y = hdr_y + row_h + 4;
    let col_w = 72u32;
    let gutter_w = 34u32;

    fill_rect(img, WorkspacePixelRect::new(sheet_rect.x + 1, hdr_y, sheet_rect.width.saturating_sub(2), row_h), pal.header);
    draw_text_5x7(img, sheet_rect.x + 8, hdr_y + 5, "#", pal.muted);

    let max_cols = (sheet_rect.width.saturating_sub(gutter_w + 8) / col_w).max(1) as usize;
    for (vis_idx, g_idx) in (frame.window.start_col..frame.window.end_col).take(max_cols).enumerate() {
        let cx = sheet_rect.x + gutter_w + (vis_idx as u32) * col_w;
        let hdr = WorkspacePixelRect::new(cx, hdr_y, col_w.saturating_sub(2), row_h);
        stroke_rect(img, hdr, pal.border);
        if let TableArchetype::MultipleVariables(mv) = &ds.archetype
            && let Some(var) = mv.variables.get(g_idx)
        {
            fill_rect(img, WorkspacePixelRect::new(cx + 2, hdr_y + 2, 16, row_h.saturating_sub(4)), variable_role_color(var.role, &app.theme));
            draw_text_clipped(img, cx + 21, hdr_y + 5, &var.title, pal.text, col_w.saturating_sub(24));
        } else {
            let label = data_column_title(ds, g_idx).unwrap_or("Col");
            draw_text_clipped(img, cx + 4, hdr_y + 5, label, pal.text, col_w.saturating_sub(8));
        }
    }

    let geom = GridCellGeometry { sheet_rect, body_y, row_h, gutter_w, col_w };
    draw_visible_grid_cells(img, &geom, &frame.window, &frame.cells, pal);
}

struct GridCellGeometry {
    sheet_rect: WorkspacePixelRect,
    body_y: u32,
    row_h: u32,
    gutter_w: u32,
    col_w: u32,
}

fn draw_visible_grid_cells(
    img: &mut RasterImage,
    geom: &GridCellGeometry,
    window: &lumencraft_tools::GridVisibleWindow,
    cells: &[lumencraft_tools::grid::VisibleCellView],
    pal: &FramePalette,
) {
    let sheet_rect = geom.sheet_rect;
    let body_y = geom.body_y;
    let row_h = geom.row_h;
    let gutter_w = geom.gutter_w;
    let col_w = geom.col_w;
    let max_rows = (sheet_rect.bottom().saturating_sub(body_y) / row_h) as usize;
    let max_cols = (sheet_rect.width.saturating_sub(gutter_w + 8) / col_w).max(1) as usize;
    for r_off in 0..max_rows.min(window.end_row.saturating_sub(window.start_row)) {
        let ry = body_y + (r_off as u32) * row_h;
        let row_num = window.start_row + r_off + 1;
        draw_text_5x7(img, sheet_rect.x + 6, ry + 5, &format!("{row_num}"), pal.muted);
        fill_rect(img, WorkspacePixelRect::new(sheet_rect.x + 1, ry + row_h - 1, sheet_rect.width.saturating_sub(2), 1), pal.border);
    }

    for cell in cells {
        let r_off = cell.coord.row.saturating_sub(window.start_row);
        let c_off = cell.flat_col.saturating_sub(window.start_col);
        if r_off >= max_rows || c_off >= max_cols {
            continue;
        }
        let cx = sheet_rect.x + gutter_w + (c_off as u32) * col_w;
        let cy = body_y + (r_off as u32) * row_h;
        let cell_rect = WorkspacePixelRect::new(cx, cy, col_w.saturating_sub(1), row_h.saturating_sub(1));
        if cell.is_selected {
            fill_rect(img, cell_rect, shift_luma(pal.surface, -10));
        }
        if cell.is_active {
            stroke_rect(img, cell_rect, pal.accent);
        } else if let Some(link) = cell.link_border {
            let link_color = match link {
                LinkBorderColor::Blue => pal.link_blue,
                LinkBorderColor::Green => pal.link_green,
                LinkBorderColor::Red => pal.link_red,
                LinkBorderColor::Charcoal => pal.muted,
            };
            stroke_rect(img, cell_rect, link_color);
        }
        let fg = if cell.excluded { pal.excluded } else { pal.text };
        draw_text_clipped(img, cx + 4, cy + 5, &cell.display_text, fg, col_w.saturating_sub(8));
    }
}

fn draw_result_view(img: &mut RasterImage, rect: WorkspacePixelRect, rs: &ResultSheet, view: &ResultSheetViewState, pal: &FramePalette) {
    let box_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    fill_rect(img, box_rect, pal.surface);
    stroke_rect(img, box_rect, pal.border);

    draw_text_clipped(img, box_rect.x + 10, box_rect.y + 8, rs.title(), pal.text, box_rect.width.saturating_sub(20));

    let mut tx = box_rect.x + 10;
    let tab_y = box_rect.y + 24;
    for (idx, tab) in rs.tabs.iter().take(4).enumerate() {
        let tw = ((tab.title.len() as u32) * 6 + 14).min(110);
        if tx + tw >= box_rect.right() {
            break;
        }
        let active = idx == view.active_tab_index;
        let trect = WorkspacePixelRect::new(tx, tab_y, tw, 18);
        fill_rect(img, trect, if active { pal.accent } else { pal.header });
        stroke_rect(img, trect, pal.border);
        draw_text_clipped(img, trect.x + 5, trect.y + 5, &tab.title, if active { Rgba8::WHITE } else { pal.text }, tw.saturating_sub(10));
        tx = tx.saturating_add(tw + 4);
    }

    let mut row_y = tab_y + 28;
    if let Some(tab) = view.active_tab(rs) {
        for (r_idx, row) in tab.rows.iter().take(6).enumerate() {
            if row_y + 16 >= box_rect.bottom().saturating_sub(56) {
                break;
            }
            let r_label = tab.row_labels.get(r_idx).map(String::as_str).unwrap_or("");
            let first_val = row.first().map(|c| ResultSheetViewState::format_result_cell(c, rs.analysis.p_value_format)).unwrap_or_default();
            draw_text_clipped(img, box_rect.x + 12, row_y, &format!("{r_label}: {first_val}"), pal.text, box_rect.width.saturating_sub(24));
            row_y = row_y.saturating_add(16);
        }
    }

    let chk_y = box_rect.bottom().saturating_sub(54).max(row_y + 6);
    fill_rect(img, WorkspacePixelRect::new(box_rect.x + 8, chk_y, box_rect.width.saturating_sub(16), 1), pal.border);
    draw_text_5x7(img, box_rect.x + 10, chk_y + 6, "ASSUMPTION CHECKLIST", pal.muted);
    let mut cy = chk_y + 18;
    for item in view.visible_checklist(rs).into_iter().take(2) {
        if cy + 12 >= box_rect.bottom() {
            break;
        }
        let status_color = match item.status {
            ChecklistStatus::Pass => pal.link_green,
            ChecklistStatus::Warn => Rgba8::rgb(0xD5, 0x5E, 0x00),
            ChecklistStatus::Violated => pal.link_red,
            ChecklistStatus::Info => pal.link_blue,
        };
        fill_rect(img, WorkspacePixelRect::new(box_rect.x + 10, cy + 1, 8, 8), status_color);
        draw_text_clipped(img, box_rect.x + 24, cy, &item.assumption, pal.text, box_rect.width.saturating_sub(32));
        cy = cy.saturating_add(14);
    }
}

fn draw_info_view(img: &mut RasterImage, rect: WorkspacePixelRect, info: &InfoSheet, pal: &FramePalette) {
    let box_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    fill_rect(img, box_rect, pal.surface);
    stroke_rect(img, box_rect, pal.border);
    draw_text_clipped(img, box_rect.x + 10, box_rect.y + 8, &info.title, pal.text, box_rect.width.saturating_sub(20));

    let split_x = box_rect.x + (box_rect.width / 2);
    fill_rect(img, WorkspacePixelRect::new(split_x, box_rect.y + 24, 1, box_rect.height.saturating_sub(28)), pal.border);
    let mut y = box_rect.y + 28;
    for c in info.constants.iter().take(8) {
        if y + 14 >= box_rect.bottom() {
            break;
        }
        let line = format!("{}: {}", c.name, c.value.display_text());
        draw_text_clipped(img, box_rect.x + 10, y, &line, pal.text, (box_rect.width / 2).saturating_sub(18));
        y = y.saturating_add(15);
    }
    draw_text_clipped(img, split_x + 10, box_rect.y + 28, &info.notes_markdown, pal.muted, (box_rect.width / 2).saturating_sub(18));
}

fn draw_inspector_pane(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.panel);
    let border_x = if is_rtl { rect.right().saturating_sub(1) } else { rect.x };
    fill_rect(img, WorkspacePixelRect::new(border_x, rect.y, 1, rect.height), pal.border);
    let text_w = rect.width.saturating_sub(18);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + 8,
            text: app.canvas_dialogs.active_inspector.label(),
            color: pal.text,
            max_width_px: text_w,
            rtl: is_rtl,
        },
    );
    fill_rect(img, WorkspacePixelRect::new(rect.x + 8, rect.y + 22, rect.width.saturating_sub(16), 1), pal.border);

    match app.canvas_dialogs.active_inspector {
        ActiveInspector::FormatGraph => draw_inspector_format_graph(img, rect, app, pal, is_rtl),
        ActiveInspector::FormatAxes => {
            let draft = &app.canvas_dialogs.format_axes.axis_draft;
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("Segments: {}", draft.segments.len()),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
            for (idx, seg) in draft.segments.iter().enumerate() {
                let y = rect.y + 46 + (idx as u32) * 16;
                draw_text_clipped_dir(
                    img,
                    DrawTextClipped {
                        x: rect.x + 10,
                        y,
                        text: &format!("S{}: {:?}", idx + 1, seg.scale),
                        color: pal.muted,
                        max_width_px: text_w,
                        rtl: is_rtl,
                    },
                );
            }
        }
        ActiveInspector::PairwiseComparisons => {
            let count = app.canvas_dialogs.pairwise.brackets.len();
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("Brackets: {count}"),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
        }
        ActiveInspector::BubblePlot => {
            let bp = &app.canvas_dialogs.bubble_plot;
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("X col {} / Y col {}", bp.x_column, bp.y_column),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 46,
                    text: if bp.show_convex_hulls { "Hulls: On" } else { "Hulls: Off" },
                    color: pal.muted,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
        }
        ActiveInspector::PageLayout => {
            let pl = &app.canvas_dialogs.page_layout;
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("Grid: {}x{}", pl.rows, pl.cols),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 46,
                    text: &format!("Items: {}", pl.items.len()),
                    color: pal.muted,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
        }
        ActiveInspector::ResultDetails | ActiveInspector::None => {
            draw_text_clipped_dir(
                img,
                DrawTextClipped { x: rect.x + 10, y: rect.y + 30, text: "Ready", color: pal.muted, max_width_px: text_w, rtl: is_rtl },
            );
        }
    }
}

fn draw_inspector_format_graph(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette, is_rtl: bool) {
    let text_w = rect.width.saturating_sub(18);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + 30,
            text: app.canvas_dialogs.format_graph.palette.label(),
            color: pal.muted,
            max_width_px: text_w,
            rtl: is_rtl,
        },
    );
    let mut local_x = 10u32;
    for i in 0..6 {
        if local_x + 18 >= rect.width {
            break;
        }
        let sx = rect.x.saturating_add(crate::i18n::mirror_x_for_rtl(local_x, 16, rect.width, is_rtl));
        let sw = WorkspacePixelRect::new(sx, rect.y + 44, 16, 14);
        fill_rect(img, sw, app.canvas_dialogs.format_graph.preview_swatch(i));
        stroke_rect(img, sw, pal.border);
        local_x = local_x.saturating_add(20);
    }
    let frame_lbl = format!("Frame: {:?}", app.canvas_dialogs.format_graph.frame_preset);
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: rect.x + 10, y: rect.y + 68, text: &frame_lbl, color: pal.text, max_width_px: text_w, rtl: is_rtl },
    );
}

fn draw_status_bar(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.header);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y, rect.width, 1), pal.border);

    let cursor_str = format!("Cursor: {:?}", app.active_cursor());
    let stats_str = app
        .grid
        .quick_stats
        .filter(|s| s.numeric_count > 0)
        .and_then(|s| s.mean.map(|m| format!("n={} Mean={}", s.numeric_count, format_compact_f64(m))))
        .unwrap_or_default();
    let banner = app.canvas_dialogs.status_banner.as_deref().unwrap_or("");
    let left_text = [cursor_str, stats_str, String::from(banner)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<String>>().join(" | ");
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + (rect.height.saturating_sub(7)) / 2,
            text: &left_text,
            color: pal.text,
            max_width_px: rect.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );
}

fn draw_modal_overlay(img: &mut RasterImage, backdrop: WorkspacePixelRect, modal: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    dim_rect(img, backdrop);
    fill_rect(img, modal, pal.surface);
    stroke_rect(img, modal, pal.accent);

    let header = WorkspacePixelRect::new(modal.x, modal.y, modal.width, 24);
    fill_rect(img, header, pal.accent);
    let title = modal_title(&app.canvas_dialogs.active_dialog);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: header.x + 10,
            y: header.y + 8,
            text: title,
            color: Rgba8::WHITE,
            max_width_px: header.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );

    let body_y = header.bottom().saturating_add(8);
    let lines = modal_preview_lines(&app.canvas_dialogs);
    for (idx, line) in lines.into_iter().take(6).enumerate() {
        let ly = body_y + (idx as u32) * 16;
        if ly + 10 >= modal.bottom() {
            break;
        }
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: modal.x + 12, y: ly, text: &line, color: pal.text, max_width_px: modal.width.saturating_sub(24), rtl: is_rtl },
        );
    }
}

fn modal_title(dialog: &ActiveDialog) -> &'static str {
    match dialog {
        ActiveDialog::None => "",
        ActiveDialog::AnalyzeSheet { .. } => "Analyze...",
        ActiveDialog::DuplicateAnalysisPrompt { .. } => "Analysis Already Exists",
        ActiveDialog::EditAnalysisParameters { .. } => "Edit Analysis Parameters (Cmd+T)",
        ActiveDialog::ChangeDataAnalyzed { .. } => "Change Data Analyzed",
        ActiveDialog::ResultViewer { .. } => "Result Sheet Viewer",
        ActiveDialog::ConstantHookPicker { .. } => "Insert Hooked Constant",
        ActiveDialog::ChangeGraphType { .. } => "Change Graph Type",
        ActiveDialog::FormatGraph { .. } => "Format Graph",
        ActiveDialog::FormatAxes { .. } => "Format Axes",
        ActiveDialog::PairwiseComparisons { .. } => "Pairwise Comparisons",
        ActiveDialog::BubblePlotInspector { .. } => "Bubble Plot Channels",
        ActiveDialog::ApplyStyleFrom { .. } => "Apply Style From...",
        ActiveDialog::PageLayoutComposer { .. } => "Page Layout Composer",
        ActiveDialog::ExportFigure { .. } => "Export Publication Figure",
        ActiveDialog::CommandPalette => "Command Palette (Cmd+K)",
        ActiveDialog::About => "About LumenCraft",
    }
}

fn modal_preview_lines(state: &CanvasDialogState) -> Vec<String> {
    match &state.active_dialog {
        ActiveDialog::AnalyzeSheet { .. } => {
            let mut rows = vec![format!("Search: {}", state.analyze.search_query)];
            for item in state.analyze.compatible_entries().into_iter().take(5) {
                rows.push(format!("- {}", item.label));
            }
            rows
        }
        ActiveDialog::CommandPalette => {
            let mut rows = vec![format!("> {}", state.command_palette.query)];
            for hit in state.command_palette.matches().into_iter().take(5) {
                rows.push(format!("{} ({})", hit.label, hit.id));
            }
            rows
        }
        ActiveDialog::ChangeGraphType { .. } => {
            state.change_graph_type.recommended_options().into_iter().take(5).map(|o| format!("[*] {}", o.label)).collect()
        }
        ActiveDialog::ApplyStyleFrom { .. } => {
            state.apply_style.donor_candidates.iter().take(5).map(|(id, title)| format!("Donor: {title} ({})", id.as_str())).collect()
        }
        _ => vec![String::from("Active scientific dialog ready")],
    }
}

// ============================================================================
// Low-Level Safe Software Raster Primitives & Compact 5x7 ASCII Font
// ============================================================================

fn fill_rect(img: &mut RasterImage, rect: WorkspacePixelRect, color: Rgba8) {
    let x_end = rect.right().min(img.width_px);
    let y_end = rect.bottom().min(img.height_px);
    for py in rect.y..y_end {
        for px in rect.x..x_end {
            img.blend_pixel(px, py, color);
        }
    }
}

fn stroke_rect(img: &mut RasterImage, rect: WorkspacePixelRect, color: Rgba8) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y, rect.width, 1), color);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.bottom().saturating_sub(1), rect.width, 1), color);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y, 1, rect.height), color);
    fill_rect(img, WorkspacePixelRect::new(rect.right().saturating_sub(1), rect.y, 1, rect.height), color);
}

fn dim_rect(img: &mut RasterImage, rect: WorkspacePixelRect) {
    fill_rect(img, rect, Rgba8::rgba(16, 20, 28, 110));
}

fn blit_image(dst: &mut RasterImage, dst_x: u32, dst_y: u32, src: &RasterImage) {
    let w = src.width_px.min(dst.width_px.saturating_sub(dst_x));
    let h = src.height_px.min(dst.height_px.saturating_sub(dst_y));
    for sy in 0..h {
        for sx in 0..w {
            let idx = ((sy as usize) * (src.width_px as usize) + (sx as usize)) * 4;
            if let Some(&[r, g, b, a]) = src.rgba.get(idx..idx + 4) {
                dst.blend_pixel(dst_x + sx, dst_y + sy, Rgba8::rgba(r, g, b, a));
            }
        }
    }
}

struct DrawTextClipped<'a> {
    x: u32,
    y: u32,
    text: &'a str,
    color: Rgba8,
    max_width_px: u32,
    rtl: bool,
}

fn draw_text_clipped(img: &mut RasterImage, x: u32, y: u32, text: &str, color: Rgba8, max_width_px: u32) {
    draw_text_clipped_dir(img, DrawTextClipped { x, y, text, color, max_width_px, rtl: false });
}

fn draw_text_clipped_dir(img: &mut RasterImage, spec: DrawTextClipped<'_>) {
    let reordered;
    let display = if spec.rtl {
        reordered = reorder_visual_bidi(spec.text, TextDirection::Rtl);
        reordered.as_str()
    } else {
        spec.text
    };
    let max_chars = (spec.max_width_px / 6) as usize;
    let count = display.chars().count().min(max_chars);
    let draw_w = (count as u32).saturating_mul(6).saturating_sub(1);
    let mut cx = if spec.rtl { spec.x.saturating_add(spec.max_width_px.saturating_sub(draw_w)) } else { spec.x };
    for ch in display.chars().take(max_chars) {
        draw_glyph_5x7(img, cx, spec.y, ch, spec.color);
        cx = cx.saturating_add(6);
    }
}

fn draw_text_5x7(img: &mut RasterImage, x: u32, y: u32, text: &str, color: Rgba8) {
    draw_text_5x7_dir(img, x, y, text, color, false);
}

fn draw_text_5x7_dir(img: &mut RasterImage, x: u32, y: u32, text: &str, color: Rgba8, rtl: bool) {
    let reordered;
    let display = if rtl {
        reordered = reorder_visual_bidi(text, TextDirection::Rtl);
        reordered.as_str()
    } else {
        text
    };
    let mut cx = x;
    for ch in display.chars() {
        draw_glyph_5x7(img, cx, y, ch, color);
        cx = cx.saturating_add(6);
    }
}

fn draw_glyph_5x7(img: &mut RasterImage, x: u32, y: u32, ch: char, color: Rgba8) {
    let rows = glyph_5x7(ch.to_ascii_uppercase());
    for (dy, bits) in rows.into_iter().enumerate() {
        for dx in 0..5u32 {
            if (bits & (1 << (4 - dx))) != 0 {
                img.blend_pixel(x.saturating_add(dx), y.saturating_add(dy as u32), color);
            }
        }
    }
}

const FONT_5X7: &[(u8, [u8; 7])] = &[
    (b' ', [0, 0, 0, 0, 0, 0, 0]),
    (b'!', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100]),
    (b'#', [0b01010, 0b11111, 0b01010, 0b01010, 0b11111, 0b01010, 0]),
    (b'%', [0b11001, 0b11010, 0b00100, 0b01000, 0b10110, 0b10011, 0]),
    (b'+', [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0]),
    (b',', [0, 0, 0, 0, 0, 0b00100, 0b01000]),
    (b'-', [0, 0, 0, 0b11111, 0, 0, 0]),
    (b'.', [0, 0, 0, 0, 0, 0b01100, 0b01100]),
    (b'/', [0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0, 0]),
    (b':', [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0]),
    (b'=', [0, 0, 0b11111, 0, 0b11111, 0, 0]),
    (b'>', [0b10000, 0b01000, 0b00100, 0b01000, 0b10000, 0, 0]),
    (b'(', [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010]),
    (b')', [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000]),
    (b'[', [0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110]),
    (b']', [0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110]),
    (b'|', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    (b'*', [0b00100, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00100]),
    (b'0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    (b'1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    (b'2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    (b'3', [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110]),
    (b'4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    (b'5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    (b'6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    (b'7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    (b'8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    (b'9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    (b'A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    (b'B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    (b'C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    (b'D', [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100]),
    (b'E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    (b'F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    (b'G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111]),
    (b'H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    (b'I', [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    (b'J', [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100]),
    (b'K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    (b'L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    (b'M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    (b'N', [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001]),
    (b'O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    (b'P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    (b'Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101]),
    (b'R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    (b'S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    (b'T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    (b'U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    (b'V', [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100]),
    (b'W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001]),
    (b'X', [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b01010, 0b10001]),
    (b'Y', [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    (b'Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
];

fn glyph_5x7(ch: char) -> [u8; 7] {
    if !ch.is_ascii() {
        return [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111];
    }
    let byte = ch as u8;
    FONT_5X7.iter().find(|(k, _)| *k == byte).map_or([0b00000, 0b01110, 0b01010, 0b01010, 0b01110, 0b00000, 0b00000], |(_, rows)| *rows)
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, ConstantValue, DataSheet, GraphSheet, InfoConstant, InfoSheet,
        JournalWidthPreset, LayoutSheet, LayoutSpec, NonlinearFitSpec, PValueFormat, ProjectDocument, ResultCell, ResultSheet, ResultStatus,
        TableArchetype, XyTable,
    };

    use super::{
        ActiveDialog, AxisTarget, CanvasDialogState, CommandPaletteState, ConstantHookPickerState, FormatAxesState, FramePalette, HookTargetKind,
        ResultSheetViewState,
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
}
