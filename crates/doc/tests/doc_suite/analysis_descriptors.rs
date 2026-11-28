//! Completeness, bijection, and metadata invariants for [`ALL_ANALYSIS_DESCRIPTORS`].

use std::collections::BTreeSet;

use curvebook_doc::analysis::{
    ALL_ANALYSIS_DESCRIPTORS, AnalysisDomainKind, AnalysisKindId, AnalysisRecomputeStrategy, AnalysisUiCategory, default_analysis_kind_id_for_table,
    descriptor_by_serde_variant, descriptor_by_shorthand, descriptor_by_slug, descriptor_by_ui_catalog_id, descriptor_for_id, descriptor_for_kind,
    resolve_analysis_descriptor,
};
use curvebook_doc::{
    AnalysisKind, AncovaSpec, AreaUnderCurveSpec, AsymmetricCiTransformPolicy, BlandAltmanSpec, ClusterDirection, ClusterDistance, ClusterLinkage,
    ContingencyMethod, ContingencySpec, CorrelationMethod, CoxTieMethod, CurveCalculusSpec, DerivedColumnFormula, DescriptiveSpec,
    ErrorPropagationMode, FactorialAnovaSpec, FractionOfTotalSpec, FrequencyBinningRule, FrequencyDistributionSpec, FrequencyOutputMode, GlmFamily,
    GlmSpec, GraphKind, GraphSpec, HistogramBinAlignment, HistogramDisplayStyle, HistogramMultiSeriesMode, HistogramSpec, ImportedSnapshotSpec,
    KMeansAlgorithm, KappaWeighting, LeveneTestSpec, LinearRegressionSpec, MathTransformPreset, MonteCarloSpec, MultipleRowTestsSpec,
    MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OutlierSpec,
    PValueStackSpec, PassingBablokSpec, PcaRetentionRule, PowerAnalysisSpec, RegressionCiMethod, RocSpec, RowStatisticsSpec, SimpleLogisticSpec,
    SimulationNoise, SplineLowessSpec, SurvivalAnalysisSpec, SurvivalOtherValuePolicy, TableKind, TransformAnalysisSpec, TwoGroupSpec,
};

fn representative_variants() -> Vec<(AnalysisKindId, AnalysisKind)> {
    vec![
        (AnalysisKindId::Transform, AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true })),
        (AnalysisKindId::DescriptiveStats, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())),
        (AnalysisKindId::OutlierDetection, AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })),
        (AnalysisKindId::TwoGroupComparison, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())),
        (AnalysisKindId::BlandAltman, AnalysisKind::BlandAltman(BlandAltmanSpec::default())),
        (AnalysisKindId::MultipleRowTTests, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())),
        (AnalysisKindId::PassingBablok, AnalysisKind::PassingBablok(PassingBablokSpec::default())),
        (AnalysisKindId::OneWayAnova, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())),
        (AnalysisKindId::FactorialAnova, AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())),
        (AnalysisKindId::NestedComparison, AnalysisKind::NestedComparison(NestedAnalysisSpec::default())),
        (AnalysisKindId::Contingency, AnalysisKind::Contingency(ContingencySpec::default())),
        (
            AnalysisKindId::Survival,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2],
                reference_levels: Vec::new(),
                tie_method: curvebook_doc::CoxTieMethod::Efron,
                formula: None,
                covariate_profiles: Vec::new(),
                nested_comparison: None,
                emit_cox_snell_residuals: false,
                ci_method: RegressionCiMethod::Wald,
                event_code: None,
                censored_code: None,
                treat_other_as: SurvivalOtherValuePolicy::Exclude,
            }),
        ),
        (AnalysisKindId::RocCurve, AnalysisKind::RocCurve(RocSpec::default())),
        (
            AnalysisKindId::GeneralizedLinearModel,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![1],
                ..GlmSpec::default()
            }),
        ),
        (AnalysisKindId::Multivariate, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false, method: None })),
        (AnalysisKindId::LinearRegression, AnalysisKind::LinearRegression(LinearRegressionSpec::default())),
        (AnalysisKindId::NonlinearRegression, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))),
        (AnalysisKindId::PValueStack, AnalysisKind::PValueStack(PValueStackSpec::default())),
        (AnalysisKindId::PowerAnalysis, AnalysisKind::PowerAnalysis(PowerAnalysisSpec::default())),
        (AnalysisKindId::SimpleLogisticRegression, AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())),
        (AnalysisKindId::FitSplineLowess, AnalysisKind::FitSplineLowess(SplineLowessSpec::default())),
        (AnalysisKindId::LeveneTest, AnalysisKind::LeveneTest(LeveneTestSpec::default())),
        (AnalysisKindId::AreaUnderCurve, AnalysisKind::AreaUnderCurve(AreaUnderCurveSpec::default())),
        (
            AnalysisKindId::ImportedSnapshot,
            AnalysisKind::ImportedSnapshot(ImportedSnapshotSpec { original_kind_label: String::from("Imported analysis") }),
        ),
        (AnalysisKindId::RowStatistics, AnalysisKind::RowStatistics(RowStatisticsSpec::default())),
        (AnalysisKindId::FractionOfTotal, AnalysisKind::FractionOfTotal(FractionOfTotalSpec::default())),
        (AnalysisKindId::CurveCalculus, AnalysisKind::CurveCalculus(CurveCalculusSpec::default())),
        (AnalysisKindId::MonteCarloSimulation, AnalysisKind::MonteCarloSimulation(MonteCarloSpec::default())),
        (AnalysisKindId::FrequencyDistribution, AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec::default())),
        (AnalysisKindId::Ancova, AnalysisKind::Ancova(AncovaSpec::default())),
    ]
}

#[test]
fn descriptors_form_exact_bijection_with_all_analysis_kind_variants() {
    let variants = representative_variants();
    assert_eq!(ALL_ANALYSIS_DESCRIPTORS.len(), AnalysisKindId::COUNT);
    assert_eq!(variants.len(), AnalysisKindId::COUNT);
    assert_eq!(AnalysisKindId::ALL.len(), AnalysisKindId::COUNT);

    let mut seen_ids = BTreeSet::new();
    for (idx, ((expected_id, kind), desc)) in variants.iter().zip(ALL_ANALYSIS_DESCRIPTORS.iter()).enumerate() {
        assert_eq!(*expected_id, AnalysisKindId::ALL[idx]);
        assert_eq!(desc.id, *expected_id);
        assert_eq!(kind.id(), *expected_id);
        assert_eq!(kind.descriptor(), desc);
        assert_eq!(descriptor_for_kind(kind), desc);
        assert_eq!(descriptor_for_id(*expected_id), desc);
        assert_eq!(kind.slug(), desc.slug);
        assert_eq!(kind.domain(), desc.domain);
        assert_eq!(kind.recompute_strategy(), desc.recompute_strategy);
        assert!(seen_ids.insert(desc.id));

        let serialized = serde_json::to_value(kind).expect("AnalysisKind must serialize");
        let obj = serialized.as_object().expect("AnalysisKind serializes as singleton object");
        assert_eq!(obj.len(), 1);
        let variant_key = obj.keys().next().expect("variant key present");
        assert_eq!(variant_key, desc.serde_variant, "serde variant mismatch for `{}`", desc.slug);

        // Every concrete top-level struct key produced by Serde must be listed in `canonical_param_keys`.
        // For sub-enums (`Transform`/`OutlierDetection`/`Survival`/`Multivariate`), inspect the inner variant's fields.
        if let Some(inner) = obj.get(variant_key).and_then(|v| v.as_object()) {
            if matches!(
                desc.id,
                AnalysisKindId::Transform | AnalysisKindId::OutlierDetection | AnalysisKindId::Survival | AnalysisKindId::Multivariate
            ) {
                for sub_val in inner.values() {
                    if let Some(sub_obj) = sub_val.as_object() {
                        for field_key in sub_obj.keys() {
                            assert!(
                                desc.accepts_canonical_param_key(field_key),
                                "sub-enum descriptor `{}` missing serialized canonical param key `{field_key}`",
                                desc.slug
                            );
                        }
                    }
                }
            } else {
                for field_key in inner.keys() {
                    assert!(
                        desc.accepts_canonical_param_key(field_key),
                        "descriptor `{}` missing serialized canonical param key `{field_key}`",
                        desc.slug
                    );
                }
            }
        }
    }
    assert_eq!(seen_ids.len(), AnalysisKindId::COUNT);
}

#[test]
fn slugs_shorthands_anchors_and_catalog_ids_are_unique_and_well_formed() {
    let mut slugs = BTreeSet::new();
    let mut serde_variants = BTreeSet::new();
    let mut anchors = BTreeSet::new();
    let mut shorthands = BTreeSet::new();
    let mut catalog_ids = BTreeSet::new();

    for desc in &ALL_ANALYSIS_DESCRIPTORS {
        assert!(!desc.slug.is_empty());
        assert!(desc.slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
        assert!(slugs.insert(desc.slug), "duplicate slug `{}`", desc.slug);

        assert!(!desc.serde_variant.is_empty());
        assert!(serde_variants.insert(desc.serde_variant), "duplicate serde variant `{}`", desc.serde_variant);

        assert_eq!(desc.docs_anchor, format!("analysis-{}", desc.slug));
        assert!(anchors.insert(desc.docs_anchor), "duplicate docs anchor `{}`", desc.docs_anchor);

        assert!(!desc.title.is_empty());
        assert!(!desc.short_label.is_empty());
        assert!(!desc.description.is_empty());
        if desc.user_creatable {
            assert!(!desc.shorthands.is_empty());
            assert!(desc.shorthands.contains(&desc.canonical_shorthand));
        } else {
            assert!(desc.shorthands.is_empty());
            assert!(desc.canonical_shorthand.is_empty());
            assert!(desc.ui_catalog_ids.is_empty());
        }

        for &alias in desc.shorthands {
            let norm = alias.to_ascii_lowercase();
            assert!(!norm.is_empty());
            assert!(shorthands.insert(norm), "duplicate shorthand `{alias}`");
        }

        let mut local_params = BTreeSet::new();
        assert!(!desc.canonical_param_keys.is_empty(), "`{}` must list canonical_param_keys", desc.slug);
        for &param in desc.canonical_param_keys {
            assert!(!param.is_empty());
            assert!(
                param.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "non-snake_case canonical param `{param}` in `{}`",
                desc.slug
            );
            assert!(local_params.insert(param), "duplicate canonical param `{param}` in `{}`", desc.slug);
        }

        for &cat_id in desc.ui_catalog_ids {
            assert!(!cat_id.is_empty());
            assert!(catalog_ids.insert(cat_id), "duplicate ui_catalog_id `{cat_id}`");
        }

        assert!(AnalysisUiCategory::ALL.contains(&desc.ui_category));
    }
}

#[test]
fn table_compatibility_and_domain_routing_invariants_hold() {
    for (_, kind) in representative_variants() {
        let desc = descriptor_for_kind(&kind);
        if desc.user_creatable {
            assert!(!desc.compatible_table_kinds.is_empty());
        } else {
            assert!(desc.compatible_table_kinds.is_empty());
            assert_eq!(desc.recompute_strategy, AnalysisRecomputeStrategy::PreserveSnapshot);
            for tk in TableKind::ALL {
                assert!(!kind.is_compatible_with(tk));
            }
            continue;
        }

        if !desc.has_method_specific_table_rules {
            for tk in TableKind::ALL {
                assert_eq!(desc.supports_table_kind(tk), kind.is_compatible_with(tk), "table compatibility drift for `{}` on `{:?}`", desc.slug, tk);
            }
        } else {
            assert_eq!(desc.id, AnalysisKindId::Contingency);
            let all_methods = [
                ContingencyMethod::PearsonChiSquare { yates_continuity: false },
                ContingencyMethod::LikelihoodRatioG2,
                ContingencyMethod::FisherExact,
                ContingencyMethod::BarnardExact,
                ContingencyMethod::CochranArmitageTrend,
                ContingencyMethod::McNemarPaired,
                ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
                ContingencyMethod::FleissKappa,
                ContingencyMethod::ExactBinomial,
            ];
            for tk in TableKind::ALL {
                let any_method_supported = all_methods
                    .iter()
                    .any(|&method| AnalysisKind::Contingency(ContingencySpec { method, ..ContingencySpec::default() }).is_compatible_with(tk));
                assert_eq!(desc.supports_table_kind(tk), any_method_supported, "Contingency compatible_table_kinds mismatch for `{tk:?}`");
            }
        }

        match desc.domain {
            AnalysisDomainKind::Transform => assert_eq!(desc.recompute_strategy, AnalysisRecomputeStrategy::Transform),
            AnalysisDomainKind::Fit => assert_eq!(desc.recompute_strategy, AnalysisRecomputeStrategy::Fit),
            AnalysisDomainKind::Stats => {
                assert!(matches!(
                    desc.recompute_strategy,
                    AnalysisRecomputeStrategy::Stats | AnalysisRecomputeStrategy::PowerWithInfoHooks | AnalysisRecomputeStrategy::MonteCarloEngine
                ))
            }
        }
    }

    for tk in TableKind::ALL {
        let default_id = default_analysis_kind_id_for_table(tk);
        let desc = descriptor_for_id(default_id);
        assert!(desc.supports_table_kind(tk), "default kind `{}` must support table `{tk:?}`", desc.slug);
    }
}

#[test]
fn lookup_helpers_resolve_every_descriptor_and_reject_unknown_queries() {
    for desc in &ALL_ANALYSIS_DESCRIPTORS {
        assert_eq!(descriptor_by_slug(desc.slug), Some(desc));
        assert_eq!(descriptor_by_serde_variant(desc.serde_variant), Some(desc));
        assert_eq!(resolve_analysis_descriptor(desc.slug), Some(desc));
        assert_eq!(resolve_analysis_descriptor(desc.serde_variant), Some(desc));
        if desc.user_creatable {
            assert_eq!(descriptor_by_shorthand(desc.canonical_shorthand), Some(desc));
            assert_eq!(descriptor_by_shorthand(&format!("  {}  ", desc.canonical_shorthand.to_ascii_uppercase())), Some(desc));
            assert_eq!(resolve_analysis_descriptor(desc.canonical_shorthand), Some(desc));
        } else {
            assert_eq!(descriptor_by_shorthand(desc.canonical_shorthand), None);
        }
        for &alias in desc.shorthands {
            assert_eq!(descriptor_by_shorthand(alias), Some(desc));
        }
        for &cat_id in desc.ui_catalog_ids {
            assert_eq!(descriptor_by_ui_catalog_id(cat_id), Some(desc));
            assert_eq!(resolve_analysis_descriptor(cat_id), Some(desc));
        }
    }

    assert_eq!(descriptor_by_slug(""), None);
    assert_eq!(descriptor_by_slug("unknown-analysis"), None);
    assert_eq!(descriptor_by_shorthand(""), None);
    assert_eq!(descriptor_by_shorthand("not_a_real_shorthand"), None);
    assert_eq!(descriptor_by_serde_variant("NonExistentVariant"), None);
    assert_eq!(descriptor_by_ui_catalog_id("stats.unknown"), None);
    assert_eq!(resolve_analysis_descriptor("  "), None);
}

#[test]
fn sub_enum_variant_keys_are_all_registered_including_pcr_outcome_column() {
    assert!(AnalysisKindId::Multivariate.descriptor().accepts_canonical_param_key("pcr_outcome_column"));
    let cases = [
        AnalysisKind::Multivariate(MultivariateSpec::Pca {
            standardize: true,
            parallel_analysis_seed: Some(42),
            pcr_outcome_column: Some(0),
            retention_rule: PcaRetentionRule::default(),
            emit_extra_matrices: false,
        }),
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true, method: Some(CorrelationMethod::KendallTauB) }),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(3),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(6),
            consensus_optimal_k: false,
            kmeans_algorithm: KMeansAlgorithm::default(),
            cluster_direction: ClusterDirection::default(),
            cut_height: None,
            emit_anova_tab: false,
        }),
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 10,
            seed: 7,
            response_column: Some(0),
            predictor_columns: vec![1, 2],
        }),
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: true }),
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: Some(0),
            mv_status_column: Some(1),
            mv_group_column: Some(2),
            event_code: None,
            censored_code: None,
            treat_other_as: SurvivalOtherValuePolicy::default(),
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        }),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
            formula: None,
            covariate_profiles: Vec::new(),
            nested_comparison: None,
            emit_cox_snell_residuals: false,
            ci_method: RegressionCiMethod::default(),
            event_code: None,
            censored_code: None,
            treat_other_as: SurvivalOtherValuePolicy::default(),
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: Some(MathTransformPreset::Log10),
            y_transform: None,
            error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            pharmacology_preset: None,
            interchange_xy: false,
            zero_x_substitute: None,
            asymmetric_ci_policy: AsymmetricCiTransformPolicy::default(),
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
            seed: 1,
            row_count: 10,
            replicates: 2,
            x_min: 0.0,
            x_max: 10.0,
            equation_id: String::from("four_pl"),
            parameters: vec![(String::from("Top"), 100.0)],
            noise: SimulationNoise::None,
            simulation_mode: None,
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 1,
            sample_size: 5,
            stratify_column: Some(0),
            proportional_allocation: true,
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
            filter_formula: Some(String::from("[A] > 0")),
            target_archetype: TableKind::Column,
            derived_columns: vec![DerivedColumnFormula::new("D", "[A] * 2")],
            complete_rows_only: false,
            subject_column: None,
            row_title_column: None,
            contingency_count_column: None,
            include_variable_name_prefixes: false,
        }),
    ];
    for kind in cases {
        let desc = kind.descriptor();
        let val = serde_json::to_value(&kind).unwrap();
        let inner = val.as_object().unwrap().get(desc.serde_variant).unwrap().as_object().unwrap();
        for sub_obj in inner.values().filter_map(|v| v.as_object()) {
            for key in sub_obj.keys() {
                assert!(desc.accepts_canonical_param_key(key), "descriptor `{}` missing key `{key}`", desc.slug);
            }
        }
    }
}

#[test]
fn r23_additive_extensions_preserve_v8_deserialization_and_round_trip() {
    use curvebook_doc::{
        BaselineKind, BaselineOperation, CurveCalculusOp, DiagnosticPlotKind, DistributionTailShade, FactorialComparisonFamilyMode, FormulaTerm,
        FractionDenominator, GlmDispersionMode, GlmWeightSpec, LognormalRowTestMode, ModelFormulaSpec, MonteCarloHitOp, MonteCarloHitRule,
        MonteCarloTrackedScalar, OneWayMethod, PharmacologyTransformPreset, PowerEffectSpec, PowerTestFamily, ProportionCiMethod,
        RelationalConstraint, RelationalConstraintOp, RemlZeroVariancePolicy, RmSphericityCorrection, RowSummaryKind, SheetId, SurvivalPowerMethod,
        TheoreticalDistributionFamily, TheoreticalDistributionSpec, TwoGroupMethod,
    };

    // Legacy v8 JSON payloads without any R23 fields must still deserialize cleanly with exact defaults.
    let legacy_glm: GlmSpec =
        serde_json::from_str(r#"{"family":"Linear","response_column":0,"predictor_columns":[1,2],"reference_levels":[],"compute_vif":true}"#)
            .expect("legacy GlmSpec must deserialize");
    assert_eq!(legacy_glm.formula, None);
    assert_eq!(legacy_glm.weights, GlmWeightSpec::None);
    assert_eq!(legacy_glm.dispersion_mode, GlmDispersionMode::Canonical);
    assert_eq!(legacy_glm.ci_method, RegressionCiMethod::Wald);
    assert!(!legacy_glm.emit_influence_diagnostics);

    let legacy_factorial: FactorialAnovaSpec = serde_json::from_str(
        r#"{"three_way":false,"multifactor_columns":[],"response_column":null,"max_interaction_order":2,"ss_type":"TypeIII","repeated_measures":"None","fit_reml_on_missing":true,"compute_effect_sizes":true,"multiplicity":{"method":"None","threshold":0.05,"report_confidence_intervals":true}}"#,
    )
    .expect("legacy FactorialAnovaSpec must deserialize");
    assert_eq!(legacy_factorial.sphericity_correction, RmSphericityCorrection::None);
    assert_eq!(legacy_factorial.reml_zero_variance_policy, RemlZeroVariancePolicy::BoundAtZero);
    assert_eq!(legacy_factorial.comparison_family_mode, FactorialComparisonFamilyMode::Standard);

    let legacy_power: PowerAnalysisSpec = serde_json::from_str(
        r#"{"family":"TwoSampleUnpairedT","solve_target":"SampleSize","tail_mode":"TwoSided","alpha":0.05,"target_power":0.8,"allocation_ratio":1.0,"effect":{"Standardized":0.5},"chi_square_df":1.0,"two_proportions_mode":"PooledNullUnpooledAlt","continuity_correction":false,"correlation_method":"FisherZ"}"#,
    )
    .expect("legacy PowerAnalysisSpec must deserialize");
    assert_eq!(legacy_power.survival_method, SurvivalPowerMethod::Schoenfeld);
    assert_eq!(legacy_power.target_events, None);

    let legacy_remove_baseline: TransformAnalysisSpec =
        serde_json::from_str(r#"{"RemoveBaseline":{"baseline_columns":[0],"operation":"Difference"}}"#)
            .expect("legacy RemoveBaseline must deserialize");
    assert!(matches!(legacy_remove_baseline, TransformAnalysisSpec::RemoveBaseline { baseline_kind: None, retain_unselected_columns: false, .. }));

    // Rich R23 payloads round-trip cleanly through JSON.
    let formula = ModelFormulaSpec {
        include_intercept: true,
        terms: vec![FormulaTerm::Main(1), FormulaTerm::Interaction2(1, 2), FormulaTerm::Power { column: 1, degree: 2 }, FormulaTerm::Sqrt(2)],
    };
    let mut fit = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit.relational_constraints.push(RelationalConstraint {
        lhs_param: String::from("Top"),
        op: RelationalConstraintOp::GreaterThan,
        rhs_param: String::from("Bottom"),
        margin: 1e-3,
    });
    fit.plot_distribution = Some(TheoreticalDistributionSpec {
        family: TheoreticalDistributionFamily::StudentT,
        parameters: vec![10.0],
        parameter_hooks: Vec::new(),
        tail_shade: DistributionTailShade::TwoTailed { lower: -2.228, upper: 2.228 },
        cumulative: false,
    });
    fit.emit_extra_gof = true;

    let kinds = vec![
        AnalysisKind::RowStatistics(RowStatisticsSpec { summary: RowSummaryKind::MedianPercentiles, upper_percentile: Some(90.0) }),
        AnalysisKind::FractionOfTotal(FractionOfTotalSpec {
            denominator: FractionDenominator::AllThree,
            output_percent: false,
            ci_method: ProportionCiMethod::Wilson,
        }),
        AnalysisKind::CurveCalculus(CurveCalculusSpec { operation: CurveCalculusOp::SmoothSavitzkyGolay { window: 5, order: 2 } }),
        AnalysisKind::MonteCarloSimulation(MonteCarloSpec {
            seed: 7,
            iterations: 250,
            downstream_result_sheet_id: Some(SheetId::new("fit-1")),
            tracked_scalars: vec![MonteCarloTrackedScalar {
                label: String::from("EC50"),
                result_sheet_id: None,
                parameter_key: String::from("EC50"),
                data_set_index: Some(0),
            }],
            hit_rules: vec![MonteCarloHitRule { scalar_index: 0, op: MonteCarloHitOp::Between, threshold: 0.1, upper_threshold: Some(10.0) }],
            exclude_flagged_fits: true,
        }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleRatioT { hypothetical: 1.0 }, ..TwoGroupSpec::default() }),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::LognormalRepeatedMeasures { geisser_greenhouse: true },
            ..OneWayAnovaSpec::default()
        }),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            lognormal_mode: LognormalRowTestMode::RatioPaired,
            ..MultipleRowTestsSpec::default()
        }),
        AnalysisKind::GeneralizedLinearModel(GlmSpec { formula: Some(formula.clone()), emit_influence_diagnostics: true, ..GlmSpec::default() }),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(3),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(8),
            consensus_optimal_k: true,
            kmeans_algorithm: KMeansAlgorithm::HartiganWong,
            cluster_direction: ClusterDirection::Both,
            cut_height: Some(2.5),
            emit_anova_tab: true,
        }),
        AnalysisKind::Multivariate(MultivariateSpec::Pca {
            standardize: true,
            parallel_analysis_seed: Some(1),
            pcr_outcome_column: None,
            retention_rule: PcaRetentionRule::CumulativeVariancePercent(85.0),
            emit_extra_matrices: true,
        }),
        AnalysisKind::PowerAnalysis(PowerAnalysisSpec {
            family: PowerTestFamily::LogRankSurvival,
            effect: PowerEffectSpec::HazardRatio { hazard_ratio: 0.65, event_prob_group1: Some(0.6), event_prob_group2: Some(0.45) },
            survival_method: SurvivalPowerMethod::Freedman,
            ..PowerAnalysisSpec::default()
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: None,
            error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            pharmacology_preset: Some(PharmacologyTransformPreset::Scatchard),
            interchange_xy: true,
            zero_x_substitute: Some(1e-12),
            asymmetric_ci_policy: AsymmetricCiTransformPolicy::AsymmetricEndpoints,
        }),
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: Vec::new(),
            operation: BaselineOperation::Add,
            baseline_kind: Some(BaselineKind::OlsLinearWithX { first_k: 3, last_k: 3 }),
            retain_unselected_columns: true,
        }),
        AnalysisKind::NonlinearRegression(fit),
        AnalysisKind::Ancova(AncovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::HolmBonferroni, ..MultiplicitySpec::default() },
            ..AncovaSpec::default()
        }),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Duncan, ..MultiplicitySpec::default() },
            ..OneWayAnovaSpec::default()
        }),
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false, method: Some(CorrelationMethod::KendallTauB) }),
    ];
    for kind in kinds {
        let json = serde_json::to_string(&kind).expect("serialize R23 AnalysisKind");
        let parsed: AnalysisKind = serde_json::from_str(&json).expect("deserialize R23 AnalysisKind");
        assert_eq!(parsed, kind);
    }

    // Legacy CorrelationMatrix JSON without `method` must still deserialize cleanly.
    let legacy_corr: AnalysisKind = serde_json::from_str(r#"{"Multivariate":{"CorrelationMatrix":{"spearman_rank":true}}}"#).expect("legacy corr");
    assert_eq!(legacy_corr, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true, method: None }));
    assert_eq!(CorrelationMethod::resolve(true, None), CorrelationMethod::Spearman);
    assert_eq!(CorrelationMethod::resolve(false, None), CorrelationMethod::Pearson);
    assert_eq!(CorrelationMethod::resolve(false, Some(CorrelationMethod::KendallTauB)), CorrelationMethod::KendallTauB);

    for plot in [
        DiagnosticPlotKind::HomoscedasticityPlot,
        DiagnosticPlotKind::ActualVsPredictedPlot,
        DiagnosticPlotKind::ResidualVsRowPlot,
        DiagnosticPlotKind::GapStatisticPlot,
        DiagnosticPlotKind::ParameterCorrelationHeatmap,
        DiagnosticPlotKind::LognormalQqPlot,
        DiagnosticPlotKind::Dendrogram,
        DiagnosticPlotKind::PcaBiplot,
    ] {
        let json = serde_json::to_string(&plot).expect("serialize DiagnosticPlotKind");
        let parsed: DiagnosticPlotKind = serde_json::from_str(&json).expect("deserialize DiagnosticPlotKind");
        assert_eq!(parsed, plot);
    }
}

#[test]
fn frequency_distribution_and_histogram_spec_serde_and_finiteness_guards() {
    use curvebook_doc::{AnalysisSpec, ColumnDesignHint, ColumnTable, DataSheet, SubcolumnAggregation, TableArchetype};

    // Minimal empty JSON object deserializes cleanly with defaults.
    let fd_default: FrequencyDistributionSpec = serde_json::from_str("{}").expect("empty FrequencyDistributionSpec");
    assert_eq!(fd_default, FrequencyDistributionSpec::default());
    assert_eq!(fd_default.binning_rule, FrequencyBinningRule::AutoFreedmanDiaconis);
    assert_eq!(fd_default.output_mode, FrequencyOutputMode::Counts);
    assert_eq!(fd_default.aggregation, SubcolumnAggregation::PooledGrandSample);
    assert!(fd_default.create_histogram_graph);
    assert!(!fd_default.overlay_gaussian);

    let fd_custom = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 2.5 },
        first_bin_center: Some(0.0),
        last_bin_center: Some(25.0),
        output_mode: FrequencyOutputMode::CumulativePercentages,
        overlay_gaussian: true,
        aggregation: SubcolumnAggregation::RowMeans,
        create_histogram_graph: false,
    };
    let kind = AnalysisKind::FrequencyDistribution(fd_custom.clone());
    assert_eq!(kind.derive_auto_title(), "Frequency distribution");
    assert!(kind.is_compatible_with(TableKind::Column));
    assert!(kind.is_compatible_with(TableKind::Xy));
    assert!(kind.is_compatible_with(TableKind::Grouped));
    assert!(kind.is_compatible_with(TableKind::MultipleVariables));
    assert!(!kind.is_compatible_with(TableKind::Contingency));
    assert!(!kind.is_compatible_with(TableKind::Survival));

    let rt: AnalysisKind = serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
    assert_eq!(rt, kind);

    // Non-finite floats in FrequencyDistributionSpec must be rejected by AnalysisSpec::new.
    let bad_width = AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: f64::NAN },
        ..FrequencyDistributionSpec::default()
    });
    assert!(AnalysisSpec::new("res-bad-w", "d1", bad_width).is_err());

    let bad_first = AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec {
        first_bin_center: Some(f64::INFINITY),
        ..FrequencyDistributionSpec::default()
    });
    assert!(AnalysisSpec::new("res-bad-f", "d1", bad_first).is_err());

    let bad_last = AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec {
        last_bin_center: Some(f64::NEG_INFINITY),
        ..FrequencyDistributionSpec::default()
    });
    assert!(AnalysisSpec::new("res-bad-l", "d1", bad_last).is_err());

    // GraphKind::Histogram and HistogramSpec backward compatibility + validation.
    let hist_kind: GraphKind = serde_json::from_str(r#""histogram""#).expect("alias deserialize");
    assert_eq!(hist_kind, GraphKind::Histogram);

    let col = ColumnTable::new(ColumnDesignHint::default(), 1, 3).unwrap();
    let ds = DataSheet::new("d1", "Data", TableArchetype::Column(col)).unwrap();
    let mut gs = GraphSpec::default_for_data_sheet(&ds);
    gs.kind = GraphKind::Histogram;
    gs.histogram = HistogramSpec {
        display_style: HistogramDisplayStyle::XyCurve,
        bin_alignment: HistogramBinAlignment::BinEdges,
        cumulative: true,
        show_gaussian_overlay: true,
        multi_series_mode: HistogramMultiSeriesMode::SideBySide,
        bar_alpha: 0.6,
        binning_rule: FrequencyBinningRule::AutoScott,
        output_mode: FrequencyOutputMode::Density,
    };
    let mut val = serde_json::to_value(&gs).unwrap();
    let gs_rt: GraphSpec = serde_json::from_value(val.clone()).unwrap();
    assert_eq!(gs_rt, gs);

    // Legacy GraphSpec JSON lacking `histogram` must deserialize to HistogramSpec::default().
    val.as_object_mut().unwrap().remove("histogram");
    let gs_legacy: GraphSpec = serde_json::from_value(val).unwrap();
    assert_eq!(gs_legacy.histogram, HistogramSpec::default());

    // Invalid bar_alpha or explicit width <= 0 rejected on deserialize / validate.
    let mut gs_bad = gs.clone();
    gs_bad.histogram.bar_alpha = 1.5;
    assert!(gs_bad.validate().is_err());
    let mut gs_bad_w = gs;
    gs_bad_w.histogram.binning_rule = FrequencyBinningRule::ExplicitWidth { width: -1.0 };
    assert!(gs_bad_w.validate().is_err());
}

#[test]
fn descriptive_spec_additive_extensions_preserve_legacy_json_and_precision_helper() {
    use curvebook_doc::DescriptiveMetricVisibility;

    // Legacy JSON omitting visible_metrics, significant_digits, and save_as_default must deserialize cleanly.
    let legacy: DescriptiveSpec =
        serde_json::from_str(r#"{"aggregation":"RowMeans","run_normality_battery":true,"hypothetical_value":null,"custom_upper_percentile":90.0}"#)
            .expect("legacy DescriptiveSpec JSON must deserialize");
    assert_eq!(legacy.visible_metrics, DescriptiveMetricVisibility::all());
    assert_eq!(legacy.visible_metrics.hidden_count(), 0);
    assert_eq!(legacy.significant_digits, None);
    assert!(!legacy.save_as_default);

    // Default serialization omits the additive defaults for backward-compatible archives.
    let default_val = serde_json::to_value(DescriptiveSpec::default()).expect("serialize default DescriptiveSpec");
    let obj = default_val.as_object().expect("DescriptiveSpec JSON object");
    assert!(!obj.contains_key("visible_metrics"));
    assert!(!obj.contains_key("significant_digits"));
    assert!(!obj.contains_key("save_as_default"));

    // Partial visibility patch preserves all omitted flags as true.
    let partial: DescriptiveSpec =
        serde_json::from_str(r#"{"visible_metrics":{"harmonic_mean":false,"sum":false},"significant_digits":4,"save_as_default":true}"#)
            .expect("partial DescriptiveSpec JSON must deserialize");
    assert!(!partial.visible_metrics.harmonic_mean);
    assert!(!partial.visible_metrics.sum);
    assert!(partial.visible_metrics.mean_sd_sem);
    assert!(partial.visible_metrics.geometric_mean);
    assert_eq!(partial.visible_metrics.hidden_count(), 2);
    assert_eq!(partial.significant_digits, Some(4));
    assert!(partial.save_as_default);

    // Round-trip non-default DescriptiveSpec.
    let round_trip: DescriptiveSpec =
        serde_json::from_str(&serde_json::to_string(&partial).expect("serialize partial")).expect("deserialize round-trip");
    assert_eq!(round_trip, partial);

    // Unified significant-digits helper covers both DescriptiveStats and NonlinearRegression.
    assert_eq!(AnalysisKind::DescriptiveStats(partial).output_significant_digits(), Some(4));
    assert_eq!(AnalysisKind::DescriptiveStats(DescriptiveSpec::default()).output_significant_digits(), None);
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.output_significant_digits = Some(5);
    assert_eq!(AnalysisKind::NonlinearRegression(nl).output_significant_digits(), Some(5));
    assert_eq!(AnalysisKind::LinearRegression(LinearRegressionSpec::default()).output_significant_digits(), None);
}
