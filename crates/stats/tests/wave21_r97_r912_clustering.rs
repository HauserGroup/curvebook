#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for Wave 21 Statistical Pillars:
//! - Pillar 2 (`R9.12` / `QOL-12`): Kappa wiring, Altman (1998) NNT, McNemar OR + Clopper-Pearson CI,
//!   PartsOfWhole custom expected proportions + G^2, ExactBinomial, Column SD CI, Paired Sign Test,
//!   Pre-summarized two-group t-tests, Factorial ANOVA multiplicity
//! - Pillar 3 (`WF-37` / `WF-38`): Clustering distances (4), linkages (7), cophenetic matrix + r,
//!   hierarchical cut silhouettes, and k-means / candidate-k WCSS + silhouette sweep.

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec,
    ContingencyTable, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GroupedTable, KappaWeighting, MultipleVariablesTable, MultiplicityMethod,
    MultiplicitySpec, MultivariateSpec, PartsOfWholeTable, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnMode,
    SumOfSquaresType, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_stats::categorical::{
    NntKind, agresti_coull_ci, altman_nnt_from_rd, analyze_2x2_ratios, clopper_pearson_ci, cohens_kappa, exact_binomial_test, fleiss_kappa,
    mcnemar_detailed, wilson_score_ci,
};
use lumencraft_stats::column::summarize_column;
use lumencraft_stats::error::StatsError;
use lumencraft_stats::multivariate::{
    cophenetic_correlation, cophenetic_distance_matrix, cut_dendrogram, hierarchical_clustering_with_metrics, kmeans_pp_with_distance,
    pairwise_distance_matrix, pairwise_distance_value,
};
use lumencraft_stats::run_analysis;

fn scalar(sheet: &ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn assert_close(actual: f64, expected: f64, tol: f64, ctx: &str) {
    assert!((actual - expected).abs() <= tol, "{ctx}: expected {expected}, got {actual} (diff = {})", (actual - expected).abs());
}

fn column_sheet(mode: SubcolumnMode, cols: &[(&str, Vec<Vec<f64>>)]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, rows)| rows.len()).max().unwrap_or(0);
    let hint = match mode {
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => ColumnDesignHint::UnpairedReplicates,
        _ => ColumnDesignHint::PrecomputedError,
    };
    let mut tab = ColumnTable::new_with_subcolumns(hint, mode, cols.len(), n_rows).unwrap();
    for (c, (title, rows)) in cols.iter().enumerate() {
        tab.data_sets[c].title = (*title).to_owned();
        for (r, subvals) in rows.iter().enumerate() {
            for (s, &v) in subvals.iter().enumerate() {
                tab.data_sets[c].cells[r][s] = Cell::number(v);
            }
        }
    }
    DataSheet::new(SheetId::new("col-1"), "Columns", TableArchetype::Column(tab)).unwrap()
}

fn raw_column_sheet(cols: &[(&str, &[f64])]) -> DataSheet {
    let wrapped: Vec<(&str, Vec<Vec<f64>>)> = cols.iter().map(|(name, vals)| (*name, vals.iter().map(|&v| vec![v]).collect())).collect();
    column_sheet(SubcolumnMode::Single, &wrapped)
}

fn contingency_sheet(rows: &[Vec<f64>]) -> DataSheet {
    let n_rows = rows.len();
    let n_cols = rows.first().map_or(0, Vec::len);
    let mut ct = ContingencyTable::new(n_cols, n_rows).unwrap();
    for (r, row) in rows.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new("ct-1"), "Contingency", TableArchetype::Contingency(ct)).unwrap()
}

fn parts_sheet(slices: &[(&str, f64)]) -> DataSheet {
    let mut pow = PartsOfWholeTable::new(1, slices.len()).unwrap();
    for (i, (label, val)) in slices.iter().enumerate() {
        pow.slice_labels[i] = (*label).to_owned();
        pow.data_sets[0].cells[i][0] = Cell::number(*val);
    }
    DataSheet::new(SheetId::new("pow-1"), "PartsOfWhole", TableArchetype::PartsOfWhole(pow)).unwrap()
}

#[test]
fn pillar2_kappa_wiring_tabs_and_scalars() {
    let matrix = [vec![20_u64, 2, 1], vec![3, 18, 2], vec![0, 1, 25]];
    let mat_f64: Vec<Vec<f64>> = matrix.iter().map(|r| r.iter().map(|&v| v as f64).collect()).collect();
    let sheet = contingency_sheet(&mat_f64);

    for weighting in [KappaWeighting::Unweighted, KappaWeighting::Linear, KappaWeighting::Quadratic] {
        let direct = cohens_kappa(&matrix, weighting, ConfidenceLevel::DEFAULT_95).unwrap();
        let spec = AnalysisSpec::new(
            SheetId::new("r-ck"),
            SheetId::new("ct-1"),
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::CohenKappa { weighting },
                compute_odds_and_risk_ratios: false,
                compute_sensitivity_specificity: false,
                compute_phi_and_cramers_v: false,
                expected_proportions: None,
                null_proportion: None,
            }),
        )
        .unwrap();
        let res = run_analysis(&sheet, &spec).unwrap();
        assert_close(scalar(&res, "kappa"), direct.kappa, 1e-12, "Cohen kappa");
        assert_close(scalar(&res, "se"), direct.se, 1e-12, "Cohen SE");
        assert_close(scalar(&res, "z"), direct.z_statistic, 1e-12, "Cohen z");
        assert_close(scalar(&res, "p_value"), direct.p_value, 1e-12, "Cohen p");
        assert_close(scalar(&res, "ci_lower"), direct.ci.0, 1e-12, "Cohen ci_lower");
        assert_close(scalar(&res, "ci_upper"), direct.ci.1, 1e-12, "Cohen ci_upper");

        assert_eq!(tab_by_id(&res, "contingency-kappa").kind, ResultTabKind::MainSummary);
        assert_eq!(tab_by_id(&res, "contingency-kappa-diagnostics").kind, ResultTabKind::Diagnostics);
        assert!(res.checklist.iter().all(|c| c.remediation.is_none()));
    }

    // Non-square N x K matrix for Fleiss kappa, including unused category check cleanly handled
    let fleiss_counts = [vec![3_u64, 0, 0], vec![0, 3, 0], vec![2, 1, 0], vec![0, 0, 3], vec![1, 1, 1]];
    let fleiss_f64: Vec<Vec<f64>> = fleiss_counts.iter().map(|r| r.iter().map(|&v| v as f64).collect()).collect();
    let f_sheet = contingency_sheet(&fleiss_f64);
    let f_direct = fleiss_kappa(&fleiss_counts, ConfidenceLevel::DEFAULT_95).unwrap();
    let f_spec = AnalysisSpec::new(
        SheetId::new("r-fk"),
        SheetId::new("ct-1"),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FleissKappa, ..Default::default() }),
    )
    .unwrap();
    let f_res = run_analysis(&f_sheet, &f_spec).unwrap();
    assert_close(scalar(&f_res, "kappa"), f_direct.kappa, 1e-12, "Fleiss kappa");
    assert_close(scalar(&f_res, "se"), f_direct.se, 1e-12, "Fleiss se");
    assert_close(scalar(&f_res, "z"), f_direct.z_statistic, 1e-12, "Fleiss z");
    assert_close(scalar(&f_res, "p_value"), f_direct.p_value, 1e-12, "Fleiss p");
    assert_close(scalar(&f_res, "ci_lower"), f_direct.ci.0, 1e-12, "Fleiss ci_lower");
    assert_close(scalar(&f_res, "ci_upper"), f_direct.ci.1, 1e-12, "Fleiss ci_upper");
    assert_eq!(tab_by_id(&f_res, "contingency-kappa").kind, ResultTabKind::MainSummary);
    assert_eq!(tab_by_id(&f_res, "contingency-kappa-diagnostics").kind, ResultTabKind::Diagnostics);
}

#[test]
fn pillar2_altman_nnt_all_branches() {
    // 1. Benefit with CI excluding 0
    let r_ben = analyze_2x2_ratios(40, 10, 10, 40, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(r_ben.nnt_kind, NntKind::Benefit);
    assert_close(r_ben.nnt_value.unwrap(), 1.0 / 0.6, 1e-12, "NNTB point");
    assert_eq!(r_ben.nnt_point_label, "NNTB 1.67");
    let (lo, hi) = r_ben.nnt_ci_bounds.unwrap();
    assert!(lo < r_ben.nnt_value.unwrap() && r_ben.nnt_value.unwrap() < hi);
    assert!(r_ben.nnt_ci_label.starts_with("NNTB "));

    // Verify ResultTab preserves existing ratio labels & adds Altman NNT
    let sheet = contingency_sheet(&[vec![40.0, 10.0], vec![10.0, 40.0]]);
    let spec = AnalysisSpec::new(
        SheetId::new("r-nnt"),
        SheetId::new("ct-1"),
        AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }),
    )
    .unwrap();
    let out = run_analysis(&sheet, &spec).unwrap();
    let ratios_tab = tab_by_id(&out, "contingency-ratios");
    assert!(ratios_tab.row_labels.iter().any(|l| l.contains("Baptista-Pike")));
    assert!(ratios_tab.row_labels.iter().any(|l| l.contains("Koopman")));
    assert!(ratios_tab.row_labels.iter().any(|l| l.contains("Newcombe")));
    let nnt_idx = ratios_tab.row_labels.iter().position(|l| l == "Number needed to treat (NNT)").unwrap();
    assert_eq!(ratios_tab.rows[nnt_idx][0], ResultCell::Text(String::from("NNTB 1.67")));
    assert!(matches!(&ratios_tab.rows[nnt_idx][1], ResultCell::Text(s) if s.starts_with("NNTB ")));

    // 2. Harm with CI excluding 0
    let r_harm = analyze_2x2_ratios(10, 40, 40, 10, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(r_harm.nnt_kind, NntKind::Harm);
    assert_close(r_harm.nnt_value.unwrap(), 1.0 / 0.6, 1e-12, "NNTH point");
    assert_eq!(r_harm.nnt_point_label, "NNTH 1.67");
    assert!(r_harm.nnt_ci_label.starts_with("NNTH "));

    // 3. RD == 0 exact infinite point & Altman continuous union CI crossing 0
    let r_zero = analyze_2x2_ratios(10, 10, 10, 10, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(r_zero.nnt_kind, NntKind::Infinite);
    assert!(r_zero.nnt_value.is_none());
    assert_eq!(r_zero.nnt_point_label, "∞");
    assert!(r_zero.nnt_ci_bounds.is_none());
    assert!(r_zero.nnt_ci_label.starts_with("NNTH ") && r_zero.nnt_ci_label.contains(" to ∞ to NNTB "));

    // 4. Non-zero point RD whose CI spans 0
    let r_cross = analyze_2x2_ratios(12, 8, 10, 10, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(r_cross.nnt_kind, NntKind::Benefit);
    assert!(r_cross.nnt_ci_bounds.is_none());
    assert!(r_cross.nnt_ci_label.contains(" to ∞ to NNTB "));

    // 5. Exact 0 boundary endpoints in Altman CI formatting
    let (_, _, _, lbl_lo0, b_lo0) = altman_nnt_from_rd(0.2, 0.0, 0.4);
    assert!(b_lo0.is_none());
    assert_eq!(lbl_lo0, "NNTH ∞ to ∞ to NNTB 2.50");
    let (_, _, _, lbl_hi0, b_hi0) = altman_nnt_from_rd(-0.2, -0.4, 0.0);
    assert!(b_hi0.is_none());
    assert_eq!(lbl_hi0, "NNTH 2.50 to ∞ to NNTB ∞");
}

#[test]
fn pillar2_mcnemar_discordant_or_and_clopper_pearson_ci() {
    let mc = mcnemar_detailed(18, 5, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(mc.discordant_odds_ratio.unwrap(), 18.0 / 5.0, 1e-12, "McNemar OR");
    let (p_lo, p_hi) = clopper_pearson_ci(18, 23, ConfidenceLevel::DEFAULT_95).unwrap();
    let (or_lo, or_hi) = mc.discordant_or_ci.unwrap();
    assert_close(or_lo, p_lo / (1.0 - p_lo), 1e-12, "McNemar OR CI lo");
    assert_close(or_hi, p_hi / (1.0 - p_hi), 1e-12, "McNemar OR CI hi");

    // End-to-end sheet execution for c = 0 (infinite OR, finite lower, Open upper CI)
    let sheet_inf = contingency_sheet(&[vec![20.0, 8.0], vec![0.0, 15.0]]);
    let spec = AnalysisSpec::new(
        SheetId::new("r-mc"),
        SheetId::new("ct-1"),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::McNemarPaired, ..Default::default() }),
    )
    .unwrap();
    let res_inf = run_analysis(&sheet_inf, &spec).unwrap();
    let mc_tab = tab_by_id(&res_inf, "contingency-mcnemar");
    assert_eq!(mc_tab.rows[1][0], ResultCell::Text(String::from("∞")));
    assert!(matches!(
        mc_tab.rows[1][1],
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(l), upper: CiBound::Open, .. } if l > 0.0
    ));

    // b = 0 edge case -> OR = 0, Finite lower 0.0, finite upper
    let mc_zero = mcnemar_detailed(0, 8, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(mc_zero.discordant_odds_ratio, Some(0.0));
    let (z_lo, z_hi) = mc_zero.discordant_or_ci.unwrap();
    assert_eq!(z_lo, 0.0);
    assert!(z_hi.is_finite() && z_hi > 0.0);

    // b = 0, c = 0 edge case -> None OR, p = 1
    let mc_none = mcnemar_detailed(0, 0, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(mc_none.discordant_odds_ratio.is_none());
    assert_eq!(mc_none.exact_p_value, 1.0);
}

#[test]
fn pillar2_parts_of_whole_custom_expected_proportions_and_validation() {
    // Classic Mendel dihybrid ratio 9:3:3:1 against [315, 101, 108, 32] (N = 556)
    let sheet = parts_sheet(&[("Yellow Round", 315.0), ("Yellow Wrinkled", 101.0), ("Green Round", 108.0), ("Green Wrinkled", 32.0)]);
    let spec = AnalysisSpec::new(
        SheetId::new("r-pow"),
        SheetId::new("pow-1"),
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::PearsonChiSquare { yates_continuity: false },
            expected_proportions: Some(vec![9.0, 3.0, 3.0, 1.0]),
            ..Default::default()
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let chi2 = scalar(&res, "Goodness-of-fit chi-square");
    let g2 = scalar(&res, "Likelihood-ratio G-squared");
    assert_close(chi2, 0.470_023_980_815_347_7, 1e-10, "Mendel GOF chi2");
    assert_close(g2, 0.475_445_238_998_232_4, 1e-10, "Mendel GOF G2");
    assert_eq!(tab_by_id(&res, "parts-gof-categories").rows.len(), 4);

    // Invalid proportion vectors fail cleanly
    for bad in [vec![0.5, 0.5], vec![0.3, 0.3, 0.4, -0.1], vec![0.0, 0.0, 0.0, 0.0], vec![1.0, 1.0, 1.0, 0.0]] {
        let bad_spec = AnalysisSpec::new(
            SheetId::new("r-bad"),
            SheetId::new("pow-1"),
            AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(bad), ..Default::default() }),
        )
        .unwrap();
        assert!(run_analysis(&sheet, &bad_spec).is_err());
    }
}

#[test]
fn pillar2_exact_binomial_all_table_inputs_p_methods_and_cis() {
    // Asymmetric null p0 = 0.2 where Sterne != doubled-tail (P(0) + P(4..=10) = 0.228248064)
    let asym = exact_binomial_test(0, 10, 0.2, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(asym.p_value_lower, 0.8_f64.powi(10), 1e-12, "lower P(X<=0)");
    assert_close(asym.p_value_sterne, 0.228_248_064, 1e-12, "Sterne P");
    assert_close(asym.p_value_doubled_tail, 2.0 * 0.8_f64.powi(10), 1e-12, "doubled tail P");

    let spec = AnalysisSpec::new(
        SheetId::new("r-bin"),
        SheetId::new("src"),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.3), ..Default::default() }),
    )
    .unwrap();

    // (a) PartsOfWhole 2 categories: k = 7 successes, 3 failures -> n = 10
    let pow_sheet = DataSheet::new(SheetId::new("src"), "POW", parts_sheet(&[("Success", 7.0), ("Failure", 3.0)]).archetype).unwrap();
    // (b) 1x2 Contingency and 2x1 Contingency
    let ct_1x2 = DataSheet::new(SheetId::new("src"), "CT1x2", contingency_sheet(&[vec![7.0, 3.0]]).archetype).unwrap();
    let ct_2x1 = DataSheet::new(SheetId::new("src"), "CT2x1", contingency_sheet(&[vec![7.0], vec![3.0]]).archetype).unwrap();
    // (c) Column binary 0/1 values
    let col_bin = DataSheet::new(
        SheetId::new("src"),
        "ColBin",
        raw_column_sheet(&[("Outcome", &[1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])]).archetype,
    )
    .unwrap();

    let direct = exact_binomial_test(7, 10, 0.3, ConfidenceLevel::DEFAULT_95).unwrap();
    let cp = clopper_pearson_ci(7, 10, ConfidenceLevel::DEFAULT_95).unwrap();
    let ws = wilson_score_ci(7, 10, ConfidenceLevel::DEFAULT_95);
    let ac = agresti_coull_ci(7, 10, ConfidenceLevel::DEFAULT_95);

    for sh in [&pow_sheet, &ct_1x2, &ct_2x1, &col_bin] {
        let res = run_analysis(sh, &spec).unwrap();
        assert_close(scalar(&res, "proportion"), 0.7, 1e-12, "proportion");
        assert_close(scalar(&res, "p_value"), direct.p_value_sterne, 1e-12, "p_value Sterne");
        assert_close(scalar(&res, "p_value_doubled_tail"), direct.p_value_doubled_tail, 1e-12, "p_value doubled");
        assert_close(scalar(&res, "clopper_pearson_lower"), cp.0, 1e-12, "CP lo");
        assert_close(scalar(&res, "clopper_pearson_upper"), cp.1, 1e-12, "CP hi");
        assert_close(scalar(&res, "wilson_lower"), ws.0, 1e-12, "Wilson lo");
        assert_close(scalar(&res, "wilson_upper"), ws.1, 1e-12, "Wilson hi");
        assert_close(scalar(&res, "agresti_coull_lower"), ac.0, 1e-12, "AC lo");
        assert_close(scalar(&res, "agresti_coull_upper"), ac.1, 1e-12, "AC hi");
        assert_eq!(tab_by_id(&res, "exact-binomial-main").kind, ResultTabKind::MainSummary);
        assert_eq!(tab_by_id(&res, "exact-binomial-cis").kind, ResultTabKind::ParametersAndConfidenceIntervals);
        assert_eq!(tab_by_id(&res, "exact-binomial-diagnostics").kind, ResultTabKind::Diagnostics);
    }

    // Invalid binary column value rejected
    let non_bin = DataSheet::new(SheetId::new("src"), "BadCol", raw_column_sheet(&[("Outcome", &[0.0, 1.0, 2.0])]).archetype).unwrap();
    assert!(matches!(run_analysis(&non_bin, &spec), Err(StatsError::InvalidParameter { .. })));

    // Invalid null_proportion rejected
    for bad_p in [0.0, 1.0, -0.2, 1.5, f64::NAN] {
        let bad_spec = AnalysisSpec::new(
            SheetId::new("r-badp"),
            SheetId::new("src"),
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::ExactBinomial,
                null_proportion: Some(bad_p),
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(matches!(run_analysis(&pow_sheet, &bad_spec), Err(StatsError::InvalidParameter { .. })));
    }
}

#[test]
fn pillar2_column_sd_confidence_interval() {
    let vals = [10.0, 12.0, 14.0, 16.0, 18.0];
    let s = summarize_column("Sample", &vals, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    let sd = s.sd.unwrap();
    let (lo, hi) = s.sd_ci.unwrap();
    assert_close(lo, sd * (4.0_f64 / 11.143_286_781_5).sqrt(), 1e-6, "SD CI lower");
    assert_close(hi, sd * (4.0_f64 / 0.484_418_557_1).sqrt(), 1e-6, "SD CI upper");

    let s1 = summarize_column("One", &[42.0], 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    assert!(s1.sd_ci.is_none());

    let sheet = raw_column_sheet(&[("Sample", &vals)]);
    let spec = AnalysisSpec::new(SheetId::new("r-desc"), SheetId::new("col-1"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let out = run_analysis(&sheet, &spec).unwrap();
    let desc_tab = out.tabs.iter().find(|t| t.kind == ResultTabKind::DescriptiveSummary).unwrap();
    let ci_row = desc_tab.row_labels.iter().position(|r| r == "CI of SD").expect("missing `CI of SD` row");
    assert!(matches!(
        desc_tab.rows[ci_row][0],
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(l), upper: CiBound::Finite(u), .. }
        if (l - lo).abs() < 1e-6 && (u - hi).abs() < 1e-6
    ));
}

#[test]
fn pillar2_paired_sign_test_and_presummarized_t_tests() {
    // d_i = x_i - y_i -> 7 positive (+2,+4,+5,+3,+1,+6,+8), 1 negative (-1), 1 zero tie (0)
    // Two-sided exact Binomial(8, 0.5) P = 2*(1+8)/256 = 9/128 = 0.0703125
    let sheet = raw_column_sheet(&[
        ("After", &[12.0, 14.0, 15.0, 13.0, 11.0, 16.0, 18.0, 19.0, 15.0]),
        ("Before", &[10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 20.0, 15.0]),
    ]);
    let spec = AnalysisSpec::new(
        SheetId::new("r-sign"),
        SheetId::new("col-1"),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..Default::default() }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_close(scalar(&res, "p_value"), 9.0 / 128.0, 1e-12, "Paired sign exact P");
    assert_close(scalar(&res, "n_positive"), 7.0, 1e-12, "n_pos");
    assert_close(scalar(&res, "n_negative"), 1.0, 1e-12, "n_neg");
    assert_close(scalar(&res, "n_ties"), 1.0, 1e-12, "n_ties");
    assert_close(scalar(&res, "median_difference"), 3.0, 1e-12, "median difference");
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::MainSummary));
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::Diagnostics));

    // Pre-summarized MeanSdN, MeanSemN, MeanCvN exact equivalence to raw replicates
    let raw_g1 = [10.0, 12.0, 14.0, 16.0, 18.0]; // mean=14, sd=sqrt(10), n=5
    let raw_g2 = [20.0, 22.0, 24.0, 26.0]; // mean=23, sd=sqrt(20/3), n=4
    let sd1 = 10.0_f64.sqrt();
    let sd2 = (20.0_f64 / 3.0).sqrt();
    let raw_sheet = raw_column_sheet(&[("A", &raw_g1), ("B", &raw_g2)]);
    let sum_sd = column_sheet(SubcolumnMode::MeanSdN, &[("A", vec![vec![14.0, sd1, 5.0]]), ("B", vec![vec![23.0, sd2, 4.0]])]);
    let sum_sem =
        column_sheet(SubcolumnMode::MeanSemN, &[("A", vec![vec![14.0, sd1 / 5.0_f64.sqrt(), 5.0]]), ("B", vec![vec![23.0, sd2 / 2.0, 4.0]])]);
    let sum_cv =
        column_sheet(SubcolumnMode::MeanCvN, &[("A", vec![vec![14.0, 100.0 * sd1 / 14.0, 5.0]]), ("B", vec![vec![23.0, 100.0 * sd2 / 23.0, 4.0]])]);

    for method in [TwoGroupMethod::UnpairedT, TwoGroupMethod::WelchT, TwoGroupMethod::OneSampleT { hypothetical: 10.0 }] {
        let raw_spec = AnalysisSpec::new(
            SheetId::new("r-raw"),
            raw_sheet.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }),
        )
        .unwrap();
        let raw_out = run_analysis(&raw_sheet, &raw_spec).unwrap();
        let raw_t = scalar(&raw_out, "t");
        let raw_p = scalar(&raw_out, "PValue");

        for s_sheet in [&sum_sd, &sum_sem, &sum_cv] {
            let s_spec = AnalysisSpec::new(
                SheetId::new("r-sum"),
                s_sheet.id.clone(),
                AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }),
            )
            .unwrap();
            let s_out = run_analysis(s_sheet, &s_spec).unwrap();
            assert_close(scalar(&s_out, "t"), raw_t, 1e-10, "pre-summarized t equivalence");
            assert_close(scalar(&s_out, "PValue"), raw_p, 1e-10, "pre-summarized P equivalence");
        }
    }

    // Nonparametric and paired tests must reject pre-summarized input cleanly
    for bad_method in [TwoGroupMethod::PairedT, TwoGroupMethod::MannWhitneyU, TwoGroupMethod::WilcoxonSignedRank, TwoGroupMethod::PairedSignTest] {
        let s_spec = AnalysisSpec::new(
            SheetId::new("r-bad"),
            sum_sd.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: bad_method, ..Default::default() }),
        )
        .unwrap();
        assert!(matches!(run_analysis(&sum_sd, &s_spec), Err(StatsError::RequiresRawReplicates)));
    }
}

#[test]
fn pillar2_factorial_anova_multiplicity_wiring() {
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(3), 3, 2).unwrap();
    gt.row_factor_title = String::from("Dose");
    gt.column_factor_title = String::from("Treatment");
    gt.row_titles = vec![String::from("Low"), String::from("High")];
    gt.data_sets[0].title = String::from("Placebo");
    gt.data_sets[1].title = String::from("DrugA");
    gt.data_sets[2].title = String::from("DrugB");
    let cell_vals = [[[10.0, 11.0, 12.0], [20.0, 21.0, 22.0], [30.0, 31.0, 32.0]], [[12.0, 13.0, 14.0], [35.0, 36.0, 37.0], [45.0, 46.0, 47.0]]];
    for (r, row) in cell_vals.iter().enumerate() {
        for (c, reps) in row.iter().enumerate() {
            for (s, &v) in reps.iter().enumerate() {
                gt.data_sets[c].cells[r][s] = Cell::number(v);
            }
        }
    }
    let sheet = DataSheet::new(SheetId::new("g-1"), "TwoWay", TableArchetype::Grouped(gt)).unwrap();

    // With TukeyHSD multiplicity: marginal A (1), marginal B (3), simple A within B (3*1=3), simple B within A (2*3=6) -> 13 comparisons
    let spec_tukey = AnalysisSpec::new(
        SheetId::new("r-fac"),
        SheetId::new("g-1"),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ss_type: SumOfSquaresType::TypeIII,
            ..Default::default()
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec_tukey).unwrap();
    assert_eq!(res.pairwise_comparisons.len(), 13);
    for id in [
        "multiple-comparisons-factor-a",
        "multiple-comparisons-factor-b",
        "multiple-comparisons-simple-a-within-b",
        "multiple-comparisons-simple-b-within-a",
    ] {
        assert_eq!(tab_by_id(&res, id).kind, ResultTabKind::MultipleComparisons);
    }

    // With MultiplicityMethod::None -> 0 MultipleComparisons tabs & pairwise records
    let spec_none =
        AnalysisSpec::new(SheetId::new("r-fac0"), SheetId::new("g-1"), AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())).unwrap();
    let res_none = run_analysis(&sheet, &spec_none).unwrap();
    assert!(res_none.pairwise_comparisons.is_empty());
    assert!(!res_none.tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons));
}

#[test]
fn pillar3_clustering_all_distances_linkages_cophenetic_silhouettes_and_sweep() {
    let rows = vec![
        vec![1.0, 1.0, 0.0],
        vec![1.1, 0.9, 0.0],
        vec![5.0, 5.1, 1.0],
        vec![5.2, 4.9, 1.1],
        vec![0.0, 0.0, 0.0], // zero norm / zero variance edge vector
        vec![2.0, 2.0, 2.0], // constant zero variance vector for Pearson
    ];

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
            assert_eq!(cut3.len(), rows.len());
            assert!(cut3.iter().all(|&c| c < 3));
        }

        let km = kmeans_pp_with_distance(&rows, 3, dist, 42).unwrap();
        assert_eq!(km.k, 3);
        assert!(km.wcss.is_finite() && km.wcss >= 0.0);
        assert!(km.mean_silhouette.is_finite());
    }

    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    for (c, title) in ["X1", "X2", "X3"].iter().enumerate() {
        mv.variables[c].title = (*title).to_owned();
    }
    let clean_rows = [[1.0, 0.2, 0.1], [0.9, 0.1, 0.2], [0.1, 1.0, 0.2], [0.2, 0.9, 0.1], [0.1, 0.2, 1.0], [0.2, 0.1, 0.9]];
    for (r, vals) in clean_rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            mv.variables[c].cells[r] = Cell::number(v);
        }
    }
    let mv_sheet = DataSheet::new(SheetId::new("mv-clust"), "Clustering", TableArchetype::MultipleVariables(mv)).unwrap();
    let cl_spec = AnalysisSpec::new(
        SheetId::new("r-cl"),
        SheetId::new("mv-clust"),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(3),
            distance: ClusterDistance::Cosine,
            linkage: ClusterLinkage::Upgma,
            k_sweep_max: Some(5),
        }),
    )
    .unwrap();
    let cl_res = run_analysis(&mv_sheet, &cl_spec).unwrap();
    let coph_r = scalar(&cl_res, "cophenetic_r");
    assert!(coph_r > 0.7, "expected strong cophenetic fit on clean 3-cluster data, got {coph_r}");

    assert_eq!(tab_by_id(&cl_res, "kmeans-clusters").rows.len(), 3);
    assert_eq!(tab_by_id(&cl_res, "kmeans-assignments").rows.len(), 6);
    assert_eq!(tab_by_id(&cl_res, "kmeans-sweep-curve").rows.len(), 4); // k = 2, 3, 4, 5
    assert_eq!(tab_by_id(&cl_res, "hierarchical-dendrogram").rows.len(), 5);
    assert_eq!(tab_by_id(&cl_res, "hierarchical-cut-silhouettes").rows.len(), 4); // k = 2, 3, 4, 5
    assert_eq!(tab_by_id(&cl_res, "clustering-distance-matrix").rows.len(), 6);
    assert_eq!(tab_by_id(&cl_res, "cophenetic-distance-matrix").rows.len(), 6);
    let diag_tab = tab_by_id(&cl_res, "clustering-diagnostics");
    assert!(diag_tab.row_labels.iter().any(|l| l == "Cophenetic correlation (r)"));
}
