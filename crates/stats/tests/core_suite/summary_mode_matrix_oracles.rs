//! Exhaustive summary-mode matrix enforcement and D1 textbook reference oracles for
//! `curvebook-stats` across `Column`, `Xy`, and `Grouped` tables × all eight pre-summarized
//! `SubcolumnMode` layouts.
//!
//! # References
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25.
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different
//!   population variances are involved. *Biometrika* 34(1–2):28–35.
//! - Welch, B. L. (1951). On the comparison of several mean values: an alternative approach.
//!   *Biometrika* 38(3–4):330–336.
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114.
//! - Brown, M. B., & Forsythe, A. B. (1974b). The small sample behavior of some statistics which
//!   test the equality of several means. *Technometrics* 16(1):129–132.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Examples 8.1 & 10.1.
//! - Tukey, J. W. (1977). *Exploratory Data Analysis*. Addison-Wesley.
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.).
//! - Higgins, J. P. T., Li, T., & Deeks, J. J. (eds.) (2023). *Cochrane Handbook for Systematic
//!   Reviews of Interventions*, §6.5.2.10 (combining summary groups).
//! - Yeh, K. C., & Kwan, K. C. (1978). *J. Pharmacokinet. Biopharm.* 6(1):79–98.

use curvebook_doc::{
    AnalysisDomainKind, AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable,
    ContingencyMethod, ContingencySpec, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable, LeveneTestSpec,
    MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, OneWayAnovaSpec, OneWayMethod, OutlierSpec, PValueStackSpec,
    PassingBablokSpec, PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, ResultStatus, RocSpec, SUMMARY_SUBCOLUMN_MODES,
    SimpleLogisticSpec, SubcolumnMode, SummaryModeProcedure, SummaryModeRefusalReason, SummaryModeSupport, SummaryPointError, TableArchetype,
    TableKind, TwoGroupMethod, TwoGroupSpec, XyTable, summary_mode_matrix_rows,
};
use curvebook_math::dist::{f_sf, student_t_two_sided_p};
use curvebook_math::sum::SummaryMoments;
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: expected {expected:.15}, got {actual:.15} (diff = {:.3e}, tol = {tol:.1e})",
        (actual - expected).abs()
    );
}

fn summary_cells_for_mode(mode: SubcolumnMode, mean: f64, sd: f64, n: f64) -> Vec<Cell> {
    match mode {
        SubcolumnMode::MeanSdN => vec![Cell::number(mean), Cell::number(sd), Cell::number(n)],
        SubcolumnMode::MeanSemN => vec![Cell::number(mean), Cell::number(sd / n.sqrt()), Cell::number(n)],
        SubcolumnMode::MeanCvN => vec![Cell::number(mean), Cell::number((sd / mean.abs()) * 100.0), Cell::number(n)],
        SubcolumnMode::MeanSd => vec![Cell::number(mean), Cell::number(sd)],
        SubcolumnMode::MeanSem => vec![Cell::number(mean), Cell::number(sd / n.sqrt())],
        SubcolumnMode::MeanLowHigh => vec![Cell::number(mean), Cell::number(sd), Cell::number(1.5 * sd)],
        SubcolumnMode::UpperLowerLimits => vec![Cell::number(mean), Cell::number(mean - sd), Cell::number(mean + 1.5 * sd)],
        SubcolumnMode::MedianIqr => vec![Cell::number(mean), Cell::number(mean - 0.75 * sd), Cell::number(mean + 0.75 * sd)],
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => panic!("expected summary mode"),
    }
}

fn make_summary_sheet(table_kind: TableKind, mode: SubcolumnMode) -> DataSheet {
    match table_kind {
        TableKind::Column => {
            let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 3, 2).unwrap();
            col.data_sets[0].title = String::from("GroupA");
            col.data_sets[1].title = String::from("GroupB");
            col.data_sets[2].title = String::from("GroupC");
            col.data_sets[0].cells = vec![summary_cells_for_mode(mode, 10.0, 2.0, 6.0), summary_cells_for_mode(mode, 11.0, 2.2, 6.0)];
            col.data_sets[1].cells = vec![summary_cells_for_mode(mode, 15.0, 2.5, 6.0), summary_cells_for_mode(mode, 16.0, 2.4, 6.0)];
            col.data_sets[2].cells = vec![summary_cells_for_mode(mode, 21.0, 3.0, 6.0), summary_cells_for_mode(mode, 22.0, 2.8, 6.0)];
            DataSheet::new("col-sum", "Col Summary", TableArchetype::Column(col)).unwrap()
        }
        TableKind::Xy => {
            let mut xy = XyTable::new(mode, 2, 4).unwrap();
            xy.data_sets[0].title = String::from("SeriesA");
            xy.data_sets[1].title = String::from("SeriesB");
            for r in 0..4 {
                let x = (r + 1) as f64;
                xy.x_cells[r] = vec![Cell::number(x)];
                xy.data_sets[0].cells[r] = summary_cells_for_mode(mode, 5.0 + 2.0 * x, 1.0, 5.0);
                xy.data_sets[1].cells[r] = summary_cells_for_mode(mode, 8.0 + 3.0 * x, 1.2, 5.0);
            }
            DataSheet::new("xy-sum", "XY Summary", TableArchetype::Xy(xy)).unwrap()
        }
        TableKind::Grouped => {
            let mut grp = GroupedTable::new(mode, 2, 3).unwrap();
            grp.row_titles = vec![String::from("R1"), String::from("R2"), String::from("R3")];
            grp.data_sets[0].title = String::from("Ctrl");
            grp.data_sets[1].title = String::from("Trt");
            for r in 0..3 {
                grp.data_sets[0].cells[r] = summary_cells_for_mode(mode, 10.0 + (r as f64) * 3.0, 1.5, 5.0);
                grp.data_sets[1].cells[r] = summary_cells_for_mode(mode, 14.0 + (r as f64) * 4.0, 1.8, 5.0);
            }
            DataSheet::new("grp-sum", "Grouped Summary", TableArchetype::Grouped(grp)).unwrap()
        }
        other => panic!("unexpected summary table kind {other:?}"),
    }
}

fn representative_stats_kinds(proc_id: SummaryModeProcedure) -> Vec<AnalysisKind> {
    match proc_id {
        SummaryModeProcedure::DescriptiveStats => vec![AnalysisKind::DescriptiveStats(DescriptiveSpec::default())],
        SummaryModeProcedure::OutlierDetection => vec![
            AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }),
            AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        ],
        SummaryModeProcedure::TwoGroupParametricSummary => vec![
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 10.0 }, ..TwoGroupSpec::default() }),
        ],
        SummaryModeProcedure::TwoGroupRawOnly => vec![
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::RatioPairedT, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::LognormalUnpairedT, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::LognormalWelchT, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WilcoxonSignedRank, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::KolmogorovSmirnov, ..TwoGroupSpec::default() }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::OneSampleWilcoxon { hypothetical: 10.0 },
                ..TwoGroupSpec::default()
            }),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::OneSampleRatioT { hypothetical: 10.0 },
                ..TwoGroupSpec::default()
            }),
        ],
        SummaryModeProcedure::BlandAltman => vec![AnalysisKind::BlandAltman(BlandAltmanSpec::default())],
        SummaryModeProcedure::MultipleRowTTests => vec![AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())],
        SummaryModeProcedure::PassingBablok => vec![AnalysisKind::PassingBablok(PassingBablokSpec::default())],
        SummaryModeProcedure::OneWayAnovaParametricSummary => vec![
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, ..OneWayAnovaSpec::default() }),
        ],
        SummaryModeProcedure::OneWayAnovaRawOnly => vec![
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
                ..OneWayAnovaSpec::default()
            }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..OneWayAnovaSpec::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::Friedman, ..OneWayAnovaSpec::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::LognormalOrdinary, ..OneWayAnovaSpec::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::LognormalWelchBf, ..OneWayAnovaSpec::default() }),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::LognormalRepeatedMeasures { geisser_greenhouse: true },
                ..OneWayAnovaSpec::default()
            }),
        ],
        SummaryModeProcedure::FactorialAnova => vec![AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())],
        SummaryModeProcedure::ContingencyColumnBinary => {
            vec![AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, ..ContingencySpec::default() })]
        }
        SummaryModeProcedure::RocCurve => vec![AnalysisKind::RocCurve(RocSpec::default())],
        SummaryModeProcedure::GeneralizedLinearModel => vec![AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            compute_vif: false,
            ..GlmSpec::default()
        })],
        SummaryModeProcedure::Multivariate => vec![
            AnalysisKind::Multivariate(MultivariateSpec::Pca {
                standardize: true,
                parallel_analysis_seed: None,
                pcr_outcome_column: None,
                retention_rule: Default::default(),
                emit_extra_matrices: false,
            }),
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: Some(2),
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Upgma,
                k_sweep_max: None,
                consensus_optimal_k: false,
                kmeans_algorithm: Default::default(),
                cluster_direction: Default::default(),
                cut_height: None,
                emit_anova_tab: false,
            }),
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 3,
                max_iters: 2,
                seed: 7,
                response_column: Some(0),
                predictor_columns: vec![1],
            }),
        ],
        SummaryModeProcedure::PValueStack => vec![AnalysisKind::PValueStack(PValueStackSpec::default())],
        SummaryModeProcedure::PowerAnalysis => vec![AnalysisKind::PowerAnalysis(PowerAnalysisSpec {
            family: PowerTestFamily::TwoSampleUnpairedT,
            solve_target: PowerSolveTarget::SampleSize,
            effect: PowerEffectSpec::Standardized(0.8),
            alpha: 0.05,
            target_power: Some(0.80),
            ..PowerAnalysisSpec::default()
        })],
        SummaryModeProcedure::SimpleLogisticRegression => vec![AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())],
        SummaryModeProcedure::LeveneTest => vec![AnalysisKind::LeveneTest(LeveneTestSpec::default())],
        other => panic!("unexpected non-stats procedure {other:?}"),
    }
}

#[test]
fn exhaustive_stats_summary_mode_matrix_matches_runtime_verdicts() {
    let mut supported_cells = 0_usize;
    let mut refused_cells = 0_usize;

    for row in summary_mode_matrix_rows().iter().filter(|r| {
        let desc = r.analysis_kind_id.descriptor();
        desc.domain == AnalysisDomainKind::Stats && desc.recompute_strategy != curvebook_doc::AnalysisRecomputeStrategy::MonteCarloEngine
    }) {
        let kinds = representative_stats_kinds(row.procedure);
        assert!(!kinds.is_empty());
        for &table_kind in row.table_kinds {
            for mode in SUMMARY_SUBCOLUMN_MODES {
                let expected = row.support_for(mode).expect("every summary mode has a verdict");
                let sheet = make_summary_sheet(table_kind, mode);

                for kind in &kinds {
                    let mut spec = AnalysisSpec::new("res-m", sheet.id.as_str(), kind.clone()).unwrap();
                    if matches!(kind, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { .. }, .. })) {
                        spec.selected_data_sets = vec![0];
                    } else if matches!(kind, AnalysisKind::TwoGroupComparison(_)) {
                        spec.selected_data_sets = vec![0, 1];
                    }

                    let outcome = run_analysis(&sheet, &spec);
                    match expected {
                        SummaryModeSupport::Supported(_) => {
                            let res = outcome.unwrap_or_else(|e| {
                                panic!("expected Supported for {:?} on {table_kind:?}/{mode:?}, got error: {e:?}", row.procedure)
                            });
                            assert_eq!(res.status, ResultStatus::UpToDate);
                            assert!(res.validate().is_ok(), "invalid ResultSheet for {:?} on {table_kind:?}/{mode:?}", row.procedure);
                            supported_cells += 1;
                        }
                        SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates) => {
                            assert!(
                                matches!(outcome, Err(StatsError::RequiresRawReplicates)),
                                "expected RequiresRawReplicates for {:?} on {table_kind:?}/{mode:?}, got {outcome:?}",
                                row.procedure
                            );
                            refused_cells += 1;
                        }
                        SummaryModeSupport::Refused(reason) => {
                            assert!(
                                matches!(
                                    outcome,
                                    Err(StatsError::UnsupportedSubcolumnMode { table_kind: actual_tk, mode: actual_mode, .. })
                                        if actual_tk == table_kind && actual_mode == mode
                                ),
                                "expected UnsupportedSubcolumnMode ({reason:?}) for {:?} on {table_kind:?}/{mode:?}, got {outcome:?}",
                                row.procedure
                            );
                            refused_cells += 1;
                        }
                    }
                }
            }
        }
    }

    assert!(supported_cells > 0 && refused_cells > 0, "matrix sweep must exercise both Supported and Refused cells");
}

fn tab_num(res: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    for tab in &res.tabs {
        for (lbl, row) in tab.row_labels.iter().zip(tab.rows.iter()) {
            if lbl == key
                && let Some(val) = row.first()
            {
                match val {
                    curvebook_doc::ResultCell::Number { value: v, .. } | curvebook_doc::ResultCell::PValue { raw: v, .. } => return *v,
                    _ => {}
                }
            }
        }
    }
    panic!("missing numeric tab row {key:?}");
}

#[test]
fn d1_oracle_student_welch_and_one_sample_t_tests_from_parametric_summaries() {
    // Oracle 1: Zar (2010), 5th ed., Example 8.1 (blood clotting times, two-sample Student t-test).
    let raw_a = [8.8, 8.4, 7.9, 8.7, 9.1, 9.6];
    let raw_b = [9.9, 9.0, 11.1, 9.6, 8.7, 10.4, 9.5];
    let ma = SummaryMoments::from_slice(&raw_a).unwrap();
    let mb = SummaryMoments::from_slice(&raw_b).unwrap();
    let (mean_a, sd_a, n_a) = (ma.mean, ma.sample_sd().unwrap(), raw_a.len() as f64);
    let (mean_b, sd_b, n_b) = (mb.mean, mb.sample_sd().unwrap(), raw_b.len() as f64);

    // Raw ColumnTable baseline for exact equivalence check.
    let mut raw_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, raw_b.len()).unwrap();
    raw_col.data_sets[0].title = String::from("Drug B");
    raw_col.data_sets[1].title = String::from("Drug G");
    for (r, &v) in raw_a.iter().enumerate() {
        raw_col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    for (r, &v) in raw_b.iter().enumerate() {
        raw_col.data_sets[1].cells[r][0] = Cell::number(v);
    }
    let raw_sheet = DataSheet::new("raw-t", "Zar Ex 8.1 Raw", TableArchetype::Column(raw_col)).unwrap();
    let raw_student = run_analysis(
        &raw_sheet,
        &AnalysisSpec::new(
            "r-st",
            "raw-t",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
        )
        .unwrap(),
    )
    .unwrap();
    let raw_welch = run_analysis(
        &raw_sheet,
        &AnalysisSpec::new(
            "r-wl",
            "raw-t",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
        )
        .unwrap(),
    )
    .unwrap();

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 2, 1).unwrap();
        col.data_sets[0].title = String::from("Drug B");
        col.data_sets[1].title = String::from("Drug G");
        col.data_sets[0].cells = vec![summary_cells_for_mode(mode, mean_a, sd_a, n_a)];
        col.data_sets[1].cells = vec![summary_cells_for_mode(mode, mean_b, sd_b, n_b)];
        let sheet = DataSheet::new("sum-t", "Zar Ex 8.1 Summary", TableArchetype::Column(col)).unwrap();

        let sum_student = run_analysis(
            &sheet,
            &AnalysisSpec::new(
                "s-st",
                "sum-t",
                AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
            )
            .unwrap(),
        )
        .unwrap();
        for key in ["t", "PValue", "CohensD", "HedgesG"] {
            assert_close(
                sum_student.lookup_scalar(key, None).unwrap(),
                raw_student.lookup_scalar(key, None).unwrap(),
                1e-12,
                &format!("Student t {mode:?} {key}"),
            );
        }
        for row_key in ["df", "Difference between means (A - B)", "SE of difference", "R squared"] {
            assert_close(tab_num(&sum_student, row_key), tab_num(&raw_student, row_key), 1e-12, &format!("Student t {mode:?} {row_key}"));
        }
        assert_close(tab_num(&sum_student, "df"), 11.0, 1e-12, "Zar Ex 8.1 df");

        let sum_welch = run_analysis(
            &sheet,
            &AnalysisSpec::new(
                "s-wl",
                "sum-t",
                AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
            )
            .unwrap(),
        )
        .unwrap();
        for key in ["t", "PValue", "CohensD", "HedgesG"] {
            assert_close(
                sum_welch.lookup_scalar(key, None).unwrap(),
                raw_welch.lookup_scalar(key, None).unwrap(),
                1e-12,
                &format!("Welch t {mode:?} {key}"),
            );
        }
        for row_key in ["df", "Difference between means (A - B)", "SE of difference", "R squared"] {
            assert_close(tab_num(&sum_welch, row_key), tab_num(&raw_welch, row_key), 1e-12, &format!("Welch t {mode:?} {row_key}"));
        }

        // Closed-form Welch (1947) / Satterthwaite (1946) unequal-variance oracle:
        // Group 1: (10, 2, 10), Group 2: (14, 6, 15) => v1=0.4, v2=2.4, SE=sqrt(2.8), df=2.8^2/(0.16/9 + 5.76/14).
        let mut wcol = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 2, 1).unwrap();
        wcol.data_sets[0].cells = vec![summary_cells_for_mode(mode, 10.0, 2.0, 10.0)];
        wcol.data_sets[1].cells = vec![summary_cells_for_mode(mode, 14.0, 6.0, 15.0)];
        let wsheet = DataSheet::new("w-cf", "Welch CF", TableArchetype::Column(wcol)).unwrap();
        let wres = run_analysis(
            &wsheet,
            &AnalysisSpec::new(
                "w-res",
                "w-cf",
                AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
            )
            .unwrap(),
        )
        .unwrap();
        let expected_se = 2.8_f64.sqrt();
        let expected_t = -4.0 / expected_se;
        let expected_df = (2.8_f64 * 2.8) / (0.16 / 9.0 + 5.76 / 14.0);
        let expected_p = student_t_two_sided_p(expected_t, expected_df).unwrap();
        assert_close(wres.lookup_scalar("t", None).unwrap(), expected_t, 1e-12, &format!("Welch CF {mode:?} t"));
        assert_close(tab_num(&wres, "df"), expected_df, 1e-12, &format!("Welch CF {mode:?} df"));
        assert_close(wres.lookup_scalar("PValue", None).unwrap(), expected_p, 1e-11, &format!("Welch CF {mode:?} P"));

        // Closed-form Student (1908) one-sample t-test oracle: (mean=12, sd=2.5, n=16, mu0=10) => t=3.2, df=15.
        let mut ocol = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 1).unwrap();
        ocol.data_sets[0].cells = vec![summary_cells_for_mode(mode, 12.0, 2.5, 16.0)];
        let osheet = DataSheet::new("os-cf", "OneSample CF", TableArchetype::Column(ocol)).unwrap();
        let ores = run_analysis(
            &osheet,
            &AnalysisSpec::new(
                "os-res",
                "os-cf",
                AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                    method: TwoGroupMethod::OneSampleT { hypothetical: 10.0 },
                    ..TwoGroupSpec::default()
                }),
            )
            .unwrap(),
        )
        .unwrap();
        assert_close(ores.lookup_scalar("t", None).unwrap(), 3.2, 1e-12, &format!("OneSampleT {mode:?} t"));
        assert_close(tab_num(&ores, "df"), 15.0, 1e-12, &format!("OneSampleT {mode:?} df"));
        assert_close(
            ores.lookup_scalar("PValue", None).unwrap(),
            student_t_two_sided_p(3.2, 15.0).unwrap(),
            1e-12,
            &format!("OneSampleT {mode:?} P"),
        );
    }
}

#[test]
fn d1_oracle_one_way_anova_ordinary_and_welch_bf_from_parametric_summaries() {
    // Closed-form Zar (2010) Ch. 10 ANOVA oracle:
    // 3 groups: (10, 2, 5), (15, 2, 5), (20, 2, 5) -> grand mean = 15, SS_b = 250, SS_w = 48, df=(2, 12), F=31.25.
    let expected_f = 31.25_f64;
    let expected_p = f_sf(expected_f, 2.0, 12.0).unwrap();
    let expected_eta2 = 250.0 / 298.0;
    let expected_omega2 = (250.0 - 2.0 * 4.0) / (298.0 + 4.0);

    // Raw-vs-summary equivalence for heteroscedastic 3-group Welch (1951) & Brown-Forsythe (1974b) ANOVA.
    let g0 = [10.0, 12.0, 11.0, 9.0, 13.0];
    let g1 = [18.0, 24.0, 15.0, 27.0, 21.0, 16.0];
    let g2 = [30.0, 31.0, 29.0, 30.5, 29.5, 31.5, 28.5];
    let mut raw_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, g2.len()).unwrap();
    for (c, slice) in [&g0[..], &g1[..], &g2[..]].into_iter().enumerate() {
        for (r, &v) in slice.iter().enumerate() {
            raw_col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let raw_sheet = DataSheet::new("raw-an", "Raw ANOVA", TableArchetype::Column(raw_col)).unwrap();
    let raw_ord = run_analysis(
        &raw_sheet,
        &AnalysisSpec::new(
            "r-ord",
            "raw-an",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::OrdinaryAnova,
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
                ..OneWayAnovaSpec::default()
            }),
        )
        .unwrap(),
    )
    .unwrap();
    let raw_uv = run_analysis(
        &raw_sheet,
        &AnalysisSpec::new(
            "r-uv",
            "raw-an",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::BrownForsytheAndWelch,
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::GamesHowell, ..MultiplicitySpec::default() },
                ..OneWayAnovaSpec::default()
            }),
        )
        .unwrap(),
    )
    .unwrap();

    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        // 1. Closed-form balanced ordinary ANOVA oracle.
        let mut cf_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 3, 1).unwrap();
        cf_col.data_sets[0].cells = vec![summary_cells_for_mode(mode, 10.0, 2.0, 5.0)];
        cf_col.data_sets[1].cells = vec![summary_cells_for_mode(mode, 15.0, 2.0, 5.0)];
        cf_col.data_sets[2].cells = vec![summary_cells_for_mode(mode, 20.0, 2.0, 5.0)];
        let cf_sheet = DataSheet::new("cf-an", "CF ANOVA", TableArchetype::Column(cf_col)).unwrap();
        let cf_res =
            run_analysis(&cf_sheet, &AnalysisSpec::new("cf-r", "cf-an", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
        assert_close(cf_res.lookup_scalar("F", None).unwrap(), expected_f, 1e-12, &format!("ANOVA CF {mode:?} F"));
        assert_close(cf_res.lookup_scalar("P value", None).unwrap(), expected_p, 1e-12, &format!("ANOVA CF {mode:?} P"));
        assert_close(cf_res.lookup_scalar("Eta squared", None).unwrap(), expected_eta2, 1e-12, &format!("ANOVA CF {mode:?} eta2"));
        assert_close(cf_res.lookup_scalar("Omega squared", None).unwrap(), expected_omega2, 1e-12, &format!("ANOVA CF {mode:?} omega2"));

        // 2. Exact raw-vs-summary equivalence for heteroscedastic ordinary + Welch/Brown-Forsythe + post-hoc.
        let mut eq_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 3, 1).unwrap();
        for (c, slice) in [&g0[..], &g1[..], &g2[..]].into_iter().enumerate() {
            let m = SummaryMoments::from_slice(slice).unwrap();
            eq_col.data_sets[c].cells = vec![summary_cells_for_mode(mode, m.mean, m.sample_sd().unwrap(), slice.len() as f64)];
        }
        let eq_sheet = DataSheet::new("eq-an", "Eq ANOVA", TableArchetype::Column(eq_col)).unwrap();
        let sum_ord = run_analysis(
            &eq_sheet,
            &AnalysisSpec::new(
                "s-ord",
                "eq-an",
                AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                    method: OneWayMethod::OrdinaryAnova,
                    multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
                    ..OneWayAnovaSpec::default()
                }),
            )
            .unwrap(),
        )
        .unwrap();
        for key in ["F", "P value", "R squared", "MS residual", "Eta squared", "Omega squared"] {
            assert_close(
                sum_ord.lookup_scalar(key, None).unwrap(),
                raw_ord.lookup_scalar(key, None).unwrap(),
                1e-12,
                &format!("Ordinary ANOVA eq {mode:?} {key}"),
            );
        }
        assert_eq!(sum_ord.pairwise_comparisons.len(), raw_ord.pairwise_comparisons.len());
        for (a, b) in sum_ord.pairwise_comparisons.iter().zip(raw_ord.pairwise_comparisons.iter()) {
            assert_close(a.raw_p_or_q, b.raw_p_or_q, 1e-11, &format!("Tukey {mode:?} p"));
        }

        let sum_uv = run_analysis(
            &eq_sheet,
            &AnalysisSpec::new(
                "s-uv",
                "eq-an",
                AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                    method: OneWayMethod::BrownForsytheAndWelch,
                    multiplicity: MultiplicitySpec { method: MultiplicityMethod::GamesHowell, ..MultiplicitySpec::default() },
                    ..OneWayAnovaSpec::default()
                }),
            )
            .unwrap(),
        )
        .unwrap();
        for key in ["Welch W", "Welch P value", "Brown-Forsythe F*", "Brown-Forsythe P value"] {
            assert_close(
                sum_uv.lookup_scalar(key, None).unwrap(),
                raw_uv.lookup_scalar(key, None).unwrap(),
                1e-12,
                &format!("Welch/BF ANOVA eq {mode:?} {key}"),
            );
        }
        assert_eq!(sum_uv.pairwise_comparisons.len(), raw_uv.pairwise_comparisons.len());
        for (a, b) in sum_uv.pairwise_comparisons.iter().zip(raw_uv.pairwise_comparisons.iter()) {
            assert_close(a.raw_p_or_q, b.raw_p_or_q, 1e-11, &format!("Games-Howell {mode:?} p"));
        }
    }
}

#[test]
fn d1_oracle_descriptive_statistics_across_all_eight_summary_modes() {
    // 1. Cochrane Handbook §6.5.2.10 / Zar (2010) Ch. 10 multi-row pooled moments oracle:
    // Row 1 = (10, 2, 5), Row 2 = (16, 3, 7) => N=12, Mean=13.5, SS_tot = 70 + 105 = 175, SD = sqrt(175/11).
    let expected_pooled_mean = 13.5_f64;
    let expected_pooled_sd = (175.0_f64 / 11.0).sqrt();
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 2).unwrap();
        col.data_sets[0].cells = vec![summary_cells_for_mode(mode, 10.0, 2.0, 5.0), summary_cells_for_mode(mode, 16.0, 3.0, 7.0)];
        let sheet = DataSheet::new("desc-pool", "Pooled", TableArchetype::Column(col)).unwrap();
        let res = run_analysis(&sheet, &AnalysisSpec::new("d-p", "desc-pool", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap())
            .unwrap();
        assert_close(res.lookup_scalar("Mean", Some(0)).unwrap(), expected_pooled_mean, 1e-12, &format!("Pooled {mode:?} mean"));
        assert_close(res.lookup_scalar("SD", Some(0)).unwrap(), expected_pooled_sd, 1e-12, &format!("Pooled {mode:?} SD"));
    }

    // 2. MeanLowHigh single-row asymmetric interval bounds & negative-offset refusal.
    let mut mlh = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanLowHigh, 1, 1).unwrap();
    mlh.data_sets[0].cells = vec![vec![Cell::number(20.0), Cell::number(3.5), Cell::number(6.5)]];
    let mlh_sheet = DataSheet::new("desc-mlh", "MLH", TableArchetype::Column(mlh)).unwrap();
    let mlh_res =
        run_analysis(&mlh_sheet, &AnalysisSpec::new("d-mlh", "desc-mlh", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap())
            .unwrap();
    assert_close(mlh_res.lookup_scalar("Mean", Some(0)).unwrap(), 20.0, 1e-12, "MLH Mean");
    assert_close(mlh_res.lookup_scalar("LowerBound", Some(0)).unwrap(), 16.5, 1e-12, "MLH LowerBound");
    assert_close(mlh_res.lookup_scalar("UpperBound", Some(0)).unwrap(), 26.5, 1e-12, "MLH UpperBound");
    assert_close(mlh_res.lookup_scalar("IntervalWidth", Some(0)).unwrap(), 10.0, 1e-12, "MLH IntervalWidth");
    assert_eq!(mlh_res.lookup_scalar("N", Some(0)), None);
    assert_eq!(mlh_res.lookup_scalar("SD", Some(0)), None);

    let mut bad_mlh = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanLowHigh, 1, 1).unwrap();
    bad_mlh.data_sets[0].cells = vec![vec![Cell::number(20.0), Cell::number(-1.0), Cell::number(2.0)]];
    let bad_mlh_sheet = DataSheet::new("bad-mlh", "Bad MLH", TableArchetype::Column(bad_mlh)).unwrap();
    let err =
        run_analysis(&bad_mlh_sheet, &AnalysisSpec::new("d-bad", "bad-mlh", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap())
            .unwrap_err();
    assert!(matches!(err, StatsError::SummaryPoint(SummaryPointError::NegativeSpread { .. })));

    // 3. UpperLowerLimits single-row explicit limits (including reversed order normalization).
    let mut ul = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::UpperLowerLimits, 2, 1).unwrap();
    ul.data_sets[0].cells = vec![vec![Cell::number(20.0), Cell::number(14.0), Cell::number(29.0)]];
    ul.data_sets[1].cells = vec![vec![Cell::number(20.0), Cell::number(29.0), Cell::number(14.0)]];
    let ul_sheet = DataSheet::new("desc-ul", "UL", TableArchetype::Column(ul)).unwrap();
    let ul_res =
        run_analysis(&ul_sheet, &AnalysisSpec::new("d-ul", "desc-ul", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap()).unwrap();
    for col_idx in [0, 1] {
        assert_close(ul_res.lookup_scalar("Mean", Some(col_idx)).unwrap(), 20.0, 1e-12, "UL Mean");
        assert_close(ul_res.lookup_scalar("LowerLimit", Some(col_idx)).unwrap(), 14.0, 1e-12, "UL LowerLimit");
        assert_close(ul_res.lookup_scalar("UpperLimit", Some(col_idx)).unwrap(), 29.0, 1e-12, "UL UpperLimit");
        assert_close(ul_res.lookup_scalar("IntervalWidth", Some(col_idx)).unwrap(), 15.0, 1e-12, "UL IntervalWidth");
        assert_eq!(ul_res.lookup_scalar("N", Some(col_idx)), None);
    }

    // 4. MedianIqr (Tukey 1977): reports Median, Q1, Q3, IQR without fabricating Mean, N, or SD.
    let mut iqr = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MedianIqr, 1, 1).unwrap();
    iqr.data_sets[0].cells = vec![vec![Cell::number(15.0), Cell::number(11.0), Cell::number(22.5)]];
    let iqr_sheet = DataSheet::new("desc-iqr", "IQR", TableArchetype::Column(iqr)).unwrap();
    let iqr_res =
        run_analysis(&iqr_sheet, &AnalysisSpec::new("d-iqr", "desc-iqr", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap())
            .unwrap();
    assert_close(iqr_res.lookup_scalar("Median", Some(0)).unwrap(), 15.0, 1e-12, "IQR Median");
    assert_close(iqr_res.lookup_scalar("Q1", Some(0)).unwrap(), 11.0, 1e-12, "IQR Q1");
    assert_close(iqr_res.lookup_scalar("Q3", Some(0)).unwrap(), 22.5, 1e-12, "IQR Q3");
    assert_close(iqr_res.lookup_scalar("IQR", Some(0)).unwrap(), 11.5, 1e-12, "IQR width");
    assert_eq!(iqr_res.lookup_scalar("Mean", Some(0)), None, "MedianIqr must not fabricate Mean");
    assert_eq!(iqr_res.lookup_scalar("N", Some(0)), None, "MedianIqr must not fabricate N");
    assert_eq!(iqr_res.lookup_scalar("SD", Some(0)), None, "MedianIqr must not fabricate SD");

    // 5. XY Trapezoidal AUC (Yeh & Kwan 1978) across ALL 8 summary modes:
    // Profile [(0, 1), (1, 5), (2, 7), (4, 3)] => 0.5*(1*6 + 1*12 + 2*10) = 19.0.
    for mode in SUMMARY_SUBCOLUMN_MODES {
        let mut xy = XyTable::new(mode, 1, 4).unwrap();
        for (r, (x, y)) in [(0.0, 1.0), (1.0, 5.0), (2.0, 7.0), (4.0, 3.0)].into_iter().enumerate() {
            xy.x_cells[r] = vec![Cell::number(x)];
            xy.data_sets[0].cells[r] = summary_cells_for_mode(mode, y, 0.5, 4.0);
        }
        let xy_sheet = DataSheet::new("xy-auc", "XY AUC", TableArchetype::Xy(xy)).unwrap();
        let res = run_analysis(&xy_sheet, &AnalysisSpec::new("d-xy", "xy-auc", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap())
            .unwrap();
        assert_close(res.lookup_scalar("Trapezoidal AUC", Some(0)).unwrap(), 19.0, 1e-12, &format!("XY AUC {mode:?}"));
    }

    // 6. PowerAnalysis (Cohen 1988): d=0.8, alpha=0.05, power=0.80 => n=26 per group across all 8 Column modes.
    for mode in SUMMARY_SUBCOLUMN_MODES {
        let sheet = make_summary_sheet(TableKind::Column, mode);
        let spec = AnalysisSpec::new(
            "pwr",
            sheet.id.as_str(),
            AnalysisKind::PowerAnalysis(PowerAnalysisSpec {
                family: PowerTestFamily::TwoSampleUnpairedT,
                solve_target: PowerSolveTarget::SampleSize,
                effect: PowerEffectSpec::Standardized(0.8),
                alpha: 0.05,
                target_power: Some(0.80),
                ..PowerAnalysisSpec::default()
            }),
        )
        .unwrap();
        let res = run_analysis(&sheet, &spec).unwrap();
        assert_close(res.lookup_scalar("NPerGroup", None).unwrap(), 26.0, 1e-12, &format!("Power {mode:?} NPerGroup"));
        assert_close(res.lookup_scalar("TotalN", None).unwrap(), 52.0, 1e-12, &format!("Power {mode:?} TotalN"));
    }
}
