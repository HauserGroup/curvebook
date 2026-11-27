#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for Wave 21 Statistical Pillars:
//! - Pillar 2 (`R9.12` / `QOL-12`): Kappa wiring, Altman (1998) NNT, McNemar OR + Clopper-Pearson CI,
//!   PartsOfWhole custom expected proportions + G^2, ExactBinomial, Column SD CI, Paired Sign Test,
//!   Pre-summarized two-group t-tests, Factorial ANOVA multiplicity
//! - Pillar 3 (`WF-37` / `WF-38`): Clustering distances (4), linkages (7), cophenetic matrix + r,
//!   hierarchical cut silhouettes, and k-means / candidate-k WCSS + silhouette sweep.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CiBound, ClusterDistance, ClusterLinkage, ContingencyMethod, ContingencySpec, DescriptiveSpec, FactorialAnovaSpec,
    KappaWeighting, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, ResultCell, ResultTabKind, SheetId, SubcolumnMode, SumOfSquaresType,
    TwoGroupMethod, TwoGroupSpec,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::categorical::{
    NntKind, agresti_coull_ci, altman_nnt_from_rd, analyze_2x2_ratios, clopper_pearson_ci, cohens_kappa, exact_binomial_test, fleiss_kappa,
    mcnemar_detailed, wilson_score_ci,
};
use curvebook_stats::column::summarize_column;
use curvebook_stats::error::StatsError;
use curvebook_stats::multivariate::{
    cophenetic_correlation, cophenetic_distance_matrix, cut_dendrogram, hierarchical_clustering_with_metrics, kmeans_pp_with_distance,
    pairwise_distance_matrix, pairwise_distance_value,
};

use crate::common::{
    assert_close, cell_ci_finite, col_sheet, contingency_sheet, grouped_sheet, mv_num_sheet, parts_sheet, presum_col_sheet, row_cell, run_kind,
    scalar, tab_by_id, tab_by_kind,
};

#[test]
fn pillar2_kappa_wiring_tabs_and_scalars() {
    let matrix = [vec![20_u64, 2, 1], vec![3, 18, 2], vec![0, 1, 25]];
    let sheet = contingency_sheet("ct-1", &[&[20, 2, 1], &[3, 18, 2], &[0, 1, 25]]);

    for weighting in [KappaWeighting::Unweighted, KappaWeighting::Linear, KappaWeighting::Quadratic] {
        let direct = cohens_kappa(&matrix, weighting, ConfidenceLevel::DEFAULT_95).unwrap();
        let res = run_kind(
            &sheet,
            AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CohenKappa { weighting }, ..Default::default() }),
        )
        .unwrap();
        for (k, exp) in [
            ("kappa", direct.kappa),
            ("se", direct.se),
            ("z", direct.z_statistic),
            ("p_value", direct.p_value),
            ("ci_lower", direct.ci.0),
            ("ci_upper", direct.ci.1),
        ] {
            assert_close(scalar(&res, k), exp, 1e-12, k);
        }
        assert_eq!(tab_by_id(&res, "contingency-kappa").kind, ResultTabKind::MainSummary);
        assert_eq!(tab_by_id(&res, "contingency-kappa-diagnostics").kind, ResultTabKind::Diagnostics);
        assert!(res.checklist.iter().all(|c| c.remediation.is_none()));
    }

    let fleiss_counts = [vec![3_u64, 0, 0], vec![0, 3, 0], vec![2, 1, 0], vec![0, 0, 3], vec![1, 1, 1]];
    let f_sheet = contingency_sheet("ct-f", &[&[3, 0, 0], &[0, 3, 0], &[2, 1, 0], &[0, 0, 3], &[1, 1, 1]]);
    let f_direct = fleiss_kappa(&fleiss_counts, ConfidenceLevel::DEFAULT_95).unwrap();
    let f_res =
        run_kind(&f_sheet, AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FleissKappa, ..Default::default() })).unwrap();
    for (k, exp) in [
        ("kappa", f_direct.kappa),
        ("se", f_direct.se),
        ("z", f_direct.z_statistic),
        ("p_value", f_direct.p_value),
        ("ci_lower", f_direct.ci.0),
        ("ci_upper", f_direct.ci.1),
    ] {
        assert_close(scalar(&f_res, k), exp, 1e-12, k);
    }
    assert_eq!(tab_by_id(&f_res, "contingency-kappa").kind, ResultTabKind::MainSummary);
    assert_eq!(tab_by_id(&f_res, "contingency-kappa-diagnostics").kind, ResultTabKind::Diagnostics);
}

#[test]
fn pillar2_altman_nnt_all_branches() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let r_ben = analyze_2x2_ratios(40, 10, 10, 40, conf).unwrap();
    assert_eq!((r_ben.nnt_kind, r_ben.nnt_point_label.as_str()), (NntKind::Benefit, "NNTB 1.67"));
    assert_close(r_ben.nnt_value.unwrap(), 1.0 / 0.6, 1e-12, "NNTB point");
    let (lo, hi) = r_ben.nnt_ci_bounds.unwrap();
    assert!(lo < r_ben.nnt_value.unwrap() && r_ben.nnt_value.unwrap() < hi && r_ben.nnt_ci_label.starts_with("NNTB "));

    let out = run_kind(
        &contingency_sheet("ct-1", &[&[40, 10], &[10, 40]]),
        AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }),
    )
    .unwrap();
    let ratios_tab = tab_by_id(&out, "contingency-ratios");
    for pat in ["Baptista-Pike", "Koopman", "Newcombe"] {
        assert!(ratios_tab.row_labels.iter().any(|l| l.contains(pat)));
    }
    assert_eq!(*row_cell(ratios_tab, "Number needed to treat (NNT)", 0), ResultCell::Text(String::from("NNTB 1.67")));
    assert!(matches!(row_cell(ratios_tab, "Number needed to treat (NNT)", 1), ResultCell::Text(s) if s.starts_with("NNTB ")));

    let r_harm = analyze_2x2_ratios(10, 40, 40, 10, conf).unwrap();
    assert_eq!((r_harm.nnt_kind, r_harm.nnt_point_label.as_str()), (NntKind::Harm, "NNTH 1.67"));
    assert_close(r_harm.nnt_value.unwrap(), 1.0 / 0.6, 1e-12, "NNTH point");
    assert!(r_harm.nnt_ci_label.starts_with("NNTH "));

    let r_zero = analyze_2x2_ratios(10, 10, 10, 10, conf).unwrap();
    assert_eq!((r_zero.nnt_kind, r_zero.nnt_value, r_zero.nnt_point_label.as_str(), r_zero.nnt_ci_bounds), (NntKind::Infinite, None, "∞", None));
    assert!(r_zero.nnt_ci_label.starts_with("NNTH ") && r_zero.nnt_ci_label.contains(" to ∞ to NNTB "));

    let r_cross = analyze_2x2_ratios(12, 8, 10, 10, conf).unwrap();
    assert_eq!((r_cross.nnt_kind, r_cross.nnt_ci_bounds), (NntKind::Benefit, None));
    assert!(r_cross.nnt_ci_label.contains(" to ∞ to NNTB "));

    for (rd, l, h, exp_lbl) in [(0.2, 0.0, 0.4, "NNTH ∞ to ∞ to NNTB 2.50"), (-0.2, -0.4, 0.0, "NNTH 2.50 to ∞ to NNTB ∞")] {
        let (_, _, _, lbl, bounds) = altman_nnt_from_rd(rd, l, h);
        assert!(bounds.is_none() && lbl == exp_lbl);
    }
}

#[test]
fn pillar2_mcnemar_discordant_or_and_clopper_pearson_ci() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let mc = mcnemar_detailed(18, 5, conf).unwrap();
    assert_close(mc.discordant_odds_ratio.unwrap(), 18.0 / 5.0, 1e-12, "McNemar OR");
    let (p_lo, p_hi) = clopper_pearson_ci(18, 23, conf).unwrap();
    let (or_lo, or_hi) = mc.discordant_or_ci.unwrap();
    assert_close(or_lo, p_lo / (1.0 - p_lo), 1e-12, "McNemar OR CI lo");
    assert_close(or_hi, p_hi / (1.0 - p_hi), 1e-12, "McNemar OR CI hi");

    let res_inf = run_kind(
        &contingency_sheet("ct-1", &[&[20, 8], &[0, 15]]),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::McNemarPaired, ..Default::default() }),
    )
    .unwrap();
    let mc_tab = tab_by_id(&res_inf, "contingency-mcnemar");
    assert_eq!(mc_tab.rows[1][0], ResultCell::Text(String::from("∞")));
    assert!(matches!(mc_tab.rows[1][1], ResultCell::ConfidenceInterval { lower: CiBound::Finite(l), upper: CiBound::Open, .. } if l > 0.0));

    let mc_zero = mcnemar_detailed(0, 8, conf).unwrap();
    assert_eq!(mc_zero.discordant_odds_ratio, Some(0.0));
    let (z_lo, z_hi) = mc_zero.discordant_or_ci.unwrap();
    assert!(z_lo == 0.0 && z_hi.is_finite() && z_hi > 0.0);

    let mc_none = mcnemar_detailed(0, 0, conf).unwrap();
    assert_eq!((mc_none.discordant_odds_ratio, mc_none.exact_p_value), (None, 1.0));
}

#[test]
fn pillar2_parts_of_whole_custom_expected_proportions_and_validation() {
    let sheet = parts_sheet("pow-1", &[("Yellow Round", 315.0), ("Yellow Wrinkled", 101.0), ("Green Round", 108.0), ("Green Wrinkled", 32.0)]);
    let res = run_kind(
        &sheet,
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::PearsonChiSquare { yates_continuity: false },
            expected_proportions: Some(vec![9.0, 3.0, 3.0, 1.0]),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_close(scalar(&res, "Goodness-of-fit chi-square"), 0.470_023_980_815_347_7, 1e-10, "Mendel GOF chi2");
    assert_close(scalar(&res, "Likelihood-ratio G-squared"), 0.475_445_238_998_232_4, 1e-10, "Mendel GOF G2");
    assert_eq!(tab_by_id(&res, "parts-gof-categories").rows.len(), 4);

    for bad in [vec![0.5, 0.5], vec![0.3, 0.3, 0.4, -0.1], vec![0.0, 0.0, 0.0, 0.0], vec![1.0, 1.0, 1.0, 0.0]] {
        assert!(run_kind(&sheet, AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(bad), ..Default::default() })).is_err());
    }
}

#[test]
fn pillar2_exact_binomial_all_table_inputs_p_methods_and_cis() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let asym = exact_binomial_test(0, 10, 0.2, conf).unwrap();
    assert_close(asym.p_value_lower, 0.8_f64.powi(10), 1e-12, "lower P(X<=0)");
    assert_close(asym.p_value_sterne, 0.228_248_064, 1e-12, "Sterne P");
    assert_close(asym.p_value_doubled_tail, 2.0 * 0.8_f64.powi(10), 1e-12, "doubled tail P");

    let kind =
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.3), ..Default::default() });
    let pow_sheet = parts_sheet("src", &[("Success", 7.0), ("Failure", 3.0)]);
    let ct_1x2 = contingency_sheet("src", &[&[7, 3]]);
    let ct_2x1 = contingency_sheet("src", &[&[7], &[3]]);
    let col_bin = col_sheet("src", &[("Outcome", &[1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])]);

    let direct = exact_binomial_test(7, 10, 0.3, conf).unwrap();
    let (cp, ws, ac) = (clopper_pearson_ci(7, 10, conf).unwrap(), wilson_score_ci(7, 10, conf), agresti_coull_ci(7, 10, conf));
    for sh in [&pow_sheet, &ct_1x2, &ct_2x1, &col_bin] {
        let res = run_kind(sh, kind.clone()).unwrap();
        for (k, exp) in [
            ("proportion", 0.7),
            ("p_value", direct.p_value_sterne),
            ("p_value_doubled_tail", direct.p_value_doubled_tail),
            ("clopper_pearson_lower", cp.0),
            ("clopper_pearson_upper", cp.1),
            ("wilson_lower", ws.0),
            ("wilson_upper", ws.1),
            ("agresti_coull_lower", ac.0),
            ("agresti_coull_upper", ac.1),
        ] {
            assert_close(scalar(&res, k), exp, 1e-12, k);
        }
        assert_eq!(tab_by_id(&res, "exact-binomial-main").kind, ResultTabKind::MainSummary);
        assert_eq!(tab_by_id(&res, "exact-binomial-cis").kind, ResultTabKind::ParametersAndConfidenceIntervals);
        assert_eq!(tab_by_id(&res, "exact-binomial-diagnostics").kind, ResultTabKind::Diagnostics);
    }

    assert!(matches!(run_kind(&col_sheet("src", &[("Outcome", &[0.0, 1.0, 2.0])]), kind), Err(StatsError::InvalidParameter { .. })));
    for bad_p in [0.0, 1.0, -0.2, 1.5] {
        let bad_k = AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::ExactBinomial,
            null_proportion: Some(bad_p),
            ..Default::default()
        });
        assert!(matches!(run_kind(&pow_sheet, bad_k), Err(StatsError::InvalidParameter { .. })));
    }
    assert!(
        AnalysisSpec::new(
            SheetId::new("r-badp-nan"),
            SheetId::new("src"),
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::ExactBinomial,
                null_proportion: Some(f64::NAN),
                ..Default::default()
            }),
        )
        .is_err()
    );
}

#[test]
fn pillar2_column_sd_confidence_interval() {
    let vals = [10.0, 12.0, 14.0, 16.0, 18.0];
    let s = summarize_column("Sample", &vals, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    let sd = s.sd.unwrap();
    let (lo, hi) = s.sd_ci.unwrap();
    assert_close(lo, sd * (4.0_f64 / 11.143_286_781_5).sqrt(), 1e-6, "SD CI lower");
    assert_close(hi, sd * (4.0_f64 / 0.484_418_557_1).sqrt(), 1e-6, "SD CI upper");
    assert!(summarize_column("One", &[42.0], 0, ConfidenceLevel::DEFAULT_95, None).unwrap().sd_ci.is_none());

    let out = run_kind(&col_sheet("col-1", &[("Sample", &vals)]), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let (l, u) = cell_ci_finite(row_cell(tab_by_kind(&out, ResultTabKind::DescriptiveSummary), "CI of SD", 0));
    assert_close(l, lo, 1e-6, "SD CI cell lo");
    assert_close(u, hi, 1e-6, "SD CI cell hi");
}

#[test]
fn pillar2_paired_sign_test_and_presummarized_t_tests() {
    let sheet = col_sheet(
        "col-1",
        &[("After", &[12.0, 14.0, 15.0, 13.0, 11.0, 16.0, 18.0, 19.0, 15.0]), ("Before", &[10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 20.0, 15.0])],
    );
    let res =
        run_kind(&sheet, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..Default::default() })).unwrap();
    for (k, exp) in [("p_value", 9.0 / 128.0), ("n_positive", 7.0), ("n_negative", 1.0), ("n_ties", 1.0), ("median_difference", 3.0)] {
        assert_close(scalar(&res, k), exp, 1e-12, k);
    }
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::MainSummary) && res.tabs.iter().any(|t| t.kind == ResultTabKind::Diagnostics));

    let (sd1, sd2) = (10.0_f64.sqrt(), (20.0_f64 / 3.0).sqrt());
    let raw_sheet = col_sheet("raw", &[("A", &[10.0, 12.0, 14.0, 16.0, 18.0]), ("B", &[20.0, 22.0, 24.0, 26.0])]);
    let sum_sd = presum_col_sheet("sd", SubcolumnMode::MeanSdN, &[("A", &[&[14.0, sd1, 5.0]]), ("B", &[&[23.0, sd2, 4.0]])]);
    let sum_sem =
        presum_col_sheet("sem", SubcolumnMode::MeanSemN, &[("A", &[&[14.0, sd1 / 5.0_f64.sqrt(), 5.0]]), ("B", &[&[23.0, sd2 / 2.0, 4.0]])]);
    let sum_cv =
        presum_col_sheet("cv", SubcolumnMode::MeanCvN, &[("A", &[&[14.0, 100.0 * sd1 / 14.0, 5.0]]), ("B", &[&[23.0, 100.0 * sd2 / 23.0, 4.0]])]);

    for method in [TwoGroupMethod::UnpairedT, TwoGroupMethod::WelchT, TwoGroupMethod::OneSampleT { hypothetical: 10.0 }] {
        let k = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() });
        let raw_out = run_kind(&raw_sheet, k.clone()).unwrap();
        for s_sheet in [&sum_sd, &sum_sem, &sum_cv] {
            let s_out = run_kind(s_sheet, k.clone()).unwrap();
            assert_close(scalar(&s_out, "t"), scalar(&raw_out, "t"), 1e-10, "pre-summarized t equivalence");
            assert_close(scalar(&s_out, "PValue"), scalar(&raw_out, "PValue"), 1e-10, "pre-summarized P equivalence");
        }
    }
    for bad_method in [TwoGroupMethod::PairedT, TwoGroupMethod::MannWhitneyU, TwoGroupMethod::WilcoxonSignedRank, TwoGroupMethod::PairedSignTest] {
        assert!(matches!(
            run_kind(&sum_sd, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: bad_method, ..Default::default() })),
            Err(StatsError::RequiresRawReplicates)
        ));
    }
}

#[test]
fn pillar2_factorial_anova_multiplicity_wiring() {
    let sheet = grouped_sheet(
        "g-1",
        SubcolumnMode::RawReplicates(3),
        &["Low", "High"],
        &[
            ("Placebo", &[&[10.0, 11.0, 12.0], &[12.0, 13.0, 14.0]]),
            ("DrugA", &[&[20.0, 21.0, 22.0], &[35.0, 36.0, 37.0]]),
            ("DrugB", &[&[30.0, 31.0, 32.0], &[45.0, 46.0, 47.0]]),
        ],
    );
    let res = run_kind(
        &sheet,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
            },
            ss_type: SumOfSquaresType::TypeIII,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(res.pairwise_comparisons.len(), 13);
    for id in [
        "multiple-comparisons-factor-a",
        "multiple-comparisons-factor-b",
        "multiple-comparisons-simple-a-within-b",
        "multiple-comparisons-simple-b-within-a",
    ] {
        assert_eq!(tab_by_id(&res, id).kind, ResultTabKind::MultipleComparisons);
    }

    let res_none = run_kind(&sheet, AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())).unwrap();
    assert!(res_none.pairwise_comparisons.is_empty() && !res_none.tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons));
}

#[test]
fn pillar3_clustering_all_distances_linkages_cophenetic_silhouettes_and_sweep() {
    let rows = vec![vec![1.0, 1.0, 0.0], vec![1.1, 0.9, 0.0], vec![5.0, 5.1, 1.0], vec![5.2, 4.9, 1.1], vec![0.0, 0.0, 0.0], vec![2.0, 2.0, 2.0]];
    for dist in [ClusterDistance::Euclidean, ClusterDistance::Manhattan, ClusterDistance::Pearson, ClusterDistance::Cosine] {
        let dmat = pairwise_distance_matrix(&rows, dist).unwrap();
        assert_eq!(dmat.len(), rows.len());
        for (i, drow) in dmat.iter().enumerate() {
            assert_close(drow[i], 0.0, 1e-12, "diagonal distance zero");
            for (j, &val) in drow.iter().enumerate() {
                assert!(val.is_finite() && val >= 0.0);
                assert_close(val, dmat[j][i], 1e-12, "symmetric distance");
            }
        }
        assert_close(pairwise_distance_value(&rows[4], &rows[4], dist).unwrap(), 0.0, 1e-12, "zero self-distance");
        assert!(pairwise_distance_value(&rows[4], &rows[0], dist).unwrap().is_finite());

        for linkage in [
            ClusterLinkage::Ward,
            ClusterLinkage::Complete,
            ClusterLinkage::Single,
            ClusterLinkage::Upgma,
            ClusterLinkage::Wpgma,
            ClusterLinkage::Centroid,
            ClusterLinkage::Median,
        ] {
            let merges = hierarchical_clustering_with_metrics(&rows, dist, linkage).unwrap();
            assert_eq!(merges.len(), rows.len() - 1);
            let coph = cophenetic_distance_matrix(rows.len(), &merges).unwrap();
            let coph_r = cophenetic_correlation(&dmat, &coph).unwrap();
            assert!(coph_r.is_finite() && (-1.0..=1.0 + 1e-12).contains(&coph_r));
            let cut3 = cut_dendrogram(rows.len(), &merges, 3).unwrap();
            assert!(cut3.len() == rows.len() && cut3.iter().all(|&c| c < 3));
        }

        let km = kmeans_pp_with_distance(&rows, 3, dist, 42).unwrap();
        assert!(km.k == 3 && km.wcss.is_finite() && km.wcss >= 0.0 && km.mean_silhouette.is_finite());
    }

    let mv_sheet = mv_num_sheet(
        "mv-clust",
        &[("X1", &[1.0, 0.9, 0.1, 0.2, 0.1, 0.2]), ("X2", &[0.2, 0.1, 1.0, 0.9, 0.2, 0.1]), ("X3", &[0.1, 0.2, 0.2, 0.1, 1.0, 0.9])],
    );
    let cl_res = run_kind(
        &mv_sheet,
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(3),
            distance: ClusterDistance::Cosine,
            linkage: ClusterLinkage::Upgma,
            k_sweep_max: Some(5),
            consensus_optimal_k: false,
            kmeans_algorithm: Default::default(),
            cluster_direction: Default::default(),
            cut_height: None,
            emit_anova_tab: false,
        }),
    )
    .unwrap();
    assert!(scalar(&cl_res, "cophenetic_r") > 0.7);
    for (tab_id, expected_rows) in [
        ("kmeans-clusters", 3),
        ("kmeans-assignments", 6),
        ("kmeans-sweep-curve", 4),
        ("hierarchical-dendrogram", 5),
        ("hierarchical-cut-silhouettes", 4),
        ("clustering-distance-matrix", 6),
        ("cophenetic-distance-matrix", 6),
    ] {
        assert_eq!(tab_by_id(&cl_res, tab_id).rows.len(), expected_rows, "{tab_id}");
    }
    assert!(tab_by_id(&cl_res, "clustering-diagnostics").row_labels.iter().any(|l| l == "Cophenetic correlation (r)"));
}
