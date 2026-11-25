//! Consolidated regression & D1 oracle tests for ANOVA (one-way, Welch/BF, RM, Kruskal-Wallis,
//! Friedman, factorial, nested, REML), sphericity, and multiplicity adjustment procedures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, Cell, CellValue, ChecklistStatus, ColumnDesignHint, ContingencySpec, DataSheet, FactorialAnovaSpec, GroupedTable,
    MixedModelDfMethod, MultipleRowTestsSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec, OneWayAnovaSpec,
    OneWayMethod, PValueStackMethod, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultStatus, SubcolumnMode, SumOfSquaresType,
    TableArchetype, ThreeWayColumnGrouping, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{
    dunnett_two_sided_p_loadings, max_studentized_range_isf, max_studentized_range_sf, student_t_two_sided_isf, student_t_two_sided_p,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_stats::StatsError;
use curvebook_stats::anova::{
    FactorialObservations, RmSphericityCorrection, fit_factorial_anova, fit_multi_vc_reml, fit_multi_vc_reml_with_df_method,
    fit_random_intercept_reml, fit_random_intercept_reml_with_df_method, fit_rm_two_way_anova, fit_rm_two_way_anova_with_sphericity,
    fit_unstructured_rm_reml, fit_unstructured_rm_reml_with_df_method, nested_one_way_anova, ordinary_one_way_anova, ordinary_one_way_from_summary,
    rm_one_way_anova, run_factorial_anova, run_nested_analysis, run_one_way_anova, unequal_variance_one_way_anova,
};
use curvebook_stats::multiplicity::{
    MultiplicityProcedure, adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni, adjust_holm_sidak,
    adjust_p_values, adjust_sidak, adjust_two_stage_bky, compare_dunn_ranks, compare_groups_with_procedure, linear_trend_test, try_adjust_p_values,
};
use curvebook_stats::pvalue_stack::analyze_pvalue_stack;
use curvebook_stats::two_group::multiple_row_t_tests;
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_close, cell_bool, cell_ci_finite, cell_num, cell_p, col_sheet, col_sheet_hint, contingency_sheet, group_input, group_input_slice,
    grouped_sheet_fill, nested_sheet, nested_table, presum_col_sheet, run_kind, run_kind_conf, run_kind_sel, scalar, scalar_ds, spec_for, tab_by_id,
};

fn build_grouped_grid(id: &str, n_rows: usize, n_cols: usize, n_rep: usize) -> DataSheet {
    let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(n_rep), n_cols, n_rows).unwrap();
    for c in 0..n_cols {
        g.data_sets[c].title = format!("B{c}");
        for r in 0..n_rows {
            for s in 0..n_rep {
                g.data_sets[c].cells[r][s] = Cell::number(10.0 + 3.0 * (r as f64) + 2.0 * (c as f64) + 0.4 * (s as f64) + 0.1 * ((r * c) as f64));
            }
        }
    }
    for r in 0..n_rows {
        g.row_titles[r] = format!("A{r}");
    }
    DataSheet::new(id, id, TableArchetype::Grouped(g)).unwrap()
}

#[test]
fn audit_30_31_unobserved_levels_and_one_way_ci_confidence() {
    // 1. Finding #30: unobserved/excluded levels dropped cleanly in 2-way, RM 2-way, 3-way, and MV factorial.
    let build_2w = |pad: bool| {
        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(3), if pad { 3 } else { 2 }, if pad { 4 } else { 2 }).unwrap();
        (g.row_factor_title, g.column_factor_title) = ("Dose".into(), "Drug".into());
        (g.data_sets[0].title, g.data_sets[1].title) = ("Vehicle".into(), "Active".into());
        let data = [[[10.0, 11.0, 12.0], [15.0, 16.0, 17.0]], [[20.0, 22.0, 21.0], [31.0, 30.0, 32.0]]];
        for (ri, &r) in (if pad { [0, 2] } else { [0, 1] }).iter().enumerate() {
            g.row_titles[r] = format!("Dose{}", ri + 1);
            for c in 0..2 {
                for (s, &v) in data[ri][c].iter().enumerate() {
                    if !(ri == 1 && c == 0 && s == 2) {
                        g.data_sets[c].cells[r][s] = Cell::number(v);
                    }
                }
            }
        }
        if pad {
            g.data_sets[0].cells[1][0] = Cell::excluded_number(999.0);
            g.data_sets[2].cells[0][0] = Cell::excluded_number(777.0);
        }
        DataSheet::new("g2w", "G2W", TableArchetype::Grouped(g)).unwrap()
    };
    let fac_b = FactorialAnovaSpec {
        ss_type: SumOfSquaresType::TypeIII,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
        ..Default::default()
    };
    let (r_t, r_p) = (
        run_kind(&build_2w(false), AnalysisKind::FactorialAnova(fac_b.clone())).unwrap(),
        run_kind(&build_2w(true), AnalysisKind::FactorialAnova(fac_b)).unwrap(),
    );
    for key in ["Dose: F", "Dose: P", "Drug: F", "Drug: P", "Dose × Drug: F", "MS residual"] {
        assert_close(scalar(&r_p, key), scalar(&r_t, key), 1e-12, key);
    }

    // RM 2-way with trailing blank/excluded row/col/sub stays balanced OLS; 3-way trailing blank row matches trimmed.
    let build_rm = |pad: bool| {
        let mut g =
            GroupedTable::new(SubcolumnMode::RawReplicates(if pad { 4 } else { 3 }), if pad { 4 } else { 3 }, if pad { 3 } else { 2 }).unwrap();
        let vals = [[[5.0, 6.0, 7.0], [8.0, 9.5, 10.5], [12.0, 13.0, 14.5]], [[7.0, 8.2, 9.1], [11.0, 12.0, 13.4], [15.5, 16.5, 17.8]]];
        for r in 0..2 {
            for c in 0..3 {
                for s in 0..3 {
                    g.data_sets[c].cells[r][s] = Cell::number(vals[r][c][s]);
                }
            }
        }
        if pad {
            g.data_sets[3].cells[0][0] = Cell::excluded_number(50.0);
            g.data_sets[0].cells[2][1] = Cell::excluded_number(60.0);
            g.data_sets[1].cells[0][3] = Cell::excluded_number(70.0);
        }
        DataSheet::new("grm", "GRM", TableArchetype::Grouped(g)).unwrap()
    };
    let rm_k = AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        fit_reml_on_missing: false,
        ..Default::default()
    });
    let (rm_t, rm_p) = (run_kind(&build_rm(false), rm_k.clone()).unwrap(), run_kind(&build_rm(true), rm_k).unwrap());
    for sc in &rm_t.scalars {
        assert_close(scalar(&rm_p, &sc.key), sc.value, 1e-12, &sc.key);
    }

    let build_mv = |pad: bool| {
        let base = [
            ("Low", "Ctrl", 10.0),
            ("Low", "Ctrl", 11.0),
            ("Low", "Drug", 15.0),
            ("Low", "Drug", 16.5),
            ("High", "Ctrl", 20.0),
            ("High", "Ctrl", 21.0),
            ("High", "Drug", 28.0),
            ("High", "Drug", 29.5),
            ("Low", "Ctrl", 12.0),
        ];
        let mut mv = MultipleVariablesTable::new(3, base.len() + if pad { 2 } else { 0 }).unwrap();
        (mv.variables[1].role, mv.variables[2].role) = (VariableRole::Categorical, VariableRole::Categorical);
        for (r, &(a, b, y)) in base.iter().enumerate() {
            (mv.variables[0].cells[r], mv.variables[1].cells[r], mv.variables[2].cells[r]) = (Cell::number(y), Cell::text(a), Cell::text(b));
        }
        if pad {
            mv.variables[0].cells[base.len()] = Cell::number(99.0);
            mv.variables[1].cells[base.len()] = Cell { value: CellValue::Text("Ghost".into()), excluded: true };
            mv.variables[2].cells[base.len()] = Cell::text("Ctrl");
            mv.variables[0].cells[base.len() + 1] = Cell::excluded_number(88.0);
            mv.variables[1].cells[base.len() + 1] = Cell::text("High");
            mv.variables[2].cells[base.len() + 1] = Cell::text("Ghost2");
        }
        DataSheet::new("mvf", "MVF", TableArchetype::MultipleVariables(mv)).unwrap()
    };
    let mvk = AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        response_column: Some(0),
        multifactor_columns: vec![1, 2],
        ss_type: SumOfSquaresType::TypeIII,
        ..Default::default()
    });
    let (mvt, mvp) = (run_kind(&build_mv(false), mvk.clone()).unwrap(), run_kind(&build_mv(true), mvk).unwrap());
    for sc in &mvt.scalars {
        assert_close(scalar(&mvp, &sc.key), sc.value, 1e-12, &sc.key);
    }

    // 2. Confidence level propagates to raw one-way effect-size CI (#31) matching summary path.
    let (g1, g2, g3) = ([5.1, 5.4, 5.8, 6.0, 5.7], [7.2, 7.6, 7.9, 8.1, 7.5], [9.0, 9.4, 9.8, 10.1, 9.6]);
    let ow_sh = col_sheet("ow31", &[("A", &g1), ("B", &g2), ("C", &g3)]);
    let widths: Vec<f64> = [90.0, 95.0, 99.0]
        .into_iter()
        .map(|pct| {
            let r =
                run_kind_conf(&ow_sh, AnalysisKind::OneWayAnova(OneWayAnovaSpec { compute_effect_sizes: true, ..Default::default() }), pct).unwrap();
            let t = tab_by_id(&r, "anova-table");
            let col = t.column_headers.iter().position(|h| h.contains(&format!("{pct}% CI"))).unwrap();
            let (lo, hi) = cell_ci_finite(&t.rows[0][col]);
            let c = ConfidenceLevel::from_percent(pct).unwrap();
            let raw = ordinary_one_way_anova(&[("A", &g1), ("B", &g2), ("C", &g3)], c).unwrap();
            let sum_in: Vec<_> =
                (0..3).map(|i| group_input(i, &raw.group_labels[i], raw.group_n[i] as f64, raw.group_means[i], raw.group_sds[i])).collect();
            let (slo, shi) = ordinary_one_way_from_summary(&sum_in, c).unwrap().effect_sizes.partial_eta_squared_ci.unwrap();
            assert_close(lo, slo, 1e-12, "CI lo");
            assert_close(hi, shi, 1e-12, "CI hi");
            hi - lo
        })
        .collect();
    assert!(widths[0] < widths[1] && widths[1] < widths[2]);
}

#[test]
fn audit_33_34_35_n1_group_welch_rm_and_friedman_posthoc() {
    // 3. n == 1 group keeps F = 27 and marks variance homogeneity unavailable; all-n1 KW H = 2 (#33).
    let n1_sh = col_sheet("n1", &[("S", &[5.0]), ("G2", &[1.0, 2.0, 3.0]), ("G3", &[7.0, 8.0, 9.0])]);
    let r_n1 = run_kind(&n1_sh, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    assert_close(scalar(&r_n1, "F"), 27.0, 1e-12, "n==1 F");
    assert!(matches!(&tab_by_id(&r_n1, "variance-homogeneity").rows[0][4], ResultCell::Text(s) if s.contains("n < 2")));
    let kw_s = run_kind(
        &col_sheet("kw1", &[("1", &[1.0]), ("2", &[2.0]), ("3", &[3.0])]),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..Default::default() }),
    )
    .unwrap();
    assert_close(scalar(&kw_s, "Kruskal-Wallis H"), 2.0, 1e-12, "all-n1 KW H");

    // 4. Welch/BF post-tests (#34) & RM/Friedman post-hoc contracts (#35).
    let w_sh = col_sheet(
        "w34",
        &[("Ctrl", &[10.0, 10.2, 10.5, 9.8, 10.1]), ("Med", &[13.0, 14.5, 11.5, 15.0, 12.5]), ("High", &[20.0, 26.0, 14.0, 29.0, 17.0])],
    );
    let run_w = |method| {
        run_kind(
            &w_sh,
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::BrownForsytheAndWelch,
                multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
                ..Default::default()
            }),
        )
    };
    assert!(run_w(MultiplicityMethod::None).unwrap().pairwise_comparisons.is_empty());
    assert_eq!(run_w(MultiplicityMethod::Dunnett { control_column: 0 }).unwrap().pairwise_comparisons.len(), 2);
    assert!(run_w(MultiplicityMethod::BenjaminiHochbergFdr).unwrap().pairwise_comparisons.iter().all(|c| c.is_fdr_q));
    assert!(run_w(MultiplicityMethod::LinearTrend).unwrap().tabs.iter().any(|t| t.id == "linear-trend"));
    assert!(run_w(MultiplicityMethod::TamhaneT2).unwrap().tabs.iter().any(|t| t.title.contains("Tamhane's T2")));
    assert!(
        run_w(MultiplicityMethod::TukeyHsd)
            .unwrap()
            .checklist
            .iter()
            .any(|c| c.assumption == "Post-hoc multiplicity procedure" && c.status == ChecklistStatus::Info)
    );
    assert!(matches!(run_w(MultiplicityMethod::DunnRankSum), Err(StatsError::IncompatibleAnalysisSpec { .. })));

    let rm_sh = col_sheet_hint(
        "rm35",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("T0", &[1.0, 2.0, 4.0, 5.0, 6.0, 7.0]), ("T1", &[3.0, 4.5, 6.0, 7.5, 8.0, 9.0]), ("T2", &[6.0, 7.0, 9.5, 11.0, 12.0, 13.0])],
    );
    for mixed_effects_reml in [false, true] {
        for bad in [MultiplicityMethod::GamesHowell, MultiplicityMethod::TamhaneT2, MultiplicityMethod::DunnettT3, MultiplicityMethod::DunnRankSum] {
            let ow = OneWayAnovaSpec {
                method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: !mixed_effects_reml },
                mixed_effects_reml,
                multiplicity: MultiplicitySpec { method: bad, threshold: 0.05, report_confidence_intervals: true },
                ..Default::default()
            };
            assert!(matches!(run_kind(&rm_sh, AnalysisKind::OneWayAnova(ow)), Err(StatsError::IncompatibleAnalysisSpec { .. })));
        }
    }
    let gg = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    assert_close(cell_num(&tab_by_id(&gg, "multiple-comparisons").rows[0][3]), 5.0, 1e-12, "GG df=n-1");
    let fr = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::Friedman,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::DunnRankSum, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(fr.pairwise_comparisons.iter().find(|p| p.label_a == "T0" && p.label_b == "T2").unwrap().significant);
    let fr_t = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::Friedman,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(fr_t.pairwise_comparisons.is_empty());

    // 5. Summary ANOVA: fractional/non-finite n (#36), homogeneity not computable (R1-S02-08), RM/Friedman reject pre-summarized (FA-SEC06-01).
}

#[test]
fn audit_36_summary_anova_malformed_factorial_and_three_way_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;
    for bad_n in [4.5, f64::NAN, f64::INFINITY] {
        let g = vec![group_input(0, "A", bad_n, 10.0, 1.5), group_input(1, "B", 5.0, 12.0, 1.5)];
        assert!(matches!(ordinary_one_way_from_summary(&g, conf), Err(StatsError::InvalidParameter { name: "group_n", .. })));
        assert!(matches!(unequal_variance_one_way_anova(&g), Err(StatsError::InvalidParameter { name: "group_n", .. })));
    }
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let psh = presum_col_sheet("pre", mode, &[("G0", &[&[10.0, 1.5, 5.0]]), ("G1", &[&[13.0, 3.5, 5.0]]), ("G2", &[&[15.0, 2.0, 5.0]])]);
        for m in [
            OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
            OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            OneWayMethod::Friedman,
        ] {
            assert!(matches!(
                run_kind(&psh, AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: m, ..Default::default() })),
                Err(StatsError::RequiresRawReplicates)
            ));
        }
        if mode == SubcolumnMode::MeanSdN {
            let out = run_kind(&psh, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
            assert!(out.checklist.iter().any(|c| c.assumption.contains("Homogeneity") && c.status == ChecklistStatus::Info));
            assert_eq!(tab_by_id(&out, "variance-homogeneity").rows[0][0], ResultCell::Empty);
        }
    }
}

#[test]
fn audit_fa_sec06_03_malformed_factorial_three_way_and_zero_residual_rm() {
    // 6. Malformed FactorialObservations (FA-SEC06-03), ragged MV (R1-S05-01), three_way column counts (RV-91), linear-trend R^2 (RV-92), zero-residual RM (R1-S02-07), selected_data_sets (R1-X2-05).
    for (obs, is_dim) in [
        (
            FactorialObservations {
                factor_names: vec!["A".into(), "B".into()],
                factor_levels: vec![vec!["A1".into(), "A2".into()]],
                rows: vec![(vec![0, 0], 1.0), (vec![0, 1], 2.0), (vec![1, 0], 3.0)],
            },
            true,
        ),
        (
            FactorialObservations {
                factor_names: vec!["A".into(), "B".into()],
                factor_levels: vec![vec!["A1".into(), "A2".into()], vec!["B1".into(), "B2".into()]],
                rows: vec![(vec![0, 0], 1.0), (vec![0], 2.0), (vec![1, 1], 3.0)],
            },
            true,
        ),
        (
            FactorialObservations {
                factor_names: vec!["A".into(), "B".into()],
                factor_levels: vec![vec!["A1".into(), "A2".into()], vec!["B1".into(), "B2".into()]],
                rows: vec![(vec![0, 0], 1.0), (vec![0, 5], 2.0), (vec![1, 1], 3.0)],
            },
            false,
        ),
    ] {
        let err = fit_factorial_anova(&obs, 2, SumOfSquaresType::TypeIII).unwrap_err();
        assert!(
            if is_dim {
                matches!(err, StatsError::DimensionMismatch { expected: 2, actual: 1 })
            } else {
                matches!(err, StatsError::InvalidParameter { name: "factor_level_index", .. })
            },
            "unexpected error for malformed FactorialObservations: {err:?}"
        );
    }

    let rm_z = rm_one_way_anova(&[vec![1.0, 2.0, 4.0], vec![2.0, 3.0, 5.0], vec![3.0, 4.0, 6.0], vec![4.0, 5.0, 7.0]]).unwrap();
    assert!(rm_z.f_treatment.is_infinite() && rm_z.p_uncorrected == 0.0 && rm_z.p_greenhouse_geisser == 0.0 && rm_z.p_huynh_feldt == 0.0);
    assert_eq!(rm_one_way_anova(&vec![vec![5.0; 3]; 3]).unwrap().p_uncorrected, 1.0);

    for n_cols in [3_usize, 5, 6] {
        let sh = build_grouped_grid("g3w-bad", 2, n_cols, 2);
        for rm in [RepeatedMeasuresDesign::None, RepeatedMeasuresDesign::BothFactorsMatched] {
            let err = run_kind(
                &sh,
                AnalysisKind::FactorialAnova(FactorialAnovaSpec {
                    three_way: true,
                    max_interaction_order: 3,
                    repeated_measures: rm,
                    ..Default::default()
                }),
            )
            .unwrap_err();
            assert!(matches!(err, StatsError::InvalidParameter { name: "three_way_column_count", value } if (value - n_cols as f64).abs() < 1e-12));
        }
    }
    let mut g4 = GroupedTable::new(SubcolumnMode::RawReplicates(4), 4, 2).unwrap();
    g4.three_way_grouping = Some(ThreeWayColumnGrouping {
        factor_1_name: "Drug".into(),
        factor_1_levels: ["Veh".into(), "Act".into()],
        factor_2_name: "Sex".into(),
        factor_2_levels: ["F".into(), "M".into()],
    });
    for c in 0..4 {
        for r in 0..2 {
            for s in 0..4 {
                if !(c == 3 && r == 1 && s == 3) {
                    g4.data_sets[c].cells[r][s] = Cell::number(12.0 + (c as f64) * 2.2 + (r as f64) * 3.1 + (s as f64) * 0.7);
                }
            }
        }
    }
    let r3w = run_kind(
        &DataSheet::new("g4-3w", "3W", TableArchetype::Grouped(g4)).unwrap(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            three_way: true,
            max_interaction_order: 3,
            repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(tab_by_id(&r3w, "factorial-anova").title, "Three-way ANOVA");
    assert!(r3w.tabs.iter().any(|t| t.id == "mixed-reml"));

    // Linear trend R^2 in [0, 1] across one-way, RM, and Nested (RV-92).
    let lt_sh = col_sheet("lt", &[("D1", &[10.0, 12.0, 11.0, 13.0]), ("D2", &[25.0, 27.0, 26.0, 28.0]), ("D3", &[42.0, 40.0, 43.0, 41.0])]);
    let r_lt = run_kind(
        &lt_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    let lt_t = tab_by_id(&r_lt, "linear-trend");
    let r2_ow = cell_num(&lt_t.rows[0][lt_t.column_headers.iter().position(|h| h == "R squared").unwrap()]);
    let aov_t = tab_by_id(&r_lt, "anova-table");
    let dt = linear_trend_test(
        &[
            group_input_slice(0, "D1", &[10.0, 12.0, 11.0, 13.0]),
            group_input_slice(1, "D2", &[25.0, 27.0, 26.0, 28.0]),
            group_input_slice(2, "D3", &[42.0, 40.0, 43.0, 41.0]),
        ],
        cell_num(&aov_t.rows[1][2]),
        cell_num(&aov_t.rows[1][1]),
    )
    .unwrap();
    assert!(dt.ss_trend > 100.0 && (0.0..=1.0).contains(&r2_ow));
    assert_approx_eq(r2_ow, dt.ss_trend / cell_num(&aov_t.rows[2][0]), 1e-12, 1e-12, "linear-trend R2");
    let nt_lt = nested_sheet(
        "nt-lt",
        &[
            ("G1", &[&[10.0, 10.4, 9.8], &[10.6, 10.2, 10.5], &[9.9, 10.3, 10.1]]),
            ("G2", &[&[20.0, 20.3, 19.7], &[20.5, 20.1, 20.4], &[19.8, 20.2, 20.0]]),
            ("G3", &[&[30.0, 30.2, 29.9], &[30.6, 30.1, 30.4], &[29.7, 30.3, 30.0]]),
        ],
    );
    let r_nt_lt = run_kind(
        &nt_lt,
        AnalysisKind::NestedComparison(NestedAnalysisSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    let nt_t = tab_by_id(&r_nt_lt, "linear-trend");
    let r2_nt = cell_num(&nt_t.rows[0][nt_t.column_headers.iter().position(|h| h == "R squared").unwrap()]);
    assert!((0.9..=1.0).contains(&r2_nt));

    // selected_data_sets skips poison middle column in Factorial, Nested, and Contingency (R1-X2-05).
    let g_full = grouped_sheet_fill("gf", 3, 2, 2, |c, r, s| {
        Some(if c == 1 { 1e7 * if r == 0 { 1.0 } else { -1.0 } } else { 10.0 + (c as f64) * 5.0 + (r as f64) * 4.0 + (s as f64) * 2.0 })
    });
    let g_two = grouped_sheet_fill("gt", 2, 2, 2, |c, r, s| Some(10.0 + (if c == 1 { 10.0 } else { 0.0 }) + (r as f64) * 4.0 + (s as f64) * 2.0));
    let r_sel = run_kind_sel(&g_full, AnalysisKind::FactorialAnova(FactorialAnovaSpec::default()), vec![0, 2]).unwrap();
    let r_two = run_kind(&g_two, AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())).unwrap();
    assert_eq!(r_sel.scalars, r_two.scalars);
    assert_close(cell_num(&r_sel.tabs[0].rows[1][1]), 1.0, 1e-12, "selected df_col == 1");
    let n_full = nested_sheet(
        "nf",
        &[("G0", &[&[10.0, 11.0], &[12.0, 13.0]]), ("P", &[&[1e6, 1.5e6], &[2e6, 2.5e6]]), ("G2", &[&[20.0, 21.0], &[22.0, 23.0]])],
    );
    let n_two = nested_sheet("nt", &[("G0", &[&[10.0, 11.0], &[12.0, 13.0]]), ("G2", &[&[20.0, 21.0], &[22.0, 23.0]])]);
    let n_k = AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, ..Default::default() });
    assert_eq!(scalar(&run_kind_sel(&n_full, n_k.clone(), vec![0, 2]).unwrap(), "Nested t"), scalar(&run_kind(&n_two, n_k).unwrap(), "Nested t"));
    let ct_full = contingency_sheet("ctf", &[&[30, 999, 12], &[10, 1, 28]]);
    let ct_ref = contingency_sheet("ctr", &[&[30, 12], &[10, 28]]);
    assert_eq!(
        scalar(&run_kind_sel(&ct_full, AnalysisKind::Contingency(ContingencySpec::default()), vec![0, 2]).unwrap(), "Chi-square"),
        scalar(&run_kind(&ct_ref, AnalysisKind::Contingency(ContingencySpec::default())).unwrap(), "Chi-square")
    );

    // Ragged MV rows return DimensionMismatch (R1-S05-01).
    let mut mv_rag = MultipleVariablesTable::new(3, 4).unwrap();
    (mv_rag.variables[0].role, mv_rag.variables[1].role) = (VariableRole::Categorical, VariableRole::Categorical);
    for r in 0..4 {
        mv_rag.variables[0].cells[r] = Cell::text(if r < 2 { "A" } else { "B" });
        mv_rag.variables[1].cells[r] = Cell::text(if r % 2 == 0 { "X" } else { "Y" });
        mv_rag.variables[2].cells[r] = Cell::number((r + 1) as f64);
    }
    let mut rag_sh = DataSheet::new("rag", "Rag", TableArchetype::MultipleVariables(mv_rag)).unwrap();
    if let TableArchetype::MultipleVariables(t) = &mut rag_sh.archetype {
        t.variables[1].cells.pop();
    }
    assert!(matches!(
        run_kind(
            &rag_sh,
            AnalysisKind::FactorialAnova(FactorialAnovaSpec { multifactor_columns: vec![0, 1], response_column: Some(2), ..Default::default() })
        ),
        Err(StatsError::DimensionMismatch { expected: 4, actual: 3 })
    ));
}

#[test]
fn audit_37_109_reml_boundaries_collinearity_sphericity_and_nested() {
    // 1. RI REML large tau (eps=0.01, 0.001) + zero/ceiling boundary flags (#37).
    for eps in [0.01_f64, 0.001] {
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut cids = Vec::new();
        let means = [[1.0, 2.0, 3.0], [1.5, 2.5, 3.5]];
        let mut nt_data = [[Vec::new(), Vec::new(), Vec::new()], [Vec::new(), Vec::new(), Vec::new()]];
        for g in 0..2 {
            for b in 0..3 {
                for off in [-eps, 0.0, eps] {
                    let v = means[g][b] + off;
                    nt_data[g][b].push(v);
                    x.extend_from_slice(&[1.0, g as f64]);
                    y.push(v);
                    cids.push(g * 3 + b);
                }
            }
        }
        let reml = fit_random_intercept_reml(&DenseMatrix::from_row_major(18, 2, x).unwrap(), &y, &cids, &[("Group", 1, 1)]).unwrap();
        assert!(reml.converged && !reml.at_boundary);
        assert_close(reml.fixed_terms[0].f_statistic.unwrap(), 0.375, 1e-6, "REML F");
        assert_close(reml.fixed_terms[0].p_value.unwrap(), 0.573_399, 1e-4, "REML P");
        let ns = nested_sheet(
            "neps",
            &[("G0", &[&nt_data[0][0], &nt_data[0][1], &nt_data[0][2]]), ("G1", &[&nt_data[1][0], &nt_data[1][1], &nt_data[1][2]])],
        );
        assert_close(
            scalar(&run_kind(&ns, AnalysisKind::NestedComparison(NestedAnalysisSpec::default())).unwrap(), "F (among groups)"),
            0.375,
            1e-6,
            "Nested F",
        );
    }
    let x18 = DenseMatrix::from_row_major(18, 2, (0..18).flat_map(|i| [1.0, (i / 9) as f64]).collect()).unwrap();
    let cid18: Vec<usize> = (0..18).map(|i| i / 3).collect();
    let y_zero: Vec<f64> = (0..18).map(|i| ((i / 9) as f64) * 2.0 + [-1.0, 0.0, 1.0][i % 3]).collect();
    let r_z = fit_random_intercept_reml(&x18, &y_zero, &cid18, &[("G", 1, 1)]).unwrap();
    assert!(r_z.at_boundary && r_z.converged && r_z.sigma2_random.abs() < 1e-12);
    let y_ceil: Vec<f64> = (0..18).map(|i| [1.0, 2.0, 3.0][(i / 3) % 3] + 0.5 * ((i / 9) as f64)).collect();
    let r_c = fit_random_intercept_reml(&x18, &y_ceil, &cid18, &[("G", 1, 1)]).unwrap();
    assert!(r_c.at_boundary && !r_c.converged);

    // 2. Public REML invalid/overflowing term ranges (FA-SEC06-04).
    let x_f = DenseMatrix::from_row_major(6, 3, vec![1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0])
        .unwrap();
    let y6 = [10.0, 11.2, 12.1, 13.4, 10.5, 13.0];
    let (c6, t6) = ([0, 0, 1, 1, 2, 2], [0, 1, 0, 1, 0, 1]);
    let g6 = DenseMatrix::from_row_major(6, 6, (0..36).map(|i| if c6[i / 6] == c6[i % 6] { 1.0 } else { 0.0 }).collect()).unwrap();
    let lbls = vec!["T0".into(), "T1".into()];
    for bad_t in [[("T", 4, 1)], [("T", usize::MAX, 1)], [("T", 1, usize::MAX)]] {
        assert!(matches!(fit_random_intercept_reml(&x_f, &y6, &c6, &bad_t), Err(StatsError::InvalidParameter { name: "terms", .. })));
        assert!(matches!(
            fit_random_intercept_reml_with_df_method(&x_f, &y6, &c6, &bad_t, MixedModelDfMethod::Satterthwaite),
            Err(StatsError::InvalidParameter { name: "terms", .. })
        ));
        assert!(matches!(fit_multi_vc_reml(&x_f, &y6, &[("S", g6.clone(), 3)], &bad_t), Err(StatsError::InvalidParameter { name: "terms", .. })));
        assert!(matches!(
            fit_multi_vc_reml_with_df_method(&x_f, &y6, &[("S", g6.clone(), 3)], &bad_t, MixedModelDfMethod::Satterthwaite),
            Err(StatsError::InvalidParameter { name: "terms", .. })
        ));
        assert!(matches!(fit_unstructured_rm_reml(&x_f, &y6, &c6, &t6, &lbls, &bad_t), Err(StatsError::InvalidParameter { name: "terms", .. })));
        assert!(matches!(
            fit_unstructured_rm_reml_with_df_method(&x_f, &y6, &c6, &t6, &lbls, &bad_t, MixedModelDfMethod::Satterthwaite),
            Err(StatsError::InvalidParameter { name: "terms", .. })
        ));
    }

    // 3. All-missing-cell CS RM REML collinearity (FA-SEC06-05), Multi-VC diagnostics (FA-SEC06-06), & #109 fit_reml_on_missing=false.
    let miss_cell = grouped_sheet_fill("mc", 2, 2, 4, |c, r, s| match (c, r) {
        (0, 0) => Some([10.0, 10.4, 9.8, 10.2][s]),
        (0, 1) => Some([14.0, 14.3, 13.7, 14.1][s]),
        (1, 0) => Some([12.5, 12.9, 12.1, 12.7][s]),
        _ => None,
    });
    for design in [RepeatedMeasuresDesign::ColumnFactorMatched, RepeatedMeasuresDesign::RowFactorMatched, RepeatedMeasuresDesign::BothFactorsMatched]
    {
        let f = FactorialAnovaSpec {
            repeated_measures: design,
            residual_covariance: ResidualCovariance::CompoundSymmetry,
            fit_reml_on_missing: true,
            ..Default::default()
        };
        let res = run_factorial_anova(&miss_cell, &spec_for("r", "mc", AnalysisKind::FactorialAnova(f.clone())), &f).unwrap();
        assert_eq!(res.status, ResultStatus::UpToDate);
    }
    let rm_miss = grouped_sheet_fill("rm109", 2, 3, 3, |c, r, s| {
        if r == 1 && c == 1 && s == 2 { None } else { Some(10.0 + (r as f64) * 2.0 + (c as f64) * 5.0 + (s as f64)) }
    });
    let rm_full = grouped_sheet_fill("rm109f", 2, 3, 3, |c, r, s| Some(10.0 + (r as f64) * 2.0 + (c as f64) * 5.0 + (s as f64)));
    let no_reml = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        fit_reml_on_missing: false,
        mixed_effects_reml: false,
        ..Default::default()
    };
    assert!(matches!(run_kind(&rm_miss, AnalysisKind::FactorialAnova(no_reml.clone())), Err(StatsError::InsufficientSampleSize { .. })));
    for (sh, fac, needle) in [
        (&rm_miss, FactorialAnovaSpec { fit_reml_on_missing: true, ..no_reml.clone() }, "Compound Symmetry"),
        (
            &rm_miss,
            FactorialAnovaSpec { fit_reml_on_missing: true, residual_covariance: ResidualCovariance::Unstructured, ..no_reml.clone() },
            "Unstructured",
        ),
        (&rm_full, no_reml, "Classical balanced"),
    ] {
        let r = run_kind(sh, AnalysisKind::FactorialAnova(fac)).unwrap();
        assert!(
            r.checklist
                .iter()
                .find(|c| c.assumption == "Repeated-measures estimator")
                .unwrap()
                .diagnostic_metric
                .as_deref()
                .unwrap()
                .contains(needle)
        );
    }
}

#[test]
fn audit_rv29_30_sphericity_and_nested_reml_guards() {
    // 4. Lecoutre (1991) split-plot HF (RV-29) & n=2 rank-1 1/(k-1) lower bound (RV-30).
    let base: [[f64; 4]; 4] = [[1.0, 2.0, 3.0, 11.0], [2.0, 3.0, 2.0, 15.0], [0.0, 1.0, 4.0, 8.0], [3.0, 2.0, 3.0, 16.0]];
    let mut sp_data = Vec::new();
    for s in 0..4 {
        let g1: Vec<f64> = (0..4).map(|t| base[s][t] + 5.0 + 0.5 * (t as f64) + if s % 2 == 0 { 0.3 } else { -0.3 } * (t as f64)).collect();
        let g2: Vec<f64> = (0..4).map(|t| base[s][t] + 10.0 + (t as f64) + if s < 2 { 0.2 } else { -0.2 } * ((t * t) as f64)).collect();
        sp_data.push(vec![base[s].to_vec(), g1, g2]);
    }
    let sph = fit_rm_two_way_anova(&sp_data, "Group", "Time", RepeatedMeasuresDesign::ColumnFactorMatched).unwrap().1.unwrap();
    let exp_lec =
        (((10.0 * 3.0 * sph.greenhouse_geisser_eps) - 2.0) / (3.0 * (9.0 - 3.0 * sph.greenhouse_geisser_eps))).clamp(sph.greenhouse_geisser_eps, 1.0);
    assert_approx_eq(sph.huynh_feldt_eps, exp_lec, 1e-12, 1e-12, "Lecoutre 1991 HF");

    for subjs in [vec![vec![10.0, 12.0, 15.0, 22.0], vec![11.0, 15.0, 14.0, 30.0]], vec![vec![10.0, 12.0, 15.0, 22.0], vec![20.0, 22.0, 25.0, 32.0]]]
    {
        let rm = rm_one_way_anova(&subjs).unwrap();
        assert_approx_eq(rm.sphericity.greenhouse_geisser_eps, 1.0 / 3.0, 1e-12, 1e-12, "n=2 GG 1/3");
        assert_approx_eq(rm.sphericity.huynh_feldt_eps, 1.0 / 3.0, 1e-12, 1e-12, "n=2 HF 1/3");
        assert!(rm.sphericity.mauchly_p.is_nan());
    }
    let tw2 =
        vec![vec![vec![10.0, 12.0, 16.0, 25.0], vec![14.0, 18.0, 21.0, 33.0]], vec![vec![11.0, 15.0, 15.0, 31.0], vec![16.0, 17.0, 26.0, 40.0]]];
    let f2 =
        fit_rm_two_way_anova_with_sphericity(&tw2, "R", "C", RepeatedMeasuresDesign::BothFactorsMatched, RmSphericityCorrection::GreenhouseGeisser)
            .unwrap();
    assert_approx_eq(f2.sphericities.row.unwrap().greenhouse_geisser_eps, 1.0, 1e-12, 1e-12, "2-lvl sph=1");
    assert_approx_eq(f2.sphericities.column.unwrap().greenhouse_geisser_eps, 1.0 / 3.0, 1e-12, 1e-12, "4-lvl n=2 sph=1/3");

    // 5. Nested error variants & ignored-text disclosure (FA-SEC06-10).
    let nt3 = nested_table(&[("A", &[&[10.0, 10.1], &[11.0, 11.1]]), ("B", &[&[13.0, 13.1], &[14.0, 14.1]]), ("C", &[&[16.0, 16.1], &[17.0, 17.1]])]);
    assert!(matches!(nested_one_way_anova(&nt3, true), Err(StatsError::InvalidParameter { name: "nested_t_test_groups", value }) if value == 3.0));
    let nt1 = nested_table(&[("A", &[&[10.0, 10.1], &[11.0, 11.1]])]);
    assert!(matches!(
        nested_one_way_anova(&nt1, true),
        Err(StatsError::InsufficientSampleSize { context: "nested ANOVA groups", required: 2, actual: 1 })
    ));
    let mut nt2 = nested_table(&[
        ("A", &[&[10.0, 10.2, 10.4, 10.5], &[11.0, 11.2, 11.4, 11.5]]),
        ("B", &[&[14.0, 14.2, 14.4, 14.5], &[15.0, 15.2, 15.4, 15.5]]),
    ]);
    nt2.groups[0].cells[3][1] = Cell::text("ND");
    let r_nt2 = run_kind(
        &DataSheet::new("nt2nd", "NT2", TableArchetype::Nested(nt2)).unwrap(),
        AnalysisKind::NestedComparison(NestedAnalysisSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(r_nt2.tabs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["nested-anova", "variance-components"]);
    assert!(r_nt2.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
}

#[test]
fn audit_ls_means_simple_effects_ci_confidence_and_dunnett_covariance() {
    // 1. Unbalanced 2x2 Type III LS-means & Games-Howell R oracle (R2-S02-01).
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(9), 2, 2).unwrap();
    (gt.row_factor_title, gt.column_factor_title) = ("Factor A".into(), "Factor B".into());
    for (s, &v) in [8.0, 9.0, 10.0, 11.0, 12.0, 9.0, 10.0, 11.0, 10.0].iter().enumerate() {
        gt.data_sets[0].cells[0][s] = Cell::number(v);
    }
    gt.data_sets[1].cells[0][0] = Cell::number(30.0);
    gt.data_sets[0].cells[1][0] = Cell::number(10.0);
    for (s, &v) in [28.0, 29.0, 30.0, 31.0, 32.0, 29.0, 30.0, 31.0, 30.0].iter().enumerate() {
        gt.data_sets[1].cells[1][s] = Cell::number(v);
    }
    let sh_u = DataSheet::new("g-u", "U", TableArchetype::Grouped(gt)).unwrap();
    let rt = run_kind(
        &sh_u,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ss_type: SumOfSquaresType::TypeIII,
            ..Default::default()
        }),
    )
    .unwrap();
    let ta = tab_by_id(&rt, "multiple-comparisons-factor-a");
    assert_close(cell_num(&ta.rows[0][0]), 0.0, 1e-12, "LS-mean diff");
    assert_close(cell_num(&ta.rows[0][1]), 0.912_870_929_175_278, 1e-10, "LS-mean SE");
    assert_close(cell_num(&ta.rows[0][3]), 16.0, 1e-12, "LS-mean df");
    assert!(!cell_bool(&ta.rows[0][7]));
    let rgh = run_kind(
        &sh_u,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::GamesHowell, threshold: 0.05, report_confidence_intervals: true },
            ss_type: SumOfSquaresType::TypeIII,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_close(cell_num(&tab_by_id(&rgh, "multiple-comparisons-factor-a").rows[0][3]), 38.554_216_867_469_9, 1e-8, "GH Welch df");

    // 2. Split-plot R aov + REML KR simple-effects oracles & non-colliding IDs (R2-S02-01).
    let subj_u = [-12.0, -5.0, 1.0, 6.0, 10.0, -9.0, -3.0, 0.0, 4.0, 8.0];
    let sh_sp = grouped_sheet_fill("sp", 4, 2, 5, |b, a, s| {
        Some(10.0 + (a as f64) * 4.0 + (b as f64) * 1.5 + subj_u[a * 5 + s] + 0.3 * ((((a + 1) * (b + 1) + (s + 1)) % 3) as f64 - 1.0))
    });
    let res_sp = run_kind(
        &sh_sp,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    let sp_a = tab_by_id(&res_sp, "multiple-comparisons-factor-a");
    assert_close(cell_num(&sp_a.rows[0][0]), -3.985, 1e-10, "SP margA diff");
    assert_close(cell_num(&sp_a.rows[0][1]), 4.874_106_328_343_69, 1e-8, "SP margA SE");
    assert_close(cell_num(&sp_a.rows[0][2]), -0.817_585_775_022_306, 1e-8, "SP margA t");
    assert_close(cell_num(&sp_a.rows[0][3]), 8.0, 1e-12, "SP margA df");
    assert_close(cell_p(&sp_a.rows[0][4]), 0.437_264_297_103_214, 1e-8, "SP margA P");
    let sp_sa = tab_by_id(&res_sp, "multiple-comparisons-simple-a-within-b");
    assert_eq!(sp_sa.rows.len(), 4);
    assert_close(cell_num(&sp_sa.rows[0][0]), -3.94, 1e-10, "SP A|B1 diff");
    assert_close(cell_num(&sp_sa.rows[0][1]), 4.876_909_882_292_27, 1e-8, "SP A|B1 SE");
    assert_close(cell_num(&sp_sa.rows[0][3]), 8.018_418_537_404_94, 1e-8, "SP A|B1 df");
    assert_close(cell_p(&sp_sa.rows[0][5]), (4.0 * cell_p(&sp_sa.rows[0][4])).min(1.0), 1e-12, "SP A|B m=4");
    let mut ids = BTreeMap::new();
    for rec in &res_sp.pairwise_comparisons {
        for (id, lbl) in [(rec.group_a, &rec.label_a), (rec.group_b, &rec.label_b)] {
            if let Some(prev) = ids.insert(id, lbl.clone()) {
                assert_eq!(&prev, lbl);
            }
        }
    }

    let reml_pts: [(usize, usize, usize, f64); 22] = [
        (0, 0, 0, 1.32742922414),
        (1, 0, 0, 7.72378701262),
        (2, 0, 0, 14.870449977),
        (3, 0, 0, 17.0846100697),
        (0, 0, 1, 9.15514528219),
        (1, 0, 1, 16.0580779843),
        (2, 0, 1, 17.5530994472),
        (3, 0, 1, 18.4819265185),
        (0, 1, 0, 3.17577657773),
        (2, 1, 0, 16.4688981569),
        (3, 1, 0, 19.4317765925),
        (0, 1, 1, 16.4809257407),
        (1, 1, 1, 21.1328192591),
        (2, 1, 1, 23.3329906383),
        (3, 1, 1, 29.1442957642),
        (0, 2, 0, 6.59742057388),
        (1, 2, 0, 9.64005941204),
        (2, 2, 0, 17.8416270819),
        (3, 2, 0, 20.4326503107),
        (0, 2, 1, 22.7186115528),
        (1, 2, 1, 28.7384301024),
        (2, 2, 1, 30.768794662),
    ];
    let mut g_reml = GroupedTable::new(SubcolumnMode::RawReplicates(4), 3, 2).unwrap();
    for (s, c, r, y) in reml_pts {
        g_reml.data_sets[c].cells[r][s] = Cell::number(y);
    }
    let res_reml = run_kind(
        &DataSheet::new("rml", "RML", TableArchetype::Grouped(g_reml)).unwrap(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            fit_reml_on_missing: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    let r_a = tab_by_id(&res_reml, "multiple-comparisons-factor-a");
    assert_close(cell_num(&r_a.rows[0][0]), -10.335_587_215_427_3, 1e-5, "REML KR diff");
    assert_close(cell_num(&r_a.rows[0][1]), 4.212_796_619_268_27, 1e-4, "REML KR SE");
    assert_close(cell_num(&r_a.rows[0][3]), 5.999_852_640_602_67, 1e-4, "REML KR df");
    let r_ab = tab_by_id(&res_reml, "multiple-comparisons-simple-a-within-b");
    assert_close(cell_num(&r_ab.rows[2][0]), -15.382_837_892_421_5, 1e-5, "REML KR A|T3 diff");
    assert_close(cell_num(&r_ab.rows[2][1]), 4.278_656_668_386_96, 1e-4, "REML KR A|T3 SE");
    assert_close(cell_num(&r_ab.rows[2][3]), 6.379_046_679_061_93, 1e-4, "REML KR A|T3 df");

    // 3. Post-hoc CI tracks confidence level independently of threshold (RV-65).
    let sh65 = col_sheet(
        "rv65",
        &[("Ctrl", &[10.0, 10.4, 9.6, 10.2, 9.8]), ("D1", &[10.8, 11.2, 10.4, 11.0, 10.6]), ("D2", &[14.0, 14.4, 13.6, 14.2, 13.8])],
    );
    for (ow_m, mult_m) in [
        (OneWayMethod::OrdinaryAnova, MultiplicityMethod::TukeyHsd),
        (OneWayMethod::OrdinaryAnova, MultiplicityMethod::Bonferroni),
        (OneWayMethod::OrdinaryAnova, MultiplicityMethod::Dunnett { control_column: 0 }),
        (OneWayMethod::BrownForsytheAndWelch, MultiplicityMethod::GamesHowell),
        (OneWayMethod::BrownForsytheAndWelch, MultiplicityMethod::DunnettT3),
    ] {
        let mk = |thr| {
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: ow_m,
                compute_effect_sizes: true,
                multiplicity: MultiplicitySpec { method: mult_m, threshold: thr, report_confidence_intervals: true },
                ..Default::default()
            })
        };
        let (r95a, r95b, r99) = (
            run_kind_conf(&sh65, mk(0.05), 95.0).unwrap(),
            run_kind_conf(&sh65, mk(0.001), 95.0).unwrap(),
            run_kind_conf(&sh65, mk(0.05), 99.0).unwrap(),
        );
        let get_ci = |r: &curvebook_doc::ResultSheet| {
            let t = tab_by_id(r, "multiple-comparisons");
            let c = t.column_headers.iter().position(|h| h.contains("CI")).unwrap();
            cell_ci_finite(&t.rows[0][c])
        };
        let (c1, c2, c99) = (get_ci(&r95a), get_ci(&r95b), get_ci(&r99));
        assert_approx_eq(c1.0, c2.0, 1e-13, 1e-13, "CI independent of threshold");
        assert!((c99.1 - c99.0) > (c1.1 - c1.0) + 1e-4);
        assert!(r95a.pairwise_comparisons[0].significant && !r95b.pairwise_comparisons[0].significant);
    }

    // 4. Factorial Dunnett asymmetric levels (RV-66), simple-effects m=9 & 4x2 SNK step-down (RV-71), REML Dunnett & SMM fallback (RV-72).
    let dun2 = AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 2 },
            threshold: 0.05,
            report_confidence_intervals: true,
        },
        ..Default::default()
    });
    let r32 = run_kind(&build_grouped_grid("g32", 3, 2, 3), dun2.clone()).unwrap();
    assert!(r32.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-a") && !r32.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-b"));
    let r23 = run_kind(&build_grouped_grid("g23", 2, 3, 3), dun2.clone()).unwrap();
    assert!(!r23.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-a") && r23.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-b"));
    let r33 = run_kind(&build_grouped_grid("g33", 3, 3, 3), dun2).unwrap();
    assert!(r33.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-a") && !r33.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-b"));
    assert!(matches!(
        run_kind(
            &build_grouped_grid("g32e", 3, 2, 3),
            AnalysisKind::FactorialAnova(FactorialAnovaSpec {
                multiplicity: MultiplicitySpec {
                    method: MultiplicityMethod::Dunnett { control_column: 5 },
                    threshold: 0.05,
                    report_confidence_intervals: true
                },
                ..Default::default()
            })
        ),
        Err(StatsError::InvalidParameter { name: "control_column", .. })
    ));

    let sh33 = build_grouped_grid("se33", 3, 3, 4);
    let r_b = run_kind(
        &sh33,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    let tb = tab_by_id(&r_b, "multiple-comparisons-simple-a-within-b");
    assert_eq!(tb.rows.len(), 9);
    assert_approx_eq(cell_p(&tb.rows[0][5]), (9.0 * cell_p(&tb.rows[0][4])).min(1.0), 1e-12, 1e-10, "3x3 Bonf m=9");
    let r_tuk = run_kind(
        &sh33,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    let tt = tab_by_id(&r_tuk, "multiple-comparisons-simple-a-within-b");
    assert_approx_eq(
        cell_p(&tt.rows[0][5]),
        max_studentized_range_sf(cell_num(&tt.rows[0][2]), 3, 3, cell_num(&tt.rows[0][3])).unwrap(),
        1e-12,
        1e-10,
        "3x3 Tukey m=3,k=3",
    );

    let diffs = [4.0, 2.2, 1.1, 0.25];
    let sh42 = grouped_sheet_fill("g42", 2, 4, 4, |c, r, s| Some(10.0 + if c == 1 { diffs[r] } else { 0.0 } + [-0.6, -0.2, 0.2, 0.6][s]));
    let r_snk = run_kind(
        &sh42,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::NewmanKeuls, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    let r_t42 = run_kind(
        &sh42,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
            ..Default::default()
        }),
    )
    .unwrap();
    let (tsnk, tt42) = (tab_by_id(&r_snk, "multiple-comparisons-simple-b-within-a"), tab_by_id(&r_t42, "multiple-comparisons-simple-b-within-a"));
    assert_approx_eq(cell_p(&tsnk.rows[0][5]), cell_p(&tt42.rows[0][5]), 1e-12, 1e-10, "SNK rank0");
    assert!(cell_p(&tsnk.rows[3][5]) < cell_p(&tt42.rows[3][5]) - 0.05);

    // RV-72: missing-cell REML Dunnett exact covariance & non-factorizable SMM fallback.
    let mut g_d = GroupedTable::new(SubcolumnMode::RawReplicates(8), 2, 3).unwrap();
    for (s, &(y0, y1)) in
        [(10.0, 12.0), (10.5, 12.8), (9.6, 11.4), (10.2, 12.1), (9.9, 11.9), (10.3, 12.3), (9.7, 11.6), (10.1, 12.0)].iter().enumerate()
    {
        (g_d.data_sets[0].cells[0][s], g_d.data_sets[1].cells[0][s]) = (Cell::number(y0), Cell::number(y1));
    }
    for (s, &(y0, y1)) in [(11.3, 13.4), (10.7, 12.7)].iter().enumerate() {
        (g_d.data_sets[0].cells[1][s], g_d.data_sets[1].cells[1][s]) = (Cell::number(y0), Cell::number(y1));
    }
    for (s, &y0) in [11.2, 10.6].iter().enumerate() {
        g_d.data_sets[0].cells[2][s] = Cell::number(y0);
    }
    let rd = run_kind(
        &DataSheet::new("gd", "GD", TableArchetype::Grouped(g_d)).unwrap(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            residual_covariance: ResidualCovariance::Unstructured,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 0 },
                threshold: 0.05,
                report_confidence_intervals: true,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    let td = tab_by_id(&rd, "multiple-comparisons-factor-a");
    let naive_p = dunnett_two_sided_p_loadings(
        cell_num(&td.rows[0][2]).abs(),
        cell_num(&td.rows[0][3]).min(cell_num(&td.rows[1][3])),
        &[std::f64::consts::FRAC_1_SQRT_2; 2],
    )
    .unwrap();
    assert!((cell_p(&td.rows[0][5]) - naive_p).abs() > 1e-4);

    let g_nf = grouped_sheet_fill("gnf", 2, 4, 3, |c, r, s| {
        let j = (s as f64 - 1.0) * 0.4;
        match (r, c) {
            (0, 0) => Some(10.0 + j),
            (0, 1) => Some(11.0 + j),
            (1, 0) => Some(14.0 + j),
            (2, 1) => Some(15.5 + j),
            (3, 0) => Some(13.0 + j),
            (3, 1) => Some(14.0 + j),
            _ => None,
        }
    });
    let rnf = run_kind(
        &g_nf,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            ss_type: SumOfSquaresType::TypeIII,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 0 },
                threshold: 0.05,
                report_confidence_intervals: true,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    let tnf = tab_by_id(&rnf, "multiple-comparisons-factor-a");
    assert!(tnf.footnotes.iter().any(|n| n.contains("Studentized Maximum Modulus")));
    let (t_nf, df_nf, se_nf) = (cell_num(&tnf.rows[0][2]), cell_num(&tnf.rows[0][3]), cell_num(&tnf.rows[0][1]));
    let (ci_lo, ci_hi) = cell_ci_finite(&tnf.rows[0][6]);
    assert_approx_eq(cell_p(&tnf.rows[0][4]), student_t_two_sided_p(t_nf, df_nf).unwrap(), 1e-12, 1e-10, "SMM raw p");
    assert_approx_eq(
        cell_p(&tnf.rows[0][5]),
        max_studentized_range_sf(std::f64::consts::SQRT_2 * t_nf.abs(), 3, 2, df_nf).unwrap(),
        1e-12,
        1e-10,
        "SMM adj p",
    );
    let smm_c = max_studentized_range_isf(0.05, 3, 2, df_nf).unwrap() * std::f64::consts::FRAC_1_SQRT_2;
    assert!(smm_c > student_t_two_sided_isf(0.05, df_nf).unwrap());
    assert_approx_eq(0.5 * (ci_hi - ci_lo), smm_c * se_nf, 1e-12, 1e-10, "SMM CI half-width");
}

#[test]
fn audit_40_81_multiplicity_bky_anova_rejection_domains_and_nan_guards() {
    // 1. BKY (2006) Def. 6 10 discoveries = 0.021 (#40).
    let raw_bky = [0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.007, 0.008, 0.009, 0.2];
    let bky = adjust_two_stage_bky(&raw_bky, 0.05);
    assert_eq!(bky.iter().filter(|&&q| q <= 0.05).count(), 10);
    assert_close(bky[9], 0.021, 1e-12, "BKY Def.6");
    assert_eq!(try_adjust_p_values(&raw_bky, MultiplicityProcedure::TwoStageBkyFdr, 0.05).unwrap(), (bky, true));
    let ps = analyze_pvalue_stack(&raw_bky, PValueStackMethod::TwoStageBky, 0.05).unwrap();
    assert_eq!(ps.discovery_count, 10);
    assert_close(ps.rows[9].adjusted_p_or_q, 0.021, 1e-12, "pvalue_stack BKY");

    // 2. ANOVA-only procedure rejection in try_adjust_p_values & MultipleRowTTests (#81), legacy adjust_p_values fallback, Dunn FWER (R1-S02-04).
    let raw3 = [0.01, 0.02, 0.04];
    let anova_procs = [
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::DunnRankSum,
        MultiplicityProcedure::LinearTrend,
        MultiplicityProcedure::TamhaneT2,
    ];
    let (ra, rb) = (vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]], vec![vec![2.0, 3.0, 4.0], vec![8.0, 9.0, 10.0]]);
    for proc in anova_procs {
        assert!(matches!(try_adjust_p_values(&raw3, proc, 0.05), Err(StatsError::IncompatibleAnalysisSpec { .. })));
        assert!(matches!(
            multiple_row_t_tests(&["R1".into(), "R2".into()], &ra, &rb, false, proc, 0.05),
            Err(StatsError::IncompatibleAnalysisSpec { .. })
        ));
    }
    let mr_sh = grouped_sheet_fill("mr81", 2, 2, 3, |c, r, s| Some((c * 5 + r * 2) as f64 + 0.1 * (s as f64)));
    for bad_m in [
        MultiplicityMethod::TukeyHsd,
        MultiplicityMethod::NewmanKeuls,
        MultiplicityMethod::Dunnett { control_column: 0 },
        MultiplicityMethod::DunnettT3,
        MultiplicityMethod::GamesHowell,
        MultiplicityMethod::DunnRankSum,
        MultiplicityMethod::LinearTrend,
        MultiplicityMethod::TamhaneT2,
    ] {
        assert!(matches!(
            run_kind(
                &mr_sh,
                AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                    multiplicity: MultiplicitySpec { method: bad_m, threshold: 0.05, report_confidence_intervals: true },
                    ..Default::default()
                })
            ),
            Err(StatsError::IncompatibleAnalysisSpec { .. })
        ));
    }
    for ok_m in [
        MultiplicityMethod::None,
        MultiplicityMethod::FisherLsd,
        MultiplicityMethod::Bonferroni,
        MultiplicityMethod::Sidak,
        MultiplicityMethod::HolmSidak,
        MultiplicityMethod::BenjaminiHochbergFdr,
        MultiplicityMethod::BenjaminiYekutieliFdr,
        MultiplicityMethod::TwoStageBkyFdr,
    ] {
        assert!(
            run_kind(
                &mr_sh,
                AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                    multiplicity: MultiplicitySpec { method: ok_m, threshold: 0.05, report_confidence_intervals: true },
                    ..Default::default()
                })
            )
            .is_ok()
        );
    }
    assert_close(adjust_p_values(&raw3, MultiplicityProcedure::TukeyKramer, 0.05).0[0], 0.03, 1e-12, "adjust_p_values Tukey->Bonf");
    assert_close(adjust_p_values(&raw3, MultiplicityProcedure::TamhaneT2, 0.05).0[0], 1.0 - 0.99_f64.powi(3), 1e-12, "adjust_p_values T2->Sidak");

    let d_lbl = [(0, "G1".into()), (1, "G2".into()), (2, "G3".into())];
    let d_tuk = compare_dunn_ranks(
        &d_lbl,
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    assert!(d_tuk.iter().any(|d| d.raw_p < 0.99 && d.adjusted_p_or_q > d.raw_p));
    let d_dun = compare_dunn_ranks(
        &d_lbl,
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 0 }, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    assert_eq!(d_dun.len(), 2);
    assert!(matches!(
        compare_dunn_ranks(
            &d_lbl,
            &[5, 5, 5],
            &[3.0, 8.0, 13.0],
            15,
            0.0,
            &MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 3 }, threshold: 0.05, report_confidence_intervals: false }
        ),
        Err(StatsError::InvalidParameter { .. })
    ));

    // Selected datasets excluding Dunnett control_column returns InvalidParameter (S3 #1).
    let d_sh = col_sheet("d-ctrl", &[("0", &[1.0, 2.0, 3.0, 4.0]), ("1", &[5.0, 6.0, 7.0, 8.0]), ("2", &[9.0, 10.0, 11.0, 12.0])]);
    assert!(matches!(
        run_kind_sel(
            &d_sh,
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                multiplicity: MultiplicitySpec {
                    method: MultiplicityMethod::Dunnett { control_column: 2 },
                    threshold: 0.05,
                    report_confidence_intervals: true
                },
                ..Default::default()
            }),
            vec![0, 1]
        ),
        Err(StatsError::InvalidParameter { name: "control_column", .. })
    ));

    // 3. Domain validation (FA-SEC05-10) & zero-variance post-hoc infinity (R1-S02-07).
}

#[test]
fn audit_fa_sec05_10_multiplicity_domain_validation_and_zero_variance() {
    let vg = vec![group_input(0, "A", 5.0, 10.0, 1.5), group_input(1, "B", 5.0, 12.0, 1.5), group_input(2, "C", 5.0, 15.0, 1.5)];
    for bad_mse in [-1.0, f64::NAN, f64::INFINITY] {
        for proc in [
            MultiplicityProcedure::Bonferroni,
            MultiplicityProcedure::TukeyKramer,
            MultiplicityProcedure::NewmanKeuls,
            MultiplicityProcedure::Dunnett { control_column: 0 },
        ] {
            assert!(matches!(
                compare_groups_with_procedure(&vg, bad_mse, 12.0, proc, 0.05, true),
                Err(StatsError::InvalidParameter { name: "mse", .. })
            ));
        }
    }
    for bad_df in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            compare_groups_with_procedure(&vg, 0.0, bad_df, MultiplicityProcedure::TukeyKramer, 0.05, true),
            Err(StatsError::InvalidParameter { name: "df_error", .. })
        ));
    }
    for wp in [MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2, MultiplicityProcedure::DunnettT3] {
        assert!(compare_groups_with_procedure(&vg, 0.0, 0.0, wp, 0.05, true).is_ok());
    }
    for (bad_g, field) in [
        (vec![group_input(0, "A", 0.0, 10.0, 1.5), vg[1].clone()], "group_n"),
        (vec![group_input(0, "A", 5.0, f64::NAN, 1.5), vg[1].clone()], "group_mean"),
        (vec![group_input(0, "A", 5.0, 10.0, -1.0), vg[1].clone()], "group_sd"),
        (vec![group_input(0, "A", 5.0, 10.0, f64::NAN), vg[1].clone()], "group_sd"),
    ] {
        assert!(
            matches!(compare_groups_with_procedure(&bad_g, 2.0, 8.0, MultiplicityProcedure::Bonferroni, 0.05, true), Err(StatsError::InvalidParameter { name, .. }) if name == field)
        );
    }
    assert!(matches!(
        compare_groups_with_procedure(&vg, 1.0, 12.0, MultiplicityProcedure::DunnRankSum, 0.05, true),
        Err(StatsError::InvalidParameter { .. })
    ));
    assert!(matches!(
        compare_dunn_ranks(&[(0, "A".into()), (1, "B".into())], &[0, 5], &[3.0, 8.0], 5, 0.0, &MultiplicitySpec::default()),
        Err(StatsError::InvalidParameter { name: "group_n", .. })
    ));
    assert!(matches!(
        linear_trend_test(&[group_input(0, "A", 5.0, f64::NAN, 1.5), vg[1].clone()], 2.0, 8.0),
        Err(StatsError::InvalidParameter { name: "group_mean", .. })
    ));
}

#[test]
fn audit_r1_s02_07_zero_variance_posthoc_and_bky_edge() {
    let zg = vec![group_input(0, "G0", 4.0, 10.0, 0.0), group_input(1, "G1", 4.0, 12.0, 0.0), group_input(2, "G2", 4.0, 12.0, 0.0)];
    for proc in [
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
    ] {
        for r in compare_groups_with_procedure(&zg, 0.0, 9.0, proc, 0.05, true).unwrap() {
            if r.mean_diff.abs() > 0.0 {
                assert!(r.statistic.is_infinite() && r.raw_p == 0.0 && r.adjusted_p_or_q == 0.0 && r.significant);
            } else {
                assert_eq!((r.statistic, r.raw_p, r.significant), (0.0, 1.0, false));
                if matches!(proc, MultiplicityProcedure::TwoStageBkyFdr) {
                    assert_close(r.adjusted_p_or_q, 0.35, 1e-12, "BKY zero-var equal");
                }
            }
        }
    }

    // 4. NaN / Inf preservation & family exclusion (RV-02).
    let mixed = [0.01, f64::NAN, 0.04, f64::INFINITY, f64::NEG_INFINITY];
    let fin = [0.01, 0.04];
    for (got, want) in [
        (adjust_bonferroni(&mixed), adjust_bonferroni(&fin)),
        (adjust_sidak(&mixed), adjust_sidak(&fin)),
        (adjust_holm_bonferroni(&mixed), adjust_holm_bonferroni(&fin)),
        (adjust_holm_sidak(&mixed), adjust_holm_sidak(&fin)),
        (adjust_benjamini_hochberg(&mixed), adjust_benjamini_hochberg(&fin)),
        (adjust_benjamini_yekutieli(&mixed), adjust_benjamini_yekutieli(&fin)),
        (adjust_two_stage_bky(&mixed, 0.05), adjust_two_stage_bky(&fin, 0.05)),
    ] {
        assert_approx_eq(got[0], want[0], 1e-15, 1e-12, "RV-02[0]");
        assert_approx_eq(got[2], want[1], 1e-15, 1e-12, "RV-02[2]");
        assert!(got[1].is_nan() && got[3].is_nan() && got[4].is_nan());
    }
    assert!(adjust_p_values(&[f64::NAN, f64::NAN], MultiplicityProcedure::HolmSidak, 0.05).0.iter().all(|v| v.is_nan()));
    assert!(
        scalar_ds(
            &run_kind(
                &col_sheet(
                    "kw-t",
                    &[("G1", &[1.0, 2.0, 3.0, 4.0, 5.0]), ("G2", &[6.0, 7.0, 8.0, 9.0, 10.0]), ("G3", &[20.0, 21.0, 22.0, 23.0, 24.0])]
                ),
                AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                    method: OneWayMethod::KruskalWallis,
                    multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
                    ..Default::default()
                })
            )
            .unwrap(),
            "Kruskal-Wallis H",
            None
        )
        .is_finite()
    );
    let _ = (run_one_way_anova, run_nested_analysis);
}
