//! D1 primary-literature and R 4.5.2 (`nlme` / `aov` / Fai–Cornelius 1996 / Kenward–Roger 1997)
//! reference oracle tests for R3.6 mixed-effects REML repeated measures and `MixedModelDfMethod`:
//! - Unbalanced & missing-cell 1-way RM ANOVA (`CompoundSymmetry` & `Unstructured`, KR vs Satterthwaite)
//! - Missing-cell 2-way RM ANOVA (`BothFactorsMatched`, KR vs Satterthwaite & contrasts)
//! - Balanced 3-way `2 × 2 × K` RM vs R `aov(... Error(S/(A*B*C)))` and missing-cell 3-way
//!   (`RowFactorMatched`, `ColumnFactorMatched`, `BothFactorsMatched`, KR vs Satterthwaite)
//! - Unbalanced nested *t*-test & nested 1-way ANOVA (`KenwardRoger` vs `Satterthwaite`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, FactorialAnovaSpec, GroupedTable, MixedModelDfMethod,
    MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod, RepeatedMeasuresDesign, ResidualCovariance,
    SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_stats::anova::{nested_one_way_anova_with_df_method, run_factorial_anova, run_nested_analysis, run_one_way_anova};
use curvebook_testkit::assert_approx_eq;

fn scalar_val(sheet: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key).expect("missing scalar").value
}

fn full_reml_loglik(sheet: &curvebook_doc::ResultSheet, n_minus_p: usize) -> f64 {
    scalar_val(sheet, "REML log-likelihood") - 0.5 * (n_minus_p as f64) * std::f64::consts::TAU.ln()
}

fn find_tab<'a>(sheet: &'a curvebook_doc::ResultSheet, id: &str) -> &'a curvebook_doc::ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).expect("missing tab")
}

fn build_3way_grouped(id: &str, n_rows: usize, n_subs: usize, entries: &[(usize, usize, usize, usize, f64)]) -> DataSheet {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(n_subs), 4, n_rows).unwrap();
    grp.row_factor_title = String::from("C");
    grp.column_factor_title = String::from("A_B");
    grp.three_way_grouping = Some(curvebook_doc::ThreeWayColumnGrouping {
        factor_1_name: String::from("A"),
        factor_1_levels: [String::from("A1"), String::from("A2")],
        factor_2_name: String::from("B"),
        factor_2_levels: [String::from("B1"), String::from("B2")],
    });
    for c in 0..4 {
        grp.data_sets[c].title = format!("A{}B{}", (c / 2) + 1, (c % 2) + 1);
    }
    for &(s, l1, l2, r, y) in entries {
        let c = l1 * 2 + l2;
        grp.data_sets[c].cells[r][s] = Cell::number(y);
    }
    DataSheet::new(SheetId::new(id), "ThreeWayRM", TableArchetype::Grouped(grp)).unwrap()
}

#[test]
fn r36_1a_one_way_rm_missing_cell_cs_and_un_kr_vs_satterthwaite() {
    // D1 Provenance: R 4.5.2 nlme::lme + nlme::gls + KR97/FC96 exact equations on 8 subjects x 4 conditions
    // with 4 missing cells (28 active observations retained, zero listwise deletion).
    let rows: [[Option<f64>; 4]; 8] = [
        [Some(10.2), Some(13.1), Some(17.8), Some(23.3)],
        [Some(12.5), None, Some(20.4), Some(23.4)],
        [Some(9.1), Some(12.5), None, Some(21.4)],
        [Some(14.8), Some(17.6), Some(22.7), None],
        [Some(11.0), Some(15.3), Some(19.3), Some(23.0)],
        [None, Some(16.3), Some(21.1), Some(26.8)],
        [Some(13.1), Some(16.5), Some(19.9), Some(25.1)],
        [Some(10.7), Some(13.7), Some(18.6), Some(22.6)],
    ];
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, 8).unwrap();
    for (c, lbl) in ["C1", "C2", "C3", "C4"].into_iter().enumerate() {
        tab.data_sets[c].title = String::from(lbl);
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, cell_val) in row.iter().enumerate() {
            if let Some(v) = cell_val {
                tab.data_sets[c].cells[r][0] = Cell::number(*v);
            }
        }
    }
    let sheet = DataSheet::new("col-rm-miss", "RM 1-way Missing", TableArchetype::Column(tab)).unwrap();

    // Compound Symmetry: Kenward-Roger vs Satterthwaite
    let cs_kr_spec = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        mixed_effects_reml: false,
        df_method: MixedModelDfMethod::KenwardRoger,
        residual_covariance: ResidualCovariance::CompoundSymmetry,
        compute_effect_sizes: true,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
    };
    let res_cs_kr =
        run_one_way_anova(&sheet, &AnalysisSpec::new("r-cs-kr", "col-rm-miss", AnalysisKind::OneWayAnova(cs_kr_spec.clone())).unwrap(), &cs_kr_spec)
            .unwrap();
    assert_approx_eq(scalar_val(&res_cs_kr, "REML sigma^2 (subject)"), 3.300_460_2, 1e-4, 1e-4, "CS s2_u");
    assert_approx_eq(scalar_val(&res_cs_kr, "REML sigma^2 (residual)"), 0.324_461_3, 1e-4, 1e-4, "CS s2_e");
    assert_approx_eq(full_reml_loglik(&res_cs_kr, 24), -38.315_895_215_860, 1e-4, 1e-4, "CS logL");
    assert_approx_eq(scalar_val(&res_cs_kr, "F (treatment)"), 581.752_888_868, 1e-2, 1e-4, "CS F_kr");
    assert_approx_eq(scalar_val(&res_cs_kr, "Denominator df"), 17.053_837_215_8, 1e-4, 1e-4, "CS nu_kr");
    assert_approx_eq(scalar_val(&res_cs_kr, "P value"), 2.285_118_176_84e-17, 1e-18, 1e-2, "CS p_kr");

    let cs_sat_spec = OneWayAnovaSpec { df_method: MixedModelDfMethod::Satterthwaite, ..cs_kr_spec.clone() };
    let res_cs_sat = run_one_way_anova(
        &sheet,
        &AnalysisSpec::new("r-cs-sat", "col-rm-miss", AnalysisKind::OneWayAnova(cs_sat_spec.clone())).unwrap(),
        &cs_sat_spec,
    )
    .unwrap();
    assert_approx_eq(scalar_val(&res_cs_sat, "F (treatment)"), 582.490_167_364, 1e-2, 1e-4, "CS F_sat");
    assert_approx_eq(scalar_val(&res_cs_sat, "Denominator df"), 17.053_837_215_8, 1e-4, 1e-4, "CS nu_sat");
    assert_approx_eq(scalar_val(&res_cs_sat, "P value"), 2.260_820_951_21e-17, 1e-18, 1e-2, "CS p_sat");

    // Check pairwise SE uses Phi_A under KenwardRoger vs Phi under Satterthwaite (C1 vs C2)
    let tab_kr = find_tab(&res_cs_kr, "multiple-comparisons");
    let tab_sat = find_tab(&res_cs_sat, "multiple-comparisons");
    let extract_num = |cell: &curvebook_doc::ResultCell| match cell {
        curvebook_doc::ResultCell::Number { value, .. } => *value,
        other => panic!("expected numeric cell, got {other:?}"),
    };
    assert_approx_eq(extract_num(&tab_kr.rows[0][0]), -3.152_531_750_124, 1e-4, 1e-4, "C1-C2 diff");
    assert_approx_eq(extract_num(&tab_kr.rows[0][1]), 0.311_941_300_358, 1e-4, 1e-4, "C1-C2 SE_kr");
    assert_approx_eq(extract_num(&tab_sat.rows[0][1]), 0.311_743_820_256, 1e-4, 1e-4, "C1-C2 SE_sat");
    assert!(extract_num(&tab_kr.rows[0][1]) > extract_num(&tab_sat.rows[0][1]), "KR Phi_A contrast SE exceeds unadjusted Satterthwaite SE");

    // Unstructured covariance REML: Kenward-Roger vs Satterthwaite
    let un_kr_spec = OneWayAnovaSpec { residual_covariance: ResidualCovariance::Unstructured, mixed_effects_reml: true, ..cs_kr_spec };
    let res_un_kr =
        run_one_way_anova(&sheet, &AnalysisSpec::new("r-un-kr", "col-rm-miss", AnalysisKind::OneWayAnova(un_kr_spec.clone())).unwrap(), &un_kr_spec)
            .unwrap();
    assert_approx_eq(full_reml_loglik(&res_un_kr, 24), -34.152_798_436_130, 5e-3, 1e-3, "UN logL");
    assert_approx_eq(scalar_val(&res_un_kr, "F (treatment)"), 812.949_026_884, 5.0, 1e-2, "UN F_kr");
    assert_approx_eq(scalar_val(&res_un_kr, "Denominator df"), 4.290_821_230_31, 5e-2, 2e-2, "UN nu_kr");
    assert_approx_eq(scalar_val(&res_un_kr, "P value"), 2.357_026_109_16e-6, 1e-6, 1e-1, "UN p_kr");

    let un_sat_spec = OneWayAnovaSpec { df_method: MixedModelDfMethod::Satterthwaite, ..un_kr_spec };
    let res_un_sat = run_one_way_anova(
        &sheet,
        &AnalysisSpec::new("r-un-sat", "col-rm-miss", AnalysisKind::OneWayAnova(un_sat_spec.clone())).unwrap(),
        &un_sat_spec,
    )
    .unwrap();
    assert_approx_eq(scalar_val(&res_un_sat, "F (treatment)"), 1_501.174_588_51, 10.0, 1e-2, "UN F_sat");
    assert_approx_eq(scalar_val(&res_un_sat, "Denominator df"), 6.166_466_330_75, 5e-2, 2e-2, "UN nu_sat");
    assert_approx_eq(scalar_val(&res_un_sat, "P value"), 3.264_001_962_56e-9, 1e-9, 2e-1, "UN p_sat");
    assert!(find_tab(&res_un_kr, "unstructured-reml-covariance").rows.len() == 4);
}

#[test]
fn r36_1b_one_way_rm_incomplete_blocks_without_complete_rows_and_dunnett_linear_trend() {
    // Balanced incomplete block design (0 complete rows, yet 3 obs per condition across 4 subjects = 9 obs).
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).unwrap();
    let bibs: [[Option<f64>; 3]; 4] =
        [[Some(10.0), Some(14.0), None], [Some(11.2), None, Some(19.5)], [None, Some(15.1), Some(20.2)], [Some(9.8), Some(13.9), None]];
    for (r, row) in bibs.iter().enumerate() {
        for (c, val) in row.iter().enumerate() {
            if let Some(v) = val {
                tab.data_sets[c].cells[r][0] = Cell::number(*v);
            }
        }
    }
    tab.data_sets[0].cells[4][0] = Cell::text("bad");
    let sheet = DataSheet::new("bib", "Incomplete Block RM", TableArchetype::Column(tab)).unwrap();

    for mult in [MultiplicityMethod::Dunnett { control_column: 0 }, MultiplicityMethod::LinearTrend, MultiplicityMethod::Sidak] {
        let ow = OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
            multiplicity: MultiplicitySpec { method: mult, threshold: 0.05, report_confidence_intervals: true },
            ..OneWayAnovaSpec::default()
        };
        let res = run_one_way_anova(&sheet, &AnalysisSpec::new("res-bib", "bib", AnalysisKind::OneWayAnova(ow.clone())).unwrap(), &ow).unwrap();
        assert!(scalar_val(&res, "F (treatment)") > 50.0);
        assert!(res.checklist.iter().any(|c| c.assumption == "Mixed-effects REML (repeated measures)"));
        assert!(res.checklist.iter().any(|c| c.assumption == "Numeric input validation"));
    }
}

#[test]
fn r36_2a_two_way_rm_missing_cell_kr_vs_satterthwaite_oracle() {
    // D1 Provenance: R 4.5.2 multi-VC REML + KR97/FC96 equations on missing-cell 2x3 BothFactorsMatched.
    let entries: [(usize, usize, usize, f64); 27] = [
        (0, 0, 0, 9.9659),
        (0, 0, 1, 14.0083),
        (0, 0, 2, 13.8722),
        (0, 1, 0, 10.4537),
        (0, 1, 1, 16.3022),
        (1, 0, 0, 8.8173),
        (1, 0, 1, 11.3453),
        (1, 0, 2, 13.1113),
        (1, 1, 0, 12.6787),
        (1, 1, 1, 17.4580),
        (1, 1, 2, 20.5969),
        (2, 0, 0, 12.3430),
        (2, 0, 2, 16.2707),
        (2, 1, 0, 15.2863),
        (2, 1, 1, 18.8319),
        (2, 1, 2, 22.0324),
        (3, 0, 0, 11.4375),
        (3, 0, 1, 12.0762),
        (3, 0, 2, 14.6184),
        (3, 1, 0, 13.9670),
        (3, 1, 1, 16.1428),
        (3, 1, 2, 19.4481),
        (4, 0, 0, 8.0685),
        (4, 0, 1, 11.0429),
        (4, 0, 2, 12.0059),
        (4, 1, 1, 15.3861),
        (4, 1, 2, 17.7235),
    ];
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(5), 3, 2).unwrap();
    grp.row_factor_title = String::from("R");
    grp.column_factor_title = String::from("C");
    for &(s, r, c, y) in &entries {
        grp.data_sets[c].cells[r][s] = Cell::number(y);
    }
    let sheet = DataSheet::new("g2-miss", "2way RM Missing", TableArchetype::Grouped(grp)).unwrap();

    let kr_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        df_method: MixedModelDfMethod::KenwardRoger,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
        ..FactorialAnovaSpec::default()
    };
    let res_kr =
        run_factorial_anova(&sheet, &AnalysisSpec::new("r2-kr", "g2-miss", AnalysisKind::FactorialAnova(kr_spec.clone())).unwrap(), &kr_spec)
            .unwrap();
    assert_approx_eq(full_reml_loglik(&res_kr, 21), -36.702_826_105_5, 1e-3, 1e-4, "2way ll");
    assert_approx_eq(scalar_val(&res_kr, "R: F"), 46.700_229_028_1, 5e-2, 2e-3, "2way R F_kr");
    assert_approx_eq(scalar_val(&res_kr, "R: Kenward-Roger df"), 3.986_169_249_5, 1e-2, 2e-3, "2way R nu_kr");
    assert_approx_eq(scalar_val(&res_kr, "C: F"), 67.661_882_535_8, 5e-2, 2e-3, "2way C F_kr");
    assert_approx_eq(scalar_val(&res_kr, "C: Kenward-Roger df"), 7.979_950_762_8, 1e-2, 2e-3, "2way C nu_kr");
    assert_approx_eq(scalar_val(&res_kr, "R × C: F"), 80.706_295_074_0, 5e-2, 2e-3, "2way RC F_kr");
    assert_approx_eq(scalar_val(&res_kr, "R × C: Kenward-Roger df"), 5.201_208_068_1, 1e-2, 2e-3, "2way RC nu_kr");

    let sat_spec = FactorialAnovaSpec { df_method: MixedModelDfMethod::Satterthwaite, ..kr_spec };
    let res_sat =
        run_factorial_anova(&sheet, &AnalysisSpec::new("r2-sat", "g2-miss", AnalysisKind::FactorialAnova(sat_spec.clone())).unwrap(), &sat_spec)
            .unwrap();
    assert_approx_eq(scalar_val(&res_sat, "R: F"), 46.712_646_549_3, 5e-2, 2e-3, "2way R F_sat");
    assert_approx_eq(scalar_val(&res_sat, "C: F"), 67.760_929_325_5, 5e-2, 2e-3, "2way C F_sat");
    assert_approx_eq(scalar_val(&res_sat, "R × C: F"), 82.590_502_408_8, 5e-2, 2e-3, "2way RC F_sat");
    assert_approx_eq(scalar_val(&res_sat, "R × C: P"), 0.000_114_617_3, 1e-5, 5e-3, "2way RC p_sat");
}

#[test]
fn r36_3a_three_way_rm_balanced_matches_aov_and_missing_cell_reml_oracles() {
    // 1. Balanced 2x2x3 BothFactorsMatched vs R `aov(y ~ A * B * C + Error(Sf / (A * B * C)))`.
    let bm_missing: [(usize, usize, usize, usize, f64); 57] = [
        (0, 0, 0, 0, 16.9200),
        (0, 0, 0, 1, 18.3602),
        (0, 0, 0, 2, 23.6879),
        (0, 0, 1, 0, 11.3935),
        (0, 0, 1, 1, 12.9154),
        (0, 0, 1, 2, 14.6740),
        (0, 1, 0, 0, 21.1499),
        (0, 1, 0, 1, 24.9756),
        (0, 1, 1, 0, 18.6939),
        (0, 1, 1, 1, 22.7535),
        (0, 1, 1, 2, 28.1073),
        (1, 0, 0, 0, 15.6432),
        (1, 0, 0, 1, 18.4454),
        (1, 0, 0, 2, 25.1337),
        (1, 0, 1, 0, 10.6218),
        (1, 0, 1, 1, 11.8531),
        (1, 0, 1, 2, 16.8192),
        (1, 1, 0, 0, 18.9141),
        (1, 1, 0, 1, 25.4360),
        (1, 1, 0, 2, 32.0080),
        (1, 1, 1, 0, 18.3193),
        (1, 1, 1, 1, 23.0788),
        (1, 1, 1, 2, 28.5433),
        (2, 0, 0, 0, 20.5139),
        (2, 0, 0, 1, 21.2541),
        (2, 0, 0, 2, 26.8844),
        (2, 0, 1, 0, 14.0286),
        (2, 0, 1, 2, 18.1492),
        (2, 1, 0, 0, 23.6995),
        (2, 1, 0, 1, 25.9707),
        (2, 1, 0, 2, 31.1525),
        (2, 1, 1, 0, 19.3956),
        (2, 1, 1, 1, 20.0019),
        (2, 1, 1, 2, 28.6880),
        (3, 0, 0, 0, 12.9297),
        (3, 0, 0, 1, 14.8166),
        (3, 0, 0, 2, 16.6393),
        (3, 0, 1, 0, 9.8128),
        (3, 0, 1, 1, 11.3597),
        (3, 0, 1, 2, 12.2192),
        (3, 1, 0, 0, 20.4947),
        (3, 1, 0, 1, 24.8722),
        (3, 1, 0, 2, 28.4321),
        (3, 1, 1, 0, 16.6024),
        (3, 1, 1, 1, 23.0644),
        (3, 1, 1, 2, 26.9585),
        (4, 0, 0, 0, 11.7466),
        (4, 0, 0, 1, 13.9697),
        (4, 0, 0, 2, 16.7769),
        (4, 0, 1, 0, 11.1308),
        (4, 0, 1, 1, 13.2638),
        (4, 0, 1, 2, 13.9137),
        (4, 1, 0, 0, 16.1815),
        (4, 1, 0, 1, 22.9875),
        (4, 1, 0, 2, 27.3009),
        (4, 1, 1, 1, 24.3865),
        (4, 1, 1, 2, 27.9729),
    ];
    let mut bal_all = bm_missing.to_vec();
    bal_all.extend_from_slice(&[(0, 1, 0, 2, 33.3716), (2, 0, 1, 1, 11.1727), (4, 1, 1, 0, 16.8932)]);
    let sheet_bal = build_3way_grouped("g3-bal", 3, 5, &bal_all);
    let fac_bal = FactorialAnovaSpec {
        three_way: true,
        max_interaction_order: 3,
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        df_method: MixedModelDfMethod::KenwardRoger,
        ..FactorialAnovaSpec::default()
    };
    let res_bal =
        run_factorial_anova(&sheet_bal, &AnalysisSpec::new("r3-bal", "g3-bal", AnalysisKind::FactorialAnova(fac_bal.clone())).unwrap(), &fac_bal)
            .unwrap();
    // Verify mixed-effects REML ran (not OLS!) and matches R aov(y ~ A*B*C + Error(S/(A*B*C))) F-ratios + df2.
    assert!(res_bal.checklist.iter().any(|c| c.assumption == "Three-way repeated-measures mixed-effects REML"));
    assert_approx_eq(scalar_val(&res_bal, "A: F"), 142.8, 1.0, 1e-2, "3way bal aov A F");
    assert_approx_eq(scalar_val(&res_bal, "B: F"), 13.39, 0.2, 2e-2, "3way bal aov B F");
    assert_approx_eq(scalar_val(&res_bal, "C: F"), 53.68, 0.5, 1e-2, "3way bal aov C F");
    assert_approx_eq(scalar_val(&res_bal, "A × B: F"), 27.34, 0.3, 1.5e-2, "3way bal aov AB F");
    assert_approx_eq(scalar_val(&res_bal, "A × C: F"), 37.49, 0.4, 1.5e-2, "3way bal aov AC F");
    assert_approx_eq(scalar_val(&res_bal, "B × C: F"), 1.58, 0.05, 3e-2, "3way bal aov BC F");
    assert_approx_eq(scalar_val(&res_bal, "A × B × C: F"), 8.169, 0.15, 2e-2, "3way bal aov ABC F");
    assert_approx_eq(scalar_val(&res_bal, "A: Kenward-Roger df"), 4.0, 0.05, 2e-2, "3way bal A df2");
    assert_approx_eq(scalar_val(&res_bal, "A × B × C: Kenward-Roger df"), 8.0, 0.1, 2e-2, "3way bal ABC df2");

    // 2. Missing-cell 3-way BothFactorsMatched (KR vs Satterthwaite)
    let sheet_bm = build_3way_grouped("g3-bm", 3, 5, &bm_missing);
    let res_bm_kr =
        run_factorial_anova(&sheet_bm, &AnalysisSpec::new("r3-bm-kr", "g3-bm", AnalysisKind::FactorialAnova(fac_bal.clone())).unwrap(), &fac_bal)
            .unwrap();
    let fac_bm_sat = FactorialAnovaSpec { df_method: MixedModelDfMethod::Satterthwaite, ..fac_bal };
    let res_bm_sat = run_factorial_anova(
        &sheet_bm,
        &AnalysisSpec::new("r3-bm-sat", "g3-bm", AnalysisKind::FactorialAnova(fac_bm_sat.clone())).unwrap(),
        &fac_bm_sat,
    )
    .unwrap();
    assert_approx_eq(full_reml_loglik(&res_bm_kr, 45), -103.397_091_838_6, 5e-2, 1e-3, "3way BM miss logL");
    assert_approx_eq(scalar_val(&res_bm_kr, "A: F"), 117.587_643_836_9, 0.5, 5e-3, "3way BM A F_kr");
    assert_approx_eq(scalar_val(&res_bm_sat, "A: F"), 117.949_434_951_3, 0.5, 5e-3, "3way BM A F_sat");
    assert_approx_eq(scalar_val(&res_bm_kr, "A × B × C: F"), 5.430_849_430_6, 0.1, 2e-2, "3way BM ABC F_kr");
    assert_approx_eq(scalar_val(&res_bm_sat, "A × B × C: F"), 5.870_874_720_6, 0.1, 2e-2, "3way BM ABC F_sat");
    assert_approx_eq(scalar_val(&res_bm_kr, "A × B × C: Kenward-Roger df"), 6.838_687_054_0, 0.1, 2e-2, "3way BM ABC nu_kr");
    assert!(scalar_val(&res_bm_kr, "A × B × C: F") < scalar_val(&res_bm_sat, "A × B × C: F"));

    // 3. Missing-cell 3-way RowFactorMatched (nlme::lme 1-RI)
    let row_entries: [(usize, usize, usize, usize, f64); 46] = [
        (0, 0, 0, 0, 23.0321),
        (0, 0, 0, 1, 26.1816),
        (0, 0, 0, 2, 28.8704),
        (0, 0, 1, 0, 17.9364),
        (0, 0, 1, 1, 21.5687),
        (0, 0, 1, 2, 22.9444),
        (0, 1, 0, 0, 29.0022),
        (0, 1, 0, 1, 34.4595),
        (0, 1, 0, 2, 39.6467),
        (0, 1, 1, 0, 15.8327),
        (0, 1, 1, 1, 21.2345),
        (0, 1, 1, 2, 27.0730),
        (1, 0, 0, 0, 17.4298),
        (1, 0, 0, 1, 20.0810),
        (1, 0, 0, 2, 25.2134),
        (1, 0, 1, 0, 14.9865),
        (1, 0, 1, 1, 18.6131),
        (1, 1, 0, 0, 28.3633),
        (1, 1, 0, 1, 31.2354),
        (1, 1, 0, 2, 35.8566),
        (1, 1, 1, 0, 15.4142),
        (1, 1, 1, 1, 19.9441),
        (1, 1, 1, 2, 26.0109),
        (2, 0, 0, 0, 22.4867),
        (2, 0, 0, 1, 23.8358),
        (2, 0, 0, 2, 27.1156),
        (2, 0, 1, 0, 12.1584),
        (2, 0, 1, 1, 14.6118),
        (2, 0, 1, 2, 16.0175),
        (2, 1, 0, 0, 24.2345),
        (2, 1, 0, 1, 28.3376),
        (2, 1, 0, 2, 33.6210),
        (2, 1, 1, 0, 16.7254),
        (2, 1, 1, 1, 23.2862),
        (2, 1, 1, 2, 27.7153),
        (3, 0, 0, 0, 23.3082),
        (3, 0, 0, 1, 26.5692),
        (3, 0, 0, 2, 28.8356),
        (3, 0, 1, 0, 16.1434),
        (3, 0, 1, 1, 17.9904),
        (3, 0, 1, 2, 20.1032),
        (3, 1, 0, 1, 29.6669),
        (3, 1, 0, 2, 33.7543),
        (3, 1, 1, 0, 12.2167),
        (3, 1, 1, 1, 17.0602),
        (3, 1, 1, 2, 22.7469),
    ];
    let sheet_row3 = build_3way_grouped("g3-row", 3, 4, &row_entries);
    let fac_row3_kr = FactorialAnovaSpec {
        three_way: true,
        max_interaction_order: 3,
        repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
        df_method: MixedModelDfMethod::KenwardRoger,
        ..FactorialAnovaSpec::default()
    };
    let res_r3_kr = run_factorial_anova(
        &sheet_row3,
        &AnalysisSpec::new("r3-row-kr", "g3-row", AnalysisKind::FactorialAnova(fac_row3_kr.clone())).unwrap(),
        &fac_row3_kr,
    )
    .unwrap();
    assert_approx_eq(scalar_val(&res_r3_kr, "REML sigma^2 (subject)"), 6.049_066_5, 1e-3, 1e-4, "3way RowMatched s2_u");
    assert_approx_eq(scalar_val(&res_r3_kr, "REML sigma^2 (residual)"), 0.458_338_8, 1e-4, 1e-4, "3way RowMatched s2_e");
    assert_approx_eq(full_reml_loglik(&res_r3_kr, 34), -77.552_128_821_380, 1e-3, 1e-4, "3way RowMatched ll");
    assert_approx_eq(scalar_val(&res_r3_kr, "C: F"), 443.047_922_680_8, 5e-2, 2e-4, "3way RowMatched C F_kr");
    assert_approx_eq(scalar_val(&res_r3_kr, "C: Kenward-Roger df"), 22.040_346_094_8, 5e-3, 2e-4, "3way RowMatched C nu_kr");
    assert_approx_eq(scalar_val(&res_r3_kr, "A × B × C: F"), 5.969_181_579_7, 5e-3, 5e-4, "3way RowMatched ABC F_kr");

    let fac_row3_sat = FactorialAnovaSpec { df_method: MixedModelDfMethod::Satterthwaite, ..fac_row3_kr };
    let res_r3_sat = run_factorial_anova(
        &sheet_row3,
        &AnalysisSpec::new("r3-row-sat", "g3-row", AnalysisKind::FactorialAnova(fac_row3_sat.clone())).unwrap(),
        &fac_row3_sat,
    )
    .unwrap();
    assert_approx_eq(scalar_val(&res_r3_sat, "C: F"), 443.350_654_817_5, 5e-2, 2e-4, "3way RowMatched C F_sat");
    assert_approx_eq(scalar_val(&res_r3_sat, "A × B × C: F"), 5.973_029_066_8, 5e-3, 5e-4, "3way RowMatched ABC F_sat");

    // 4. Missing-cell 3-way ColumnFactorMatched (C between, A & B within: 4-stratum REML)
    let col_entries: [(usize, usize, usize, usize, f64); 46] = [
        (0, 0, 0, 0, 17.0093),
        (0, 0, 1, 0, 14.6233),
        (0, 1, 0, 0, 20.9341),
        (0, 1, 1, 0, 19.1894),
        (0, 0, 0, 1, 20.8985),
        (0, 0, 1, 1, 18.3574),
        (0, 1, 0, 1, 24.7166),
        (0, 0, 0, 2, 17.5348),
        (0, 0, 1, 2, 14.2037),
        (0, 1, 0, 2, 22.1836),
        (0, 1, 1, 2, 19.7769),
        (1, 0, 0, 0, 21.2883),
        (1, 0, 1, 0, 18.9603),
        (1, 1, 0, 0, 23.2783),
        (1, 1, 1, 0, 20.5139),
        (1, 0, 0, 1, 22.3124),
        (1, 0, 1, 1, 19.3157),
        (1, 1, 0, 1, 25.8352),
        (1, 1, 1, 1, 24.5702),
        (1, 0, 0, 2, 24.3353),
        (1, 0, 1, 2, 21.3751),
        (1, 1, 0, 2, 30.9468),
        (1, 1, 1, 2, 29.2637),
        (2, 0, 0, 0, 19.8226),
        (2, 0, 1, 0, 19.7749),
        (2, 1, 0, 0, 24.8286),
        (2, 1, 1, 0, 24.4400),
        (2, 0, 0, 1, 18.8383),
        (2, 0, 1, 1, 16.1727),
        (2, 1, 0, 1, 23.8787),
        (2, 1, 1, 1, 20.7195),
        (2, 0, 0, 2, 17.6145),
        (2, 1, 0, 2, 21.2005),
        (2, 1, 1, 2, 20.4334),
        (3, 0, 0, 0, 17.2483),
        (3, 0, 1, 0, 16.3552),
        (3, 1, 0, 0, 21.1269),
        (3, 1, 1, 0, 19.8088),
        (3, 0, 0, 1, 21.2226),
        (3, 0, 1, 1, 18.5399),
        (3, 1, 0, 1, 25.4615),
        (3, 1, 1, 1, 23.7585),
        (3, 0, 0, 2, 25.8465),
        (3, 0, 1, 2, 22.4818),
        (3, 1, 0, 2, 30.5300),
        (3, 1, 1, 2, 28.9179),
    ];
    let sheet_col3 = build_3way_grouped("g3-col", 3, 4, &col_entries);
    let fac_col3_kr = FactorialAnovaSpec {
        three_way: true,
        max_interaction_order: 3,
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        df_method: MixedModelDfMethod::KenwardRoger,
        ..FactorialAnovaSpec::default()
    };
    let res_c3_kr = run_factorial_anova(
        &sheet_col3,
        &AnalysisSpec::new("r3-col-kr", "g3-col", AnalysisKind::FactorialAnova(fac_col3_kr.clone())).unwrap(),
        &fac_col3_kr,
    )
    .unwrap();
    let fac_col3_sat = FactorialAnovaSpec { df_method: MixedModelDfMethod::Satterthwaite, ..fac_col3_kr };
    let res_c3_sat = run_factorial_anova(
        &sheet_col3,
        &AnalysisSpec::new("r3-col-sat", "g3-col", AnalysisKind::FactorialAnova(fac_col3_sat.clone())).unwrap(),
        &fac_col3_sat,
    )
    .unwrap();
    assert_approx_eq(full_reml_loglik(&res_c3_kr, 34), -74.452_993_542_4, 1e-2, 1e-3, "3way ColMatched ll");
    assert_approx_eq(scalar_val(&res_c3_kr, "A: F"), 229.295_783_810_4, 0.5, 2e-3, "3way ColMatched A F_kr");
    assert_approx_eq(scalar_val(&res_c3_sat, "A: F"), 230.346_425_330_9, 0.5, 2e-3, "3way ColMatched A F_sat");
    assert_approx_eq(scalar_val(&res_c3_kr, "A × B: Kenward-Roger df"), 7.761_397_416_0, 2e-2, 2e-3, "3way ColMatched AB nu_kr");
    assert_approx_eq(scalar_val(&res_c3_kr, "A × B × C: F"), 4.354_847_308_6, 2e-2, 3e-3, "3way ColMatched ABC F_kr");
    assert_approx_eq(scalar_val(&res_c3_sat, "A × B × C: F"), 4.453_185_150_4, 2e-2, 3e-3, "3way ColMatched ABC F_sat");
    assert!(res_c3_kr.pairwise_comparisons.is_empty()); // Default multiplicity is None -> verify post-hoc too!

    let fac_col3_post = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
        ..FactorialAnovaSpec {
            three_way: true,
            max_interaction_order: 3,
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            ..FactorialAnovaSpec::default()
        }
    };
    let res_c3_post = run_factorial_anova(
        &sheet_col3,
        &AnalysisSpec::new("r3-col-post", "g3-col", AnalysisKind::FactorialAnova(fac_col3_post.clone())).unwrap(),
        &fac_col3_post,
    )
    .unwrap();
    assert!(!res_c3_post.pairwise_comparisons.is_empty(), "3-way REML Bonferroni comparisons should be generated");
    assert!(find_tab(&res_c3_post, "multiple-comparisons-factor-3").rows.len() == 3);
}

#[test]
fn r36_4a_nested_t_test_and_one_way_kr_vs_satterthwaite_oracles() {
    // 1. Unbalanced 2-group Nested t-test
    let mut nt2 = NestedTable::new(2, 4, 5).unwrap();
    let d2: [&[&[f64]]; 2] = [
        &[&[10.1, 10.5, 9.8], &[13.2, 13.6, 12.9, 13.4], &[8.6, 8.9], &[15.0, 15.3, 14.8, 15.1, 14.9]],
        &[&[17.2, 17.5, 16.9, 17.4], &[20.1, 20.4], &[16.0, 16.3, 15.8]],
    ];
    for (g, cl) in d2.iter().enumerate() {
        nt2.groups[g].title = format!("G{}", g + 1);
        for (b, reps) in cl.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                nt2.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    let kr_2 = nested_one_way_anova_with_df_method(&nt2, true, MixedModelDfMethod::KenwardRoger).unwrap();
    let sat_2 = nested_one_way_anova_with_df_method(&nt2, true, MixedModelDfMethod::Satterthwaite).unwrap();
    assert_approx_eq(kr_2.variance_subclusters, 6.782_804_22, 1e-4, 1e-4, "Nested t s2_u");
    assert_approx_eq(kr_2.variance_error, 0.068_056_99, 1e-5, 1e-4, "Nested t s2_e");
    assert_approx_eq(kr_2.f_among_groups, 9.206_036_723_618_7, 1e-3, 1e-4, "Nested t F_kr");
    assert_approx_eq(sat_2.f_among_groups, 9.206_048_042_097_0, 1e-3, 1e-4, "Nested t F_sat");
    assert_approx_eq(kr_2.p_among_groups, 0.028_938_818_347_4, 1e-5, 1e-3, "Nested t p_kr");
    assert_approx_eq(sat_2.p_among_groups, 0.028_938_756_611_3, 1e-5, 1e-3, "Nested t p_sat");
    assert_approx_eq(kr_2.df2_groups_kenward_roger, 5.000_678_651_584_8, 1e-4, 1e-4, "Nested t nu_kr");
    assert_approx_eq(sat_2.df2_groups_satterthwaite, 5.000_678_651_584_8, 1e-4, 1e-4, "Nested t nu_sat");
    assert!(kr_2.f_among_groups < sat_2.f_among_groups, "KR Phi_A scales F below unadjusted Satterthwaite F");

    // Verify run_nested_analysis propagates df_method to F, P, Nested t, and Group denominator df
    let s_nt2 = DataSheet::new("nt2", "Nested t", TableArchetype::Nested(nt2)).unwrap();
    let res_n2_sat = run_nested_analysis(
        &s_nt2,
        &AnalysisSpec::new(
            "rn2-sat",
            "nt2",
            AnalysisKind::NestedComparison(NestedAnalysisSpec {
                two_group_t_test: true,
                df_method: MixedModelDfMethod::Satterthwaite,
                multiplicity: MultiplicitySpec::default(),
            }),
        )
        .unwrap(),
        &NestedAnalysisSpec { two_group_t_test: true, df_method: MixedModelDfMethod::Satterthwaite, multiplicity: MultiplicitySpec::default() },
    )
    .unwrap();
    assert_approx_eq(scalar_val(&res_n2_sat, "F (among groups)"), sat_2.f_among_groups, 1e-9, 1e-9, "run_nested_analysis Sat F");
    assert_approx_eq(scalar_val(&res_n2_sat, "Nested t"), sat_2.t_nested.unwrap(), 1e-9, 1e-9, "run_nested_analysis Sat t");

    // 2. Unbalanced 3-group Nested 1-way ANOVA
    let mut nt3 = NestedTable::new(3, 5, 5).unwrap();
    let d3: [&[&[f64]]; 3] = [
        &[&[10.1, 10.3, 9.8], &[14.2, 14.5, 13.9, 14.1], &[8.4, 8.6], &[19.1, 19.3, 18.8, 19.0, 19.2]],
        &[&[15.2, 15.5, 14.9, 15.1], &[18.1, 18.4, 17.9], &[12.3, 12.6, 12.1]],
        &[&[22.1, 22.4], &[26.2, 26.5, 25.9, 26.1], &[19.5, 19.8, 19.2], &[31.0, 31.3, 30.8], &[24.0, 24.2, 23.9, 24.1]],
    ];
    for (g, cl) in d3.iter().enumerate() {
        nt3.groups[g].title = format!("U{}", g + 1);
        for (b, reps) in cl.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                nt3.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    let kr_3 = nested_one_way_anova_with_df_method(&nt3, false, MixedModelDfMethod::KenwardRoger).unwrap();
    let sat_3 = nested_one_way_anova_with_df_method(&nt3, false, MixedModelDfMethod::Satterthwaite).unwrap();
    assert_approx_eq(kr_3.variance_subclusters, 17.728_777_49, 1e-3, 1e-4, "Nested 3grp s2_u");
    assert_approx_eq(kr_3.variance_error, 0.054_006_13, 1e-4, 1e-4, "Nested 3grp s2_e");
    assert_approx_eq(kr_3.f_among_groups, 9.615_621_346_361_93, 1e-3, 1e-4, "Nested 3grp F_kr");
    assert_approx_eq(sat_3.f_among_groups, 9.615_621_834_891_68, 1e-3, 1e-4, "Nested 3grp F_sat");
    assert_approx_eq(kr_3.df2_groups_kenward_roger, 8.999_766_557_901_31, 1e-3, 1e-4, "Nested 3grp nu_kr");
    assert_approx_eq(sat_3.df2_groups_satterthwaite, 8.999_766_562_454_09, 1e-3, 1e-4, "Nested 3grp nu_sat");
    assert_approx_eq(kr_3.p_among_groups, 0.005_832_164_062_51, 1e-5, 1e-3, "Nested 3grp p_kr");
}
