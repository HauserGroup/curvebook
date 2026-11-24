//! Static searchable `Analyze...` catalog, categories, and default [`AnalysisKind`] mapping.

use curvebook_doc::{
    AnalysisKind, BaselineOperation, BlandAltmanSpec, ContingencySpec, CoxTieMethod, DescriptiveSpec, ErrorPropagationMode, FactorialAnovaSpec,
    GlmFamily, GlmSpec, LinearRegressionSpec, MathTransformPreset, MultipleRowTestsSpec, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec,
    NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OutlierSpec, PValueStackSpec, PassingBablokSpec, RocSpec, SimpleLogisticSpec,
    SplineLowessSpec, SurvivalAnalysisSpec, TableKind, TransformAnalysisSpec, TwoGroupSpec,
};

use serde::{Deserialize, Serialize};

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
        id: "stats.simple_logistic",
        label: "Simple Logistic Regression",
        category: AnalysisCategory::XyAndRegression,
        keywords: "simple logistic regression binary xy odds ratio x50 profile likelihood roc tjur",
        summary: "Single-predictor binary logistic regression on XY tables with profile-likelihood CIs, X50, and ROC AUC.",
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
    AnalysisCatalogItem {
        id: "fit.spline_lowess",
        label: "Fit Spline / LOWESS Curve",
        category: AnalysisCategory::XyAndRegression,
        keywords: "spline lowess akima smoothing cubic point to point curve interpolation nonparametric",
        summary: "Model-free XY natural cubic spline, Akima subspline, smoothing spline, LOWESS, or point-to-point curve.",
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
        "fit.spline_lowess" => Some(AnalysisKind::FitSplineLowess(SplineLowessSpec::default())),
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
        "stats.pca" | "stats.pcr" => {
            Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(7), pcr_outcome_column: None }))
        }
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
        "stats.simple_logistic" => Some(AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())),
        _ => None,
    }
}
