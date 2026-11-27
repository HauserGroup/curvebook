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
use curvebook_math::dist::{
    dunnett_two_sided_p_loadings, f_sf, max_studentized_range_isf, max_studentized_range_sf, student_t_two_sided_isf, student_t_two_sided_p,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::StatsError;
use curvebook_stats::anova::{
    FactorialObservations, RmSphericityCorrection, compute_anova_effect_sizes_for_n, fit_factorial_anova, fit_multi_vc_reml,
    fit_multi_vc_reml_with_df_method, fit_random_intercept_reml, fit_random_intercept_reml_with_df_method, fit_rm_two_way_anova,
    fit_rm_two_way_anova_with_sphericity, fit_unstructured_rm_reml, fit_unstructured_rm_reml_with_df_method, nested_one_way_anova,
    nested_one_way_anova_with_df_method, ordinary_one_way_anova, ordinary_one_way_from_summary, rm_one_way_anova, run_factorial_anova,
    run_nested_analysis, run_one_way_anova, unequal_variance_one_way_anova,
};
use curvebook_stats::multiplicity::{
    MultiplicityProcedure, adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni, adjust_holm_sidak,
    adjust_p_values, adjust_sidak, adjust_two_stage_bky, compare_dunn_ranks, compare_groups_with_procedure, linear_trend_test, try_adjust_p_values,
};
use curvebook_stats::pvalue_stack::analyze_pvalue_stack;
use curvebook_stats::two_group::multiple_row_t_tests;
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_close, cell_bool, cell_ci_finite, cell_num, cell_p, col_sheet, col_sheet_hint, col_sheet_opts, contingency_sheet, group_input,
    group_input_slice, grouped_sheet_fill, mv_cells_sheet, nested_sheet, nested_table, presum_col_sheet, run_kind, run_kind_conf, run_kind_sel,
    scalar, scalar_ds, spec_for, tab_by_id,
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
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Bonferroni,
            threshold: 0.05,
            report_confidence_intervals: true,
            control_column: None,
            selected_pairs: None,
        },
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
                multiplicity: MultiplicitySpec {
                    method,
                    threshold: 0.05,
                    report_confidence_intervals: true,
                    control_column: None,
                    selected_pairs: None,
                },
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
                multiplicity: MultiplicitySpec {
                    method: bad,
                    threshold: 0.05,
                    report_confidence_intervals: true,
                    control_column: None,
                    selected_pairs: None,
                },
                ..Default::default()
            };
            assert!(matches!(run_kind(&rm_sh, AnalysisKind::OneWayAnova(ow)), Err(StatsError::IncompatibleAnalysisSpec { .. })));
        }
    }
    let gg = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Bonferroni,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    assert_close(cell_num(&tab_by_id(&gg, "multiple-comparisons").rows[0][3]), 5.0, 1e-12, "GG df=n-1");
    let fr = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::Friedman,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::DunnRankSum,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(fr.pairwise_comparisons.iter().find(|p| p.label_a == "T0" && p.label_b == "T2").unwrap().significant);
    let fr_t = run_kind(
        &rm_sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::Friedman,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::LinearTrend,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::LinearTrend,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::GamesHowell,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Bonferroni,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
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
                multiplicity: MultiplicitySpec {
                    method: mult_m,
                    threshold: thr,
                    report_confidence_intervals: true,
                    control_column: None,
                    selected_pairs: None,
                },
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
            control_column: None,
            selected_pairs: None,
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
                    report_confidence_intervals: true,
                    control_column: None,
                    selected_pairs: None,
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Bonferroni,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    let tb = tab_by_id(&r_b, "multiple-comparisons-simple-a-within-b");
    assert_eq!(tb.rows.len(), 9);
    assert!(tb.footnotes.iter().any(|f| f.contains("9 simple-effect")));
    assert_approx_eq(cell_p(&tb.rows[0][5]), (9.0 * cell_p(&tb.rows[0][4])).min(1.0), 1e-12, 1e-10, "3x3 Bonf m=9");
    let r_tuk = run_kind(
        &sh33,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
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
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::NewmanKeuls,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    let r_t42 = run_kind(
        &sh42,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
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
                control_column: None,
                selected_pairs: None,
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
                control_column: None,
                selected_pairs: None,
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
                    multiplicity: MultiplicitySpec {
                        method: bad_m,
                        threshold: 0.05,
                        report_confidence_intervals: true,
                        control_column: None,
                        selected_pairs: None
                    },
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
                    multiplicity: MultiplicitySpec {
                        method: ok_m,
                        threshold: 0.05,
                        report_confidence_intervals: true,
                        control_column: None,
                        selected_pairs: None
                    },
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
        &MultiplicitySpec {
            method: MultiplicityMethod::TukeyHsd,
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
            selected_pairs: None,
        },
    )
    .unwrap();
    assert!(d_tuk.iter().any(|d| d.raw_p < 0.99 && d.adjusted_p_or_q > d.raw_p));
    let d_dun = compare_dunn_ranks(
        &d_lbl,
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 0 },
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
            selected_pairs: None,
        },
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
            &MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 3 },
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None
            }
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
                    report_confidence_intervals: true,
                    control_column: None,
                    selected_pairs: None,
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
                    multiplicity: MultiplicitySpec {
                        method: MultiplicityMethod::TukeyHsd,
                        threshold: 0.05,
                        report_confidence_intervals: false,
                        control_column: None,
                        selected_pairs: None
                    },
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

fn sum_contrast_two_way(a_ids: &[usize], b_ids: &[usize], a_lvls: usize, b_lvls: usize) -> DenseMatrix {
    let (da, db) = (a_lvls - 1, b_lvls - 1);
    let mut raw = Vec::with_capacity(a_ids.len() * (1 + da + db + da * db));
    for (&a, &b) in a_ids.iter().zip(b_ids) {
        let ca: Vec<f64> = (0..da)
            .map(|i| {
                if a == da {
                    -1.0
                } else if a == i {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let cb: Vec<f64> = (0..db)
            .map(|i| {
                if b == db {
                    -1.0
                } else if b == i {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        raw.push(1.0);
        raw.extend_from_slice(&ca);
        raw.extend_from_slice(&cb);
        raw.extend(ca.iter().flat_map(|&va| cb.iter().map(move |&vb| va * vb)));
    }
    DenseMatrix::from_row_major(a_ids.len(), 1 + da + db + da * db, raw).unwrap()
}

fn cluster_g_matrix(cluster_ids: &[usize]) -> (DenseMatrix, usize) {
    let mut remap = BTreeMap::<usize, usize>::new();
    let mapped: Vec<usize> = cluster_ids
        .iter()
        .map(|&c| {
            let n = remap.len();
            *remap.entry(c).or_insert(n)
        })
        .collect();
    let ms = mapped.as_slice();
    let g: Vec<f64> = ms.iter().flat_map(|&ci| ms.iter().map(move |&cj| if ci == cj { 1.0 } else { 0.0 })).collect();
    (DenseMatrix::from_row_major(ms.len(), ms.len(), g).unwrap(), remap.len())
}

#[test]
fn anova_aud05_capacity_dense_allocation_and_mv_factorial_guards() {
    // AUD-05-13: Factorial interaction expansion & REML design column caps.
    for (id, n_fac, n_rows, n_lvls, order) in [("mv25", 25_usize, 30_usize, 2_usize, 25_usize), ("mv200", 200, 20, 10, 2)] {
        let mut mv = MultipleVariablesTable::new(n_fac + 1, n_rows).unwrap();
        mv.variables[0].lock_role(VariableRole::Continuous);
        mv.variables[0].cells = (0..n_rows).map(|r| Cell::number(1.0 + (r as f64) * 0.5)).collect();
        for f in 1..=n_fac {
            mv.variables[f].lock_role(VariableRole::Categorical);
            mv.variables[f].cells =
                (0..n_rows).map(|r| Cell::text(format!("L{}", if n_lvls == 2 { (r >> (f % 4)) & 1 } else { (r + f) % n_lvls }))).collect();
        }
        let sh = DataSheet::new(id, id, TableArchetype::MultipleVariables(mv)).unwrap();
        let spec = FactorialAnovaSpec {
            response_column: Some(0),
            multifactor_columns: (1..=n_fac).collect(),
            max_interaction_order: order,
            ..Default::default()
        };
        assert!(matches!(
            run_kind(&sh, AnalysisKind::FactorialAnova(spec)),
            Err(StatsError::InvalidParameter { name: "factorial_terms" | "factorial_design_columns", .. })
        ));
    }
    let obs25 = FactorialObservations {
        factor_names: (0..25).map(|i| format!("F{i}")).collect(),
        factor_levels: vec![vec!["A".into(), "B".into()]; 25],
        rows: (0..30).map(|r| ((0..25).map(|f| (r + f) % 2).collect(), r as f64)).collect(),
    };
    assert!(matches!(
        fit_factorial_anova(&obs25, 25, SumOfSquaresType::TypeIII),
        Err(StatsError::InvalidParameter { name: "factorial_terms" | "factorial_design_columns", .. })
    ));
    let reml_both = AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        mixed_effects_reml: true,
        ..Default::default()
    });
    assert!(matches!(
        run_kind(&grouped_sheet_fill("g-big", 60, 100, 1, |c, r, _| Some((r + c) as f64)), reml_both.clone()),
        Err(StatsError::InvalidParameter { name: "factorial_design_columns", .. })
    ));

    // AUD-05-15: Checked dense matrix allocations across RM two-way sphericity, multi-VC REML, and nested ANOVA.
    let rm_big: Vec<Vec<Vec<f64>>> = (0..2).map(|s| (0..101).map(|r| (0..100).map(|c| (r + c + s) as f64).collect()).collect()).collect();
    assert!(matches!(
        fit_rm_two_way_anova(&rm_big, "A", "B", RepeatedMeasuresDesign::BothFactorsMatched),
        Err(StatsError::Math(MathError::InvalidParameter { name: "matrix_elements", .. }))
    ));
    assert!(matches!(
        run_kind(&grouped_sheet_fill("g-mvc", 3, 500, 7, |c, r, s| Some((r + c + s) as f64)), reml_both),
        Err(StatsError::Math(MathError::InvalidParameter { name: "matrix_elements", .. }))
    ));
    let mut n_big = curvebook_doc::NestedTable::new(1_000, 2, 51).unwrap();
    for (g, grp) in n_big.groups.iter_mut().enumerate() {
        for (r, row) in grp.cells.iter_mut().enumerate() {
            (row[0], row[1]) = (Cell::number((g + r) as f64), Cell::number((g + r + 1) as f64));
        }
    }
    assert!(matches!(nested_one_way_anova(&n_big, false), Err(StatsError::Math(MathError::InvalidParameter { name: "matrix_elements", .. }))));

    // AUD-05-43: MV factorial rejects RM/REML/Unstructured and duplicate/overlapping factor columns.
    let mv_sh = mv_cells_sheet(
        "mv-fac",
        &[
            ("Y", Some(VariableRole::Continuous), &[10.1, 10.5, 14.2, 14.8, 12.0, 12.6, 19.1, 19.7].map(Cell::number)),
            ("A", Some(VariableRole::Categorical), &["A0", "A0", "A0", "A0", "A1", "A1", "A1", "A1"].map(Cell::text)),
            ("B", Some(VariableRole::Categorical), &["B0", "B0", "B1", "B1", "B0", "B0", "B1", "B1"].map(Cell::text)),
        ],
    );
    let base = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: vec![1, 2], ..Default::default() };
    for bad in [
        FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched, ..base.clone() },
        FactorialAnovaSpec { mixed_effects_reml: true, ..base.clone() },
        FactorialAnovaSpec { residual_covariance: ResidualCovariance::Unstructured, ..base.clone() },
    ] {
        assert!(matches!(run_kind(&mv_sh, AnalysisKind::FactorialAnova(bad)), Err(StatsError::IncompatibleAnalysisSpec { .. })));
    }
    for cols in [vec![1, 1], vec![0, 1]] {
        assert!(matches!(
            run_kind(&mv_sh, AnalysisKind::FactorialAnova(FactorialAnovaSpec { multifactor_columns: cols, ..base.clone() })),
            Err(StatsError::InvalidParameter { name: "multifactor_columns", .. })
        ));
    }
    assert!(scalar(&run_kind(&mv_sh, AnalysisKind::FactorialAnova(base)).unwrap(), "A: F") > 0.0);
}

#[test]
fn anova_aud05_reml_empty_cells_stationarity_and_picomolar_scale() {
    // AUD-05-17 (Searle 1987): 3x3 pure-interaction empty cell (2,2) Type III df & relabeling invariance across all 3 REML solvers.
    let means33: [[Option<f64>; 3]; 3] = [[Some(10.0), Some(-10.0), Some(0.0)], [Some(-10.0), Some(10.0), Some(0.0)], [Some(0.0), Some(0.0), None]];
    let build33 = |perm: [usize; 3]| {
        let (mut a, mut b, mut y, mut s, mut rows) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (ao, row) in means33.iter().enumerate() {
            for (bi, &mu) in row.iter().enumerate() {
                if let Some(m) = mu {
                    for (si, &e) in [-0.012, -0.004, 0.001, 0.006, 0.009].iter().enumerate() {
                        let v = m + e + 0.002 * (si as f64 - 2.0);
                        a.push(perm[ao]);
                        b.push(bi);
                        y.push(v);
                        s.push(ao * 5 + si);
                        rows.push((vec![perm[ao], bi], v));
                    }
                }
            }
        }
        (a, b, y, s, rows)
    };
    let ((a0, b0, y0, s0, obs0), (a1, b1, y1, s1, _)) = (build33([0, 1, 2]), build33([2, 1, 0]));
    let ols = fit_factorial_anova(
        &FactorialObservations {
            factor_names: vec!["A".into(), "B".into()],
            factor_levels: vec![vec!["A0".into(), "A1".into(), "A2".into()], vec!["B0".into(), "B1".into(), "B2".into()]],
            rows: obs0,
        },
        2,
        SumOfSquaresType::TypeIII,
    )
    .unwrap();
    let ols_a = ols.terms.iter().find(|t| t.name == "A").unwrap();
    assert!(ols_a.df == 1.0 && ols_a.f_statistic.unwrap() < 1.0);

    let (x0, x1) = (sum_contrast_two_way(&a0, &b0, 3, 3), sum_contrast_two_way(&a1, &b1, 3, 3));
    let terms: &[(&str, usize, usize)] = &[("A", 1, 2), ("B", 3, 2), ("A × B", 5, 4)];
    let chk33 = |t0: &[curvebook_stats::anova::FactorialAnovaTerm], t1: &[curvebook_stats::anova::FactorialAnovaTerm], tol: f64, chk_b_p: bool| {
        assert_close(t0[0].df, 1.0, 1e-12, "df_A");
        assert_close(t0[2].df, 3.0, 1e-12, "df_AB");
        assert!(t0[0].f_statistic.unwrap() < 1.0);
        assert_approx_eq(t0[0].f_statistic.unwrap(), t1[0].f_statistic.unwrap(), tol, tol, "relabel F_A");
        if chk_b_p {
            assert_close(t0[1].df, 1.0, 1e-12, "df_B");
            assert_approx_eq(t0[0].p_value.unwrap(), t1[0].p_value.unwrap(), tol, tol, "relabel P_A");
        }
    };
    let (ri0, ri1) = (fit_random_intercept_reml(&x0, &y0, &s0, terms).unwrap(), fit_random_intercept_reml(&x1, &y1, &s1, terms).unwrap());
    chk33(&ri0.fixed_terms, &ri1.fixed_terms, 1e-6, true);
    let ((gs0, cs0), (gs1, cs1)) = (cluster_g_matrix(&s0), cluster_g_matrix(&s1));
    let (mvc0, mvc1) = (
        fit_multi_vc_reml(&x0, &y0, &[("Subject", gs0, cs0)], terms).unwrap(),
        fit_multi_vc_reml(&x1, &y1, &[("Subject", gs1, cs1)], terms).unwrap(),
    );
    chk33(&mvc0.fixed_terms, &mvc1.fixed_terms, 1e-5, false);
    let tp3 = ["B0".into(), "B1".into(), "B2".into()];
    let (un0, un1) =
        (fit_unstructured_rm_reml(&x0, &y0, &s0, &b0, &tp3, terms).unwrap(), fit_unstructured_rm_reml(&x1, &y1, &s1, &b1, &tp3, terms).unwrap());
    chk33(&un0.fixed_terms, &un1.fixed_terms, 1e-4, false);

    // 2x2 empty cell (1,1): all 3 terms retained (df=0, F/P None) across all 3 solvers + finite sheet scalars.
    let (mut a22, mut b22, mut y22, mut s22) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (a, b, mu) in [(0, 0, 10.0), (0, 1, 20.0), (1, 0, 15.0)] {
        for s in 0..4 {
            a22.push(a);
            b22.push(b);
            y22.push(mu + 0.3 * s as f64);
            s22.push(a * 4 + s);
        }
    }
    let (x22, t22) = (sum_contrast_two_way(&a22, &b22, 2, 2), &[("A", 1, 1), ("B", 2, 1), ("A × B", 3, 1)][..]);
    let ri22 = fit_random_intercept_reml(&x22, &y22, &s22, t22).unwrap();
    assert_eq!((ri22.fixed_terms.len(), ri22.satterthwaite_df.len()), (3, 3));
    assert!(ri22.fixed_terms.iter().all(|t| t.df == 0.0 && t.f_statistic.is_none() && t.p_value.is_none()));
    assert!(ri22.kenward_roger_df.iter().all(|v| v.is_nan()));
    let (gs22, cs22) = cluster_g_matrix(&s22);
    let mvc22 = fit_multi_vc_reml(&x22, &y22, &[("Subject", gs22, cs22)], t22).unwrap();
    let un22 = fit_unstructured_rm_reml(&x22, &y22, &s22, &b22, &["B0".into(), "B1".into()], t22).unwrap();
    assert!(
        mvc22.fixed_terms.len() == 3
            && un22.fixed_terms.len() == 3
            && mvc22.fixed_terms[2].f_statistic.is_none()
            && un22.fixed_terms[2].f_statistic.is_none()
    );
    let sh22 = grouped_sheet_fill("g22", 2, 2, 4, |c, r, s| match (r, c) {
        (0, 0) => Some(10.0 + 0.3 * s as f64),
        (0, 1) => Some(20.0 + 0.3 * s as f64),
        (1, 0) => Some(15.0 + 0.3 * s as f64),
        _ => None,
    });
    let res22 = run_kind(
        &sh22,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
            fit_reml_on_missing: true,
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(res22.scalars.iter().all(|s| s.value.is_finite()));

    // AUD-05-41 (Gilmour, Thompson & Cullis 1995): AI+EM stationarity guard & 1e-4 scale invariance on ill-conditioned multi-VC design.
    let subj_eff = [-120.0, -85.0, -40.0, -10.0, 15.0, 45.0, 80.0, 115.0];
    let (mut ai, mut bi, mut cs, mut csa, mut csb, mut yi) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (s, &se) in subj_eff.iter().enumerate() {
        for a in 0..2_usize {
            for b in 0..3_usize {
                if (s == 1 && a == 1 && b == 2) || (s == 6 && a == 0 && b == 1) {
                    continue;
                }
                let sa = if a == 0 { -0.02 } else { 0.02 } * (s as f64 - 3.5);
                let sb = (b as f64 - 1.0) * ((s % 4) as f64 - 1.5) * 2.5;
                ai.push(a);
                bi.push(b);
                cs.push(s);
                csa.push(s * 2 + a);
                csb.push(s * 3 + b);
                yi.push(50.0 + se + 4.0 * a as f64 + 3.0 * b as f64 + sa + sb + (((s * 11 + a * 5 + b * 3 + s * a * b) % 7) as f64 - 3.0) * 0.4);
            }
        }
    }
    let (xm, m_terms) = (sum_contrast_two_way(&ai, &bi, 2, 3), &[("A", 1, 1), ("B", 2, 2), ("A × B", 4, 2)][..]);
    let ((g_s, n_s), (g_sa, n_sa), (g_sb, n_sb)) = (cluster_g_matrix(&cs), cluster_g_matrix(&csa), cluster_g_matrix(&csb));
    let fit_ill = fit_multi_vc_reml(
        &xm,
        &yi,
        &[("Subject", g_s.clone(), n_s), ("Subject × A", g_sa.clone(), n_sa), ("Subject × B", g_sb.clone(), n_sb)],
        m_terms,
    )
    .unwrap();
    assert!(fit_ill.converged && fit_ill.reml_log_lik.is_finite() && fit_ill.sigma2_error > 0.0 && fit_ill.variance_components[0].1 > 1000.0);
    let yi_sc: Vec<f64> = yi.iter().map(|&v| v * 1e-4).collect();
    let fit_sc = fit_multi_vc_reml(&xm, &yi_sc, &[("Subject", g_s, n_s), ("Subject × A", g_sa, n_sa), ("Subject × B", g_sb, n_sb)], m_terms).unwrap();
    assert!(fit_sc.converged);
    for (t0, t1) in fit_ill.fixed_terms.iter().zip(&fit_sc.fixed_terms) {
        assert_approx_eq(t0.f_statistic.unwrap(), t1.f_statistic.unwrap(), 1e-4, 1e-4, "Multi-VC scale F");
    }

    // AUD-05-24: Picomolar (1e-11) scale invariance for multi-VC REML & nested ICC / % variance.
    let (sc, se6) = (1e-11_f64, [-3.0, -1.8, -0.6, 0.7, 1.9, 2.8]);
    let mk_rm = |k: f64| {
        grouped_sheet_fill("rm-p", 2, 2, 6, |c, r, s| {
            Some((10.0 + se6[s] + 1.5 * r as f64 + 2.0 * c as f64 + 0.15 * (((s + r + c) % 3) as f64 - 1.0)) * k)
        })
    };
    let rm_k = AnalysisKind::FactorialAnova(FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        mixed_effects_reml: true,
        ..Default::default()
    });
    let (rm_u, rm_p) = (run_kind(&mk_rm(1.0), rm_k.clone()).unwrap(), run_kind(&mk_rm(sc), rm_k).unwrap());
    assert!(scalar(&rm_u, "REML ICC") > 0.5);
    assert_approx_eq(scalar(&rm_p, "REML ICC"), scalar(&rm_u, "REML ICC"), 1e-5, 1e-5, "picomolar multi-VC ICC");
    let vc_tab = tab_by_id(&rm_p, "mixed-reml");
    let pct_c = vc_tab.column_headers.iter().position(|h| h == "% of total").unwrap();
    assert_approx_eq(vc_tab.rows.iter().map(|r| cell_num(&r[pct_c])).sum(), 100.0, 1e-4, 1e-4, "picomolar multi-VC %");

    let n_base = [[[10.0, 10.2, 9.8], [13.0, 13.1, 12.9], [16.0, 16.2, 15.8]], [[20.0, 20.1, 19.9], [23.0, 23.2, 22.8], [26.0, 26.1, 25.9]]];
    let mk_n = |k: f64| {
        let s: [[[f64; 3]; 3]; 2] = n_base.map(|g| g.map(|b| b.map(|v| v * k)));
        nested_sheet("np", &[("C", &[&s[0][0], &s[0][1], &s[0][2]]), ("D", &[&s[1][0], &s[1][1], &s[1][2]])])
    };
    let n_k = AnalysisKind::NestedComparison(NestedAnalysisSpec::default());
    let (n_u, n_p) = (run_kind(&mk_n(1.0), n_k.clone()).unwrap(), run_kind(&mk_n(sc), n_k).unwrap());
    assert_approx_eq(scalar(&n_p, "ICC (subcluster)"), scalar(&n_u, "ICC (subcluster)"), 1e-5, 1e-5, "picomolar nested ICC");
    assert_approx_eq(tab_by_id(&n_p, "variance-components").rows.iter().map(|r| cell_num(&r[2])).sum(), 100.0, 1e-4, 1e-4, "picomolar nested %");
}

#[test]
fn anova_aud05_nested_gls_posthoc_t_summary_and_lazy_p() {
    // AUD-05-19 & AUD-05-20: Simpson-reversal unbalanced nested table uses REML GLS (not raw pooled means).
    let u0a = &[99.6, 100.4, 99.8, 100.2, 100.1, 99.9, 100.3, 99.7, 100.0, 100.0][..];
    let u = [&[9.8, 10.2][..], &[69.8, 70.2], &[69.9, 70.1], &[89.7, 90.3], &[89.9, 90.1]];
    let g2: &[(&str, &[&[f64]])] = &[("G0", &[u0a, u[0]]), ("G1", &[u[1], u[2]])];
    let r2 = nested_one_way_anova(&nested_table(g2), true).unwrap();
    let t2 = r2.t_nested.unwrap();
    assert!(t2 < 0.0);
    assert_approx_eq(t2 * t2, r2.f_among_groups, 1e-9, 1e-9, "t_nested^2 == F");
    let n_sp = |two_g, method| {
        AnalysisKind::NestedComparison(NestedAnalysisSpec {
            two_group_t_test: two_g,
            multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true, control_column: None, selected_pairs: None },
            ..Default::default()
        })
    };
    let out2 = run_kind(&nested_sheet("ns2", g2), n_sp(true, MultiplicityMethod::None)).unwrap();
    let (md, se) = (scalar(&out2, "Mean difference"), scalar(&out2, "SE of difference"));
    assert!(md < -14.0 && md > -16.0);
    assert_approx_eq(scalar(&out2, "Nested t"), md / se, 1e-8, 1e-8, "Nested t == md/se");

    let sh3 = nested_sheet("ns3", &[("G0", &[u0a, u[0]]), ("G1", &[u[1], u[2]]), ("G2", &[u[3], u[4]])]);
    let g01 = cell_num(&tab_by_id(&run_kind(&sh3, n_sp(false, MultiplicityMethod::TukeyHsd)).unwrap(), "multiple-comparisons").rows[0][0]);
    assert!(g01 < -14.0 && g01 > -16.0);
    for bad in [MultiplicityMethod::GamesHowell, MultiplicityMethod::TamhaneT2, MultiplicityMethod::DunnettT3, MultiplicityMethod::DunnRankSum] {
        assert!(matches!(run_kind(&sh3, n_sp(false, bad)), Err(StatsError::IncompatibleAnalysisSpec { .. })));
    }

    // AUD-05-20: sigma_u^2 == 0 exact REML df=3.0 (not Welch 2.0), m_g=1 inherits REML df > 3.5, Nakagawa-Schielzeth (2013) R^2.
    let out_zu = run_kind(
        &nested_sheet(
            "nzu",
            &[
                ("Low", &[&[10.0, 12.0, 8.0, 11.0, 9.0], &[9.5, 11.5, 8.5, 10.5, 10.0]]),
                ("Mid", &[&[15.0, 17.0, 13.0, 16.0, 14.0], &[14.5, 16.5, 13.5, 15.5, 15.0]]),
                ("High", &[&[20.0, 22.0, 18.0, 21.0, 19.0], &[19.5, 21.5, 18.5, 20.5, 20.0]]),
            ],
        ),
        n_sp(false, MultiplicityMethod::TukeyHsd),
    )
    .unwrap();
    assert_close(scalar(&out_zu, "Subcluster variance"), 0.0, 1e-10, "sigma_u^2 == 0");
    let zu_df = scalar(&out_zu, "Group denominator df");
    assert_close(zu_df, 3.0, 1e-6, "REML df = c - g = 3");
    for row in &tab_by_id(&out_zu, "multiple-comparisons").rows {
        assert_close(cell_num(&row[3]), zu_df, 1e-6, "pair uses exact REML df");
    }
    let out_m1 = run_kind(
        &nested_sheet(
            "nm1",
            &[
                ("Single", &[&[10.0, 11.0, 9.0, 10.5, 9.5]]),
                ("M1", &[&[15.0, 16.0, 14.0, 15.5, 14.5], &[15.2, 16.2, 14.2, 15.7, 14.7], &[14.8, 15.8, 13.8, 15.3, 14.3]]),
                ("M2", &[&[20.0, 21.0, 19.0, 20.5, 19.5], &[20.2, 21.2, 19.2, 20.7, 19.7], &[19.8, 20.8, 18.8, 20.3, 19.3]]),
            ],
        ),
        n_sp(false, MultiplicityMethod::TukeyHsd),
    )
    .unwrap();
    assert!(cell_num(&tab_by_id(&out_m1, "multiple-comparisons").rows[0][3]) > 3.5);

    let out_tr = run_kind(
        &nested_sheet(
            "ntr",
            &[
                ("D1", &[&[10.0, 10.4, 9.6], &[14.0, 14.3, 13.7], &[12.0, 12.2, 11.8]]),
                ("D2", &[&[15.0, 15.4, 14.6], &[19.0, 19.3, 18.7], &[17.0, 17.2, 16.8]]),
                ("D3", &[&[20.0, 20.4, 19.6], &[24.0, 24.3, 23.7], &[22.0, 22.2, 21.8]]),
            ],
        ),
        n_sp(false, MultiplicityMethod::LinearTrend),
    )
    .unwrap();
    let tr_tab = tab_by_id(&out_tr, "linear-trend");
    let r2_c = tr_tab.column_headers.iter().position(|h| h == "R squared").unwrap();
    let exp_r2 = (50.0 / 3.0) / ((50.0 / 3.0) + scalar(&out_tr, "Subcluster variance") + scalar(&out_tr, "Residual variance"));
    assert_approx_eq(cell_num(&tr_tab.rows[0][r2_c]), exp_r2, 1e-4, 1e-4, "Nakagawa-Schielzeth marginal R^2");

    // AUD-05-57 (part 4): Nested two_group_t_test emits 90% CI scalars & nested-t-summary tab; AUD-05-59 (part 1): exact f_sf.
    let out_nt = run_kind_conf(
        &nested_sheet(
            "ntci",
            &[
                ("Control", &[&[10.0, 10.4, 9.6], &[11.2, 11.5, 10.9], &[10.5, 10.8, 10.2]]),
                ("Treated", &[&[15.0, 15.3, 14.7], &[16.1, 16.4, 15.8], &[15.6, 15.9, 15.3]]),
            ],
        ),
        n_sp(true, MultiplicityMethod::None),
        90.0,
    )
    .unwrap();
    let (d, s, df, lo, hi) = (
        scalar(&out_nt, "Mean difference"),
        scalar(&out_nt, "SE of difference"),
        scalar(&out_nt, "Group denominator df"),
        scalar(&out_nt, "90% CI lower"),
        scalar(&out_nt, "90% CI upper"),
    );
    let tc = student_t_two_sided_isf(0.10, df).unwrap();
    assert_approx_eq(scalar(&out_nt, "Nested t"), d / s, 1e-9, 1e-9, "nested-t t");
    assert_approx_eq(lo, d - tc * s, 1e-9, 1e-9, "nested-t 90% lo");
    assert_approx_eq(hi, d + tc * s, 1e-9, 1e-9, "nested-t 90% hi");
    let st = tab_by_id(&out_nt, "nested-t-summary");
    assert_eq!(st.row_labels, vec!["Control - Treated"]);
    let (tlo, thi) = cell_ci_finite(&st.rows[0][5]);
    assert_approx_eq(tlo, lo, 1e-12, 1e-12, "tab CI lo");
    assert_approx_eq(thi, hi, 1e-12, 1e-12, "tab CI hi");

    let tab3 = nested_table(&[
        ("A", &[&[10.0, 10.5, 9.5], &[12.0, 12.2, 11.8]]),
        ("B", &[&[15.0, 15.4, 14.6], &[17.5, 17.8, 17.2]]),
        ("C", &[&[21.0, 21.3, 20.7], &[23.0, 23.1, 22.9]]),
    ]);
    for df_m in [MixedModelDfMethod::KenwardRoger, MixedModelDfMethod::Satterthwaite] {
        let r = nested_one_way_anova_with_df_method(&tab3, false, df_m).unwrap();
        let adf2 = if df_m == MixedModelDfMethod::KenwardRoger { r.df2_groups_kenward_roger } else { r.df2_groups_satterthwaite };
        assert_approx_eq(r.p_among_groups, f_sf(r.f_among_groups, r.df1_groups, adf2).unwrap(), 1e-10, 1e-10, "nested f_sf");
    }
}

fn run_ow_rm(sh: &DataSheet, gg: bool, mult: MultiplicityMethod, cov: ResidualCovariance) -> curvebook_doc::ResultSheet {
    run_kind(
        sh,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg },
            residual_covariance: cov,
            multiplicity: MultiplicitySpec {
                method: mult,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap()
}

#[test]
fn anova_aud05_rm_posthoc_and_effect_sizes() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // AUD-05-18 & AUD-05-55 (Steiger 2004): Zero within-error CIs are None; multi-way zero-other-SS keeps eta^2 CI None.
    let z_ow = ordinary_one_way_anova(&[("A", &[10.0; 3]), ("B", &[20.0; 3])], conf).unwrap();
    assert!(z_ow.f_statistic.is_infinite() && z_ow.f_statistic.is_sign_positive() && z_ow.p_value == 0.0);
    assert!(
        z_ow.effect_sizes.eta_squared == 1.0
            && z_ow.effect_sizes.partial_eta_squared == 1.0
            && z_ow.effect_sizes.cohens_f.is_infinite()
            && z_ow.effect_sizes.cohens_f.is_sign_positive()
    );
    assert_eq!((z_ow.effect_sizes.eta_squared_ci, z_ow.effect_sizes.partial_eta_squared_ci, z_ow.effect_sizes.cohens_f_ci), (None, None, None));
    let z_dir = compute_anova_effect_sizes_for_n(150.0, 0.0, 150.0, 1.0, 4.0, 6, conf);
    assert_eq!((z_dir.eta_squared_ci, z_dir.partial_eta_squared_ci, z_dir.cohens_f_ci), (None, None, None));
    let mw_es = compute_anova_effect_sizes_for_n(40.0, 100.0, 140.0, 1.0, 36.0, 40, conf);
    assert!(mw_es.eta_squared_ci.is_none() && mw_es.partial_eta_squared_ci.is_some() && mw_es.cohens_f_ci.is_some());
    let ow_es = compute_anova_effect_sizes_for_n(40.0, 100.0, 140.0, 1.0, 36.0, 38, conf);
    assert!(ow_es.eta_squared_ci.is_some() && ow_es.eta_squared_ci == ow_es.partial_eta_squared_ci);

    // AUD-05-21: Factorial rejects LinearTrend & DunnRankSum (3x3 simple-effects Bonferroni m=9 & footnote verified above).
    let g33 = build_grouped_grid("f21", 3, 3, 3);
    for rm in [RepeatedMeasuresDesign::None, RepeatedMeasuresDesign::ColumnFactorMatched] {
        for bad in [MultiplicityMethod::LinearTrend, MultiplicityMethod::DunnRankSum] {
            let sp = FactorialAnovaSpec {
                repeated_measures: rm,
                mixed_effects_reml: rm != RepeatedMeasuresDesign::None,
                multiplicity: MultiplicitySpec {
                    method: bad,
                    threshold: 0.05,
                    report_confidence_intervals: false,
                    control_column: None,
                    selected_pairs: None,
                },
                ..Default::default()
            };
            assert!(matches!(run_kind(&g33, AnalysisKind::FactorialAnova(sp)), Err(StatsError::IncompatibleAnalysisSpec { .. })));
        }
    }

    // AUD-05-22 (Maxwell & Delaney 2004): RM one-way GG=true subject-level contrasts for Tukey/Dunnett/NK/LinearTrend.
    let c1 = [10.0, 12.0, 11.0, 13.0, 9.5, 14.0, 10.5, 12.5, 11.5, 13.5];
    let d12 = [0.50, 0.54, 0.48, 0.56, 0.52, 0.51, 0.55, 0.49, 0.53, 0.55];
    let d13 = [-18.0, 15.0, -14.0, 20.0, -16.0, 17.0, -15.0, 19.0, -17.0, 14.0];
    let (c2, c3): (Vec<f64>, Vec<f64>) = (c1.iter().zip(&d12).map(|(a, d)| a + d).collect(), c1.iter().zip(&d13).map(|(a, d)| a + d).collect());
    let sh_gg = col_sheet_hint("rm-gg", ColumnDesignHint::PairedRepeatedMeasures, &[("C1", &c1), ("C2", &c2), ("C3", &c3)]);
    let m12 = d12.iter().sum::<f64>() / 10.0;
    let exp_se12 = (d12.iter().map(|&d| (d - m12).powi(2)).sum::<f64>() / 90.0).sqrt();
    for m in [MultiplicityMethod::TukeyHsd, MultiplicityMethod::Dunnett { control_column: 0 }, MultiplicityMethod::NewmanKeuls] {
        let r = run_ow_rm(&sh_gg, true, m, ResidualCovariance::CompoundSymmetry);
        let mc = tab_by_id(&r, "multiple-comparisons");
        assert_approx_eq(cell_num(&mc.rows[0][1]), exp_se12, 1e-12, 1e-12, "RM GG subject SE");
        assert!(cell_num(&mc.rows[0][3]) == 9.0 && cell_p(&mc.rows[0][4]) < 1e-6);
    }
    let r_lt = run_ow_rm(&sh_gg, true, MultiplicityMethod::LinearTrend, ResidualCovariance::CompoundSymmetry);
    let ml = d13.iter().sum::<f64>() / 10.0;
    let exp_f_lt = (ml / (d13.iter().map(|&v| (v - ml).powi(2)).sum::<f64>() / 90.0).sqrt()).powi(2);
    assert_approx_eq(scalar(&r_lt, "Linear trend F"), exp_f_lt, 1e-10, 1e-10, "RM GG LT F");
    assert_approx_eq(scalar(&r_lt, "Linear trend P"), f_sf(exp_f_lt, 1.0, 9.0).unwrap(), 1e-10, 1e-10, "RM GG LT P");
    assert_close(cell_num(&tab_by_id(&r_lt, "linear-trend").rows[0][2]), 9.0, 1e-12, "RM GG LT DFd");
    let r_sph = run_ow_rm(&sh_gg, false, MultiplicityMethod::TukeyHsd, ResidualCovariance::CompoundSymmetry);
    let mc_sph = tab_by_id(&r_sph, "multiple-comparisons");
    assert!(cell_num(&mc_sph.rows[0][3]) == 18.0 && cell_num(&mc_sph.rows[0][1]) > 2.0);

    // AUD-05-23: Newman-Keuls preserves level mean ordering when a level has an empty cell (A2, B2).
    let nk_sh =
        grouped_sheet_fill("nk-e", 3, 3, 3, |c, r, s| if r == 2 && c == 2 { None } else { Some(100.0 + 5.0 * r as f64 + [-6.0, 0.0, 6.0][s]) });
    let r_nk = run_kind(
        &nk_sh,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::NewmanKeuls,
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    let ta = tab_by_id(&r_nk, "multiple-comparisons-factor-a");
    let adj_nk = ta.column_headers.iter().position(|h| h.to_ascii_lowercase().contains("adj")).unwrap();
    let (i01, i02) = (
        ta.row_labels.iter().position(|l| l.contains("Row 1") && l.contains("Row 2")).unwrap(),
        ta.row_labels.iter().position(|l| l.contains("Row 1") && l.contains("Row 3")).unwrap(),
    );
    let exp_02 = max_studentized_range_sf(cell_num(&ta.rows[i02][2]), 1, 3, cell_num(&ta.rows[i02][3])).unwrap();
    let raw_01 = max_studentized_range_sf(cell_num(&ta.rows[i01][2]), 1, 2, cell_num(&ta.rows[i01][3])).unwrap();
    let wrong_01 = max_studentized_range_sf(cell_num(&ta.rows[i01][2]), 1, 3, cell_num(&ta.rows[i01][3])).unwrap();
    let p_01 = cell_p(&ta.rows[i01][adj_nk]);
    assert_approx_eq(cell_p(&ta.rows[i02][adj_nk]), exp_02, 1e-10, 1e-10, "NK A0-A2 span 3");
    assert_approx_eq(p_01, raw_01.max(exp_02), 1e-10, 1e-10, "NK A0-A1 span 2");
    assert!((p_01 - wrong_01).abs() > 1e-4);
}

#[test]
fn anova_aud05_rm_sphericity_missing_and_summary_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;
    // AUD-05-42: Missing-cell RM one-way GG=true + CompoundSymmetry emits diagnostics-only Warn & footnote.
    let rm_miss = col_sheet_opts(
        "rm-gg-m",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[
            ("T1", &[Some(10.0), Some(12.0), Some(11.0), Some(13.0), Some(9.5), Some(14.0)]),
            ("T2", &[Some(11.0), None, Some(12.2), Some(14.1), Some(10.4), Some(15.2)]),
            ("T3", &[Some(18.0), Some(15.0), Some(20.0), Some(17.0), Some(22.0), Some(16.0)]),
        ],
    );
    let r_cs = run_ow_rm(&rm_miss, true, MultiplicityMethod::None, ResidualCovariance::CompoundSymmetry);
    assert!(r_cs.scalars.iter().any(|s| s.key == "REML sigma^2 (subject)"));
    let w_cl =
        r_cs.checklist.iter().find(|c| c.status == ChecklistStatus::Warn && c.assumption == "Geisser-Greenhouse sphericity adjustment").unwrap();
    assert!(w_cl.remediation.is_none());
    assert!(tab_by_id(&r_cs, "rm-one-way-anova").footnotes.iter().any(|f| f.contains("Geisser-Greenhouse") && f.contains("CompoundSymmetry")));
    for (gg, cov) in [(false, ResidualCovariance::CompoundSymmetry), (true, ResidualCovariance::Unstructured)] {
        assert!(
            !run_ow_rm(&rm_miss, gg, MultiplicityMethod::None, cov)
                .checklist
                .iter()
                .any(|c| c.assumption == "Geisser-Greenhouse sphericity adjustment")
        );
    }

    // AUD-05-56: Zero-SE incomplete RM linear trend diverges; summary ANOVA overflow/cap guards.
    let rm_nl = col_sheet_opts(
        "rm-nl",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[
            ("C1", &[Some(10.0), Some(12.0), Some(14.0), Some(16.0), Some(18.0)]),
            ("C2", &[Some(15.0), None, Some(19.0), Some(21.0), Some(23.0)]),
            ("C3", &[Some(20.0), Some(22.0), Some(24.0), Some(26.0), Some(28.0)]),
        ],
    );
    let r_nl = run_ow_rm(&rm_nl, false, MultiplicityMethod::LinearTrend, ResidualCovariance::CompoundSymmetry);
    assert!((scalar(&r_nl, "Linear trend F") > 1e8 || scalar(&r_nl, "Linear trend F").is_infinite()) && scalar(&r_nl, "Linear trend P") < 1e-10);
    assert!(matches!(
        ordinary_one_way_from_summary(&[group_input(0, "A", 1e200, 1e200, 0.0), group_input(1, "B", 1e200, -1e200, 0.0)], conf),
        Err(StatsError::InvalidParameter { name: "group_n", .. })
    ));
    for ovf in [
        [group_input(0, "A", 10.0, 1e308, 0.0), group_input(1, "B", 10.0, -1e308, 0.0)],
        [group_input(0, "A", 10.0, 1.0, 1e200), group_input(1, "B", 10.0, 2.0, 1e200)],
    ] {
        assert!(matches!(ordinary_one_way_from_summary(&ovf, conf), Err(StatsError::Math(MathError::DomainError { .. }))));
    }

    // AUD-05-57 (parts 1-3): 2-level RM sphericity NaN/df=0 suppression & Bartlett zero-variance contract.
    let rm1 = rm_one_way_anova(&[vec![10.0, 12.5], vec![11.2, 13.4], vec![9.8, 12.1], vec![10.5, 13.0]]).unwrap();
    assert!(rm1.sphericity.mauchly_p.is_nan() && rm1.sphericity.mauchly_df == 0.0 && rm1.sphericity.greenhouse_geisser_eps == 1.0);
    let rm2_d = vec![
        vec![vec![10.0, 14.0], vec![12.0, 18.0]],
        vec![vec![11.0, 15.2], vec![13.5, 19.1]],
        vec![vec![12.0, 16.1], vec![14.2, 20.4]],
        vec![vec![13.0, 17.5], vec![15.0, 21.2]],
    ];
    let rm2 =
        fit_rm_two_way_anova_with_sphericity(&rm2_d, "A", "B", RepeatedMeasuresDesign::BothFactorsMatched, RmSphericityCorrection::None).unwrap();
    for s in [&rm2.sphericities.row, &rm2.sphericities.column, &rm2.sphericities.interaction] {
        let s = s.as_ref().unwrap();
        assert!(s.mauchly_p.is_nan() && s.mauchly_df == 0.0 && s.greenhouse_geisser_eps == 1.0);
    }
    let rm_both =
        AnalysisKind::FactorialAnova(FactorialAnovaSpec { repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched, ..Default::default() });
    assert!(
        !run_kind(&grouped_sheet_fill("rm22", 2, 2, 4, |c, r, s| Some(rm2_d[s][r][c])), rm_both.clone())
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.id == "rm-sphericity")
    );
    let mut sh23 = grouped_sheet_fill("rm23", 3, 2, 4, |c, r, s| {
        Some(10.0 + 2.0 * r as f64 + 3.0 * c as f64 + 0.5 * s as f64 + ((r + c * 2 + s) % 3) as f64 * 0.2)
    });
    if let TableArchetype::Grouped(g) = &mut sh23.archetype {
        (g.row_factor_title, g.column_factor_title) = ("Row2".into(), "Col3".into());
    }
    let out23 = run_kind(&sh23, rm_both).unwrap();
    let sph23 = tab_by_id(&out23, "rm-sphericity");
    assert!(!sph23.row_labels.iter().any(|l| l == "Row2") && sph23.row_labels.iter().any(|l| l == "Col3"));

    let b1 = ordinary_one_way_anova(&[("A", &[5.0; 3]), ("B", &[5.0, 6.0, 7.0])], conf).unwrap();
    assert!(
        b1.variance_tests.bartlett_chi2.is_infinite() && b1.variance_tests.bartlett_chi2.is_sign_positive() && b1.variance_tests.bartlett_p == 0.0
    );
    let b2 = ordinary_one_way_anova(&[("A", &[5.0; 3]), ("B", &[8.0; 3])], conf).unwrap();
    assert_eq!((b2.variance_tests.bartlett_chi2, b2.variance_tests.bartlett_p), (0.0, 1.0));
}

#[test]
fn multiplicity_aud05_08_09_bky_sub_1e6_and_welch_posthoc_scaling() {
    // AUD-05-08 (Benjamini, Krieger & Yekutieli 2006 Def. 6): sub-1e-6 fdr_q oracle & invalid q -> NaN.
    let (q, mut raw_p, mut strong) = (1e-7_f64, vec![9.5e-8; 9], vec![1e-9; 5]);
    raw_p.push(0.5);
    strong.extend([0.5; 5]);
    let exp_first = (10.0 * 9.5e-8 / 9.0) * (1.0 + q);
    for &v in &adjust_two_stage_bky(&raw_p, q)[..9] {
        assert_close(v, exp_first, 1e-18, "BKY q=1e-7");
        assert!(v > q);
    }
    assert_eq!(analyze_pvalue_stack(&raw_p, PValueStackMethod::TwoStageBky, q).unwrap().discovery_count, 0);
    for &v in &adjust_two_stage_bky(&strong, q)[..5] {
        assert_close(v, 1e-9 * (1.0 + q), 1e-20, "BKY strong q=1e-7");
        assert!(v <= q);
    }
    for bad_q in [0.0, -0.05, 1.0, 1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let out = adjust_two_stage_bky(&raw_p, bad_q);
        assert!(out.len() == raw_p.len() && out.iter().all(|v| v.is_nan()));
    }

    // AUD-05-09 (Games & Howell 1976; Welch 1947): unequal-variance post-hoc n>=2 guard & scaled Welch df.
    let n1 = [group_input(0, "G1", 1.0, 10.0, 0.0), group_input(1, "G2", 1.0, 11.0, 0.0), group_input(2, "G3", 5.0, 12.0, 1.0)];
    let huge = [group_input(0, "A", 20.0, 0.0, 1e100), group_input(1, "B", 20.0, 1e100, 1e100)];
    let tiny = [group_input(0, "A", 20.0, 0.0, 1e-100), group_input(1, "B", 20.0, 1e-100, 1e-102)];
    let (exp_se, exp_df) = (1e100 * (2.0_f64 / 20.0).sqrt(), (1.0 + 1e-4_f64).powi(2) / (1.0 / 19.0 + 1e-8_f64 / 19.0));
    for proc in [MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2, MultiplicityProcedure::DunnettT3] {
        assert!(matches!(
            compare_groups_with_procedure(&n1, 1.0, 4.0, proc, 0.05, true),
            Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
        ));
        let rh = compare_groups_with_procedure(&huge, 0.0, 1.0, proc, 0.05, true).unwrap();
        assert!(rh[0].df == 38.0 && (rh[0].se_diff - exp_se).abs() / exp_se < 1e-12);
        assert_close(compare_groups_with_procedure(&tiny, 0.0, 1.0, proc, 0.05, true).unwrap()[0].df, exp_df, 1e-10, "tiny Welch df");
    }
    assert!(matches!(
        compare_groups_with_procedure(
            &[group_input(0, "A", 1.5, 1.0, 0.5), group_input(1, "B", 5.0, 2.0, 0.5)],
            1.0,
            4.0,
            MultiplicityProcedure::GamesHowell,
            0.05,
            false
        ),
        Err(StatsError::InsufficientSampleSize { required: 2, .. })
    ));
}

#[test]
fn one_way_all_data_skips_empty_data_sets_but_explicit_empty_selection_fails() {
    let blank: [Option<f64>; 5] = [None; 5];
    let ctrl = [Some(10.0), Some(10.4), Some(9.7), Some(10.1), Some(9.9)];
    let mid = [Some(12.0), Some(13.1), Some(11.6), Some(12.7), Some(12.2)];
    let high = [Some(15.2), Some(14.1), Some(16.0), Some(15.5), Some(14.8)];
    let sheet =
        col_sheet_opts("blank", ColumnDesignHint::default(), &[("Blank", &blank), ("Ctrl", &ctrl), ("Mid", &mid), ("High", &high), ("Tail", &blank)]);

    // Blank leading and trailing data sets are ignored when every data set is analysed: the result
    // equals an explicit selection of the non-empty ones, and the Dunnett control keeps its
    // sheet index (1) although a blank data set precedes it.
    for method in [OneWayMethod::OrdinaryAnova, OneWayMethod::KruskalWallis] {
        for mult in [MultiplicityMethod::TukeyHsd, MultiplicityMethod::Dunnett { control_column: 1 }, MultiplicityMethod::DunnRankSum] {
            if matches!(
                (method, mult),
                (OneWayMethod::OrdinaryAnova, MultiplicityMethod::DunnRankSum) | (OneWayMethod::KruskalWallis, MultiplicityMethod::TukeyHsd)
            ) {
                continue;
            }
            let kind = AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method,
                multiplicity: MultiplicitySpec {
                    method: mult,
                    threshold: 0.05,
                    report_confidence_intervals: true,
                    control_column: None,
                    selected_pairs: None,
                },
                ..Default::default()
            });
            let all = run_kind(&sheet, kind.clone()).unwrap_or_else(|e| panic!("{method:?}/{mult:?} all data: {e}"));
            let sel = run_kind_sel(&sheet, kind.clone(), vec![1, 2, 3]).unwrap();
            assert_eq!(all.scalars, sel.scalars, "{method:?}/{mult:?} scalars");
            assert_eq!(all.pairwise_comparisons, sel.pairwise_comparisons, "{method:?}/{mult:?} pairwise");
            if matches!(mult, MultiplicityMethod::Dunnett { .. }) {
                assert!(all.pairwise_comparisons.iter().all(|p| p.label_a == "Ctrl" || p.label_b == "Ctrl"), "{method:?} Dunnett control");
            }
            // Asking for an empty data set explicitly is still an error.
            assert!(run_kind_sel(&sheet, kind, vec![0, 1, 2]).is_err(), "{method:?}/{mult:?} explicit empty selection");
        }
    }
}

#[test]
fn issue_82_posttest_families_match_scalar_studentized_range_sf() {
    // Verify batched post-test family evaluation across one-way (Tukey, Games-Howell, Dunnett T3)
    // and two-way (Standard marginal + simple effects, TwoFactorFamilies, PerSliceFamilies) agrees
    // with scalar `max_studentized_range_sf` / `studentized_range_sf` within tight D1 tolerances.
    let g = vec![
        group_input(0, "G1", 10.0, 1.1, 6.0),
        group_input(1, "G2", 11.4, 1.8, 7.0),
        group_input(2, "G3", 13.9, 0.9, 5.0),
        group_input(3, "G4", 18.2, 2.3, 8.0),
        group_input(4, "G5", 22.5, 1.4, 6.0),
    ];
    let df_pool = 27.0;
    let mse = 2.4;

    let tukey = compare_groups_with_procedure(&g, mse, df_pool, MultiplicityProcedure::TukeyKramer, 0.05, true).unwrap();
    assert_eq!(tukey.len(), 10);
    for row in &tukey {
        let scalar_p = curvebook_math::dist::studentized_range_sf(row.statistic, g.len(), row.df).unwrap();
        assert_approx_eq(row.adjusted_p_or_q, scalar_p, 1e-14, 1e-11, "one-way Tukey vs scalar");
    }

    let gh = compare_groups_with_procedure(&g, 0.0, 1.0, MultiplicityProcedure::GamesHowell, 0.05, true).unwrap();
    for row in &gh {
        let scalar_p = curvebook_math::dist::studentized_range_sf(row.statistic, g.len(), row.df).unwrap();
        assert_approx_eq(row.adjusted_p_or_q, scalar_p, 1e-14, 1e-11, "Games-Howell vs scalar");
    }

    let dt3 = compare_groups_with_procedure(&g, 0.0, 1.0, MultiplicityProcedure::DunnettT3, 0.05, true).unwrap();
    let m = g.len() * (g.len() - 1) / 2;
    for row in &dt3 {
        let scalar_p = max_studentized_range_sf(std::f64::consts::SQRT_2 * row.statistic.abs(), m, 2, row.df).unwrap();
        assert_approx_eq(row.adjusted_p_or_q, scalar_p, 1e-14, 1e-11, "Dunnett T3 vs scalar");
    }

    let sh_6x2 = grouped_sheet_fill("g62_i82", 2, 6, 3, |c, r, s| Some(10.0 + 1.1 * (r as f64) + 2.4 * (c as f64) + [-0.5, 0.1, 0.4][s]));
    let res_std = run_kind(
        &sh_6x2,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..Default::default()
        }),
    )
    .unwrap();
    let tab_simple = tab_by_id(&res_std, "multiple-comparisons-simple-a-within-b");
    assert_eq!(tab_simple.rows.len(), 30);
    for row in &tab_simple.rows {
        let q = cell_num(&row[2]);
        let df = cell_num(&row[3]);
        let p = cell_p(&row[5]);
        let scalar_p = max_studentized_range_sf(q, 2, 6, df).unwrap();
        assert_approx_eq(p, scalar_p, 1e-14, 1e-11, "two-way simple A within B vs scalar");
    }
    let tab_marg = tab_by_id(&res_std, "multiple-comparisons-factor-a");
    assert_eq!(tab_marg.rows.len(), 15);
    for row in &tab_marg.rows {
        let q = cell_num(&row[2]);
        let df = cell_num(&row[3]);
        let p = cell_p(&row[5]);
        let scalar_p = max_studentized_range_sf(q, 1, 6, df).unwrap();
        assert_approx_eq(p, scalar_p, 1e-14, 1e-11, "two-way marginal A vs scalar");
    }
}
