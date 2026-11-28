//! Cross-cutting engine contracts across all statistical analyses:
//! - Unified `selected_data_sets` OOB / duplicate / arity validation matrix (`#32`, `FA-SEC05-02/03/11`)
//! - `MultipleVariables` `Label` / `Categorical` role isolation (`#66`, `FA-SEC05-03/11`)
//! - Alpha-aware `ResultCell::PValue` significance stars (`#82`, `FA-SEC05-12`, `FA-SEC06-09`)
//! - Ignored-text warnings, listwise-deletion notices, and finite-sheet invariants (`R1-S02-13`, `R1-X2-08/09`, `FA-SEC06-11`)
//! - Diagnostics-only assumption checklist policy (`remediation: None`, zero forbidden phrases)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, BlandAltmanSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec,
    CoxTieMethod, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, KappaWeighting, MultipleRowTestsSpec, MultiplicityMethod,
    MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, OneWayAnovaSpec, OneWayMethod, OutlierSpec, PValueFormat, PValueStackMethod,
    PValueStackSpec, PassingBablokSpec, ResultCell, RocCiMethod, RocSpec, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod,
    TwoGroupSpec, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::StatsError;
use curvebook_stats::common::{p_cell_at_alpha, resolve_selected_indices, try_resolve_selected_indices};
use curvebook_stats::multiplicity::{MultiplicityProcedure, build_multiplicity_tab, compare_groups_with_procedure};
use curvebook_stats::two_group::unpaired_t_test;

use crate::common::{
    assert_all_sheet_numbers_finite, assert_bad_selections, assert_close, assert_diagnostics_only, assert_p_stars_ns, cell_num, cell_p_stars,
    col_sheet, col_sheet_cells, contingency_sheet, group_input, grouped_sheet_fill, has_ignored_text_warn, has_listwise_info, mv_cells_sheet,
    mv_num_sheet, nested_sheet, parts_sheet, presum_col_sheet, row_cell, run_kind, run_kind_conf, run_kind_sel, scalar, scalar_ds, survival_sheet,
    tab_by_id, xy_multi_sheet,
};

#[test]
fn audit_32_unified_selected_data_sets_validation_matrix() {
    assert_eq!(try_resolve_selected_indices(&[], 3).unwrap(), vec![0, 1, 2]);
    assert_eq!(try_resolve_selected_indices(&[2, 0], 3).unwrap(), vec![2, 0]);
    assert_eq!(resolve_selected_indices(&[99, 1], 3), vec![1]);
    assert!(matches!(try_resolve_selected_indices(&[3], 3), Err(StatsError::MissingColumn { index: 3 })));
    assert!(matches!(try_resolve_selected_indices(&[1, 1], 3), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    // 1. OOB & duplicate rejection matrix across Column, Grouped, Nested, Contingency, Survival, MV, XY.
    let col2 = col_sheet("c2", &[("A", &[1.0, 2.0, 3.0, 4.0, 5.0]), ("B", &[2.0, 3.0, 4.0, 5.0, 6.0])]);
    assert_bad_selections(
        &col2,
        &[
            AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
            AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..Default::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..Default::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..Default::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, ..Default::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..Default::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, ..Default::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::Friedman, ..Default::default() }),
        ],
        &[0, 99],
        &[0, 0],
    );

    let grp = grouped_sheet_fill("g2", 2, 2, 2, |c, r, s| Some((c + r + s + 1) as f64));
    assert_bad_selections(&grp, &[AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())], &[0, 4], &[1, 1]);

    let nest = nested_sheet("n2", &[("G0", &[&[1.0, 2.0], &[3.0, 4.0]]), ("G1", &[&[10.0, 11.0], &[12.0, 13.0]])]);
    assert_bad_selections(&nest, &[AnalysisKind::NestedComparison(NestedAnalysisSpec::default())], &[0, 7], &[0, 1, 0]);

    let ct = contingency_sheet("ct2", &[&[10, 2], &[3, 9]]);
    assert_bad_selections(
        &ct,
        &[
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::PearsonChiSquare { yates_continuity: false },
                ..Default::default()
            }),
            AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FisherExact, ..Default::default() }),
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
                ..Default::default()
            }),
        ],
        &[0, 1, 99],
        &[0, 0],
    );
    let ct12 = contingency_sheet("ct12", &[&[14, 6]]);
    assert_bad_selections(
        &ct12,
        &[AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.5), ..Default::default() })],
        &[0, 99],
        &[0, 0],
    );

    let surv = survival_sheet(
        "s2",
        &[("A", &[(1.0, true), (2.0, true), (3.0, true), (4.0, true)]), ("B", &[(1.0, true), (2.0, true), (3.0, true), (4.0, true)])],
    );
    assert_bad_selections(
        &surv,
        &[AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
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
        })],
        &[9],
        &[0, 0],
    );

    let mv = mv_num_sheet(
        "mv3",
        &[("Y", &[3.0, 5.0, 7.0, 9.0, 11.0, 13.0]), ("X1", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), ("X2", &[0.0, 1.0, 0.0, 1.0, 0.0, 1.0])],
    );
    assert_bad_selections(
        &mv,
        &[
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 2,
                predictor_columns: vec![1],
                reference_levels: vec![],
                tie_method: CoxTieMethod::Breslow,
                formula: None,
                covariate_profiles: Vec::new(),
                nested_comparison: None,
                emit_cox_snell_residuals: false,
                ci_method: Default::default(),
                event_code: None,
                censored_code: None,
                treat_other_as: Default::default(),
            }),
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![1],
                reference_levels: vec![],
                compute_vif: false,
                ..Default::default()
            }),
            AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 1, y_column: 0 }),
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false, method: None }),
            AnalysisKind::Multivariate(MultivariateSpec::Pca {
                standardize: true,
                parallel_analysis_seed: None,
                pcr_outcome_column: None,
                retention_rule: Default::default(),
                emit_extra_matrices: false,
            }),
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: Some(2),
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Ward,
                k_sweep_max: None,
                consensus_optimal_k: false,
                kmeans_algorithm: Default::default(),
                cluster_direction: Default::default(),
                cut_height: None,
                emit_anova_tab: false,
            }),
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 3,
                max_iters: 5,
                seed: 7,
                response_column: None,
                predictor_columns: vec![],
            }),
        ],
        &[12],
        &[0, 1, 0],
    );

    let xy = xy_multi_sheet("xy2", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Y0", &[2.0, 4.0, 6.0, 8.0, 10.0]), ("Y1", &[3.0, 6.0, 9.0, 12.0, 15.0])]);
    assert_bad_selections(
        &xy,
        &[AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![0],
            reference_levels: vec![],
            compute_vif: false,
            ..Default::default()
        })],
        &[9],
        &[0, 0],
    );

    // 2. Explicit column arity validation (FA-SEC05-02/03/11): no silent truncation on TwoGroup, OneSample, ExactBinomial, ROC.
    let c3 = col_sheet("c3", &[("A", &[1.0, 2.0, 3.0, 4.0]), ("B", &[5.0, 6.0, 7.0, 8.0]), ("C", &[20.0, 22.0, 24.0, 26.0])]);
    for method in [
        TwoGroupMethod::UnpairedT,
        TwoGroupMethod::WelchT,
        TwoGroupMethod::LognormalUnpairedT,
        TwoGroupMethod::PairedT,
        TwoGroupMethod::RatioPairedT,
        TwoGroupMethod::MannWhitneyU,
        TwoGroupMethod::WilcoxonSignedRank,
        TwoGroupMethod::PairedSignTest,
        TwoGroupMethod::KolmogorovSmirnov,
    ] {
        for bad_sel in [vec![0], vec![0, 1, 2]] {
            assert!(matches!(
                run_kind_sel(&c3, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }), bad_sel),
                Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
            ));
        }
    }
    for method in [TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, TwoGroupMethod::OneSampleWilcoxon { hypothetical: 0.0 }] {
        assert!(matches!(
            run_kind_sel(&c3, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }), vec![0, 1]),
            Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
        ));
        assert!(run_kind_sel(&c3, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }), vec![1]).is_ok());
    }
    let pre3 =
        presum_col_sheet("pre3", SubcolumnMode::MeanSdN, &[("A", &[&[10.0, 1.0, 5.0]]), ("B", &[&[15.0, 1.5, 5.0]]), ("C", &[&[20.0, 2.0, 5.0]])]);
    assert!(matches!(
        run_kind_sel(&pre3, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()), vec![0, 1, 2]),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
    ));
    assert!(matches!(
        run_kind_sel(
            &pre3,
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..Default::default() }),
            vec![0, 1]
        ),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
    ));

    let col_bin = col_sheet("cbin", &[("C0", &[1.0, 0.0, 1.0, 0.0]), ("C1", &[1.0, 1.0, 1.0, 1.0, 1.0, 0.0])]);
    let bin_k =
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.5), ..Default::default() });
    assert!(matches!(run_kind_sel(&col_bin, bin_k.clone(), vec![0, 1]), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
    assert_close(scalar_ds(&run_kind_sel(&col_bin, bin_k, vec![1]).unwrap(), "successes", None), 5.0, 1e-12, "ExactBinomial Col[1]");
    for bad_sel in [vec![0], vec![0, 1, 2]] {
        assert!(matches!(
            run_kind_sel(&c3, AnalysisKind::RocCurve(RocSpec::default()), bad_sel),
            Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
        ));
    }
}

#[test]
fn audit_66_mv_label_and_categorical_role_isolation() {
    // 1. Default MV selections exclude Label & Categorical from Descriptive, Outliers, OneSample, Unpaired, Paired (#66).
    let ids = [101.0, 102.0, 103.0, 104.0, 105.0, 106.0].map(Cell::number);
    let grps = ["Ctrl", "Ctrl", "Ctrl", "Drug", "Drug", "Drug"].map(Cell::text);
    let scores = [10.0, 12.0, 11.0, 20.0, 22.0, 21.0].map(Cell::number);
    let mv = mv_cells_sheet("mv66", &[("SubjectID", Some(VariableRole::Label), &ids), ("Group", None, &grps), ("Score", None, &scores)]);

    assert_eq!(
        tab_by_id(&run_kind(&mv, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap(), "desc-summary").column_headers,
        vec!["Score"]
    );
    assert!(
        run_kind(&mv, AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }))
            .unwrap()
            .scalars
            .iter()
            .all(|s| s.data_set_index == Some(2))
    );
    let r1s = run_kind(
        &mv,
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 16.0 }, ..Default::default() }),
    )
    .unwrap();
    assert_close(scalar_ds(&r1s, "t", None), 0.0, 1e-12, "1s targets Score");
    let runp = run_kind(&mv, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    assert_close(
        cell_num(row_cell(tab_by_id(&runp, "two-group"), "Difference between means (A - B)", 0)),
        -10.0,
        1e-12,
        "unpaired groups Score by Group",
    );

    let mv1 = mv_cells_sheet("mv1", &[("ID", Some(VariableRole::Label), &ids[..4]), ("Score", None, &scores[..4])]);
    assert!(matches!(
        run_kind(&mv1, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..Default::default() })),
        Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
    ));

    // 2. Ambiguous >= 2 Categorical factors rejected unless narrowed or explicit (FA-SEC05-03).
    let mv2c = mv_cells_sheet(
        "mv2c",
        &[
            ("Sex", Some(VariableRole::Categorical), &["F", "F", "M", "M"].map(Cell::text)),
            ("Arm", Some(VariableRole::Categorical), &["Ctrl", "Trt", "Ctrl", "Trt"].map(Cell::text)),
            ("Before", None, &[10.0, 11.0, 12.0, 13.0].map(Cell::number)),
            ("After", None, &[20.0, 22.0, 25.0, 27.0].map(Cell::number)),
        ],
    );
    for sel in [vec![], vec![0, 1, 2, 3]] {
        assert!(
            matches!(run_kind_sel(&mv2c, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()), sel), Err(StatsError::InvalidParameter { name: "mv_group_column", value }) if (value - 2.0).abs() < 1e-12)
        );
    }
    let exp_w = unpaired_t_test(&[10.0, 11.0, 12.0, 13.0], &[20.0, 22.0, 25.0, 27.0], true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(
        scalar_ds(&run_kind_sel(&mv2c, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()), vec![2, 3]).unwrap(), "t", None),
        exp_w.t_statistic,
        1e-12,
        "wide [2,3]",
    );
    let exp_sex = unpaired_t_test(&[10.0, 11.0], &[12.0, 13.0], true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(
        scalar_ds(
            &run_kind(
                &mv2c,
                AnalysisKind::TwoGroupComparison(TwoGroupSpec { mv_group_column: Some(0), mv_response_column: Some(2), ..Default::default() }),
            )
            .unwrap(),
            "t",
            None,
        ),
        exp_sex.t_statistic,
        1e-12,
        "explicit group+resp",
    );

    let mv_nr = mv_cells_sheet("mvnr", &[("Arm", Some(VariableRole::Categorical), &grps[..4]), ("ID", Some(VariableRole::Label), &ids[..4])]);
    assert!(matches!(
        run_kind(&mv_nr, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())),
        Err(StatsError::InvalidParameter { name: "mv_response_column", .. })
    ));

    // 3. Explicit selections including Label skip Label in OneSample, TwoGroup, MultipleRow, and ROC (FA-SEC05-03/11).
    let mv_lbl = mv_cells_sheet(
        "mvlbl",
        &[
            ("ID", Some(VariableRole::Label), &[1000.0, 1001.0, 1002.0, 1003.0].map(Cell::number)),
            ("A1", None, &[1.0, 2.0, 3.0, 4.0].map(Cell::number)),
            ("A2", None, &[1.5, 2.5, 3.5, 4.5].map(Cell::number)),
            ("B1", None, &[10.0, 11.0, 12.0, 13.0].map(Cell::number)),
            ("B2", None, &[10.5, 11.5, 12.5, 13.5].map(Cell::number)),
        ],
    );
    let os_k = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..Default::default() });
    assert!(matches!(run_kind_sel(&mv_lbl, os_k.clone(), vec![0, 1, 2]), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
    assert!(matches!(run_kind_sel(&mv_lbl, os_k.clone(), vec![0]), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
    assert_close(cell_num(&tab_by_id(&run_kind_sel(&mv_lbl, os_k, vec![0, 1]).unwrap(), "one-sample-t").rows[1][0]), 2.5, 1e-12, "1s skips Label");
    assert_close(
        cell_num(
            &tab_by_id(
                &run_kind(
                    &mv_lbl,
                    AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                        method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 },
                        mv_response_column: Some(0),
                        ..Default::default()
                    }),
                )
                .unwrap(),
                "one-sample-t",
            )
            .rows[1][0],
        ),
        1001.5,
        1e-12,
        "explicit mv_response_column=0",
    );
    assert!(matches!(
        run_kind_sel(&mv_lbl, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()), vec![0, 1]),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
    ));
    assert!(matches!(
        run_kind_sel(&mv_lbl, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default()), vec![0, 1, 2, 3]),
        Err(StatsError::InsufficientSampleSize { required: 4, actual: 3, .. })
    ));
    let mr5 = run_kind_sel(&mv_lbl, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default()), vec![0, 1, 2, 3, 4]).unwrap();
    let cmp_tab = tab_by_id(&mr5, "multiple-row-comparisons");
    assert_close(cell_num(&cmp_tab.rows[0][0]), 1.25, 1e-12, "MR skips Label Row 0 Mean(A)");
    assert_close(cell_num(&cmp_tab.rows[0][1]), 10.25, 1e-12, "MR skips Label Row 0 Mean(B)");
    assert_close(
        scalar(&run_kind_sel(&mv_lbl, AnalysisKind::RocCurve(RocSpec::default()), vec![0, 1, 3]).unwrap(), "Area under ROC curve (AUC)"),
        1.0,
        1e-12,
        "ROC skips Label",
    );
    assert!(matches!(
        run_kind_sel(&mv_lbl, AnalysisKind::RocCurve(RocSpec::default()), vec![0, 1]),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })
    ));
}

#[test]
fn audit_82_alpha_aware_pvalue_stars_across_all_families() {
    // 1. Direct p_cell_at_alpha boundary checks.
    for (p, fmt, alpha, want) in [
        (0.0369, PValueFormat::FourDecimal, 0.01, "ns"),
        (0.01, PValueFormat::FourDecimal, 0.01, "*"),
        (0.010_000_1, PValueFormat::FourDecimal, 0.01, "ns"),
        (0.002, PValueFormat::FourDecimal, 0.01, "**"),
        (0.05, PValueFormat::Apa, 0.05, "*"),
        (0.050_000_1, PValueFormat::Apa, 0.05, "ns"),
    ] {
        assert_eq!(cell_p_stars(&p_cell_at_alpha(p, fmt, alpha)).1, want);
    }

    // 2. Two-group, Descriptive one-sample, MultipleRow, PValueStack, Multiplicity tab, Contingency, PartsOfWhole, ROC.
    let (a, b) = ([1.0, 2.0, 3.0, 4.0, 5.0], [3.5, 4.5, 5.5, 6.5, 7.5]);
    let sh_tg = col_sheet("tg82", &[("A", &a), ("B", &b)]);
    assert_eq!(
        cell_p_stars(row_cell(
            tab_by_id(&run_kind_conf(&sh_tg, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()), 95.0).unwrap(), "two-group"),
            "P value",
            0
        ))
        .1,
        "*"
    );
    assert_p_stars_ns(
        row_cell(
            tab_by_id(&run_kind_conf(&sh_tg, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default()), 99.0).unwrap(), "two-group"),
            "P value",
            0,
        ),
        "TwoGroup 99%",
    );

    let sh_1s = col_sheet("1s82", &[("X", &a)]);
    let r_1s = run_kind_conf(
        &sh_1s,
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, hypothetical_value: Some(1.0), ..Default::default() }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(row_cell(tab_by_id(&r_1s, "one-sample"), "One-sample t P", 0), "Descriptive 1s 99%");

    let g_mr = grouped_sheet_fill("mr82", 2, 1, 5, |c, _, s| Some(if c == 0 { a[s] } else { b[s] }));
    let r_mr = run_kind_conf(
        &g_mr,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Bonferroni,
                threshold: 0.01,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
        95.0,
    )
    .unwrap();
    assert_eq!(cell_p_stars(row_cell(tab_by_id(&r_mr, "multiple-row-comparisons"), "Row 1", 5)).1, "*");
    assert_eq!(cell_p_stars(row_cell(tab_by_id(&r_mr, "multiple-row-comparisons"), "Row 1", 6)).1, "ns");

    let r_ps = run_kind(
        &col_sheet("ps82", &[("P", &[0.03, 0.002, 0.50])]),
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::Bonferroni, threshold: 0.01, column: None }),
    )
    .unwrap();
    let t_ps = tab_by_id(&r_ps, "pvalue-stack-adjusted");
    assert_eq!((&t_ps.rows[0][5], cell_p_stars(&t_ps.rows[0][3]).1, cell_p_stars(&t_ps.rows[0][4]).1), (&ResultCell::Boolean(false), "ns", "ns"));
    assert_eq!((&t_ps.rows[1][5], cell_p_stars(&t_ps.rows[1][4]).1), (&ResultCell::Boolean(true), "*"));

    let details = compare_groups_with_procedure(
        &[group_input(0, "A", 6.0, 10.0, 1.5), group_input(1, "B", 6.0, 12.1, 1.5)],
        2.25,
        10.0,
        MultiplicityProcedure::None,
        0.01,
        false,
    )
    .unwrap();
    assert_eq!(cell_p_stars(&build_multiplicity_tab(&details, "MC", PValueFormat::FourDecimal, ConfidenceLevel::PERCENT_99).rows[0][5]).1, "ns");
    assert_eq!(cell_p_stars(&build_multiplicity_tab(&details, "MC", PValueFormat::FourDecimal, ConfidenceLevel::DEFAULT_95).rows[0][5]).1, "ns");

    let r_ct = run_kind_conf(
        &contingency_sheet("ct82", &[&[11, 4], &[5, 10]]),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ..Default::default() }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_ct, "contingency-main").rows[0][2], "Contingency 99%");
    let r_pow = run_kind_conf(
        &parts_sheet("pow82", &[("A", 14.0), ("B", 4.0)]),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ..Default::default() }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_pow, "parts-gof").rows[0][2], "PoW 99%");
    let r_roc = run_kind_conf(
        &col_sheet("roc82", &[("C", &[1.0, 2.0, 3.0, 4.0, 5.0]), ("D", &[3.0, 4.0, 5.0, 6.0, 7.5])]),
        AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_roc, "roc-summary").rows[0][3], "ROC 99%");

    // 3. Passing-Bablok, Linear GLM, OneWay ANOVA, Correlation Matrix, Nested ANOVA, Survival log-rank at 99% (FA-SEC06-09).
    let xs20: Vec<f64> = (1..=20).map(f64::from).collect();
    let ys20: Vec<f64> = xs20.iter().map(|&x| 0.25 * x * x + 0.5 * x + 1.0).collect();
    let r_pb = run_kind_conf(
        &mv_num_sheet("pb99", &[("A", &xs20), ("B", &ys20)]),
        AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_pb, "pb-summary").rows[5][0], "PB 99%");

    let sh_glm = mv_num_sheet("g99", &[("Y", &[1.0, 1.0, 3.1, 1.9, 3.8, 4.0]), ("X", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])]);
    let r_glm = run_kind_conf(
        &sh_glm,
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: vec![],
            compute_vif: false,
            ..Default::default()
        }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_glm, "glm-linear-params").rows[1][3], "GLM param 99%");
    assert_p_stars_ns(&tab_by_id(&r_glm, "glm-linear-anova").rows[0][4], "GLM ANOVA 99%");
    assert_p_stars_ns(&tab_by_id(&r_glm, "glm-linear-type-iii").rows[0][4], "GLM T3 99%");
    let r_corr =
        run_kind_conf(&sh_glm, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false, method: None }), 99.0).unwrap();
    assert_p_stars_ns(&tab_by_id(&r_corr, "corr-matrix-p").rows[0][1], "Corr P 99%");
    assert_p_stars_ns(&tab_by_id(&r_corr, "corr-matrix-fdr").rows[0][1], "Corr q 99%");

    let r_ow = run_kind_conf(
        &col_sheet("ow99", &[("G0", &[1.0, 1.2, 1.5, 1.8, 2.0, 2.1]), ("G1", &[1.7, 1.9, 2.2, 2.5, 2.7, 2.8])]),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_ow, "anova-table").rows[0][4], "OneWay 99%");
    let r_nt = run_kind_conf(
        &nested_sheet(
            "nt99",
            &[("A", &[&[1.0, 1.1, 1.2], &[2.0, 2.1, 2.2], &[3.0, 3.1, 3.2]]), ("B", &[&[4.0, 4.1, 4.2], &[5.0, 5.1, 5.2], &[6.0, 6.1, 6.2]])],
        ),
        AnalysisKind::NestedComparison(NestedAnalysisSpec::default()),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_nt, "nested-anova").rows[0][4], "Nested 99%");

    let s_sh = survival_sheet(
        "sv99",
        &[
            ("A", &[(2.0, true), (3.0, true), (4.0, true), (5.0, true), (9.0, true)]),
            ("B", &[(6.0, true), (8.0, true), (11.0, true), (12.0, true), (14.0, true)]),
        ],
    );
    let r_sv = run_kind_conf(
        &s_sh,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
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
        }),
        99.0,
    )
    .unwrap();
    assert_p_stars_ns(&tab_by_id(&r_sv, "curve-comparison").rows[0][2], "Survival 99%");
}

#[test]
fn audit_ignored_text_listwise_finite_and_diagnostics_only_contracts() {
    // 1. Excluded non-numeric text does NOT warn; unexcluded text warns across Column, Grouped, MV, ROC, MultipleRow, GLM, PB.
    let mut excl_txt = Cell::text("<0.5");
    excl_txt.excluded = true;
    let clean = col_sheet_cells(
        "cl",
        ColumnDesignHint::UnpairedReplicates,
        &[
            ("A", &[Cell::number(1.0), Cell::number(1.5), Cell::number(2.0), Cell::number(2.5), excl_txt]),
            ("B", &[Cell::number(2.0), Cell::number(2.5), Cell::number(3.0), Cell::number(3.5), Cell::number(4.0)]),
        ],
    );
    assert!(!has_ignored_text_warn(&run_kind(&clean, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()));

    let dirty = col_sheet_cells(
        "dc",
        ColumnDesignHint::UnpairedReplicates,
        &[
            ("A", &[Cell::number(1.0), Cell::number(1.5), Cell::text("<0.5"), Cell::number(2.5), Cell::number(3.0), Cell::empty()]),
            ("B", &[Cell::number(3.0), Cell::number(4.3), Cell::number(4.8), Cell::number(6.1), Cell::number(6.6), Cell::text("BDL")]),
            ("C", &[Cell::number(6.0), Cell::number(8.0), Cell::number(8.6), Cell::number(10.6), Cell::number(11.2), Cell::empty()]),
        ],
    );
    for kind in [
        AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }),
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..Default::default() }),
        AnalysisKind::BlandAltman(BlandAltmanSpec::default()),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..Default::default() }),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, ..Default::default() }),
    ] {
        assert!(has_ignored_text_warn(&run_kind(&dirty, kind.clone()).unwrap()), "missing warn for {kind:?}");
    }
    let res_roc = run_kind_sel(&dirty, AnalysisKind::RocCurve(RocSpec::default()), vec![0, 1]).unwrap();
    assert!(has_ignored_text_warn(&res_roc));
    assert_all_sheet_numbers_finite(&res_roc, "ROC with ND");

    let mv_nd = mv_cells_sheet(
        "mvnd",
        &[
            (
                "Y",
                None,
                &[
                    Cell::number(2.0),
                    Cell::number(4.1),
                    Cell::number(6.0),
                    Cell::number(8.1),
                    Cell::number(10.0),
                    Cell::text("ND"),
                    Cell::number(14.0),
                ],
            ),
            (
                "X",
                None,
                &[Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0), Cell::number(5.0), Cell::number(6.0), Cell::empty()],
            ),
        ],
    );
    for kind in [
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: vec![],
            compute_vif: false,
            ..Default::default()
        }),
        AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 1, y_column: 0 }),
    ] {
        let r = run_kind(&mv_nd, kind).unwrap();
        assert!(has_ignored_text_warn(&r) && has_listwise_info(&r));
    }

    // 2. Listwise omission tracking on Friedman, Cox PH, MV KM, and REML non-deletion on RM one-way (R1-S02-13).
    let mut rm_tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 6).unwrap();
    for (r, vals) in [(0, [10.0, 12.0, 15.0]), (1, [11.0, 14.0, 17.0]), (3, [12.0, 15.0, 19.0])] {
        for c in 0..3 {
            rm_tab.data_sets[c].cells[r][0] = Cell::number(vals[c]);
        }
    }
    (rm_tab.data_sets[0].cells[2][0], rm_tab.data_sets[1].cells[2][0], rm_tab.data_sets[2].cells[2][0]) =
        (Cell::number(20.0), Cell::excluded_number(22.0), Cell::number(25.0));
    (rm_tab.data_sets[0].cells[4][0], rm_tab.data_sets[1].cells[4][0]) = (Cell::number(30.0), Cell::number(31.0));
    let rm_sh = DataSheet::new("rmlw", "RMLW", TableArchetype::Column(rm_tab)).unwrap();
    let rm_r = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, ..Default::default() }),
    )
    .unwrap();
    assert!(!has_listwise_info(&rm_r) && rm_r.checklist.iter().any(|c| c.assumption == "Mixed-effects REML (repeated measures)"));
    let fr_r = run_kind(&rm_sh, AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::Friedman, ..Default::default() })).unwrap();
    assert!(has_listwise_info(&fr_r) && fr_r.checklist.iter().any(|c| c.diagnostic_metric.as_deref() == Some("2 row(s) omitted; 3 retained")));

    let mv_s = mv_cells_sheet(
        "mvs-lw",
        &[
            (
                "T",
                None,
                &[
                    Cell::number(5.0),
                    Cell::number(8.0),
                    Cell::number(10.0),
                    Cell::number(12.0),
                    Cell::empty(),
                    Cell::number(15.0),
                    Cell::number(20.0),
                ],
            ),
            ("S", None, &[1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0].map(Cell::number)),
            ("G", Some(VariableRole::Categorical), &["A", "A", "B", "B", "A", "B", "A"].map(Cell::text)),
            (
                "X",
                None,
                &[Cell::number(1.2), Cell::number(0.9), Cell::empty(), Cell::number(2.1), Cell::number(1.8), Cell::number(2.5), Cell::number(0.7)],
            ),
        ],
    );
    let r_cox = run_kind(
        &mv_s,
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: vec![(2, "A".into())],
            tie_method: CoxTieMethod::Efron,
            formula: None,
            covariate_profiles: Vec::new(),
            nested_comparison: None,
            emit_cox_snell_residuals: false,
            ci_method: Default::default(),
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
        }),
    )
    .unwrap();
    assert!(has_listwise_info(&r_cox) && r_cox.checklist.iter().any(|c| c.diagnostic_metric.as_deref() == Some("2 row(s) omitted; 5 retained")));
    let r_km = run_kind(
        &mv_s,
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
            treat_other_as: Default::default(),
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        }),
    )
    .unwrap();
    assert!(has_listwise_info(&r_km) && r_km.checklist.iter().any(|c| c.diagnostic_metric.as_deref() == Some("1 row(s) omitted; 6 retained")));

    // 3. Diagnostics-only checklist policy across all major analysis families (remediation_checklist).
    let skewed =
        col_sheet("sk", &[("Ctrl", &[1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 50.0]), ("Treat", &[10.0, 10.2, 10.5, 10.8, 11.0, 11.2, 11.4, 250.0])]);
    let small = col_sheet("sm", &[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])]);
    let aov = col_sheet(
        "av",
        &[
            ("G1", &[1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 12.0]),
            ("G2", &[2.0, 5.0, 9.0, 14.0, 22.0, 35.0, 80.0]),
            ("G3", &[3.0, 3.2, 3.3, 3.5, 3.6, 3.9, 4.0]),
        ],
    );
    let ct_sp = contingency_sheet("csp", &[&[1, 2], &[3, 4]]);
    let mv_sep = mv_num_sheet("msep", &[("X", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]), ("Y", &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0])]);
    let mv_pois = mv_num_sheet("mpoi", &[("X", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]), ("Y", &[0.0, 1.0, 0.0, 12.0, 1.0, 25.0, 2.0, 40.0])]);
    let mv_cox = mv_num_sheet(
        "mcox",
        &[
            ("T", &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0]),
            ("S", &[1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0]),
            ("X", &[0.2, 0.4, 0.3, 0.8, 0.9, 0.5, 1.2, 1.4]),
        ],
    );
    let mv_cl = mv_num_sheet("mcl", &[("X", &[0.0, 1.0, 2.0, 3.0, 4.0]), ("Y", &[0.0, 1.0, 4.0, 9.0, 16.0])]);
    let roc_low = col_sheet("rl", &[("Ctrl", &[10.0, 11.0, 12.0, 13.0]), ("Case", &[1.0, 2.0, 3.0, 4.0])]);

    for (name, sh, kind) in [
        ("UnpairedT", &skewed, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() })),
        ("WelchT", &skewed, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..Default::default() })),
        ("PairedT", &skewed, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..Default::default() })),
        (
            "OneSampleT",
            &skewed,
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..Default::default() }),
        ),
        ("PairedSign", &skewed, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..Default::default() })),
        ("SmallT", &small, AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())),
        ("Descriptive", &skewed, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())),
        ("Outliers", &skewed, AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })),
        ("OneWay", &aov, AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..Default::default() })),
        ("WelchAOV", &aov, AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, ..Default::default() })),
        (
            "RMAOV",
            &aov,
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, ..Default::default() }),
        ),
        ("Contingency", &ct_sp, AnalysisKind::Contingency(ContingencySpec::default())),
        (
            "Logistic",
            &mv_sep,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Logistic,
                response_column: 1,
                predictor_columns: vec![0],
                reference_levels: vec![],
                compute_vif: false,
                ..Default::default()
            }),
        ),
        (
            "Poisson",
            &mv_pois,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Poisson,
                response_column: 1,
                predictor_columns: vec![0],
                reference_levels: vec![],
                compute_vif: false,
                ..Default::default()
            }),
        ),
        (
            "CoxPH",
            &mv_cox,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2],
                reference_levels: vec![],
                tie_method: CoxTieMethod::Efron,
                formula: None,
                covariate_profiles: Vec::new(),
                nested_comparison: None,
                emit_cox_snell_residuals: false,
                ci_method: Default::default(),
                event_code: None,
                censored_code: None,
                treat_other_as: Default::default(),
            }),
        ),
        (
            "Clustering",
            &mv_cl,
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: Some(2),
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Ward,
                k_sweep_max: Some(3),
                consensus_optimal_k: false,
                kmeans_algorithm: Default::default(),
                cluster_direction: Default::default(),
                cut_height: None,
                emit_anova_tab: false,
            }),
        ),
        ("ROCLow", &roc_low, AnalysisKind::RocCurve(RocSpec::default())),
    ] {
        let res = run_kind(sh, kind).unwrap();
        assert_diagnostics_only(&res, name);
        assert_all_sheet_numbers_finite(&res, name);
    }
}
