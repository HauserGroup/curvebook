#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Round 2 (`R0.19`) Slice `S02` (`crates/stats/*`) regression and mathematical / R 4.5.2 oracle tests.
//!
//! Covers all 15 surviving findings (`R2-S02-01`..`08`, `R2-S02-10`..`16`) plus `R2-S02-09` sanity check.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, ContingencyMethod,
    ContingencySpec, ContingencyTable, CoxTieMethod, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable,
    KappaWeighting, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, PartsOfWholeTable, RepeatedMeasuresDesign,
    ResultCell, ResultSheet, ResultTab, SheetId, SubcolumnMode, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, TwoGroupSpec,
    VariableRole, XyTable,
};
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::categorical::cohens_kappa;
use curvebook_stats::error::StatsError;
use curvebook_stats::multivariate::{
    MAX_CLUSTERING_N, cophenetic_correlation, cophenetic_distance_matrix, cut_dendrogram, hierarchical_clustering_with_metrics,
    kmeans_pp_with_distance, pairwise_distance_matrix,
};
use curvebook_stats::power::{
    OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode, TwoProportionsMethod, compute_power_curve,
    compute_sensitivity_table, solve_power,
};
use curvebook_stats::run_analysis;
use curvebook_stats::two_group::paired_sign_test;

fn assert_close(actual: f64, expected: f64, tol: f64, ctx: &str) {
    assert!((actual - expected).abs() <= tol, "{ctx}: expected {expected} +/- {tol}, got {actual}");
}

fn scalar(sheet: &ResultSheet, key: &str, ds: Option<usize>) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index == ds).unwrap_or_else(|| panic!("missing scalar {key:?} ds={ds:?}")).value
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected Number cell, got {other:?}"),
    }
}

fn cell_p(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => panic!("expected PValue cell, got {other:?}"),
    }
}

fn cell_bool(cell: &ResultCell) -> bool {
    match cell {
        ResultCell::Boolean(b) => *b,
        other => panic!("expected Boolean cell, got {other:?}"),
    }
}

// ============================================================================
// R2-S02-01: Factorial & RM ANOVA Post-Hoc LS-Means, Strata, Multiplicity & IDs
// ============================================================================

#[test]
fn r2_s02_01_unbalanced_2x2_ls_means_and_games_howell() {
    // Unbalanced 2x2 with zero true Factor A effect and large Factor B main effect (+20).
    // R oracle: mse = 1.5, df = 16, diff_A = 0.0, se_A = 0.912870929175278, p_A = 1.0.
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(9), 2, 2).unwrap();
    gt.row_factor_title = String::from("Factor A");
    gt.column_factor_title = String::from("Factor B");
    gt.row_titles = vec![String::from("A1"), String::from("A2")];
    gt.data_sets[0].title = String::from("B1");
    gt.data_sets[1].title = String::from("B2");

    let c11 = [8.0, 9.0, 10.0, 11.0, 12.0, 9.0, 10.0, 11.0, 10.0];
    let c12 = [30.0];
    let c21 = [10.0];
    let c22 = [28.0, 29.0, 30.0, 31.0, 32.0, 29.0, 30.0, 31.0, 30.0];
    for (s, &v) in c11.iter().enumerate() {
        gt.data_sets[0].cells[0][s] = Cell::number(v);
    }
    gt.data_sets[1].cells[0][0] = Cell::number(c12[0]);
    gt.data_sets[0].cells[1][0] = Cell::number(c21[0]);
    for (s, &v) in c22.iter().enumerate() {
        gt.data_sets[1].cells[1][s] = Cell::number(v);
    }
    let sheet = DataSheet::new(SheetId::new("g-unbal"), "Unbal2x2", TableArchetype::Grouped(gt)).unwrap();

    // 1. Equal-variance Tukey HSD tests unweighted Type III marginal LS-means (diff = 0, not -16).
    let spec_tukey = AnalysisSpec::new(
        SheetId::new("r-u1"),
        SheetId::new("g-unbal"),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ss_type: SumOfSquaresType::TypeIII,
            ..Default::default()
        }),
    )
    .unwrap();
    let res_tukey = run_analysis(&sheet, &spec_tukey).unwrap();
    let tab_a = tab_by_id(&res_tukey, "multiple-comparisons-factor-a");
    assert_eq!(tab_a.rows.len(), 1);
    assert_close(cell_num(&tab_a.rows[0][0]), 0.0, 1e-12, "Unbalanced A LS-mean diff");
    assert_close(cell_num(&tab_a.rows[0][1]), 0.912_870_929_175_278, 1e-10, "Unbalanced A LS-mean SE");
    assert_close(cell_num(&tab_a.rows[0][3]), 16.0, 1e-12, "Unbalanced A df");
    assert_close(cell_p(&tab_a.rows[0][4]), 1.0, 1e-12, "Unbalanced A raw p");
    assert!(!cell_bool(&tab_a.rows[0][7]));

    // 2. Unequal-variance Games-Howell uses cell-level variances (not Factor-B-inflated raw SDs).
    let spec_gh = AnalysisSpec::new(
        SheetId::new("r-u2"),
        SheetId::new("g-unbal"),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::GamesHowell, threshold: 0.05, report_confidence_intervals: true },
            ss_type: SumOfSquaresType::TypeIII,
            ..Default::default()
        }),
    )
    .unwrap();
    let res_gh = run_analysis(&sheet, &spec_gh).unwrap();
    let tab_gh_a = tab_by_id(&res_gh, "multiple-comparisons-factor-a");
    assert_close(cell_num(&tab_gh_a.rows[0][0]), 0.0, 1e-12, "Games-Howell A diff");
    assert_close(cell_num(&tab_gh_a.rows[0][1]), 0.912_870_929_175_278, 1e-10, "Games-Howell A cell SE");
    assert_close(cell_num(&tab_gh_a.rows[0][3]), 38.554_216_867_469_9, 1e-8, "Games-Howell A Welch df");
}

#[test]
fn r2_s02_01_rm_split_plot_reml_and_simple_effects_multiplicity() {
    // 1. Balanced split-plot ColumnFactorMatched (A=2 between-subj, s=5/group, B=4 repeated).
    // R aov(y ~ A * B + Error(sub/B)) oracle:
    //   margA: diff=-3.985, se=4.87410632834369, t=-0.817585775022306, df=8, p=0.437264297103214
    //   A|B1: diff=-3.94, se=4.87690988229227, df=8.01841853740494
    let subj_u = [-12.0, -5.0, 1.0, 6.0, 10.0, -9.0, -3.0, 0.0, 4.0, 8.0];
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(5), 4, 2).unwrap();
    gt.row_factor_title = String::from("Group");
    gt.column_factor_title = String::from("Time");
    gt.row_titles = vec![String::from("A1"), String::from("A2")];
    for c in 0..4 {
        gt.data_sets[c].title = format!("B{}", c + 1);
    }
    for s in 0..5 {
        for a in 0..2 {
            for b in 0..4 {
                let u = subj_u[a * 5 + s];
                let noise = 0.3 * ((((a + 1) * (b + 1) + (s + 1)) % 3) as f64 - 1.0);
                let y = 10.0 + (a as f64) * 4.0 + (b as f64) * 1.5 + u + noise;
                gt.data_sets[b].cells[a][s] = Cell::number(y);
            }
        }
    }
    let sheet_sp = DataSheet::new(SheetId::new("sp-1"), "SplitPlot", TableArchetype::Grouped(gt)).unwrap();
    let spec_sp = AnalysisSpec::new(
        SheetId::new("r-sp"),
        SheetId::new("sp-1"),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    let res_sp = run_analysis(&sheet_sp, &spec_sp).unwrap();

    let tab_a = tab_by_id(&res_sp, "multiple-comparisons-factor-a");
    assert_close(cell_num(&tab_a.rows[0][0]), -3.985, 1e-10, "Split-plot margA diff");
    assert_close(cell_num(&tab_a.rows[0][1]), 4.874_106_328_343_69, 1e-8, "Split-plot margA between-subj SE");
    assert_close(cell_num(&tab_a.rows[0][2]), -0.817_585_775_022_306, 1e-8, "Split-plot margA t");
    assert_close(cell_num(&tab_a.rows[0][3]), 8.0, 1e-12, "Split-plot margA df=8");
    assert_close(cell_p(&tab_a.rows[0][4]), 0.437_264_297_103_214, 1e-8, "Split-plot margA P");

    // Simple effects of 2-level A across 4 timepoints B: Satterthwaite composite df & m=4 Bonferroni adjustment.
    let tab_simple_a = tab_by_id(&res_sp, "multiple-comparisons-simple-a-within-b");
    assert_eq!(tab_simple_a.rows.len(), 4);
    assert_close(cell_num(&tab_simple_a.rows[0][0]), -3.94, 1e-10, "Split-plot A|B1 diff");
    assert_close(cell_num(&tab_simple_a.rows[0][1]), 4.876_909_882_292_27, 1e-8, "Split-plot A|B1 composite SE");
    assert_close(cell_num(&tab_simple_a.rows[0][3]), 8.018_418_537_404_94, 1e-8, "Split-plot A|B1 Satterthwaite df");
    let raw_p_b1 = cell_p(&tab_simple_a.rows[0][4]);
    let adj_p_b1 = cell_p(&tab_simple_a.rows[0][5]);
    assert_close(adj_p_b1, (4.0 * raw_p_b1).min(1.0), 1e-12, "Simple A|B adjusted across all 4 B slices");

    // Verify no colliding group IDs across tabs/slices in out.pairwise_comparisons.
    let mut id_to_label: BTreeMap<usize, String> = BTreeMap::new();
    for rec in &res_sp.pairwise_comparisons {
        for (id, lbl) in [(rec.group_a, &rec.label_a), (rec.group_b, &rec.label_b)] {
            if let Some(prev) = id_to_label.get(&id) {
                assert_eq!(prev, lbl, "group ID collision: {id} mapped to {prev:?} and {lbl:?}");
            } else {
                id_to_label.insert(id, lbl.clone());
            }
        }
    }

    // 2. Missing-replicate REML fallback (ColumnFactorMatched) against nlme + exact Kenward-Roger oracle.
    let y_reml = [
        1.32742922414,
        7.72378701262,
        14.870449977,
        17.0846100697,
        9.15514528219,
        16.0580779843,
        17.5530994472,
        18.4819265185,
        3.17577657773,
        16.4688981569,
        19.4317765925,
        16.4809257407,
        21.1328192591,
        23.3329906383,
        29.1442957642,
        6.59742057388,
        9.64005941204,
        17.8416270819,
        20.4326503107,
        22.7186115528,
        28.7384301024,
        30.768794662,
    ];
    let subjs = [
        "Ctrl_1", "Ctrl_2", "Ctrl_3", "Ctrl_4", "Drug_1", "Drug_2", "Drug_3", "Drug_4", "Ctrl_1", "Ctrl_3", "Ctrl_4", "Drug_1", "Drug_2", "Drug_3",
        "Drug_4", "Ctrl_1", "Ctrl_2", "Ctrl_3", "Ctrl_4", "Drug_1", "Drug_2", "Drug_3",
    ];
    let a_vals = [
        "Ctrl", "Ctrl", "Ctrl", "Ctrl", "Drug", "Drug", "Drug", "Drug", "Ctrl", "Ctrl", "Ctrl", "Drug", "Drug", "Drug", "Drug", "Ctrl", "Ctrl",
        "Ctrl", "Ctrl", "Drug", "Drug", "Drug",
    ];
    let b_vals = ["T1", "T1", "T1", "T1", "T1", "T1", "T1", "T1", "T2", "T2", "T2", "T2", "T2", "T2", "T2", "T3", "T3", "T3", "T3", "T3", "T3", "T3"];
    let mut gt_reml = GroupedTable::new(SubcolumnMode::RawReplicates(4), 3, 2).unwrap();
    gt_reml.row_factor_title = String::from("Arm");
    gt_reml.column_factor_title = String::from("Time");
    gt_reml.row_titles = vec![String::from("Ctrl"), String::from("Drug")];
    gt_reml.data_sets[0].title = String::from("T1");
    gt_reml.data_sets[1].title = String::from("T2");
    gt_reml.data_sets[2].title = String::from("T3");
    for idx in 0..y_reml.len() {
        let r = usize::from(a_vals[idx] != "Ctrl");
        let c = match b_vals[idx] {
            "T1" => 0,
            "T2" => 1,
            _ => 2,
        };
        let s: usize = subjs[idx].split('_').nth(1).unwrap().parse::<usize>().unwrap() - 1;
        gt_reml.data_sets[c].cells[r][s] = Cell::number(y_reml[idx]);
    }
    let sheet_reml = DataSheet::new(SheetId::new("reml-1"), "REML_RM", TableArchetype::Grouped(gt_reml)).unwrap();
    let spec_reml = AnalysisSpec::new(
        SheetId::new("r-reml"),
        SheetId::new("reml-1"),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            fit_reml_on_missing: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    let res_reml = run_analysis(&sheet_reml, &spec_reml).unwrap();
    let tab_reml_a = tab_by_id(&res_reml, "multiple-comparisons-factor-a");
    assert_close(cell_num(&tab_reml_a.rows[0][0]), -10.335_587_215_427_3, 1e-5, "REML KR margA diff");
    assert_close(cell_num(&tab_reml_a.rows[0][1]), 4.212_796_619_268_27, 1e-4, "REML KR margA SE");
    assert_close(cell_num(&tab_reml_a.rows[0][3]), 5.999_852_640_602_67, 1e-4, "REML KR margA df");

    let tab_reml_ab = tab_by_id(&res_reml, "multiple-comparisons-simple-a-within-b");
    assert_eq!(tab_reml_ab.rows.len(), 3);
    assert_close(cell_num(&tab_reml_ab.rows[2][0]), -15.382_837_892_421_5, 1e-5, "REML KR A|T3 diff");
    assert_close(cell_num(&tab_reml_ab.rows[2][1]), 4.278_656_668_386_96, 1e-4, "REML KR A|T3 SE");
    assert_close(cell_num(&tab_reml_ab.rows[2][3]), 6.379_046_679_061_93, 1e-4, "REML KR A|T3 df");
}

// ============================================================================
// R2-S02-02: Cohen's Kappa Fleiss-Cohen-Everitt (1969) Asymptotic SE
// ============================================================================

#[test]
fn r2_s02_02_cohens_kappa_fleiss_everitt_se_matches_oracle() {
    // 2x2 [[45, 5], [5, 45]]: kappa = 0.8, true SE = 0.06, 95% CI = [0.8 +/- 1.95996398454 * 0.06].
    let k2 = cohens_kappa(&[vec![45, 5], vec![5, 45]], KappaWeighting::Unweighted, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(k2.kappa, 0.8, 1e-12, "2x2 kappa");
    assert_close(k2.se, 0.06, 1e-12, "2x2 kappa SE");
    assert_close(k2.ci.0, 0.8 - 1.959_963_984_540_054 * 0.06, 1e-10, "2x2 kappa CI lo");
    assert_close(k2.ci.1, 0.8 + 1.959_963_984_540_054 * 0.06, 1e-10, "2x2 kappa CI hi");

    // 3x3 [[40, 8, 2], [6, 30, 4], [1, 3, 26]] across Unweighted, Linear, Quadratic weightings.
    let m3 = vec![vec![40, 8, 2], vec![6, 30, 4], vec![1, 3, 26]];
    let expected = [
        (KappaWeighting::Unweighted, 0.695_238_095_238_095, 0.055_822_163_014_395_6),
        (KappaWeighting::Linear, 0.739_967_897_271_268, 0.051_171_359_296_692_1),
        (KappaWeighting::Quadratic, 0.785_714_285_714_286, 0.051_734_596_988_002_6),
    ];
    for (wt, exp_k, exp_se) in expected {
        let r = cohens_kappa(&m3, wt, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_close(r.kappa, exp_k, 1e-12, "3x3 kappa");
        assert_close(r.se, exp_se, 1e-12, "3x3 kappa SE");
    }

    // End-to-end ResultSheet wiring.
    let mut ct = ContingencyTable::new(3, 3).unwrap();
    for (r, row) in m3.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell::number(v as f64);
        }
    }
    let sheet = DataSheet::new(SheetId::new("ct-kap"), "Kappa3x3", TableArchetype::Contingency(ct)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-kap"),
        SheetId::new("ct-kap"),
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic },
            ..Default::default()
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_close(scalar(&res, "kappa", None), 0.785_714_285_714_286, 1e-12, "sheet kappa");
    assert_close(scalar(&res, "se", None), 0.051_734_596_988_002_6, 1e-12, "sheet kappa SE");
}

// ============================================================================
// R2-S02-03, 04, 05, 10, 11, 13: Power Analysis Fixes
// ============================================================================

#[test]
fn r2_s02_03_welch_unpaired_t_power_and_detectable_effect() {
    let req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(5.0)
        .with_sample_size_group2(25.0)
        .with_alpha(0.05)
        .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 10.0, sd: 10.0, sd_group2: Some(1.0), within_pair_correlation: None });
    let res = solve_power(&req).unwrap();
    assert_close(res.actual_df.unwrap(), 4.016_013_322_658_279, 1e-10, "Welch Satterthwaite df");
    assert_close(res.noncentrality_param, 2.233_835_258_044_689, 1e-10, "Welch noncentrality");
    assert_close(res.critical_value, 2.772_084_483_361_45, 1e-10, "Welch t critical value");
    assert_close(res.achieved_power, 0.401_502_1, 1e-6, "Exact Welch NCT power (R oracle)");
    assert!(res.notes.iter().any(|n| n.contains("Welch")));

    // DetectableEffect with unequal SDs solves mean_diff & round-trips back to target_power.
    let mde_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(8.0)
        .with_sample_size_group2(20.0)
        .with_alpha(0.05)
        .with_target_power(0.80)
        .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 0.0, sd: 8.0, sd_group2: Some(2.0), within_pair_correlation: None });
    let mde_res = solve_power(&mde_req).unwrap();
    assert!(mde_res.raw_effect_size.unwrap() > 0.0);
    let check_req = mde_req.clone().with_target(PowerSolveTarget::Power).with_effect(PowerEffectSpec::MeanDiffAndSd {
        mean_diff: mde_res.raw_effect_size.unwrap(),
        sd: 8.0,
        sd_group2: Some(2.0),
        within_pair_correlation: None,
    });
    let check = solve_power(&check_req).unwrap();
    assert_close(check.achieved_power, 0.80, 1e-9, "Welch MDE round-trip");
}

#[test]
fn r2_s02_04_explicit_group2_propagates_allocation_ratio() {
    let req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(20.0)
        .with_sample_size_group2(50.0)
        .with_allocation_ratio(1.0) // stale default intentionally different from 50/20 = 2.5
        .with_alpha(0.05)
        .with_target_power(0.80);
    let res = solve_power(&req).unwrap();
    let pt_20 = res.power_curve.iter().find(|pt| (pt.n - 20.0).abs() < 0.5).expect("missing curve point at n=20");
    assert_close(pt_20.total_n, 70.0, 1e-12, "curve propagates effective allocation_ratio=2.5");
    assert_close(pt_20.power, res.achieved_power, 1e-9, "curve point matches achieved power");

    // Sensitivity table with Power + explicit n2 also propagates effective k=2.5.
    let pwr_req = PowerAnalysisRequest::unpaired_t(0.6, 0.05, 0.80)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(20.0)
        .with_sample_size_group2(50.0)
        .with_allocation_ratio(1.0);
    let curve_direct = compute_power_curve(&pwr_req, 10, 30, 5).unwrap();
    assert_close(curve_direct[0].total_n, 35.0, 1e-12, "direct compute_power_curve propagates k=2.5");
    let sens = compute_sensitivity_table(&pwr_req, &[0.05], &[0.80]).unwrap();
    let n2_expected = ((sens[0].sample_size as f64) * 2.5).ceil() as usize;
    assert_eq!(sens[0].total_n, sens[0].sample_size + n2_expected);
}

#[test]
fn r2_s02_05_small_allocation_ratio_and_discrete_ceiling_verification() {
    // 1. k < 1.0 in TwoProportions and k < 0.5 in UnpairedT must succeed through curve generation.
    let tp_half = PowerAnalysisRequest::two_proportions(0.6, 0.3, 0.05, 0.80).with_allocation_ratio(0.5);
    let r_half = solve_power(&tp_half).unwrap();
    assert!(r_half.power_curve.first().unwrap().n >= 2.0);
    assert!(r_half.achieved_power >= 0.80);

    let ut_quarter = PowerAnalysisRequest::unpaired_t(0.8, 0.05, 0.80).with_allocation_ratio(0.25);
    let r_quarter = solve_power(&ut_quarter).unwrap();
    assert!(r_quarter.power_curve.first().unwrap().n >= 4.0);
    assert!(r_quarter.achieved_power >= 0.80);

    // 2. PooledNullUnpooledAlt non-monotone rounding trap: (47, 3) only gets 0.6903; must advance to (51, 3) >= 0.70.
    let trap = PowerAnalysisRequest::two_proportions(0.001, 0.20, 0.05, 0.70)
        .with_two_proportions_mode(TwoProportionsMethod::PooledNullUnpooledAlt)
        .with_allocation_ratio(0.05);
    let r_trap = solve_power(&trap).unwrap();
    assert_eq!(r_trap.n_per_group, 51);
    assert_eq!(r_trap.n_group2, Some(3));
    assert!(r_trap.achieved_power >= 0.70, "expected >= 0.70, got {}", r_trap.achieved_power);
    assert_close(r_trap.achieved_power, 0.701_269, 1e-5, "verified (51,3) R power");
}

#[test]
fn r2_s02_10_power_checked_usize_bounds() {
    let huge_k =
        PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(1e8).with_allocation_ratio(1e12);
    assert!(matches!(solve_power(&huge_k), Err(StatsError::InvalidParameter { name: "allocation_ratio", .. })));

    let huge_groups =
        PowerAnalysisRequest::one_way_anova(usize::MAX / 2, 0.3, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(10.0);
    assert!(matches!(solve_power(&huge_groups), Err(StatsError::InvalidParameter { name: "num_groups", .. })));

    let huge_curve = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_allocation_ratio(1e9);
    assert!(matches!(compute_power_curve(&huge_curve, 10, 100, 10), Err(StatsError::InvalidParameter { .. })));
}

#[test]
fn r2_s02_11_one_sided_sample_size_rejects_power_le_alpha() {
    for tail in [TailMode::OneSided(OneSidedDirection::Greater), TailMode::OneSided(OneSidedDirection::Less)] {
        let sign = if tail == TailMode::OneSided(OneSidedDirection::Less) { -1.0 } else { 1.0 };
        let req_lt = PowerAnalysisRequest::unpaired_t(2.0 * sign, 0.10, 0.05).with_tail(tail);
        assert!(matches!(solve_power(&req_lt), Err(StatsError::InvalidParameter { name: "target_power", .. })));

        let req_eq = PowerAnalysisRequest::unpaired_t(2.0 * sign, 0.10, 0.10).with_tail(tail);
        assert!(matches!(solve_power(&req_eq), Err(StatsError::InvalidParameter { name: "target_power", .. })));
    }
}

#[test]
fn r2_s02_13_detectable_effect_curve_regression_less_and_anova_inferred_k() {
    let reg_less = PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_tail(TailMode::OneSided(OneSidedDirection::Less))
        .with_sample_size(40.0)
        .with_alpha(0.05)
        .with_target_power(0.80);
    let r_reg = solve_power(&reg_less).unwrap();
    assert!(!r_reg.power_curve.is_empty());
    assert!(!r_reg.sensitivity_table.is_empty());
    assert!(r_reg.effect_size > 0.0);
    assert_close(r_reg.achieved_power, 0.80, 1e-9, "Regression OneSided(Less) MDE achieved power");

    let reg_slope_less = PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_tail(TailMode::OneSided(OneSidedDirection::Less))
        .with_sample_size(40.0)
        .with_alpha(0.05)
        .with_target_power(0.80)
        .with_effect(PowerEffectSpec::RegressionSlope { beta1: 0.0, sigma_x: 2.0, sigma_eps: 3.0 });
    let r_slope = solve_power(&reg_slope_less).unwrap();
    assert!(r_slope.raw_effect_size.unwrap() < 0.0);
    assert!(!r_slope.power_curve.is_empty());

    let anova_no_k = PowerAnalysisRequest::new(PowerTestFamily::OneWayAnova)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(20.0)
        .with_alpha(0.05)
        .with_target_power(0.80)
        .with_effect(PowerEffectSpec::GroupMeansAndSd { means: vec![0.0, 1.0, 2.0], within_sd: 1.0 });
    let r_aov = solve_power(&anova_no_k).unwrap();
    assert_eq!(r_aov.total_n, 60);
    assert!(!r_aov.power_curve.is_empty());
    assert!(!r_aov.sensitivity_table.is_empty());
}

// ============================================================================
// R2-S02-06 & 07: Column XY Trapezoidal AUC ties & MultipleVariables Label Exclusion
// ============================================================================

#[test]
fn r2_s02_06_xy_trapezoidal_auc_collapses_tied_x_to_mean() {
    // Replicate rows out of order with tied X: (0,0), (1,40), (0,10), (1,20)
    // Mean curve: (0, 5) -> (1, 30) => AUC = 17.5 regardless of ordering.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
    let pts = [(0.0, 0.0), (1.0, 40.0), (0.0, 10.0), (1.0, 20.0)];
    for (r, &(x, y)) in pts.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new(SheetId::new("xy-tie"), "XYTie", TableArchetype::Xy(xy)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("r-xy"), SheetId::new("xy-tie"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_close(scalar(&res, "Trapezoidal AUC", Some(0)), 17.5, 1e-12, "Tied-X AUC uses per-X mean");

    // Pre-summarized MeanSdN weighted by n: at X=0 (mean=4, n=1 & mean=10, n=2 => weighted mean=8); at X=2 (mean=12, n=1) => AUC = 0.5*2*(8+12)=20.
    let mut xy_pre = XyTable::new(SubcolumnMode::MeanSdN, 1, 3).unwrap();
    for (r, &(x, m, n)) in [(0.0, 4.0, 1.0), (0.0, 10.0, 2.0), (2.0, 12.0, 1.0)].iter().enumerate() {
        xy_pre.x_cells[r][0] = Cell::number(x);
        xy_pre.data_sets[0].cells[r][0] = Cell::number(m);
        xy_pre.data_sets[0].cells[r][1] = Cell::number(1.0);
        xy_pre.data_sets[0].cells[r][2] = Cell::number(n);
    }
    let pre_sheet = DataSheet::new(SheetId::new("xy-pre-tie"), "XYPreTie", TableArchetype::Xy(xy_pre)).unwrap();
    let pre_spec =
        AnalysisSpec::new(SheetId::new("r-xyp"), SheetId::new("xy-pre-tie"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let pre_res = run_analysis(&pre_sheet, &pre_spec).unwrap();
    assert_close(scalar(&pre_res, "Trapezoidal AUC", Some(0)), 20.0, 1e-12, "Pre-summarized tied-X weighted AUC");
}

#[test]
fn r2_s02_07_mv_row_descriptive_excludes_numeric_label_role() {
    let mut mv = MultipleVariablesTable::new(3, 2).unwrap();
    mv.variables[0].title = String::from("SubjectID");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("SysBP");
    mv.variables[2].title = String::from("DiaBP");

    mv.variables[0].cells[0] = Cell::number(1001.0);
    mv.variables[1].cells[0] = Cell::number(120.0);
    mv.variables[2].cells[0] = Cell::number(80.0);
    mv.variables[0].cells[1] = Cell::number(1002.0);
    mv.variables[1].cells[1] = Cell::number(130.0);
    mv.variables[2].cells[1] = Cell::number(90.0);

    let sheet = DataSheet::new(SheetId::new("mv-lbl"), "MVLabel", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-lbl"),
        SheetId::new("mv-lbl"),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, ..DescriptiveSpec::default() }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let tab = tab_by_id(&res, "row-descriptive-stats");
    assert_close(cell_num(&tab.rows[0][0]), 2.0, 1e-12, "Row 0 excludes Label ID col (N=2)");
    assert_close(cell_num(&tab.rows[0][1]), 100.0, 1e-12, "Row 0 Mean excludes 1001");
    assert_close(cell_num(&tab.rows[0][4]), 120.0, 1e-12, "Row 0 Max excludes 1001");
}

// ============================================================================
// R2-S02-08: paired_sign_test Difference Overflow
// ============================================================================

#[test]
fn r2_s02_08_paired_sign_test_rejects_diff_overflow_cleanly() {
    let res_all = paired_sign_test(&[1e308, 1e308], &[-1e308, -1e308], true, ConfidenceLevel::DEFAULT_95);
    assert!(matches!(res_all, Err(StatsError::Math(MathError::DomainError { .. }))));

    let res_part = paired_sign_test(&[1e308, 2.0], &[-1e308, 1.0], true, ConfidenceLevel::DEFAULT_95);
    assert!(matches!(res_part, Err(StatsError::Math(MathError::DomainError { .. }))));

    // Also via DataSheet run_analysis.
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 2).unwrap();
    col.data_sets[0].cells[0][0] = Cell::number(1e308);
    col.data_sets[0].cells[1][0] = Cell::number(1e308);
    col.data_sets[1].cells[0][0] = Cell::number(-1e308);
    col.data_sets[1].cells[1][0] = Cell::number(-1e308);
    let sheet = DataSheet::new(SheetId::new("col-ovf"), "Ovf", TableArchetype::Column(col)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-ovf"),
        SheetId::new("col-ovf"),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..Default::default() }),
    )
    .unwrap();
    assert!(run_analysis(&sheet, &spec).is_err());
}

// ============================================================================
// R2-S02-12: PartsOfWhole GOF Clamping Negative G^2 Rounding Residual & Structural Zeros
// ============================================================================

#[test]
fn r2_s02_12_parts_of_whole_gof_clamps_g2_and_structural_zero_df() {
    // 1. Exact fit [1, 5, 29] where raw G^2 evaluates to -6.439e-15 in f64 without clamp.
    let mut pow = PartsOfWholeTable::new(1, 3).unwrap();
    for (i, &v) in [1.0, 5.0, 29.0].iter().enumerate() {
        pow.slice_labels[i] = format!("S{}", i + 1);
        pow.data_sets[0].cells[i][0] = Cell::number(v);
    }
    let sheet = DataSheet::new(SheetId::new("pow-g2"), "POWG2", TableArchetype::PartsOfWhole(pow)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-pow-g2"),
        SheetId::new("pow-g2"),
        AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(vec![1.0, 5.0, 29.0]), ..Default::default() }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_eq!(scalar(&res, "Likelihood-ratio G-squared", None), 0.0);
    assert_close(scalar(&res, "P value", None), 1.0, 1e-12, "Exact fit chi2 p=1");
    assert_close(scalar(&res, "G-squared P value", None), 1.0, 1e-12, "Exact fit G2 p=1");

    // 2. Structural zero (obs=0, weight=0) excluded from df: 4 slices with 1 structural zero -> df = 2.
    let mut pow_sz = PartsOfWholeTable::new(1, 4).unwrap();
    for (i, &v) in [10.0, 20.0, 30.0, 0.0].iter().enumerate() {
        pow_sz.slice_labels[i] = format!("C{}", i + 1);
        pow_sz.data_sets[0].cells[i][0] = Cell::number(v);
    }
    let sheet_sz = DataSheet::new(SheetId::new("pow-sz"), "POWSZ", TableArchetype::PartsOfWhole(pow_sz)).unwrap();
    let spec_sz = AnalysisSpec::new(
        SheetId::new("r-pow-sz"),
        SheetId::new("pow-sz"),
        AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(vec![1.0, 1.0, 1.0, 0.0]), ..Default::default() }),
    )
    .unwrap();
    let res_sz = run_analysis(&sheet_sz, &spec_sz).unwrap();
    let gof_tab = tab_by_id(&res_sz, "parts-gof");
    assert_close(cell_num(&gof_tab.rows[0][1]), 2.0, 1e-12, "Structural zero reduces df to active-1");
}

// ============================================================================
// R2-S02-14: Cox PH Categorical Complete-Case Filtering & Extreme Hazard Overlays
// ============================================================================

#[test]
fn r2_s02_14_cox_categorical_complete_case_filtering_and_overlay_stability() {
    let t_cox = [4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 5.0, 7.0];
    let e_cox = [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0];
    let grp_cox = ["A", "A", "A", "B", "B", "B", "A", "B", "A", "B", "C", "C"];
    let cov_cox = [Some(1.2), Some(2.1), Some(0.8), Some(3.0), Some(2.5), Some(1.8), Some(0.9), Some(3.4), Some(1.1), Some(2.9), None, None];

    let mut mv = MultipleVariablesTable::new(4, 12).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Event");
    mv.variables[2].title = String::from("Arm");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("X");
    for r in 0..12 {
        mv.variables[0].cells[r] = Cell::number(t_cox[r]);
        mv.variables[1].cells[r] = Cell::number(e_cox[r]);
        mv.variables[2].cells[r] = Cell::text(grp_cox[r]);
        mv.variables[3].cells[r] = match cov_cox[r] {
            Some(v) => Cell::number(v),
            None => Cell::empty(),
        };
    }
    let sheet = DataSheet::new(SheetId::new("mv-cox-cc"), "CoxCC", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-cox-cc"),
        SheetId::new("mv-cox-cc"),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: vec![(2, String::from("A"))],
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let ph_tab = tab_by_id(&res, "cox-ph");
    // Level "C" only appeared in incomplete rows -> excluded cleanly, leaving 2 terms (Arm[B vs A], X).
    assert_eq!(ph_tab.row_labels, vec![String::from("Arm[B vs A]"), String::from("X")]);
    assert_close(cell_num(&ph_tab.rows[0][0]), 1.105_599_189_738_33, 1e-7, "Cox coef Arm[B vs A]");
    assert_close(cell_num(&ph_tab.rows[0][1]), 1.922_725_378_395_87, 1e-7, "Cox SE Arm[B vs A]");
    assert_close(cell_num(&ph_tab.rows[1][0]), -1.319_374_615_050_9, 1e-7, "Cox coef X");
    assert_close(cell_num(&ph_tab.rows[1][1]), 1.186_728_572_958_91, 1e-7, "Cox SE X");
    assert_close(scalar(&res, "AIC", None), 21.294_461_191_473_5, 1e-7, "Cox AIC (df=2)");
    assert!(!res.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("convergence")));

    // Check finite curve overlays even with many events / large hazard span.
    for curve in &res.fitted_curves {
        for pt in &curve.points {
            assert!(pt.y.is_finite() && (0.0..=1.0).contains(&pt.y));
        }
    }
}

// ============================================================================
// R2-S02-15: Linear GLM Joint Multi-df Categorical Factor Type III Partial SS
// ============================================================================

#[test]
fn r2_s02_15_linear_glm_type_iii_joint_multidf_factors() {
    // R oracle: lm(y ~ dose + age), drop1(m, test="F") with dose having 3 levels (2 df).
    let y = [12.1, 13.4, 11.8, 15.2, 16.0, 14.5, 22.3, 24.1, 21.8, 20.5, 23.0, 25.4, 26.2, 27.1];
    let dose = ["Low", "Low", "Low", "Low", "Med", "Med", "Med", "Med", "Med", "High", "High", "High", "High", "High"];
    let age = [20.0, 25.0, 22.0, 30.0, 28.0, 32.0, 40.0, 45.0, 38.0, 35.0, 42.0, 48.0, 50.0, 52.0];

    let mut mv = MultipleVariablesTable::new(3, 14).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("Dose");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Age");
    for r in 0..14 {
        mv.variables[0].cells[r] = Cell::number(y[r]);
        mv.variables[1].cells[r] = Cell::text(dose[r]);
        mv.variables[2].cells[r] = Cell::number(age[r]);
    }
    let sheet = DataSheet::new(SheetId::new("mv-glm3"), "GlmTypeIII", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-glm3"),
        SheetId::new("mv-glm3"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: vec![(1, String::from("Low"))],
            compute_vif: true,
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let t3 = tab_by_id(&res, "glm-linear-type-iii");
    assert_eq!(t3.row_labels, vec![String::from("Dose"), String::from("Age")]);
    assert_close(cell_num(&t3.rows[0][0]), 1.529_160_527_705_57, 1e-10, "Dose joint Type III SS");
    assert_close(cell_num(&t3.rows[0][1]), 2.0, 1e-12, "Dose df=K-1=2");
    assert_close(cell_num(&t3.rows[0][2]), 1.529_160_527_705_57 / 2.0, 1e-10, "Dose MS");
    assert_close(cell_num(&t3.rows[0][3]), 0.497_311_575_846_308, 1e-10, "Dose joint F");
    assert_close(cell_p(&t3.rows[0][4]), 0.622_441_096_295_923, 1e-10, "Dose joint P");

    assert_close(cell_num(&t3.rows[1][0]), 91.757_229_603_183_9, 1e-10, "Age Type III SS");
    assert_close(cell_num(&t3.rows[1][1]), 1.0, 1e-12, "Age df=1");
    assert_close(cell_num(&t3.rows[1][3]), 59.682_331_086_219_3, 1e-10, "Age F");
    assert_close(cell_p(&t3.rows[1][4]), 1.596_720_368_339_24e-5, 1e-12, "Age P");
}

// ============================================================================
// R2-S02-16: Clustering Caps, O(n^2) Linkage, Single Dist Matrix, Dense Tab Cap
// ============================================================================

#[test]
fn r2_s02_16_clustering_hclust_oracle_bounds_and_dense_tab_cap() {
    let rows = vec![vec![0.0, 0.1], vec![0.2, 0.0], vec![5.0, 5.1], vec![5.3, 4.9], vec![2.0, 2.5], vec![2.2, 2.1], vec![8.0, 8.2], vec![8.4, 7.8]];
    let dmat = pairwise_distance_matrix(&rows, ClusterDistance::Euclidean).unwrap();

    // Verify all 7 linkage methods (5 NN-chain reducible + 2 heap-based) match R hclust(method=...).
    let expected = [
        (
            ClusterLinkage::Single,
            0.737_496_810_905_61,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 2.9, 3.96988664826, 4.24499705536],
        ),
        (
            ClusterLinkage::Complete,
            0.804_033_950_247_816,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.12409987036, 4.34165866922, 11.3951744173],
        ),
        (
            ClusterLinkage::Upgma,
            0.813_323_509_880_609,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.01947449499, 4.29109694192, 7.71674937249],
        ),
        (
            ClusterLinkage::Wpgma,
            0.813_323_509_880_609,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.01947449499, 4.29109694192, 7.71674937249],
        ),
        (
            ClusterLinkage::Ward,
            0.805_636_760_560_721,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 4.25734659148, 6.05020660804, 15.4189818081],
        ),
        (
            ClusterLinkage::Centroid,
            0.813_322_928_679_129_5,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.0103986447, 4.27814212013, 7.70949090407],
        ),
        (
            ClusterLinkage::Median,
            0.813_322_928_679_129_5,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.0103986447, 4.27814212013, 7.70949090407],
        ),
    ];
    for (linkage, exp_r, exp_h) in expected {
        let merges = hierarchical_clustering_with_metrics(&rows, ClusterDistance::Euclidean, linkage).unwrap();
        let coph = cophenetic_distance_matrix(rows.len(), &merges).unwrap();
        let rc = cophenetic_correlation(&dmat, &coph).unwrap();
        assert_close(rc, exp_r, 1e-10, "cophenetic r");
        for (idx, (m, &eh)) in merges.iter().zip(exp_h.iter()).enumerate() {
            assert_close(m.distance, eh, 1e-9, &format!("{linkage:?} height {idx}"));
        }
        let cut = cut_dendrogram(rows.len(), &merges, 4).unwrap();
        assert_eq!(cut[0], cut[1]);
        assert_eq!(cut[2], cut[3]);
        assert_eq!(cut[4], cut[5]);
        assert_eq!(cut[6], cut[7]);
    }

    // Verify Pearson zero-variance constant row maps cleanly and Manhattan k-means reports L1 total distance (not L1^2).
    let km_man = kmeans_pp_with_distance(&rows, 4, ClusterDistance::Manhattan, 7).unwrap();
    let mut manual_l1 = 0.0;
    for (r, &c) in rows.iter().zip(km_man.assignments.iter()) {
        manual_l1 += (r[0] - km_man.centroids[c][0]).abs() + (r[1] - km_man.centroids[c][1]).abs();
    }
    assert_close(km_man.wcss, manual_l1, 1e-12, "Manhattan WCSS is L1 objective");

    // Verify MAX_CLUSTERING_N cap rejects n > 2000 cleanly without OOM/hang.
    let over_rows = vec![vec![1.0, 2.0]; MAX_CLUSTERING_N + 1];
    assert!(matches!(
        pairwise_distance_matrix(&over_rows, ClusterDistance::Euclidean),
        Err(StatsError::InvalidParameter { name: "clustering_n", .. })
    ));

    // Verify n = 550 (> 500 matrix tab cap, <= MAX_CLUSTERING_N) succeeds quickly & omits n x n dense matrix tabs.
    let n_med = 550usize;
    let mut mv = MultipleVariablesTable::new(2, n_med).unwrap();
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Y");
    for i in 0..n_med {
        let grp = (i % 3) as f64 * 10.0;
        mv.variables[0].cells[i] = Cell::number(grp + ((i % 7) as f64) * 0.1);
        mv.variables[1].cells[i] = Cell::number(grp + ((i % 5) as f64) * 0.1);
    }
    let sheet_med = DataSheet::new(SheetId::new("mv-550"), "Clust550", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec_med = AnalysisSpec::new(
        SheetId::new("r-cl-550"),
        SheetId::new("mv-550"),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: None,
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(100), // capped cleanly
        }),
    )
    .unwrap();
    let res_med = run_analysis(&sheet_med, &spec_med).unwrap();
    assert!(scalar(&res_med, "cophenetic_r", None).is_finite());
    assert!(!res_med.tabs.iter().any(|t| t.id == "clustering-distance-matrix"));
    assert!(!res_med.tabs.iter().any(|t| t.id == "cophenetic-distance-matrix"));
}
