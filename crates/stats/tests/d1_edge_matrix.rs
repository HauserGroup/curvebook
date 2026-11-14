#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! D1 Edge Oracle Matrix for `crates/stats` (`Wave 2 G1`).
//!
//! Covers edge-case numerical accuracy, rank-deficiency / separation stability, tie handling,
//! unbalanced/missing-cell convergence, and multiple-comparison calibration across six core
//! statistical families without duplicating `d1_reference_oracles.rs`.
//!
//! Clean-room black-box provenance (2026-11-13):
//! - Reference values generated independently via R 4.5.2 (2025-10-31) base `stats`, `nlme 3.1`,
//!   and `survival 3.8` using purely synthetic fixtures and `options(digits = 17)`.
//! - Zero proprietary software references, zero vendor text, and no runtime R dependency.

use std::collections::HashMap;

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConfidenceLevel, CoxTieMethod, DataSheet, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable,
    MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, RepeatedMeasuresDesign, ResultCell, ResultTabKind, SubcolumnMode, SumOfSquaresType,
    TableArchetype, VariableRole,
};
use lumencraft_math::DenseMatrix;
use lumencraft_stats::anova::{
    FactorialAnovaTerm, FactorialObservations, fit_factorial_anova, fit_random_intercept_reml, friedman_test, kruskal_wallis_test,
    run_factorial_anova, unequal_variance_one_way_anova,
};
use lumencraft_stats::multiplicity::{GroupComparisonInput, compare_groups_with_procedure};
use lumencraft_stats::multivariate::{LinkageMethod, hierarchical_clustering, kmeans_pp, pca_analysis};
use lumencraft_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression, run_glm_analysis};
use lumencraft_stats::survival::{CoxPhResult, CoxSubject, cox_ph_regression};
use lumencraft_testkit::metrics::{assert_approx_eq, assert_lre_at_least};

fn unbalanced_3way_fixture() -> FactorialObservations {
    let counts = [
        // C1 (k=0): (A1, B1..B3), (A2, B1..B3)
        (0_usize, 0_usize, 0_usize, &[12.1, 13.4][..]),
        (0, 1, 0, &[18.2, 17.5, 19.1, 16.8][..]),
        (0, 2, 0, &[24.0][..]),
        (1, 0, 0, &[9.2, 10.1, 8.7, 9.8][..]),
        (1, 1, 0, &[14.2][..]),
        (1, 2, 0, &[19.5, 20.2, 18.8, 21.0, 19.9, 20.4][..]),
        // C2 (k=1): (A1, B1..B3), (A2, B1..B3)
        (0, 0, 1, &[14.8, 15.2, 13.9][..]),
        (0, 1, 1, &[22.1, 21.4, 23.0, 20.8, 22.7][..]),
        (0, 2, 1, &[28.5, 29.1][..]),
        (1, 0, 1, &[11.4, 12.0][..]),
        (1, 1, 1, &[17.6][..]),
        (1, 2, 1, &[25.2, 24.8, 26.1][..]),
    ];

    let mut rows = Vec::new();
    for (a, b, c, ys) in counts {
        for &y in ys {
            rows.push((vec![a, b, c], y));
        }
    }

    FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B"), String::from("C")],
        factor_levels: vec![
            vec![String::from("A1"), String::from("A2")],
            vec![String::from("B1"), String::from("B2"), String::from("B3")],
            vec![String::from("C1"), String::from("C2")],
        ],
        rows,
    }
}

fn find_term<'a>(terms: &'a [FactorialAnovaTerm], name: &str) -> &'a FactorialAnovaTerm {
    terms.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("missing ANOVA term `{name}` in {terms:?}"))
}

fn build_group_input(index: usize, label: &str, data: &[f64]) -> GroupComparisonInput {
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    let css: f64 = data.iter().map(|&x| (x - mean) * (x - mean)).sum();
    let sd = (css / (n - 1.0)).sqrt();
    GroupComparisonInput { index, label: label.to_owned(), n, mean, sd }
}

const RM_MISSING_ROWS: &[(usize, usize, usize, f64)] = &[
    // (subj 0..7, A: 0=Placebo, 1=Drug, B: 0=T1, 1=T2, 2=T3, y)
    (0, 0, 0, 12.4),
    (1, 0, 0, 8.1),
    (2, 0, 0, 11.0),
    (3, 0, 0, 13.5),
    (4, 0, 0, 7.7),
    (5, 0, 0, 11.5),
    (6, 0, 0, 9.1),
    (7, 0, 0, 7.5),
    (0, 0, 1, 14.2),
    (1, 0, 1, 10.0),
    (2, 0, 1, 13.4),
    (4, 0, 1, 10.2),
    (5, 0, 1, 12.8),
    (6, 0, 1, 11.5),
    (7, 0, 1, 9.5),
    (0, 0, 2, 17.1),
    (1, 0, 2, 14.2),
    (2, 0, 2, 15.9),
    (3, 0, 2, 18.5),
    (4, 0, 2, 12.6),
    (5, 0, 2, 16.5),
    (6, 0, 2, 14.1),
    (7, 0, 2, 13.1),
    (0, 1, 0, 16.8),
    (1, 1, 0, 12.7),
    (2, 1, 0, 15.7),
    (4, 1, 0, 12.6),
    (5, 1, 0, 15.4),
    (6, 1, 0, 14.0),
    (7, 1, 0, 11.4),
    (0, 1, 1, 20.2),
    (1, 1, 1, 16.0),
    (2, 1, 1, 18.6),
    (3, 1, 1, 20.3),
    (4, 1, 1, 15.8),
    (5, 1, 1, 18.5),
    (7, 1, 1, 14.8),
    (1, 1, 2, 21.3),
    (2, 1, 2, 23.1),
    (3, 1, 2, 26.0),
    (4, 1, 2, 20.2),
    (5, 1, 2, 23.8),
    (6, 1, 2, 21.6),
    (7, 1, 2, 20.2),
];

fn heavily_tied_cox_subjects() -> Vec<CoxSubject> {
    let times = [
        2.0, 2.0, 2.0, 2.0, 2.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 8.0, 8.0, 8.0, 8.0, 12.0, 12.0, 12.0, 12.0, 12.0, 15.0, 15.0, 15.0, 20.0, 20.0, 20.0,
        20.0, 20.0,
    ];
    let events = [
        true, true, true, true, false, true, true, true, true, true, false, true, true, true, true, true, true, true, false, false, true, true,
        false, false, false, false, false, false,
    ];
    let x1 =
        [1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let x2 =
        [3.2, 2.8, 1.9, 2.5, 1.1, 2.9, 2.4, 3.1, 1.7, 2.0, 0.9, 1.8, 2.6, 1.2, 2.1, 1.4, 1.5, 2.3, 0.8, 1.0, 1.3, 2.0, 0.7, 0.5, 0.6, 0.4, 0.9, 1.1];
    (0..times.len()).map(|i| CoxSubject { time: times[i], event: events[i], x: vec![x1[i], x2[i]] }).collect()
}

fn assert_cox_residuals_valid(tag: &str, fit: &CoxPhResult) {
    assert_eq!(fit.global_schoenfeld_df, 2.0);
    assert!(fit.global_schoenfeld_chi2.is_finite() && fit.global_schoenfeld_chi2 >= 0.0);
    assert!(fit.global_schoenfeld_p > 0.15, "{tag} global Schoenfeld p={}", fit.global_schoenfeld_p);
    for cov in &fit.covariates {
        assert!((-1.0..=1.0).contains(&cov.schoenfeld_rho), "{tag} rho={}", cov.schoenfeld_rho);
        assert!(cov.schoenfeld_p > 0.15, "{tag} cov {} PH p={}", cov.name, cov.schoenfeld_p);
    }
    let m_sum: f64 = fit.martingale_residuals.iter().sum();
    assert_approx_eq(m_sum, 0.0, 1e-9, 1e-9, &format!("{tag} sum martingale residuals == 0"));
    assert!(fit.deviance_residuals.iter().all(|v| v.is_finite()));
}

fn unequal_n_groups() -> [GroupComparisonInput; 4] {
    let g1 = [10.2, 11.8, 9.5];
    let g2: Vec<f64> = (0..45).map(|i| 14.0 + 0.1 * (i as f64)).collect();
    let g3 = [22.1, 18.4, 25.6, 20.0, 24.3, 19.2, 23.0];
    let g4: Vec<f64> = (0..60).map(|i| 30.0 + 0.1 * (i as f64)).collect();
    [build_group_input(0, "G1", &g1), build_group_input(1, "G2", &g2), build_group_input(2, "G3", &g3), build_group_input(3, "G4", &g4)]
}

#[test]
fn edge_01a_unbalanced_three_way_type_iii_vs_type_i_and_r_reference() {
    // Provenance: R 4.5.2 (2025-10-31) `lm(y ~ A * B * C, contrasts=contr.sum)` / `drop1`
    // and sequential `anova(fit)` Type I table on synthetic 2x3x2 design (n = 34, cell counts 1..6).
    let data = unbalanced_3way_fixture();
    let res_iii = fit_factorial_anova(&data, 3, SumOfSquaresType::TypeIII).unwrap();
    assert!(res_iii.unbalanced, "severe cell-size range 1..6 must flag unbalanced=true");
    assert_eq!(res_iii.total_n, 34);
    assert_approx_eq(res_iii.ss_residual, 13.241_666_666_666_694, 1e-10, 1e-11, "3way SSE");
    assert_approx_eq(res_iii.df_residual, 22.0, 1e-12, 1e-12, "3way df_err");
    assert_approx_eq(res_iii.ms_residual, 0.601_893_939_393_941, 1e-10, 1e-11, "3way MSE");

    let expected_type_iii = [
        ("A", 1.0, 78.769_060_773_480_62, 130.868_672_398_985_78, 9.901_196_911_894_687e-11),
        ("B", 2.0, 687.025_265_279_231_5, 570.719_540_697_661_2, 1.105_272_485_096_524e-19),
        ("C", 1.0, 79.010_128_913_443_96, 131.269_188_377_276_04, 9.618_366_602_246_354e-11),
        ("A × B", 2.0, 0.948_506_762_779_536, 0.787_935_133_334_792, 4.672_017_124_165_459e-01),
        ("A × C", 1.0, 0.011_786_372_007_371, 0.019_582_141_031_756, 8.899_836_419_059_89e-01),
        ("B × C", 2.0, 10.412_189_280_316_207, 8.649_521_617_380_342, 1.692_184_602_630_792e-03),
        ("A × B × C", 2.0, 0.425_652_831_066_145, 0.353_594_548_148_087, 7.060_774_577_768_505e-01),
    ];

    for (name, df, ss, f_val, p_val) in expected_type_iii {
        let term = find_term(&res_iii.terms, name);
        assert_approx_eq(term.df, df, 1e-12, 1e-12, &format!("III {name} df"));
        assert_approx_eq(term.ss, ss, 1e-9, 1e-9, &format!("III {name} SS"));
        assert_approx_eq(term.f_statistic.unwrap(), f_val, 1e-9, 1e-9, &format!("III {name} F"));
        assert_approx_eq(term.p_value.unwrap(), p_val, 1e-12, 1e-7, &format!("III {name} p"));
    }

    let res_i = fit_factorial_anova(&data, 3, SumOfSquaresType::TypeI).unwrap();
    let expected_type_i = [
        ("A", 56.167_352_941_176_42, 93.317_691_481_878_95),
        ("B", 820.367_854_163_221_4, 681.488_714_597_515_1),
        ("C", 114.566_536_979_786_26, 190.343_396_870_129_37),
        ("A × B", 3.413_984_403_564_856_8, 2.836_034_872_690_754),
        ("A × C", 0.455_638_733_052_829_4, 0.757_008_341_887_645_7),
        ("B × C", 12.796_901_516_759_908, 10.630_528_635_697_337),
        ("A × B × C", 0.425_652_831_066_150_26, 0.353_594_548_148_091_9),
    ];
    let mut type_i_ss_sum = 0.0;
    let mut type_iii_ss_sum = 0.0;
    for (name, ss1, f1) in expected_type_i {
        let t1 = find_term(&res_i.terms, name);
        let t3 = find_term(&res_iii.terms, name);
        assert_approx_eq(t1.ss, ss1, 1e-9, 1e-9, &format!("I {name} SS"));
        assert_approx_eq(t1.f_statistic.unwrap(), f1, 1e-9, 1e-8, &format!("I {name} F"));
        type_i_ss_sum += t1.ss;
        type_iii_ss_sum += t3.ss;
    }
    assert_approx_eq(type_i_ss_sum, res_i.ss_total - res_i.ss_residual, 1e-9, 1e-10, "Type I SS partition");
    assert!((type_iii_ss_sum - (res_iii.ss_total - res_iii.ss_residual)).abs() > 100.0);
}

#[test]
fn edge_01b_three_way_additive_and_weighted_contrast_invariance() {
    // Additive 3-way model + balanced vs cell-weighted design invariance checks.
    let data = unbalanced_3way_fixture();
    let add_iii = fit_factorial_anova(&data, 1, SumOfSquaresType::TypeIII).unwrap();
    assert_approx_eq(add_iii.ss_residual, 30.333_844_151_110_423, 1e-9, 1e-9, "Add SSE");
    assert_approx_eq(add_iii.df_residual, 29.0, 1e-12, 1e-12, "Add df_err");
    assert_approx_eq(find_term(&add_iii.terms, "A").ss, 78.620_069_464_685_51, 1e-9, 1e-9, "Add III A SS");
    assert_approx_eq(find_term(&add_iii.terms, "B").ss, 814.900_550_312_557_6, 1e-9, 1e-9, "Add III B SS");
    assert_approx_eq(find_term(&add_iii.terms, "C").ss, 114.566_536_979_786_3, 1e-9, 1e-9, "Add III C SS");

    let cell_means = [
        (0_usize, 0_usize, 0_usize, [10.0, 11.0]),
        (0, 0, 1, [12.0, 13.0]),
        (0, 1, 0, [14.0, 15.0]),
        (0, 1, 1, [16.0, 17.0]),
        (1, 0, 0, [20.0, 21.0]),
        (1, 0, 1, [22.0, 23.0]),
        (1, 1, 0, [24.0, 25.0]),
        (1, 1, 1, [26.0, 27.0]),
    ];
    let mut bal_rows = Vec::new();
    for (a, b, c, ys) in cell_means {
        for &y in &ys {
            bal_rows.push((vec![a, b, c], y));
        }
    }
    let bal_obs = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B"), String::from("C")],
        factor_levels: vec![
            vec![String::from("A1"), String::from("A2")],
            vec![String::from("B1"), String::from("B2")],
            vec![String::from("C1"), String::from("C2")],
        ],
        rows: bal_rows.clone(),
    };
    let bal_i = fit_factorial_anova(&bal_obs, 3, SumOfSquaresType::TypeI).unwrap();
    let bal_iii = fit_factorial_anova(&bal_obs, 3, SumOfSquaresType::TypeIII).unwrap();
    assert!(!bal_i.unbalanced);
    for (t1, t3) in bal_i.terms.iter().zip(&bal_iii.terms) {
        assert_approx_eq(t1.ss, t3.ss, 1e-11, 1e-11, &format!("balanced {} Type I == III", t1.name));
    }

    let skewed_cells = [
        (0_usize, 0_usize, 0_usize, &[10.0, 10.2][..]),
        (0, 1, 0, &[12.0, 12.2][..]),
        (0, 0, 1, &[14.0, 14.2][..]),
        (0, 1, 1, &[16.0, 16.2][..]),
        (1, 0, 0, &[11.0, 11.2][..]),
        (1, 1, 0, &[25.0, 25.2, 25.1, 24.9][..]),
        (1, 0, 1, &[15.0, 15.2][..]),
        (1, 1, 1, &[29.0, 29.2, 29.1, 28.9, 29.0, 29.3][..]),
    ];
    let mut skewed_rows = Vec::new();
    for (a, b, c, ys) in skewed_cells {
        for &y in ys {
            skewed_rows.push((vec![a, b, c], y));
        }
    }
    let skewed_obs = FactorialObservations { factor_names: bal_obs.factor_names, factor_levels: bal_obs.factor_levels, rows: skewed_rows };
    let sk_i = fit_factorial_anova(&skewed_obs, 3, SumOfSquaresType::TypeI).unwrap();
    let sk_iii = fit_factorial_anova(&skewed_obs, 3, SumOfSquaresType::TypeIII).unwrap();
    assert!(sk_i.unbalanced);
    let sk_i_a = find_term(&sk_i.terms, "A").ss;
    let sk_iii_a = find_term(&sk_iii.terms, "A").ss;
    assert_approx_eq(sk_i_a, 536.355_584_415_584_5, 1e-8, 1e-8, "skewed Type I A SS");
    assert_approx_eq(sk_iii_a, 228.372_032_520_325_4, 1e-8, 1e-8, "skewed Type III A SS");
    assert!((sk_i_a - sk_iii_a).abs() > 300.0);
}

#[test]
fn edge_02a_missing_replicate_rm_reml_satterthwaite_kr_vs_nlme() {
    // Provenance: R 4.5.2 (2025-10-31) `nlme::lme(y ~ A * B, random = ~ 1 | subj, method = "REML")`
    // on synthetic 8-subject 2x3 repeated-measures cohort with 4 missing cells (44 / 48 rows).
    let mut x_data = Vec::with_capacity(RM_MISSING_ROWS.len() * 6);
    let mut y_vec = Vec::with_capacity(RM_MISSING_ROWS.len());
    let mut cluster_ids = Vec::with_capacity(RM_MISSING_ROWS.len());
    for &(subj, a, b, y) in RM_MISSING_ROWS {
        let ca = if a == 0 { 1.0 } else { -1.0 };
        let (cb1, cb2) = match b {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            _ => (-1.0, -1.0),
        };
        x_data.extend_from_slice(&[1.0, ca, cb1, cb2, ca * cb1, ca * cb2]);
        y_vec.push(y);
        cluster_ids.push(subj);
    }

    let x_mat = DenseMatrix::from_row_major(RM_MISSING_ROWS.len(), 6, x_data).unwrap();
    let reml = fit_random_intercept_reml(&x_mat, &y_vec, &cluster_ids, &[("A", 1, 1), ("B", 2, 2), ("A × B", 4, 2)]).unwrap();

    let exp_s2_u = 4.558_845_077_276_424;
    let exp_s2_e = 0.121_430_006_359_814;
    let exp_icc = exp_s2_u / (exp_s2_u + exp_s2_e);
    assert_approx_eq(reml.sigma2_random, exp_s2_u, 1e-4, 1e-4, "REML sigma2_random");
    assert_approx_eq(reml.sigma2_error, exp_s2_e, 1e-4, 1e-4, "REML sigma2_error");
    assert_approx_eq(reml.icc, exp_icc, 1e-4, 1e-4, "REML ICC");
    // Profile REML log-likelihood omits the constant -0.5 * (N - p) * ln(2*pi) with N - p = 44 - 6 = 38.
    let exp_profile_reml = -42.698_093_229_508_97 + 0.5 * 38.0 * std::f64::consts::TAU.ln();
    assert_approx_eq(reml.reml_log_lik, exp_profile_reml, 1e-4, 1e-4, "REML logLik");
    assert!(reml.lrt_random_chi2 > 50.0 && reml.lrt_random_p < 1e-10);

    let exp_f = [("A", 2_988.069_729_587_45), ("B", 1_343.763_711_046_137_6), ("A × B", 67.127_302_874_484_6)];
    for ((idx, (name, f_expected)), term) in exp_f.iter().enumerate().zip(&reml.fixed_terms) {
        assert_eq!(term.name, *name);
        assert_approx_eq(term.f_statistic.unwrap(), *f_expected, 1e-2, 5e-4, &format!("REML F {name}"));
        assert!(term.p_value.unwrap() < 1e-10);
        let sat = reml.satterthwaite_df[idx];
        let kr = reml.kenward_roger_df[idx];
        assert!(sat.is_finite() && kr.is_finite());
        assert!(kr >= 1.0 && kr <= sat + 1e-12 && sat <= 38.0);
        assert!(kr < sat, "KR adjustment should shrink finite-cluster df vs Satterthwaite");
    }
}

#[test]
fn edge_02b_grouped_rm_missing_cell_fallback_and_sparse_cluster_reml() {
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(8), 2, 3).unwrap();
    grouped.row_factor_title = String::from("Timepoint");
    grouped.column_factor_title = String::from("Treatment");
    grouped.row_titles = vec![String::from("T1"), String::from("T2"), String::from("T3")];
    grouped.data_sets[0].title = String::from("Placebo");
    grouped.data_sets[1].title = String::from("Drug");
    for &(subj, col, row, y) in RM_MISSING_ROWS {
        grouped.data_sets[col].cells[row][subj] = Cell::number(y);
    }

    let ds_id = lumencraft_doc::SheetId::new("g-rm-miss");
    let mut sheet = DataSheet::new(ds_id.clone(), "RM Missing Cells", TableArchetype::Grouped(grouped)).unwrap();
    sheet.id = ds_id.clone();

    let fac_spec = FactorialAnovaSpec {
        three_way: false,
        multifactor_columns: Vec::new(),
        response_column: None,
        max_interaction_order: 2,
        ss_type: SumOfSquaresType::TypeIII,
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        fit_reml_on_missing: true,
        compute_effect_sizes: true,
        multiplicity: MultiplicitySpec::default(),
    };
    let spec = AnalysisSpec::new("an-rm-miss", ds_id, AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap();
    let res_sheet = run_factorial_anova(&sheet, &spec, &fac_spec).unwrap();

    let scalar_map: HashMap<&str, f64> = res_sheet.scalars.iter().map(|s| (s.key.as_str(), s.value)).collect();
    let exp_s2_u = 4.558_845_077_276_424;
    let exp_s2_e = 0.121_430_006_359_814;
    assert_approx_eq(scalar_map["REML sigma^2 (subject)"], exp_s2_u, 1e-4, 1e-4, "Sheet REML s2_u");
    assert_approx_eq(scalar_map["REML sigma^2 (residual)"], exp_s2_e, 1e-4, 1e-4, "Sheet REML s2_e");
    assert_approx_eq(scalar_map["REML ICC"], exp_s2_u / (exp_s2_u + exp_s2_e), 1e-4, 1e-4, "Sheet REML ICC");
    let exp_profile_reml = -42.698_093_229_508_97 + 0.5 * 38.0 * std::f64::consts::TAU.ln();
    assert_approx_eq(scalar_map["REML log-likelihood"], exp_profile_reml, 1e-4, 1e-4, "Sheet REML logLik");
    assert!(
        res_sheet
            .tabs
            .iter()
            .any(|t| t.title == "Mixed-effects REML variance components" && t.kind == ResultTabKind::ParametersAndConfidenceIntervals)
    );
    assert!(res_sheet.checklist.iter().any(|c| c.assumption.contains("REML") && c.explanation.contains("instead of listwise deletion")));

    let sparse_clusters = [0, 0, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4];
    let sparse_x_vals: Vec<f64> = sparse_clusters.iter().enumerate().flat_map(|(i, _)| [1.0, if i % 2 == 0 { 1.0 } else { -1.0 }]).collect();
    let cluster_off = [3.1, -2.4, 0.8, 1.5, -3.0];
    let sparse_y: Vec<f64> = sparse_clusters
        .iter()
        .enumerate()
        .map(|(i, &c)| 10.0 + cluster_off[c] + (if i % 2 == 0 { 2.5 } else { -2.5 }) + 0.1 * ((i % 3) as f64 - 1.0))
        .collect();
    let sparse_x = DenseMatrix::from_row_major(sparse_clusters.len(), 2, sparse_x_vals).unwrap();
    let sparse_reml = fit_random_intercept_reml(&sparse_x, &sparse_y, &sparse_clusters, &[("Cond", 1, 1)]).unwrap();
    assert!(sparse_reml.sigma2_random > 1.0 && sparse_reml.sigma2_error > 0.0 && sparse_reml.icc > 0.8);
    assert!(sparse_reml.kenward_roger_df[0] < sparse_reml.satterthwaite_df[0]);
}

#[test]
fn edge_03_heavily_tied_cox_ph_breslow_vs_efron_and_schoenfeld_diagnostics() {
    // Provenance: R 4.5.2 (2025-10-31) `survival::coxph(Surv(time, status) ~ x1 + x2, ties="breslow"/"efron")`
    // on synthetic 28-subject heavy-tie cohort (18 events across 5 tied time clusters + tied censors).
    let subjects = heavily_tied_cox_subjects();
    let names = vec![String::from("x1"), String::from("x2")];
    let bres = cox_ph_regression(&subjects, &names, CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95).unwrap();
    let efrn = cox_ph_regression(&subjects, &names, CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).unwrap();

    assert!(bres.converged && efrn.converged);
    assert_eq!((bres.n_subjects, bres.n_events), (28, 18));

    assert_approx_eq(bres.covariates[0].beta, -2.507_024_691_411_398_3, 1e-6, 1e-6, "Breslow beta x1");
    assert_approx_eq(bres.covariates[0].se, 1.032_958_442_525_786_3, 1e-6, 1e-6, "Breslow SE x1");
    assert_approx_eq(bres.covariates[0].hazard_ratio, 0.081_510_397_400_069_77, 1e-6, 1e-6, "Breslow HR x1");
    assert_approx_eq(bres.covariates[1].beta, 3.301_149_562_691_036_5, 1e-6, 1e-6, "Breslow beta x2");
    assert_approx_eq(bres.covariates[1].se, 0.848_703_810_445_872_1, 1e-6, 1e-6, "Breslow SE x2");
    assert_approx_eq(bres.covariates[1].hazard_ratio, 27.143_824_520_283_236, 1e-5, 1e-6, "Breslow HR x2");
    assert_approx_eq(bres.log_lik_null, -52.192_873_652_315_71, 1e-6, 1e-7, "Breslow logLik0");
    assert_approx_eq(bres.log_lik_model, -38.147_457_075_983_95, 1e-6, 1e-7, "Breslow logLik");
    assert_approx_eq(bres.likelihood_ratio_chi2, 28.090_833_152_663_52, 1e-5, 1e-6, "Breslow LR");
    assert_approx_eq(bres.wald_chi2, 18.789_276_166_983_612, 1e-6, 1e-6, "Breslow Wald");
    assert_approx_eq(bres.score_chi2, 25.107_715_670_935_3, 1e-4, 1e-5, "Breslow Score");
    assert_approx_eq(bres.concordance, 0.921_428_571_428_571, 1e-12, 1e-12, "Breslow C-index");

    assert_approx_eq(efrn.covariates[0].beta, -2.994_256_788_991_598_3, 1e-6, 1e-6, "Efron beta x1");
    assert_approx_eq(efrn.covariates[0].se, 1.070_276_579_939_171, 1e-6, 1e-6, "Efron SE x1");
    assert_approx_eq(efrn.covariates[0].hazard_ratio, 0.050_073_828_681_265_28, 1e-6, 1e-6, "Efron HR x1");
    assert_approx_eq(efrn.covariates[1].beta, 3.913_231_499_129_283_4, 1e-6, 1e-6, "Efron beta x2");
    assert_approx_eq(efrn.covariates[1].se, 0.899_866_017_245_41, 1e-6, 1e-6, "Efron SE x2");
    assert_approx_eq(efrn.covariates[1].hazard_ratio, 50.060_461_211_194_9, 1e-4, 1e-6, "Efron HR x2");
    assert_approx_eq(efrn.log_lik_null, -50.742_256_666_597_06, 1e-6, 1e-7, "Efron logLik0");
    assert_approx_eq(efrn.log_lik_model, -32.895_820_790_578_97, 1e-6, 1e-7, "Efron logLik");
    assert_approx_eq(efrn.likelihood_ratio_chi2, 35.692_871_752_036_17, 1e-5, 1e-6, "Efron LR");
    assert_approx_eq(efrn.wald_chi2, 23.208_087_627_216_862, 1e-6, 1e-6, "Efron Wald");
    assert_approx_eq(efrn.score_chi2, 32.082_681_331_849_13, 1e-4, 1e-5, "Efron Score");
    assert_approx_eq(efrn.concordance, 0.921_428_571_428_571, 1e-12, 1e-12, "Efron C-index");

    assert!((efrn.covariates[0].beta - bres.covariates[0].beta).abs() > 0.45);
    assert!((efrn.covariates[1].beta - bres.covariates[1].beta).abs() > 0.55);
    assert!(efrn.likelihood_ratio_chi2 > bres.likelihood_ratio_chi2 + 7.0);

    assert_cox_residuals_valid("Breslow", &bres);
    assert_cox_residuals_valid("Efron", &efrn);
}

#[test]
fn edge_04a_logistic_separation_and_multicollinear_vif() {
    // Provenance: R 4.5.2 (2025-10-31) `glm(family=binomial)` and `lm` auxiliary VIFs.
    let x1_glm = [0.5, 1.0, 1.2, 1.8, 2.1, 2.5, 2.9, 3.2, 3.5, 3.9, 4.2, 4.8];
    let x2_glm = [2.0, 1.8, 2.2, 1.5, 1.9, 1.1, 1.4, 0.8, 1.2, 0.5, 0.9, 0.4];
    let y_bin = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let x_log = DenseMatrix::from_row_major(12, 3, (0..12).flat_map(|i| [1.0, x1_glm[i], x2_glm[i]]).collect()).unwrap();
    let log_res = multiple_logistic_regression(
        &x_log,
        &y_bin,
        &[String::from("Intercept"), String::from("x1"), String::from("x2")],
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert!(log_res.converged && !log_res.separation_detected);
    assert_approx_eq(log_res.terms[0].beta, -0.783_423_867_835_24, 1e-6, 1e-6, "Logit b0");
    assert_approx_eq(log_res.terms[1].beta, 1.024_081_862_270_01, 1e-6, 1e-6, "Logit b1");
    assert_approx_eq(log_res.terms[2].beta, -1.020_528_523_327_89, 1e-6, 1e-6, "Logit b2");
    assert_approx_eq(log_res.deviance_null, 16.300_638_383_796_613, 1e-6, 1e-7, "Logit null dev");
    assert_approx_eq(log_res.deviance_residual, 10.489_871_992_290_69, 1e-6, 1e-7, "Logit res dev");
    assert_approx_eq(log_res.aic, 16.489_871_992_290_69, 1e-6, 1e-7, "Logit AIC");
    assert_approx_eq(log_res.terms[1].vif.unwrap(), 5.520_283_445_094_411, 1e-7, 1e-7, "Logit VIF");

    let x_comp_vals = [0.1, 0.4, 0.8, 1.1, 1.5, 2.4, 2.9, 3.3, 3.7, 4.1];
    let y_comp = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let x_comp = DenseMatrix::from_row_major(10, 2, x_comp_vals.iter().flat_map(|&x| [1.0, x]).collect()).unwrap();
    let sep_comp =
        multiple_logistic_regression(&x_comp, &y_comp, &[String::from("Intercept"), String::from("x")], false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(sep_comp.separation_detected);
    assert!(sep_comp.terms.iter().all(|t| t.beta.is_finite() && t.se.is_finite()));
    assert!(sep_comp.predicted_probs[..5].iter().all(|&p| p < 0.15));
    assert!(sep_comp.predicted_probs[5..].iter().all(|&p| p > 0.85));

    let x_quasi_vals = [0.5, 1.0, 1.5, 2.0, 2.0, 2.0, 2.5, 3.0, 3.5, 4.0];
    let y_quasi = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let x_quasi = DenseMatrix::from_row_major(10, 2, x_quasi_vals.iter().flat_map(|&x| [1.0, x]).collect()).unwrap();
    let sep_quasi =
        multiple_logistic_regression(&x_quasi, &y_quasi, &[String::from("Intercept"), String::from("x")], false, ConfidenceLevel::DEFAULT_95)
            .unwrap();
    assert!(sep_quasi.terms[1].beta > 2.0 && sep_quasi.terms[1].beta.is_finite());
    assert!(sep_quasi.deviance_residual < sep_quasi.deviance_null);

    let z1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let z2 = [2.1, 1.5, 3.2, 2.8, 4.1, 3.9, 5.2, 4.8, 6.1, 5.9];
    let noise_z = [0.01, -0.02, 0.015, -0.01, 0.02, -0.015, 0.01, -0.02, 0.01, -0.005];
    let err_y = [0.1, -0.2, 0.15, -0.1, 0.05, 0.2, -0.15, 0.1, -0.05, 0.0];
    let z3: Vec<f64> = (0..10).map(|i| 2.0 * z1[i] - z2[i] + noise_z[i]).collect();
    let y_vif: Vec<f64> = (0..10).map(|i| 3.0 + 1.5 * z1[i] - 0.8 * z2[i] + 0.5 * z3[i] + err_y[i]).collect();
    let x_vif = DenseMatrix::from_row_major(10, 4, (0..10).flat_map(|i| [1.0, z1[i], z2[i], z3[i]]).collect()).unwrap();
    let mlr = multiple_linear_regression(
        &x_vif,
        &y_vif,
        &[String::from("Intercept"), String::from("z1"), String::from("z2"), String::from("z3")],
        true,
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert_eq!(mlr.rank, 4);
    assert_lre_at_least(mlr.terms[1].estimate, 14.647_752_108_302_235, 8.0, "Near-collinear b_z1");
    assert_lre_at_least(mlr.terms[1].vif.unwrap(), 1_582_136.547_445_832_5, 7.0, "VIF(z1)");
    assert_lre_at_least(mlr.terms[2].vif.unwrap(), 100_877.876_351_290_24, 7.0, "VIF(z2)");
    assert_lre_at_least(mlr.terms[3].vif.unwrap(), 920_481.326_983_620_4, 7.0, "VIF(z3)");

    let x_exact = DenseMatrix::from_row_major(10, 4, (0..10).flat_map(|i| [1.0, z1[i], z2[i], 2.0 * z1[i] - z2[i]]).collect()).unwrap();
    let mlr_alias = multiple_linear_regression(
        &x_exact,
        &y_vif,
        &[String::from("Intercept"), String::from("z1"), String::from("z2"), String::from("z3_exact")],
        true,
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert_eq!(mlr_alias.rank, 3);
    assert!(mlr_alias.aliased_columns.iter().any(|&a| a));
}

#[test]
fn edge_04b_poisson_exposure_scaling_overdispersion_and_sheet_dispatch() {
    // Provenance: R 4.5.2 (2025-10-31) `glm(family=poisson)` baseline, log-exposure, and overdispersed fixtures.
    let dose = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5];
    let grp = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let cnt = [2.0, 4.0, 8.0, 11.0, 9.0, 14.0, 24.0, 31.0, 22.0, 45.0];
    let x_p = DenseMatrix::from_row_major(10, 3, (0..10).flat_map(|i| [1.0, dose[i], grp[i]]).collect()).unwrap();
    let p_res = multiple_poisson_regression(
        &x_p,
        &cnt,
        &[String::from("Intercept"), String::from("dose"), String::from("grp")],
        false,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert!(p_res.converged);
    assert_approx_eq(p_res.terms[0].beta, 1.199_847_125_704_637, 1e-6, 1e-6, "Poisson b0");
    assert_approx_eq(p_res.terms[1].beta, 0.488_073_784_267_687, 1e-6, 1e-6, "Poisson b_dose");
    assert_approx_eq(p_res.terms[2].beta, 0.463_801_847_786_576, 1e-6, 1e-6, "Poisson b_grp");
    assert_approx_eq(p_res.deviance_null, 94.098_097_483_681_3, 1e-6, 1e-7, "Poisson null dev");
    assert_approx_eq(p_res.deviance_residual, 1.688_535_471_289_937, 1e-6, 1e-7, "Poisson res dev");
    assert_approx_eq(p_res.aic, 51.184_357_737_969_67, 1e-5, 1e-6, "Poisson AIC");
    assert_approx_eq(p_res.dispersion_pearson, 0.235_448_816_672_422, 1e-6, 1e-6, "Poisson dispersion");

    let cnt_4x: Vec<f64> = cnt.iter().map(|&c| 4.0 * c).collect();
    let p_scaled = multiple_poisson_regression(
        &x_p,
        &cnt_4x,
        &[String::from("Intercept"), String::from("dose"), String::from("grp")],
        false,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert_approx_eq(p_scaled.terms[0].beta - p_res.terms[0].beta, 4.0_f64.ln(), 1e-8, 1e-8, "exposure shift ln(4)");
    assert_approx_eq(p_scaled.terms[1].beta, p_res.terms[1].beta, 1e-8, 1e-8, "exposure slope invariance");

    let exposure = [1.0_f64, 2.0, 1.5, 3.0, 1.2, 2.5, 2.0, 4.0, 1.8, 3.5];
    let x_pe = DenseMatrix::from_row_major(10, 4, (0..10).flat_map(|i| [1.0, dose[i], grp[i], exposure[i].ln()]).collect()).unwrap();
    let p_exp = multiple_poisson_regression(
        &x_pe,
        &cnt,
        &[String::from("Intercept"), String::from("dose"), String::from("grp"), String::from("log_exp")],
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert_approx_eq(p_exp.terms[1].beta, 0.466_838_521_497_392_4, 1e-6, 1e-6, "Poisson+exp dose");
    assert_approx_eq(p_exp.terms[3].beta, 0.156_680_576_514_195_23, 1e-6, 1e-6, "Poisson+exp log_exp");
    assert_approx_eq(p_exp.aic, 52.921_865_596_522_47, 1e-5, 1e-6, "Poisson+exp AIC");

    let cnt_od = [0.0, 1.0, 18.0, 2.0, 0.0, 27.0, 4.0, 62.0, 1.0, 95.0];
    let p_od = multiple_poisson_regression(
        &x_p,
        &cnt_od,
        &[String::from("Intercept"), String::from("dose"), String::from("grp")],
        false,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert_approx_eq(p_od.deviance_residual, 142.002_019_851_751_53, 1e-4, 1e-5, "OD Poisson res dev");
    assert_approx_eq(p_od.dispersion_pearson, 20.979_590_398_940_587, 1e-4, 1e-5, "OD Poisson Pearson disp");

    let mut mv = MultipleVariablesTable::new(4, 10).unwrap();
    mv.variables[2].lock_role(VariableRole::Categorical);
    for i in 0..10 {
        mv.variables[0].cells[i] = Cell::number(cnt[i]);
        mv.variables[1].cells[i] = Cell::number(dose[i]);
        mv.variables[2].cells[i] = Cell::text(if grp[i] > 0.5 { "B_Active" } else { "A_Ctrl" });
        mv.variables[3].cells[i] = Cell::number(exposure[i].ln());
    }
    let sid = lumencraft_doc::SheetId::new("mv-glm-edge");
    let mut ds = DataSheet::new(sid.clone(), "GLM Edge Table", TableArchetype::MultipleVariables(mv)).unwrap();
    ds.id = sid.clone();
    let glm_spec = GlmSpec {
        family: GlmFamily::Poisson,
        response_column: 0,
        predictor_columns: vec![1, 2, 3],
        reference_levels: vec![(2, String::from("A_Ctrl"))],
        compute_vif: true,
    };
    let aspec = AnalysisSpec::new("an-glm-edge", sid, AnalysisKind::GeneralizedLinearModel(glm_spec.clone())).unwrap();
    let rsheet = run_glm_analysis(&ds, &aspec, &glm_spec).unwrap();
    let tab = rsheet.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals).unwrap();
    assert_eq!(tab.rows.len(), 4);
    assert!(matches!(tab.rows[1][0], ResultCell::Number { value, .. } if (value - 0.466_838_521_497_392_4).abs() < 1e-5));
}

#[test]
fn edge_05_pca_horn_parallel_kmeans_and_ward_clustering_recovery() {
    // Provenance: R 4.5.2 (2025-10-31) `prcomp(..., scale.=TRUE/FALSE)` and `hclust(dist(pts), method="ward.D2")`.
    #[rustfmt::skip]
    let rows15 = [
        [2.5, 2.4, 1.0, 1.1], [0.5, 0.7, 3.0, 2.9], [2.2, 2.9, 1.2, 1.0], [1.9, 2.2, 0.9, 1.3],
        [3.1, 3.0, 0.5, 0.4], [2.3, 2.7, 1.4, 1.5], [2.0, 1.6, 1.1, 0.8], [1.0, 1.1, 2.8, 3.1],
        [1.5, 1.6, 2.2, 2.0], [1.1, 0.9, 2.5, 2.6], [0.4, 0.6, 3.4, 3.2], [2.8, 2.6, 0.7, 0.9],
        [0.7, 0.8, 3.1, 2.8], [2.4, 2.5, 1.3, 1.2], [1.2, 1.0, 2.7, 2.9],
    ];
    let mat15 = DenseMatrix::from_row_major(15, 4, rows15.iter().flatten().copied().collect()).unwrap();
    let vlabels = vec![String::from("V1"), String::from("V2"), String::from("V3"), String::from("V4")];

    let pca_cor = pca_analysis(&mat15, &vlabels, true, None).unwrap();
    let exp_cor_eig = [3.823_475_679_287_808_4, 0.114_357_770_445_117_95, 0.040_713_419_740_539_77, 0.021_453_130_526_533_856];
    let exp_cor_pc1_abs = [0.503_625_493_390_222_8, 0.492_759_486_997_539_46, 0.504_021_112_882_026_9, 0.499_511_929_938_154];
    for j in 0..4 {
        assert_approx_eq(pca_cor.eigenvalues[j], exp_cor_eig[j], 1e-8, 1e-8, &format!("PCA cor eig[{j}]"));
        assert_approx_eq(pca_cor.loadings[j][0].abs(), exp_cor_pc1_abs[j], 1e-7, 1e-7, &format!("PCA cor |PC1[{j}]|"));
    }
    assert_approx_eq(pca_cor.cumulative_proportion[3], 1.0, 1e-12, 1e-12, "PCA cum var == 1");
    assert_eq!(pca_cor.kaiser_components, 1);

    let pca_cov = pca_analysis(&mat15, &vlabels, false, None).unwrap();
    let exp_cov_eig = [3.288_697_695_920_445_7, 0.095_929_643_651_877_08, 0.032_177_713_931_883_04, 0.019_861_613_162_461_1];
    for (j, &exp_eig) in exp_cov_eig.iter().enumerate() {
        assert_approx_eq(pca_cov.eigenvalues[j], exp_eig, 1e-8, 1e-8, &format!("PCA cov eig[{j}]"));
    }

    #[rustfmt::skip]
    let rows30 = [
        [-2.725808, -2.708910, -3.273447,  0.078423, -0.098725, -0.108140],
        [-2.864054, -2.507918, -2.921270,  0.348392,  0.479964,  0.268410],
        [-2.437365, -2.282989, -2.604148,  1.100899,  1.264381,  1.366686],
        [-2.156383, -2.438830, -2.209806,  1.564926,  1.308263,  1.407641],
        [-1.992877, -2.025278, -2.413358,  1.113254,  1.526020,  1.412940],
        [-1.903973, -2.260653, -1.794659,  0.938340,  1.204147,  1.011135],
        [-1.404761, -1.849826, -1.782915,  0.429742,  0.716282,  0.764713],
        [-1.559844, -1.629269, -1.442065, -0.233113, -0.150121, -0.038790],
        [-0.973073, -1.706084, -1.266302, -0.552611, -0.602317, -0.662409],
        [-1.219244, -0.986074, -1.104002, -0.976770, -0.851014, -0.880715],
        [-0.762958, -0.734868, -1.248176, -1.215341, -1.124974, -1.552502],
        [-0.366662, -0.648220, -0.840378, -1.337126, -1.448246, -1.836148],
        [-0.883365, -0.242775, -0.471196, -1.560393, -1.202422, -1.398219],
        [-0.426620, -0.374479, -0.551998, -0.558755, -0.601976, -1.051171],
        [-0.151610, -0.348711, -0.229659, -0.462707, -0.006091, -0.229398],
        [ 0.252135,  0.161618,  0.237293,  0.350455,  0.074639,  0.633885],
        [ 0.314012,  0.066859,  0.514929,  0.844000,  0.803090,  0.780085],
        [ 0.074301,  0.683228,  0.688653,  1.335281,  1.442734,  1.521791],
        [ 0.335898,  0.489719,  0.645185,  1.583715,  1.255725,  1.638771],
        [ 1.287955,  0.907197,  0.819596,  1.479366,  1.228005,  1.696510],
        [ 1.145373,  1.057684,  1.550719,  1.102393,  0.869696,  1.092295],
        [ 1.020496,  1.066474,  1.502032,  0.590222,  0.179831,  0.723034],
        [ 1.506529,  1.774233,  1.667455, -0.155817, -0.343752, -0.470277],
        [ 1.950000,  1.821515,  1.825905, -0.756933, -0.844906, -0.428374],
        [ 2.261787,  1.935204,  1.816301, -1.442719, -1.372573, -1.085830],
        [ 1.987637,  2.181579,  2.390299, -1.411879, -1.303708, -1.536664],
        [ 2.231501,  2.452902,  2.446326, -1.389234, -1.603789, -1.752171],
        [ 2.157358,  2.507976,  2.683961, -0.430451, -1.326074, -1.004651],
        [ 2.843134,  2.050267,  3.144985, -0.716050, -0.634679, -0.482080],
        [ 2.872001,  2.856977,  3.364355,  0.227451, -0.411274,  0.098729],
    ];
    let mat30 = DenseMatrix::from_row_major(30, 6, rows30.iter().flatten().copied().collect()).unwrap();
    let vlabels6 =
        vec![String::from("F1_A"), String::from("F1_B"), String::from("F1_C"), String::from("F2_A"), String::from("F2_B"), String::from("F2_C")];
    let horn_a = pca_analysis(&mat30, &vlabels6, true, Some(2026)).unwrap();
    let horn_b = pca_analysis(&mat30, &vlabels6, true, Some(2026)).unwrap();
    assert_approx_eq(horn_a.eigenvalues[0], 3.951_078_860_999_725_6, 1e-4, 1e-4, "2comp eig0");
    assert_approx_eq(horn_a.eigenvalues[1], 1.967_558_523_118_268, 1e-4, 1e-4, "2comp eig1");
    assert_eq!(horn_a.kaiser_components, 2);
    assert_eq!(horn_a.parallel_analysis_components, Some(2));
    assert_eq!(horn_a.parallel_analysis_thresholds, horn_b.parallel_analysis_thresholds);

    let pts = vec![
        vec![0.0, 0.0],
        vec![0.2, 0.1],
        vec![-0.1, 0.2],
        vec![10.0, 0.0],
        vec![10.2, -0.1],
        vec![9.9, 0.2],
        vec![0.0, 10.0],
        vec![0.1, 10.2],
        vec![-0.2, 9.8],
    ];
    let km = kmeans_pp(&pts, 3, 7).unwrap();
    assert!(km.mean_silhouette > 0.95);
    assert!(km.assignments[0] == km.assignments[1] && km.assignments[1] == km.assignments[2]);
    assert!(km.assignments[3] == km.assignments[4] && km.assignments[4] == km.assignments[5]);
    assert!(km.assignments[6] == km.assignments[7] && km.assignments[7] == km.assignments[8]);
    assert!(km.assignments[0] != km.assignments[3] && km.assignments[0] != km.assignments[6] && km.assignments[3] != km.assignments[6]);

    let merges = hierarchical_clustering(&pts, LinkageMethod::WardD2).unwrap();
    assert_eq!(merges.len(), 8);
    let exp_ward = [
        0.223_606_797_749_978_35,
        0.223_606_797_749_978_35,
        0.223_606_797_749_979,
        0.288_675_134_594_812_9,
        0.369_684_550_213_646_7,
        0.450_924_975_282_288_53,
        17.147_691_778_584_466,
        22.435_215_374_247_89,
    ];
    for (step, (m, &exp_h)) in merges.iter().zip(&exp_ward).enumerate() {
        assert_approx_eq(m.distance, exp_h, 1e-8, 1e-8, &format!("Ward.D2 step {step}"));
    }
}

#[test]
fn edge_06a_extreme_unequal_n_welch_games_howell_and_dunnett_t3() {
    // Provenance: R 4.5.2 (2025-10-31) `oneway.test(var.equal=FALSE)`, exact bivariate Gauss-Kronrod
    // chi-scale Studentized range / SMM integrals (`integrate()`) on n=[3,45,7,60].
    // Note: in extreme low-df / large-q regime (`pair(0,3)`: df=2.462, q=44.277), R's fixed-panel
    // `ptukey()` under-predicts (4.5535e-4), whereas exact saddle-point quadrature gives 8.0426024e-4.
    let groups = unequal_n_groups();
    let uv = unequal_variance_one_way_anova(&groups).unwrap();
    assert_approx_eq(uv.welch_w, 996.604_050_374_671_3, 1e-6, 1e-7, "Welch F");
    assert_approx_eq(uv.welch_df1, 3.0, 1e-12, 1e-12, "Welch df1");
    assert_approx_eq(uv.welch_df2, 7.961_808_055_864_5, 1e-7, 1e-7, "Welch df2");
    assert_approx_eq(uv.welch_p, 1.367_115_649_907_929e-10, 1e-13, 1e-5, "Welch p");
    assert!(uv.brown_forsythe_f_star > 500.0 && uv.brown_forsythe_p < 1e-8);

    let gh = compare_groups_with_procedure(&groups, 0.0, 0.0, MultiplicityMethod::GamesHowell.into(), 0.05, true).unwrap();
    let exp_gh = [
        (-5.70, 0.708_284_312_029, 2.343_895_760_585, 8.047_615_771_229 * std::f64::consts::SQRT_2, 2.494_885_266_696_142e-02),
        (-11.30, 1.227_075_501_228, 7.860_349_469_621, 9.208_887_300_488 * std::f64::consts::SQRT_2, 8.107_326_400_088_916e-05),
        (-22.45, 0.717_054_158_810, 2.461_917_940_712, 31.308_653_222_594 * std::f64::consts::SQRT_2, 8.042_602_415_458_355e-04),
        (-5.60, 1.039_574_088_612, 6.448_222_548_068, 5.386_821_450_576 * std::f64::consts::SQRT_2, 5.506_947_540_820_726e-03),
        (-16.75, 0.298_607_881_119, 102.996_881_151_063, 56.093_630_004_688 * std::f64::consts::SQRT_2, 2.778_178_660_583_967e-78),
        (-11.15, 1.045_568_881_382, 6.597_872_099_383, 10.664_051_119_485 * std::f64::consts::SQRT_2, 9.209_989_747_804_312e-05),
    ];
    for (idx, (row, &(diff, se, df_w, q_exp, p_exp))) in gh.iter().zip(&exp_gh).enumerate() {
        assert_approx_eq(row.mean_diff, diff, 1e-9, 1e-9, &format!("GH[{idx}] diff"));
        assert_approx_eq(row.se_diff, se, 1e-8, 1e-8, &format!("GH[{idx}] se"));
        assert_approx_eq(row.df, df_w, 1e-8, 1e-8, &format!("GH[{idx}] df"));
        assert_approx_eq(row.statistic, q_exp, 1e-7, 1e-7, &format!("GH[{idx}] q"));
        assert_approx_eq(row.adjusted_p_or_q, p_exp, 1e-10, 1e-6, &format!("GH[{idx}] p"));
    }

    let t3 = compare_groups_with_procedure(&groups, 0.0, 0.0, MultiplicityMethod::DunnettT3.into(), 0.05, true).unwrap();
    let exp_p_t3 = [
        3.053_191_512_786_472e-02,
        9.693_204_300_020_34e-05,
        9.883_323_017_295_944e-04,
        6.820_958_622_630_774e-03,
        2.755_945_526_498_93e-80,
        1.121_145_897_378_736e-04,
    ];
    for (idx, (row, &p_exp)) in t3.iter().zip(&exp_p_t3).enumerate() {
        assert_approx_eq(row.adjusted_p_or_q, p_exp, 1e-10, 1e-6, &format!("T3[{idx}] p"));
        if p_exp > 1e-12 {
            assert!(row.adjusted_p_or_q > gh[idx].adjusted_p_or_q);
        }
        assert!(row.ci.is_some());
    }

    let t2 = compare_groups_with_procedure(&groups, 0.0, 0.0, MultiplicityMethod::TamhaneT2.into(), 0.05, true).unwrap();
    assert!(t2.iter().all(|r| r.adjusted_p_or_q.is_finite() && r.ci.is_some()));
}

#[test]
fn edge_06b_extreme_tied_rank_kruskal_wallis_and_friedman() {
    // Provenance: R 4.5.2 (2025-10-31) `kruskal.test` and `friedman.test` on dense tie matrices.
    let kw1 = [1.0, 1.0, 1.0, 2.0, 2.0];
    let kw2 = [1.0, 2.0, 2.0, 2.0, 3.0];
    let kw3 = [2.0, 2.0, 3.0, 3.0, 3.0];
    let kw4 = [3.0, 3.0, 3.0, 3.0, 2.0];
    let kw = kruskal_wallis_test(&[&kw1, &kw2, &kw3, &kw4]).unwrap();
    assert_approx_eq(kw.df, 3.0, 1e-12, 1e-12, "Tied KW df");
    assert_approx_eq(kw.tie_correction, 1.0 - 1068.0 / 7980.0, 1e-12, 1e-12, "Tied KW tie factor");
    assert_approx_eq(kw.h_statistic, 10.080_555_555_555_566, 1e-9, 1e-9, "Tied KW H");
    assert_approx_eq(kw.p_value, 1.789_364_980_525_992e-02, 1e-9, 1e-8, "Tied KW p");

    let blocks = vec![
        vec![1.0, 1.0, 2.0, 3.0],
        vec![2.0, 2.0, 2.0, 4.0],
        vec![1.0, 3.0, 3.0, 3.0],
        vec![2.0, 2.0, 4.0, 4.0],
        vec![1.0, 1.0, 1.0, 3.0],
        vec![2.0, 3.0, 3.0, 5.0],
    ];
    let fr = friedman_test(&blocks).unwrap();
    assert_approx_eq(fr.df, 3.0, 1e-12, 1e-12, "Tied Friedman df");
    assert_approx_eq(fr.q_statistic, 13.568_181_818_181_818, 1e-9, 1e-9, "Tied Friedman Q");
    assert_approx_eq(fr.p_value, 3.555_940_280_396_475e-03, 1e-9, 1e-8, "Tied Friedman p");
    assert_approx_eq(fr.kendalls_w, 0.753_787_878_787_879, 1e-9, 1e-9, "Tied Friedman Kendall W");
}
