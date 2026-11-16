//! Stage 4A regression tests for S02, X2, and S05-01 twin fixes in `curvebook-stats`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, ContingencyTable,
    CoxTieMethod, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable, MultipleVariablesTable, MultiplicityMethod,
    MultiplicitySpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod, OutlierSpec, RepeatedMeasuresDesign, ResultCell, SheetId,
    SubcolumnAggregation, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, ThreeWayColumnGrouping, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::anova::{ordinary_one_way_from_summary, rm_one_way_anova};
use curvebook_stats::column::rout_outliers;
use curvebook_stats::multiplicity::{GroupComparisonInput, MultiplicityProcedure, compare_groups_with_procedure};
use curvebook_stats::{StatsError, run_analysis};

fn scalar_val(sheet: &curvebook_doc::ResultSheet, key: &str, ds_idx: Option<usize>) -> Option<f64> {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index == ds_idx).map(|s| s.value)
}

fn cell_num(cell: &ResultCell) -> Option<f64> {
    match cell {
        ResultCell::Number { value, .. } => Some(*value),
        _ => None,
    }
}

#[test]
fn r1_s02_07_zero_residual_rm_anova_and_multiplicity() {
    // Additive blocks yield MS_residual == 0 while treatment means differ strictly.
    let blocks = vec![vec![1.0, 2.0, 4.0], vec![2.0, 3.0, 5.0], vec![3.0, 4.0, 6.0], vec![4.0, 5.0, 7.0]];
    let rm = rm_one_way_anova(&blocks).unwrap();
    assert!(rm.f_treatment.is_infinite() && rm.f_treatment.is_sign_positive(), "expected +inf F, got {}", rm.f_treatment);
    assert_eq!(rm.p_uncorrected, 0.0);
    assert_eq!(rm.p_greenhouse_geisser, 0.0);
    assert_eq!(rm.p_huynh_feldt, 0.0);

    // Zero treatment effect and zero residual still gives F = 0, P = 1.
    let constant_blocks = vec![vec![5.0, 5.0, 5.0]; 3];
    let rm_zero = rm_one_way_anova(&constant_blocks).unwrap();
    assert_eq!(rm_zero.f_treatment, 0.0);
    assert_eq!(rm_zero.p_uncorrected, 1.0);

    // All parametric post-hoc procedures must emit INFINITY stat & P=0 when groups differ under zero variance.
    let groups = vec![
        GroupComparisonInput { index: 0, label: String::from("G0"), n: 4.0, mean: 10.0, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("G1"), n: 4.0, mean: 12.0, sd: 0.0 },
        GroupComparisonInput { index: 2, label: String::from("G2"), n: 4.0, mean: 12.0, sd: 0.0 },
    ];
    let procedures = [
        MultiplicityProcedure::None,
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::TamhaneT2,
        MultiplicityProcedure::FisherLsd,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
        MultiplicityProcedure::TwoStageBkyFdr,
    ];
    for proc in procedures {
        let rows = compare_groups_with_procedure(&groups, 0.0, 9.0, proc, 0.05, true).unwrap();
        assert!(!rows.is_empty(), "empty comparisons for {proc:?}");
        for r in rows {
            if (r.mean_diff).abs() > 0.0 {
                assert!(r.statistic.is_infinite(), "{proc:?} ({} vs {}): expected inf stat, got {}", r.label_a, r.label_b, r.statistic);
                assert_eq!(r.raw_p, 0.0, "{proc:?}: expected raw_p == 0");
                assert_eq!(r.adjusted_p_or_q, 0.0, "{proc:?}: expected adj_p == 0");
                assert!(r.significant, "{proc:?}: expected significant == true");
            } else {
                assert_eq!(r.statistic, 0.0, "{proc:?}: equal means stat must be 0");
                assert_eq!(r.raw_p, 1.0, "{proc:?}: equal means raw_p must be 1");
                assert_eq!(r.adjusted_p_or_q, 1.0, "{proc:?}: equal means adj_p must be 1");
                assert!(!r.significant, "{proc:?}: equal means must not be significant");
            }
        }
    }
}

#[test]
fn r1_s02_08_summary_one_way_anova_marks_homogeneity_not_computable() {
    let groups = vec![
        GroupComparisonInput { index: 0, label: String::from("A"), n: 5.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 5.0, mean: 13.0, sd: 3.5 },
        GroupComparisonInput { index: 2, label: String::from("C"), n: 5.0, mean: 15.0, sd: 2.0 },
    ];
    let res = ordinary_one_way_from_summary(&groups, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(!res.variance_tests.available);
    assert!(res.variance_tests.bartlett_chi2.is_nan());
    assert!(res.variance_tests.bartlett_p.is_nan());
    assert!(res.variance_tests.brown_forsythe_f.is_nan());
    assert!(res.variance_tests.brown_forsythe_p.is_nan());

    // Sheet-level MeanSdN table checks checklist and tab.
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 1).unwrap();
    tab.subcolumn_mode = SubcolumnMode::MeanSdN;
    for (c, (mean, sd, n)) in [(10.0, 1.5, 5.0), (13.0, 3.5, 5.0), (15.0, 2.0, 5.0)].into_iter().enumerate() {
        tab.data_sets[c].cells = vec![vec![Cell::number(mean), Cell::number(sd), Cell::number(n)]];
    }
    let sheet = DataSheet::new(SheetId::new("s-sum"), "Summary", TableArchetype::Column(tab)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("a-sum"),
        sheet.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec::default(),
        }),
    )
    .unwrap();
    let out = run_analysis(&sheet, &spec).unwrap();

    // Never report a fake Pass or "p = 1.0000" for homogeneity of variances on summary data.
    assert!(
        !out.checklist
            .iter()
            .any(|item| item.assumption.contains("Homogeneity") && matches!(item.status, ChecklistStatus::Pass | ChecklistStatus::Warn)),
        "summary ANOVA must not fake Pass/Warn variance homogeneity checklist: {:?}",
        out.checklist
    );
    let hom_item = out
        .checklist
        .iter()
        .find(|item| item.assumption == "Homogeneity of variances (Brown-Forsythe / Bartlett)")
        .expect("missing homogeneity Info checklist item");
    assert_eq!(hom_item.status, ChecklistStatus::Info);
    assert!(hom_item.explanation.to_lowercase().contains("not computable"));

    let hom_tab = out.tabs.iter().find(|t| t.id == "variance-homogeneity").expect("missing variance-homogeneity tab");
    assert_eq!(hom_tab.rows[0][0], ResultCell::Empty);
    assert!(matches!(&hom_tab.rows[0][4], ResultCell::Text(s) if s.contains("Not computable")));

    // Welch/Brown-Forsythe on summary data also emits Info checklist + footnote rather than fake homogeneity Pass.
    let spec_w = AnalysisSpec::new(
        SheetId::new("a-sum-w"),
        sheet.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::BrownForsytheAndWelch,
            compute_effect_sizes: false,
            multiplicity: MultiplicitySpec::default(),
        }),
    )
    .unwrap();
    let out_w = run_analysis(&sheet, &spec_w).unwrap();
    assert!(
        out_w
            .checklist
            .iter()
            .any(|item| item.assumption == "Homogeneity of variances (Brown-Forsythe / Bartlett)" && item.status == ChecklistStatus::Info)
    );
}

#[test]
fn r1_s02_09_cox_and_mv_km_strict_binary_survival_status() {
    let build_mv_surv = |statuses: &[f64]| -> DataSheet {
        let mut mv = MultipleVariablesTable::new(3, statuses.len()).unwrap();
        mv.variables[0].title = String::from("Time");
        mv.variables[1].title = String::from("Status");
        mv.variables[2].title = String::from("Dose");
        for (r, &st) in statuses.iter().enumerate() {
            mv.variables[0].cells[r] = Cell::number(((r + 1) * 5) as f64);
            mv.variables[1].cells[r] = Cell::number(st);
            mv.variables[2].cells[r] = Cell::number((r + 1) as f64);
        }
        DataSheet::new(SheetId::new("mv-surv"), "Survival", TableArchetype::MultipleVariables(mv)).unwrap()
    };

    for bad_status in [2.0, -1.0, 0.5] {
        let sheet = build_mv_surv(&[0.0, bad_status, 1.0, 0.0]);
        let cox_spec = AnalysisSpec::new(
            SheetId::new("a-cox"),
            sheet.id.clone(),
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2],
                reference_levels: Vec::new(),
                tie_method: CoxTieMethod::Efron,
            }),
        )
        .unwrap();
        assert!(
            matches!(run_analysis(&sheet, &cox_spec), Err(StatsError::InvalidParameter { name: "survival_status", value }) if (value - bad_status).abs() < 1e-12),
            "Cox PH must reject status={bad_status}"
        );

        let km_spec = AnalysisSpec::new(
            SheetId::new("a-km"),
            sheet.id.clone(),
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec::default(),
                mv_time_column: Some(0),
                mv_status_column: Some(1),
                mv_group_column: None,
            }),
        )
        .unwrap();
        assert!(
            matches!(run_analysis(&sheet, &km_spec), Err(StatsError::InvalidParameter { name: "survival_status", value }) if (value - bad_status).abs() < 1e-12),
            "MV KM must reject status={bad_status}"
        );
    }

    // Strict binary {0, 1} works cleanly for Cox PH.
    let valid_sheet = build_mv_surv(&[1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
    let cox_ok = AnalysisSpec::new(
        SheetId::new("a-cox-ok"),
        valid_sheet.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert!(run_analysis(&valid_sheet, &cox_ok).is_ok());
}

#[test]
fn r1_s02_10_outlier_row_indices_and_dataset_scalar_indices() {
    // Column 0 completely empty; Column 1 ("TreatA") has blank row 0, excluded row 1, blank row 4, outlier 500 at 0-based row 6 (1-based row 7).
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 8).unwrap();
    tab.data_sets[0].title = String::from("AllEmpty");
    tab.data_sets[1].title = String::from("TreatA");
    tab.data_sets[2].title = String::from("TreatB");

    tab.data_sets[1].cells[1][0] = Cell::excluded_number(10.0);
    tab.data_sets[1].cells[2][0] = Cell::number(10.1);
    tab.data_sets[1].cells[3][0] = Cell::number(10.2);
    tab.data_sets[1].cells[5][0] = Cell::number(9.9);
    tab.data_sets[1].cells[6][0] = Cell::number(500.0);
    tab.data_sets[1].cells[7][0] = Cell::number(10.0);

    for (r, v) in [1.0, 1.1, 0.9, 1.05, 0.95].into_iter().enumerate() {
        tab.data_sets[2].cells[r + 2][0] = Cell::number(v);
    }
    let sheet = DataSheet::new(SheetId::new("col-idx"), "Indexed", TableArchetype::Column(tab)).unwrap();

    for outlier_spec in [OutlierSpec::Rout { fdr_q_percent: 1.0 }, OutlierSpec::Grubbs { alpha: 0.05, iterative: true }] {
        let spec = AnalysisSpec::new(SheetId::new("out-idx"), sheet.id.clone(), AnalysisKind::OutlierDetection(outlier_spec)).unwrap();
        let res = run_analysis(&sheet, &spec).unwrap();
        assert_eq!(scalar_val(&res, "OutlierCount", Some(1)), Some(1.0));
        assert_eq!(scalar_val(&res, "OutlierCount", Some(2)), Some(0.0));
        assert_eq!(scalar_val(&res, "OutlierCount", Some(0)), None);

        let outliers_tab = &res.tabs[0];
        assert_eq!(outliers_tab.rows.len(), 1);
        assert_eq!(outliers_tab.rows[0][0], ResultCell::Text(String::from("TreatA")));
        assert_eq!(cell_num(&outliers_tab.rows[0][1]), Some(7.0), "1-based row index must account for empty/excluded preceding cells");
        assert_eq!(cell_num(&outliers_tab.rows[0][2]), Some(500.0));
    }

    // Descriptive stats also emits per-column scalars tagged with actual data_set_index (1 and 2, skipping empty 0).
    let desc_spec = AnalysisSpec::new(
        SheetId::new("desc-idx"),
        sheet.id.clone(),
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::PooledGrandSample,
            custom_upper_percentile: None,
            run_normality_battery: false,
            hypothetical_value: None,
        }),
    )
    .unwrap();
    let desc_res = run_analysis(&sheet, &desc_spec).unwrap();
    assert_eq!(scalar_val(&desc_res, "N", Some(1)), Some(5.0));
    assert_eq!(scalar_val(&desc_res, "N", Some(2)), Some(5.0));
    assert_eq!(scalar_val(&desc_res, "N", Some(0)), None);

    // Pooled summary DescriptiveStats preserves original column index too.
    let mut sum_tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 1).unwrap();
    sum_tab.subcolumn_mode = SubcolumnMode::MeanSdN;
    for ds in &mut sum_tab.data_sets {
        ds.cells = vec![vec![Cell::empty(); 3]];
    }
    sum_tab.data_sets[2].cells = vec![vec![Cell::number(42.0), Cell::number(2.0), Cell::number(6.0)]];
    let sum_sheet = DataSheet::new(SheetId::new("sum-idx"), "SumIdx", TableArchetype::Column(sum_tab)).unwrap();
    let sum_spec =
        AnalysisSpec::new(SheetId::new("a-sum-idx"), sum_sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let sum_res = run_analysis(&sum_sheet, &sum_spec).unwrap();
    assert_eq!(scalar_val(&sum_res, "Mean", Some(2)), Some(42.0));
    assert_eq!(scalar_val(&sum_res, "Mean", Some(0)), None);
}

#[test]
fn r1_s02_11_rout_zero_rsdr_heavy_ties_and_constant_series() {
    let mut heavy_ties = vec![5.0; 8];
    heavy_ties.push(100.0);
    let rout = rout_outliers(&heavy_ties, 1.0).unwrap();
    assert_eq!(rout.rsdr, 0.0);
    assert_eq!(rout.outliers.len(), 1);
    assert_eq!(rout.outliers[0].index, 8);
    assert_eq!(rout.outliers[0].value, 100.0);
    assert!(rout.outliers[0].score.is_infinite());
    assert_eq!(rout.outliers[0].threshold_or_p, 0.0);
    assert_eq!(rout.cleaned_values, vec![5.0; 8]);

    // Exact constant data must never flag outliers when rsdr == 0.
    let constant = rout_outliers(&[5.0; 10], 1.0).unwrap();
    assert_eq!(constant.rsdr, 0.0);
    assert!(constant.outliers.is_empty());

    // Sheet-level ROUT renders infinite statistic cleanly as "∞".
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 9).unwrap();
    for (r, &v) in heavy_ties.iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let sheet = DataSheet::new(SheetId::new("rout-ties"), "RoutTies", TableArchetype::Column(col)).unwrap();
    let spec =
        AnalysisSpec::new(SheetId::new("a-rout-ties"), sheet.id.clone(), AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }))
            .unwrap();
    let out = run_analysis(&sheet, &spec).unwrap();
    assert_eq!(scalar_val(&out, "OutlierCount", Some(0)), Some(1.0));
    assert_eq!(out.tabs[0].rows[0][3], ResultCell::Text(String::from("∞")));
}

#[test]
fn r1_x2_05_selected_data_sets_respected_in_factorial_nested_and_contingency() {
    // 1. Factorial ANOVA (2-way) ignores unselected middle column even when it holds huge values.
    let fill_grouped = |cols: &[(&str, &[[f64; 2]; 2])]| -> DataSheet {
        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(2), cols.len(), 2).unwrap();
        for (c, (title, vals)) in cols.iter().enumerate() {
            g.data_sets[c].title = (*title).to_owned();
            for r in 0..2 {
                g.data_sets[c].cells[r] = vec![Cell::number(vals[r][0]), Cell::number(vals[r][1])];
            }
        }
        DataSheet::new(SheetId::new("grp-x2"), "Grouped", TableArchetype::Grouped(g)).unwrap()
    };
    let c0 = [[10.0, 12.0], [14.0, 16.0]];
    let c1_poison = [[1e7, 2e7], [-1e7, -2e7]];
    let c2 = [[20.0, 22.0], [26.0, 28.0]];

    let full_grp = fill_grouped(&[("A", &c0), ("Poison", &c1_poison), ("B", &c2)]);
    let two_grp = fill_grouped(&[("A", &c0), ("B", &c2)]);

    let mut sel_fac_spec = AnalysisSpec::new(
        SheetId::new("fac-sel"),
        full_grp.id.clone(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();
    sel_fac_spec.selected_data_sets = vec![0, 2];
    let two_fac_spec = AnalysisSpec::new(
        SheetId::new("fac-ref"),
        two_grp.id.clone(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();

    let res_sel_fac = run_analysis(&full_grp, &sel_fac_spec).unwrap();
    let res_ref_fac = run_analysis(&two_grp, &two_fac_spec).unwrap();
    assert_eq!(scalar_val(&res_sel_fac, "Column factor: F", None), scalar_val(&res_ref_fac, "Column factor: F", None));
    assert_eq!(scalar_val(&res_sel_fac, "MS residual", None), scalar_val(&res_ref_fac, "MS residual", None));
    assert_eq!(cell_num(&res_sel_fac.tabs[0].rows[1][1]), Some(1.0), "df_col for 2 selected columns must be 1");

    // Verify 3-way and RM 2-way REML fallback also honor selected_data_sets cleanly.
    let mut rm_tab = GroupedTable::new(SubcolumnMode::RawReplicates(2), 5, 2).unwrap();
    rm_tab.three_way_grouping = Some(ThreeWayColumnGrouping {
        factor_1_name: String::from("Drug"),
        factor_1_levels: [String::from("Veh"), String::from("Tx")],
        factor_2_name: String::from("Sex"),
        factor_2_levels: [String::from("F"), String::from("M")],
    });
    for (dst_c, vals) in [(0, c0), (1, [[11.0, 13.0], [15.0, 17.0]]), (2, c1_poison), (3, c2), (4, [[22.0, 24.0], [28.0, 30.0]])] {
        for r in 0..2 {
            rm_tab.data_sets[dst_c].cells[r] = vec![Cell::number(vals[r][0]), Cell::number(vals[r][1])];
        }
    }
    let rm_sheet = DataSheet::new(SheetId::new("grp-3w"), "ThreeWay", TableArchetype::Grouped(rm_tab)).unwrap();
    let mut spec_3w = AnalysisSpec::new(
        SheetId::new("a-3w"),
        rm_sheet.id.clone(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec { three_way: true, ..Default::default() }),
    )
    .unwrap();
    spec_3w.selected_data_sets = vec![0, 1, 3, 4];
    let res_3w = run_analysis(&rm_sheet, &spec_3w).unwrap();
    assert!(scalar_val(&res_3w, "MS residual", None).is_some_and(|m| m < 10.0));

    let mut reml_tab = GroupedTable::new(SubcolumnMode::RawReplicates(2), 3, 2).unwrap();
    for (c, vals) in [(0, c0), (1, c1_poison), (2, c2)] {
        for r in 0..2 {
            reml_tab.data_sets[c].cells[r] = vec![Cell::number(vals[r][0]), Cell::number(vals[r][1])];
        }
    }
    // Missing observation forces REML fallback under RM design.
    reml_tab.data_sets[0].cells[1][1] = Cell::empty();
    let reml_sheet = DataSheet::new(SheetId::new("grp-reml"), "Reml", TableArchetype::Grouped(reml_tab)).unwrap();
    let mut spec_reml = AnalysisSpec::new(
        SheetId::new("a-reml"),
        reml_sheet.id.clone(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
            fit_reml_on_missing: true,
            ..Default::default()
        }),
    )
    .unwrap();
    spec_reml.selected_data_sets = vec![0, 2];
    let res_reml = run_analysis(&reml_sheet, &spec_reml).unwrap();
    assert!(scalar_val(&res_reml, "REML sigma^2 (residual)", None).is_some_and(|v| v < 100.0));

    // 2. Nested comparison honors selected_data_sets for both ANOVA and nested t-test.
    let fill_nested = |groups: &[(&str, &[[f64; 2]; 2])]| -> DataSheet {
        let mut nt = NestedTable::new(groups.len(), 2, 2).unwrap();
        for (g, (title, vals)) in groups.iter().enumerate() {
            nt.groups[g].title = (*title).to_owned();
            for u in 0..2 {
                nt.groups[g].cells[0][u] = Cell::number(vals[0][u]);
                nt.groups[g].cells[1][u] = Cell::number(vals[1][u]);
            }
        }
        DataSheet::new(SheetId::new("nest-x2"), "Nested", TableArchetype::Nested(nt)).unwrap()
    };
    let n0 = [[10.0, 12.0], [11.0, 13.0]];
    let n1_poison = [[1e6, 2e6], [1.5e6, 2.5e6]];
    let n2 = [[20.0, 22.0], [21.0, 23.0]];
    let full_nest = fill_nested(&[("G0", &n0), ("Poison", &n1_poison), ("G2", &n2)]);
    let two_nest = fill_nested(&[("G0", &n0), ("G2", &n2)]);

    let mut sel_nest_spec = AnalysisSpec::new(
        SheetId::new("nest-sel"),
        full_nest.id.clone(),
        AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, multiplicity: MultiplicitySpec::default() }),
    )
    .unwrap();
    sel_nest_spec.selected_data_sets = vec![0, 2];
    let ref_nest_spec = AnalysisSpec::new(
        SheetId::new("nest-ref"),
        two_nest.id.clone(),
        AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, multiplicity: MultiplicitySpec::default() }),
    )
    .unwrap();

    let res_sel_nest = run_analysis(&full_nest, &sel_nest_spec).unwrap();
    let res_ref_nest = run_analysis(&two_nest, &ref_nest_spec).unwrap();
    assert_eq!(scalar_val(&res_sel_nest, "Nested t", None), scalar_val(&res_ref_nest, "Nested t", None));
    assert_eq!(scalar_val(&res_sel_nest, "Residual variance", None), scalar_val(&res_ref_nest, "Residual variance", None));

    // 3. Contingency analysis (Pearson chi-square & ExactBinomial) honors selected_data_sets.
    let fill_ct = |cols: &[&[u64]]| -> DataSheet {
        let mut ct = ContingencyTable::new(cols.len(), cols[0].len()).unwrap();
        for (c, col) in cols.iter().enumerate() {
            for (r, &v) in col.iter().enumerate() {
                ct.data_sets[c].cells[r][0] = Cell::number(v as f64);
            }
        }
        DataSheet::new(SheetId::new("ct-x2"), "Contingency", TableArchetype::Contingency(ct)).unwrap()
    };
    let full_ct = fill_ct(&[&[30, 10], &[999, 1], &[12, 28]]);
    let ref_ct = fill_ct(&[&[30, 10], &[12, 28]]);

    let mut sel_ct_spec =
        AnalysisSpec::new(SheetId::new("ct-sel"), full_ct.id.clone(), AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    sel_ct_spec.selected_data_sets = vec![0, 2];
    let ref_ct_spec = AnalysisSpec::new(SheetId::new("ct-ref"), ref_ct.id.clone(), AnalysisKind::Contingency(ContingencySpec::default())).unwrap();

    let res_sel_ct = run_analysis(&full_ct, &sel_ct_spec).unwrap();
    let res_ref_ct = run_analysis(&ref_ct, &ref_ct_spec).unwrap();
    assert_eq!(scalar_val(&res_sel_ct, "Chi-square", None), scalar_val(&res_ref_ct, "Chi-square", None));
    assert_eq!(scalar_val(&res_sel_ct, "Chi-square P value", None), scalar_val(&res_ref_ct, "Chi-square P value", None));

    // ExactBinomial on 1x3 contingency selecting columns [0, 2].
    let bin_ct = fill_ct(&[&[14], &[999], &[6]]);
    let mut bin_spec = AnalysisSpec::new(
        SheetId::new("bin-sel"),
        bin_ct.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.5), ..Default::default() }),
    )
    .unwrap();
    bin_spec.selected_data_sets = vec![0, 2];
    let res_bin = run_analysis(&bin_ct, &bin_spec).unwrap();
    assert_eq!(scalar_val(&res_bin, "total", None), Some(20.0));
    assert_eq!(scalar_val(&res_bin, "successes", None), Some(14.0));
}

#[test]
fn r1_s05_01_ragged_mv_rows_return_clean_err() {
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[1].role = VariableRole::Categorical;
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::text(if r < 2 { "A" } else { "B" });
        mv.variables[1].cells[r] = Cell::text(if r % 2 == 0 { "X" } else { "Y" });
        mv.variables[2].cells[r] = Cell::number((r + 1) as f64);
    }
    let mut sheet = DataSheet::new(SheetId::new("mv-rag"), "Ragged", TableArchetype::MultipleVariables(mv)).unwrap();
    // Simulate post-construction malformed/ragged variable length before analysis dispatch.
    if let TableArchetype::MultipleVariables(ref mut inner) = sheet.archetype {
        inner.variables[1].cells.pop();
    }

    let fac_spec = AnalysisSpec::new(
        SheetId::new("a-rag-fac"),
        sheet.id.clone(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec { multifactor_columns: vec![0, 1], response_column: Some(2), ..Default::default() }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &fac_spec), Err(StatsError::DimensionMismatch { expected: 4, actual: 3 })));

    let glm_spec = AnalysisSpec::new(
        SheetId::new("a-rag-glm"),
        sheet.id.clone(),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 2,
            predictor_columns: vec![0, 1],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &glm_spec), Err(StatsError::DimensionMismatch { expected: 4, actual: 3 })));
}
