//! Completeness, bijection, and metadata invariants for [`ALL_ANALYSIS_DESCRIPTORS`].

use std::collections::BTreeSet;

use curvebook_doc::analysis::{
    ALL_ANALYSIS_DESCRIPTORS, AnalysisDomainKind, AnalysisKindId, AnalysisRecomputeStrategy, AnalysisUiCategory, default_analysis_kind_id_for_table,
    descriptor_by_serde_variant, descriptor_by_shorthand, descriptor_by_slug, descriptor_by_ui_catalog_id, descriptor_for_id, descriptor_for_kind,
    resolve_analysis_descriptor,
};
use curvebook_doc::{
    AnalysisKind, BlandAltmanSpec, ContingencyMethod, ContingencySpec, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, KappaWeighting,
    LinearRegressionSpec, MultipleRowTestsSpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, OneWayAnovaSpec, OutlierSpec,
    PValueStackSpec, PassingBablokSpec, PowerAnalysisSpec, RocSpec, SimpleLogisticSpec, SplineLowessSpec, SurvivalAnalysisSpec, TableKind,
    TransformAnalysisSpec, TwoGroupSpec,
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
            }),
        ),
        (AnalysisKindId::RocCurve, AnalysisKind::RocCurve(RocSpec::default())),
        (
            AnalysisKindId::GeneralizedLinearModel,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![1],
                reference_levels: Vec::new(),
                compute_vif: true,
            }),
        ),
        (AnalysisKindId::Multivariate, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false })),
        (AnalysisKindId::LinearRegression, AnalysisKind::LinearRegression(LinearRegressionSpec::default())),
        (AnalysisKindId::NonlinearRegression, AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))),
        (AnalysisKindId::PValueStack, AnalysisKind::PValueStack(PValueStackSpec::default())),
        (AnalysisKindId::PowerAnalysis, AnalysisKind::PowerAnalysis(PowerAnalysisSpec::default())),
        (AnalysisKindId::SimpleLogisticRegression, AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())),
        (AnalysisKindId::FitSplineLowess, AnalysisKind::FitSplineLowess(SplineLowessSpec::default())),
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

        // Every concrete top-level struct key produced by Serde must be listed in `canonical_param_keys`
        // (sub-enums `Transform`/`OutlierDetection`/`Survival`/`Multivariate` wrap another variant layer).
        if let Some(inner) = obj.get(variant_key).and_then(|v| v.as_object())
            && !matches!(
                desc.id,
                AnalysisKindId::Transform | AnalysisKindId::OutlierDetection | AnalysisKindId::Survival | AnalysisKindId::Multivariate
            )
        {
            for field_key in inner.keys() {
                assert!(
                    desc.accepts_canonical_param_key(field_key),
                    "descriptor `{}` missing serialized canonical param key `{field_key}`",
                    desc.slug
                );
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
        assert!(!desc.shorthands.is_empty());
        assert!(desc.shorthands.contains(&desc.canonical_shorthand));

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
        assert!(!desc.compatible_table_kinds.is_empty());

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
                assert!(matches!(desc.recompute_strategy, AnalysisRecomputeStrategy::Stats | AnalysisRecomputeStrategy::PowerWithInfoHooks))
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
        assert_eq!(descriptor_by_shorthand(desc.canonical_shorthand), Some(desc));
        assert_eq!(descriptor_by_shorthand(&format!("  {}  ", desc.canonical_shorthand.to_ascii_uppercase())), Some(desc));
        assert_eq!(resolve_analysis_descriptor(desc.slug), Some(desc));
        assert_eq!(resolve_analysis_descriptor(desc.serde_variant), Some(desc));
        assert_eq!(resolve_analysis_descriptor(desc.canonical_shorthand), Some(desc));
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
