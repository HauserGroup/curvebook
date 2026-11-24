//! Regression and D1 oracle tests for audit findings #30–#37, #40, #81, and #109
//! covering ANOVA (one-way, Welch/BF, RM, Kruskal-Wallis, Friedman, factorial, REML/nested),
//! multiplicity adjustment (`try_adjust_p_values`, BKY 2006 Def. 6), and p-value stack validation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellValue, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, DataSheet, FactorialAnovaSpec,
    GroupedTable, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod,
    PValueStackMethod, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, SheetId, SubcolumnMode, SumOfSquaresType, TableArchetype,
    VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_p;
use curvebook_math::linalg::DenseMatrix;
use curvebook_stats::anova::{
    fit_random_intercept_reml, ordinary_one_way_anova, ordinary_one_way_from_summary, run_factorial_anova, run_nested_analysis, run_one_way_anova,
    unequal_variance_one_way_anova,
};
use curvebook_stats::multiplicity::{GroupComparisonInput, MultiplicityProcedure, adjust_p_values, adjust_two_stage_bky, try_adjust_p_values};
use curvebook_stats::pvalue_stack::analyze_pvalue_stack;
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::assert_close;

fn scalar(res: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    res.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn column_sheet(cols: &[(&str, &[f64])], hint: ColumnDesignHint) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut tab = ColumnTable::new(hint, cols.len(), n_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tab.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new("col-ds"), "Column Data", TableArchetype::Column(tab)).unwrap()
}

#[test]
fn finding_30_grouped_and_mv_drop_unobserved_levels_before_type_iii_and_rm() {
    // 1. Standard 2-way unbalanced GroupedTable: extra trailing blank row, excluded middle row,
    //    and completely blank/excluded column must match the exact trimmed 2x2 table.
    let build_2way = |with_unobserved: bool| -> DataSheet {
        let rows = if with_unobserved { 4 } else { 2 };
        let cols = if with_unobserved { 3 } else { 2 };
        let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(3), cols, rows).unwrap();
        gt.row_factor_title = String::from("Dose");
        gt.column_factor_title = String::from("Drug");
        gt.data_sets[0].title = String::from("Vehicle");
        gt.data_sets[1].title = String::from("Active");
        if with_unobserved {
            gt.data_sets[2].title = String::from("EmptyCol");
        }
        let target_rows = if with_unobserved { [0usize, 2usize] } else { [0usize, 1usize] };
        let data = [[[10.0, 11.0, 12.0], [15.0, 16.0, 17.0]], [[20.0, 22.0, 21.0], [31.0, 30.0, 32.0]]];
        // Unbalanced: omit one replicate in row 1, col 0 so Type III differs strongly from blank coding.
        for (r_idx, &r) in target_rows.iter().enumerate() {
            gt.row_titles[r] = format!("Dose{}", r_idx + 1);
            for c in 0..2 {
                for (s, &v) in data[r_idx][c].iter().enumerate() {
                    if !(r_idx == 1 && c == 0 && s == 2) {
                        gt.data_sets[c].cells[r][s] = Cell::number(v);
                    }
                }
            }
        }
        if with_unobserved {
            // Excluded values in row 1 (not in target_rows) and in col 2 (EmptyCol).
            gt.row_titles[1] = String::from("ExcludedRow");
            gt.row_titles[3] = String::from("BlankRow");
            gt.data_sets[0].cells[1][0] = Cell { value: CellValue::Number(999.0), excluded: true };
            gt.data_sets[1].cells[1][1] = Cell { value: CellValue::Number(888.0), excluded: true };
            gt.data_sets[2].cells[0][0] = Cell { value: CellValue::Number(777.0), excluded: true };
        }
        DataSheet::new(SheetId::new("g-2w"), "Grouped 2-Way", TableArchetype::Grouped(gt)).unwrap()
    };

    let fac_spec = FactorialAnovaSpec {
        ss_type: SumOfSquaresType::TypeIII,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
        ..FactorialAnovaSpec::default()
    };
    let trimmed = run_factorial_anova(
        &build_2way(false),
        &AnalysisSpec::new("res-t", "g-2w", AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap(),
        &fac_spec,
    )
    .unwrap();
    let padded = run_factorial_anova(
        &build_2way(true),
        &AnalysisSpec::new("res-p", "g-2w", AnalysisKind::FactorialAnova(fac_spec.clone())).unwrap(),
        &fac_spec,
    )
    .unwrap();

    for key in ["Dose: F", "Dose: P", "Drug: F", "Drug: P", "Dose × Drug: F", "Dose × Drug: P", "MS residual"] {
        assert_close(scalar(&padded, key), scalar(&trimmed, key), 1e-12, &format!("2way {key}"));
    }
    assert_eq!(padded.pairwise_comparisons.len(), trimmed.pairwise_comparisons.len());

    // 2. RM 2-way with trailing blank row/col/subcolumn and fit_reml_on_missing: false stays classical balanced RM.
    let build_rm_2way = |with_unobserved: bool| -> DataSheet {
        let rows = if with_unobserved { 3 } else { 2 };
        let cols = if with_unobserved { 4 } else { 3 };
        let subs = if with_unobserved { 4 } else { 3 };
        let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(subs), cols, rows).unwrap();
        let vals = [[[5.0, 6.0, 7.0], [8.0, 9.5, 10.5], [12.0, 13.0, 14.5]], [[7.0, 8.2, 9.1], [11.0, 12.0, 13.4], [15.5, 16.5, 17.8]]];
        for r in 0..2 {
            gt.row_titles[r] = format!("R{}", r + 1);
            for c in 0..3 {
                gt.data_sets[c].title = format!("C{}", c + 1);
                for s in 0..3 {
                    gt.data_sets[c].cells[r][s] = Cell::number(vals[r][c][s]);
                }
            }
        }
        if with_unobserved {
            gt.data_sets[3].cells[0][0] = Cell { value: CellValue::Number(50.0), excluded: true };
            gt.data_sets[0].cells[2][1] = Cell { value: CellValue::Number(60.0), excluded: true };
            gt.data_sets[1].cells[0][3] = Cell { value: CellValue::Number(70.0), excluded: true };
        }
        DataSheet::new(SheetId::new("g-rm"), "Grouped RM", TableArchetype::Grouped(gt)).unwrap()
    };
    let rm_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        fit_reml_on_missing: false,
        ..FactorialAnovaSpec::default()
    };
    let rm_trimmed = run_factorial_anova(
        &build_rm_2way(false),
        &AnalysisSpec::new("res-rm-t", "g-rm", AnalysisKind::FactorialAnova(rm_spec.clone())).unwrap(),
        &rm_spec,
    )
    .unwrap();
    let rm_padded = run_factorial_anova(
        &build_rm_2way(true),
        &AnalysisSpec::new("res-rm-p", "g-rm", AnalysisKind::FactorialAnova(rm_spec.clone())).unwrap(),
        &rm_spec,
    )
    .unwrap();
    for s in &rm_trimmed.scalars {
        assert_close(scalar(&rm_padded, &s.key), s.value, 1e-12, &format!("RM 2way {}", s.key));
    }

    // 3. Three-way 2x2xK with trailing blank row matches trimmed 2x2x2.
    let build_3way = |with_blank_row: bool| -> DataSheet {
        let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(2), 4, if with_blank_row { 3 } else { 2 }).unwrap();
        let vals = [[[10.0, 11.0], [13.0, 14.0], [16.0, 17.5], [20.0, 21.0]], [[12.0, 13.5], [15.5, 16.0], [19.0, 20.0], [25.0, 26.5]]];
        for r in 0..2 {
            gt.row_titles[r] = format!("Dose{}", r + 1);
            for c in 0..4 {
                for s in 0..2 {
                    gt.data_sets[c].cells[r][s] = Cell::number(vals[r][c][s]);
                }
            }
        }
        DataSheet::new(SheetId::new("g-3w"), "Three-Way", TableArchetype::Grouped(gt)).unwrap()
    };
    let three_spec = FactorialAnovaSpec { three_way: true, max_interaction_order: 3, ..FactorialAnovaSpec::default() };
    let t3 = run_factorial_anova(
        &build_3way(false),
        &AnalysisSpec::new("r-3t", "g-3w", AnalysisKind::FactorialAnova(three_spec.clone())).unwrap(),
        &three_spec,
    )
    .unwrap();
    let p3 = run_factorial_anova(
        &build_3way(true),
        &AnalysisSpec::new("r-3p", "g-3w", AnalysisKind::FactorialAnova(three_spec.clone())).unwrap(),
        &three_spec,
    )
    .unwrap();
    for s in &t3.scalars {
        assert_close(scalar(&p3, &s.key), s.value, 1e-12, &format!("3way {}", s.key));
    }

    // 4. MultipleVariables table with excluded/unobserved factor levels matches trimmed table.
    let build_mv = |with_excluded_level: bool| -> DataSheet {
        let base_rows = [
            ("Low", "Ctrl", 10.0),
            ("Low", "Ctrl", 11.0),
            ("Low", "Drug", 15.0),
            ("Low", "Drug", 16.5),
            ("High", "Ctrl", 20.0),
            ("High", "Ctrl", 21.0),
            ("High", "Drug", 28.0),
            ("High", "Drug", 29.5),
            ("Low", "Ctrl", 12.0), // unbalanced cell
        ];
        let total_rows = if with_excluded_level { base_rows.len() + 2 } else { base_rows.len() };
        let mut mv = MultipleVariablesTable::new(3, total_rows).unwrap();
        mv.variables[0].role = VariableRole::Continuous;
        mv.variables[0].title = String::from("Response");
        mv.variables[1].role = VariableRole::Categorical;
        mv.variables[1].title = String::from("Dose");
        mv.variables[2].role = VariableRole::Categorical;
        mv.variables[2].title = String::from("Regimen");
        for (r, &(a, b, y)) in base_rows.iter().enumerate() {
            mv.variables[0].cells[r] = Cell::number(y);
            mv.variables[1].cells[r] = Cell::text(a);
            mv.variables[2].cells[r] = Cell::text(b);
        }
        if with_excluded_level {
            let r1 = base_rows.len();
            mv.variables[0].cells[r1] = Cell::number(99.0);
            mv.variables[1].cells[r1] = Cell { value: CellValue::Text(String::from("GhostDose")), excluded: true };
            mv.variables[2].cells[r1] = Cell::text("Ctrl");

            let r2 = base_rows.len() + 1;
            mv.variables[0].cells[r2] = Cell { value: CellValue::Number(88.0), excluded: true };
            mv.variables[1].cells[r2] = Cell::text("High");
            mv.variables[2].cells[r2] = Cell::text("GhostRegimen");
        }
        DataSheet::new(SheetId::new("mv-fac"), "MV Factorial", TableArchetype::MultipleVariables(mv)).unwrap()
    };
    let mv_spec = FactorialAnovaSpec {
        response_column: Some(0),
        multifactor_columns: vec![1, 2],
        ss_type: SumOfSquaresType::TypeIII,
        ..FactorialAnovaSpec::default()
    };
    let mv_trimmed =
        run_factorial_anova(&build_mv(false), &AnalysisSpec::new("mv-t", "mv-fac", AnalysisKind::FactorialAnova(mv_spec.clone())).unwrap(), &mv_spec)
            .unwrap();
    let mv_padded =
        run_factorial_anova(&build_mv(true), &AnalysisSpec::new("mv-p", "mv-fac", AnalysisKind::FactorialAnova(mv_spec.clone())).unwrap(), &mv_spec)
            .unwrap();
    for s in &mv_trimmed.scalars {
        assert_close(scalar(&mv_padded, &s.key), s.value, 1e-12, &format!("MV {}", s.key));
    }
}

#[test]
fn finding_31_raw_one_way_anova_honors_requested_confidence_level_for_effect_size_ci() {
    let g1 = [5.1, 5.4, 5.8, 6.0, 5.7];
    let g2 = [7.2, 7.6, 7.9, 8.1, 7.5];
    let g3 = [9.0, 9.4, 9.8, 10.1, 9.6];
    let sheet = column_sheet(&[("A", &g1), ("B", &g2), ("C", &g3)], ColumnDesignHint::UnpairedReplicates);

    let make_ci = |pct: f64| -> (f64, f64) {
        let conf = ConfidenceLevel::from_percent(pct).unwrap();
        let ow = OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, compute_effect_sizes: true, ..OneWayAnovaSpec::default() };
        let mut spec = AnalysisSpec::new(SheetId::new("r"), sheet.id.clone(), AnalysisKind::OneWayAnova(ow)).unwrap();
        spec.confidence_level = conf;
        let res = run_analysis(&sheet, &spec).unwrap();
        let tab = res.tabs.iter().find(|t| t.id == "anova-table").unwrap();
        let ci_col = tab.column_headers.iter().position(|h| h.contains(&format!("{}% CI", conf.percent()))).unwrap();
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = tab.rows[0][ci_col] else {
            panic!("expected CI cell")
        };

        // Must exactly match summary path at identical ConfidenceLevel (Steiger 2004).
        let raw_direct = ordinary_one_way_anova(&[("A", &g1), ("B", &g2), ("C", &g3)], conf).unwrap();
        let sum_inputs: Vec<GroupComparisonInput> = raw_direct
            .group_n
            .iter()
            .enumerate()
            .map(|(i, &n)| GroupComparisonInput {
                index: i,
                label: raw_direct.group_labels[i].clone(),
                n: n as f64,
                mean: raw_direct.group_means[i],
                sd: raw_direct.group_sds[i],
            })
            .collect();
        let sum_direct = ordinary_one_way_from_summary(&sum_inputs, conf).unwrap();
        let (slo, shi) = sum_direct.effect_sizes.partial_eta_squared_ci.unwrap();
        assert_close(lo, slo, 1e-12, "raw vs summary CI lo");
        assert_close(hi, shi, 1e-12, "raw vs summary CI hi");
        (lo, hi)
    };

    let (lo90, hi90) = make_ci(90.0);
    let (lo95, hi95) = make_ci(95.0);
    let (lo99, hi99) = make_ci(99.0);
    assert!(hi90 - lo90 < hi95 - lo95 && hi95 - lo95 < hi99 - lo99, "CI widths must increase strictly with confidence level");
}

#[test]
fn finding_32_anova_entrypoints_reject_oob_and_duplicate_selected_data_sets() {
    let raw_col = column_sheet(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])], ColumnDesignHint::UnpairedReplicates);
    for method in [
        OneWayMethod::OrdinaryAnova,
        OneWayMethod::BrownForsytheAndWelch,
        OneWayMethod::KruskalWallis,
        OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        OneWayMethod::Friedman,
    ] {
        let ow = OneWayAnovaSpec { method, ..OneWayAnovaSpec::default() };
        let mut oob = AnalysisSpec::new("oob", raw_col.id.clone(), AnalysisKind::OneWayAnova(ow.clone())).unwrap();
        oob.selected_data_sets = vec![0, 5];
        assert!(matches!(run_one_way_anova(&raw_col, &oob, &ow), Err(StatsError::MissingColumn { index: 5 })));

        let mut dup = AnalysisSpec::new("dup", raw_col.id.clone(), AnalysisKind::OneWayAnova(ow.clone())).unwrap();
        dup.selected_data_sets = vec![0, 0, 1];
        assert!(matches!(run_one_way_anova(&raw_col, &dup, &ow), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
    }

    // Grouped factorial ANOVA rejects OOB and duplicate selected_data_sets.
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).unwrap();
    for r in 0..2 {
        for c in 0..2 {
            for s in 0..2 {
                gt.data_sets[c].cells[r][s] = Cell::number((r + c + s + 1) as f64);
            }
        }
    }
    let gs = DataSheet::new("g-sel", "G", TableArchetype::Grouped(gt)).unwrap();
    let fac = FactorialAnovaSpec::default();
    let mut g_oob = AnalysisSpec::new("goob", gs.id.clone(), AnalysisKind::FactorialAnova(fac.clone())).unwrap();
    g_oob.selected_data_sets = vec![0, 4];
    assert!(matches!(run_factorial_anova(&gs, &g_oob, &fac), Err(StatsError::MissingColumn { index: 4 })));
    let mut g_dup = AnalysisSpec::new("gdup", gs.id.clone(), AnalysisKind::FactorialAnova(fac.clone())).unwrap();
    g_dup.selected_data_sets = vec![1, 1];
    assert!(matches!(run_factorial_anova(&gs, &g_dup, &fac), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    // Nested ANOVA rejects OOB and duplicate selected_data_sets.
    let mut nt = NestedTable::new(2, 2, 2).unwrap();
    for g in 0..2 {
        for b in 0..2 {
            for r in 0..2 {
                nt.groups[g].cells[r][b] = Cell::number((g * 10 + b * 2 + r) as f64);
            }
        }
    }
    let ns = DataSheet::new("n-sel", "N", TableArchetype::Nested(nt)).unwrap();
    let nspec = NestedAnalysisSpec::default();
    let mut n_oob = AnalysisSpec::new("noob", ns.id.clone(), AnalysisKind::NestedComparison(nspec.clone())).unwrap();
    n_oob.selected_data_sets = vec![0, 7];
    assert!(matches!(run_nested_analysis(&ns, &n_oob, &nspec), Err(StatsError::MissingColumn { index: 7 })));
    let mut n_dup = AnalysisSpec::new("ndup", ns.id.clone(), AnalysisKind::NestedComparison(nspec.clone())).unwrap();
    n_dup.selected_data_sets = vec![0, 1, 0];
    assert!(matches!(run_nested_analysis(&ns, &n_dup, &nspec), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
}

#[test]
fn finding_33_n_eq_1_group_keeps_primary_one_way_and_kw_marks_variance_unavailable() {
    let sheet = column_sheet(&[("Single", &[5.0]), ("G2", &[1.0, 2.0, 3.0]), ("G3", &[7.0, 8.0, 9.0])], ColumnDesignHint::UnpairedReplicates);
    let direct =
        ordinary_one_way_anova(&[("Single", &[5.0]), ("G2", &[1.0, 2.0, 3.0]), ("G3", &[7.0, 8.0, 9.0])], ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(!direct.variance_tests.available);
    assert!(direct.variance_tests.bartlett_p.is_nan() && direct.variance_tests.brown_forsythe_p.is_nan());
    // SS_within = 2 + 2 = 4, df_within = 7 - 3 = 4 -> MS_within = 1.0; group means = 5, 2, 8 (grand mean = 5)
    // SS_between = 1*(0)^2 + 3*(9) + 3*(9) = 54, MS_between = 27 -> F = 27.0.
    assert_close(direct.f_statistic, 27.0, 1e-12, "n==1 F");

    let ow_spec = OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() };
    let res =
        run_one_way_anova(&sheet, &AnalysisSpec::new("r1", sheet.id.clone(), AnalysisKind::OneWayAnova(ow_spec.clone())).unwrap(), &ow_spec).unwrap();
    assert_close(scalar(&res, "F"), 27.0, 1e-12, "one-way sheet n==1 F");
    let vh_tab = res.tabs.iter().find(|t| t.id == "variance-homogeneity").unwrap();
    assert!(matches!(&vh_tab.rows[0][4], ResultCell::Text(s) if s.contains("n < 2")));
    assert!(!vh_tab.footnotes.iter().any(|f| f.contains("Mean/SD/N")));

    let kw_spec = OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..OneWayAnovaSpec::default() };
    let kw_res =
        run_one_way_anova(&sheet, &AnalysisSpec::new("rkw", sheet.id.clone(), AnalysisKind::OneWayAnova(kw_spec.clone())).unwrap(), &kw_spec)
            .unwrap();
    assert!(scalar(&kw_res, "Kruskal-Wallis H").is_finite());

    // Even when every group has n == 1 (total N = 3), Kruskal-Wallis succeeds.
    let singletons = column_sheet(&[("S1", &[1.0]), ("S2", &[2.0]), ("S3", &[3.0])], ColumnDesignHint::UnpairedReplicates);
    let kw_s = run_one_way_anova(
        &singletons,
        &AnalysisSpec::new("rkws", singletons.id.clone(), AnalysisKind::OneWayAnova(kw_spec.clone())).unwrap(),
        &kw_spec,
    )
    .unwrap();
    assert_close(scalar(&kw_s, "Kruskal-Wallis H"), 2.0, 1e-12, "all-n1 Kruskal-Wallis H");
}

#[test]
fn finding_34_welch_bf_anova_honors_all_post_tests_without_silent_substitution() {
    let g0 = [10.0, 10.2, 10.5, 9.8, 10.1];
    let g1 = [13.0, 14.5, 11.5, 15.0, 12.5];
    let g2 = [20.0, 26.0, 14.0, 29.0, 17.0];
    let sheet = column_sheet(&[("Ctrl", &g0), ("Med", &g1), ("High", &g2)], ColumnDesignHint::UnpairedReplicates);

    let run_w = |method: MultiplicityMethod| {
        let ow = OneWayAnovaSpec {
            method: OneWayMethod::BrownForsytheAndWelch,
            multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
            ..OneWayAnovaSpec::default()
        };
        run_one_way_anova(&sheet, &AnalysisSpec::new("w", sheet.id.clone(), AnalysisKind::OneWayAnova(ow.clone())).unwrap(), &ow)
    };

    // None -> zero pairwise comparisons, never Games-Howell!
    let none_res = run_w(MultiplicityMethod::None).unwrap();
    assert!(none_res.pairwise_comparisons.is_empty());
    assert!(!none_res.tabs.iter().any(|t| t.id == "multiple-comparisons"));

    // Dunnett -> k - 1 = 2 comparisons against Ctrl.
    let dun_res = run_w(MultiplicityMethod::Dunnett { control_column: 0 }).unwrap();
    assert_eq!(dun_res.pairwise_comparisons.len(), 2);
    assert!(dun_res.pairwise_comparisons.iter().all(|c| c.label_b == "Ctrl"));

    // Bonferroni -> Welch t raw_p adjusted by exact Bonferroni factor m = 3.
    let bonf_res = run_w(MultiplicityMethod::Bonferroni).unwrap();
    assert_eq!(bonf_res.pairwise_comparisons.len(), 3);
    let m0 = 10.12_f64;
    let m1 = 13.3_f64;
    let v0 = g0.iter().map(|&x| (x - m0).powi(2)).sum::<f64>() / (4.0 * 5.0);
    let v1 = g1.iter().map(|&x| (x - m1).powi(2)).sum::<f64>() / (4.0 * 5.0);
    let se01 = (v0 + v1).sqrt();
    let df01 = (v0 + v1).powi(2) / (v0 * v0 / 4.0 + v1 * v1 / 4.0);
    let raw01 = student_t_two_sided_p((m0 - m1) / se01, df01).unwrap();
    assert_close(bonf_res.pairwise_comparisons[0].raw_p_or_q, (3.0 * raw01).min(1.0), 1e-10, "Welch Bonferroni pair 0-1");

    // BH FDR -> marks FDR flag true.
    let bh_res = run_w(MultiplicityMethod::BenjaminiHochbergFdr).unwrap();
    assert!(bh_res.pairwise_comparisons.iter().all(|c| c.is_fdr_q));

    // LinearTrend -> emits Linear trend scalars & tab.
    let lt_res = run_w(MultiplicityMethod::LinearTrend).unwrap();
    assert!(lt_res.tabs.iter().any(|t| t.id == "linear-trend"));
    assert!(scalar(&lt_res, "Linear trend F").is_finite());

    // TamhaneT2 / DunnettT3 -> accurate method titles (not lumped "Games-Howell / T2 / T3").
    let t2_res = run_w(MultiplicityMethod::TamhaneT2).unwrap();
    assert!(t2_res.tabs.iter().any(|t| t.title.contains("Tamhane's T2")));

    // TukeyHsd -> explicit documented Games-Howell substitution in checklist & footnote.
    let tuk_res = run_w(MultiplicityMethod::TukeyHsd).unwrap();
    let mc = tuk_res.tabs.iter().find(|t| t.id == "multiple-comparisons").unwrap();
    assert!(mc.title.contains("Games-Howell") && !mc.footnotes.is_empty());
    assert!(tuk_res.checklist.iter().any(|c| c.assumption == "Post-hoc multiplicity procedure" && c.status == ChecklistStatus::Info));

    // DunnRankSum -> rejected with IncompatibleAnalysisSpec.
    assert!(matches!(run_w(MultiplicityMethod::DunnRankSum), Err(StatsError::IncompatibleAnalysisSpec { .. })));
}

#[test]
fn finding_35_rm_anova_posthoc_validation_and_friedman_dunn() {
    let sheet = column_sheet(
        &[("T0", &[1.0, 2.0, 4.0, 5.0, 6.0, 7.0]), ("T1", &[3.0, 4.5, 6.0, 7.5, 8.0, 9.0]), ("T2", &[6.0, 7.0, 9.5, 11.0, 12.0, 13.0])],
        ColumnDesignHint::PairedRepeatedMeasures,
    );

    // Classical & REML RM reject GamesHowell / TamhaneT2 / DunnettT3 / DunnRankSum.
    for rm_opt in [false, true] {
        for bad_method in
            [MultiplicityMethod::GamesHowell, MultiplicityMethod::TamhaneT2, MultiplicityMethod::DunnettT3, MultiplicityMethod::DunnRankSum]
        {
            let ow = OneWayAnovaSpec {
                method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: !rm_opt },
                mixed_effects_reml: rm_opt,
                multiplicity: MultiplicitySpec { method: bad_method, threshold: 0.05, report_confidence_intervals: true },
                ..OneWayAnovaSpec::default()
            };
            let res = run_one_way_anova(&sheet, &AnalysisSpec::new("rm-bad", sheet.id.clone(), AnalysisKind::OneWayAnova(ow.clone())).unwrap(), &ow);
            assert!(matches!(res, Err(StatsError::IncompatibleAnalysisSpec { .. })), "expected rejection for {bad_method:?}");
        }
    }

    // RM with geisser_greenhouse=true & Bonferroni computes matched paired-difference contrasts (df = n - 1 = 5).
    let rm_gg = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
        ..OneWayAnovaSpec::default()
    };
    let gg_res =
        run_one_way_anova(&sheet, &AnalysisSpec::new("gg", sheet.id.clone(), AnalysisKind::OneWayAnova(rm_gg.clone())).unwrap(), &rm_gg).unwrap();
    let gg_tab = gg_res.tabs.iter().find(|t| t.id == "multiple-comparisons").unwrap();
    let ResultCell::Number { value: df_val, .. } = gg_tab.rows[0][3] else { panic!("expected numeric df") };
    assert_close(df_val, 5.0, 1e-12, "GG paired contrast df = n - 1");

    // Friedman with DunnRankSum emits Dunn's Friedman post-test (exact SE = sqrt(k*(k+1)/(6*n)) = sqrt(12/36) = 0.57735...).
    let fr_dunn = OneWayAnovaSpec {
        method: OneWayMethod::Friedman,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::DunnRankSum, threshold: 0.05, report_confidence_intervals: false },
        ..OneWayAnovaSpec::default()
    };
    let fr_res =
        run_one_way_anova(&sheet, &AnalysisSpec::new("frd", sheet.id.clone(), AnalysisKind::OneWayAnova(fr_dunn.clone())).unwrap(), &fr_dunn)
            .unwrap();
    assert_eq!(fr_res.pairwise_comparisons.len(), 3);
    // Strict consistent ranking 1 < 2 < 3 across all 6 blocks: |mean_rank_diff(T0, T2)| = 2.0, SE = 1/sqrt(3) -> |Z| = sqrt(12)
    let t0_t2 = fr_res.pairwise_comparisons.iter().find(|p| p.label_a == "T0" && p.label_b == "T2").unwrap();
    assert!(t0_t2.significant);

    // Friedman with TukeyHsd omits parametric post-test cleanly and flags Info checklist + footnote.
    let fr_tuk = OneWayAnovaSpec {
        method: OneWayMethod::Friedman,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
        ..OneWayAnovaSpec::default()
    };
    let fr_tuk_res =
        run_one_way_anova(&sheet, &AnalysisSpec::new("frt", sheet.id.clone(), AnalysisKind::OneWayAnova(fr_tuk.clone())).unwrap(), &fr_tuk).unwrap();
    assert!(fr_tuk_res.pairwise_comparisons.is_empty());
    assert!(fr_tuk_res.checklist.iter().any(|c| c.assumption == "Post-hoc multiple comparisons" && c.status == ChecklistStatus::Info));
}

#[test]
fn finding_36_summary_anova_rejects_fractional_and_non_finite_n() {
    for bad_n in [4.5, f64::NAN, f64::INFINITY] {
        let g = vec![
            GroupComparisonInput { index: 0, label: String::from("A"), n: bad_n, mean: 10.0, sd: 1.5 },
            GroupComparisonInput { index: 1, label: String::from("B"), n: 5.0, mean: 12.0, sd: 1.5 },
        ];
        assert!(matches!(ordinary_one_way_from_summary(&g, ConfidenceLevel::DEFAULT_95), Err(StatsError::InvalidParameter { name: "group_n", .. })));
        assert!(matches!(unequal_variance_one_way_anova(&g), Err(StatsError::InvalidParameter { name: "group_n", .. })));

        for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
            let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1).unwrap();
            tab.subcolumn_mode = mode;
            tab.data_sets[0].cells = vec![vec![Cell::number(10.0), Cell::number(1.5), Cell { value: CellValue::Number(bad_n), excluded: false }]];
            tab.data_sets[1].cells = vec![vec![Cell::number(12.0), Cell::number(1.5), Cell::number(5.0)]];
            let ds = DataSheet::new("sum-badn", "Summary", TableArchetype::Column(tab)).unwrap();
            let ow = OneWayAnovaSpec::default();
            let res = run_one_way_anova(&ds, &AnalysisSpec::new("sb", ds.id.clone(), AnalysisKind::OneWayAnova(ow.clone())).unwrap(), &ow);
            assert!(matches!(res, Err(StatsError::InvalidParameter { name: "group_n", .. })));
        }
    }
}

#[test]
fn finding_37_random_intercept_reml_large_tau_eps_01_and_001_and_boundary_flags() {
    // Balanced 2-group nested design: 3 clusters/group, 3 replicates/cluster.
    // Group 1 cluster means = [1.0, 2.0, 3.0], Group 2 cluster means = [1.5, 2.5, 3.5].
    // Exact hierarchical F = MS_group / MS_subcluster = (18 * 0.25^2 / 1) / (3 * 2 * 2 / 4) = 1.125 / 3.0 = 0.375,
    // P(F_{1,4} >= 0.375) = 0.573399... regardless of tiny within-cluster spread eps = 0.01 or 0.001!
    for eps in [0.01_f64, 0.001_f64] {
        let mut nt = NestedTable::new(2, 3, 3).unwrap();
        let means = [[1.0, 2.0, 3.0], [1.5, 2.5, 3.5]];
        let mut x_data = Vec::with_capacity(18 * 2);
        let mut y_vec = Vec::with_capacity(18);
        let mut cluster_ids = Vec::with_capacity(18);
        for g in 0..2 {
            for b in 0..3 {
                let mu = means[g][b];
                for (r, offset) in [-eps, 0.0, eps].into_iter().enumerate() {
                    let y = mu + offset;
                    nt.groups[g].cells[r][b] = Cell::number(y);
                    x_data.push(1.0);
                    x_data.push(if g == 1 { 1.0 } else { 0.0 });
                    y_vec.push(y);
                    cluster_ids.push(g * 3 + b);
                }
            }
        }
        let x_mat = DenseMatrix::from_row_major(18, 2, x_data).unwrap();
        let reml = fit_random_intercept_reml(&x_mat, &y_vec, &cluster_ids, &[("Group", 1, 1)]).unwrap();
        assert!(reml.converged && !reml.at_boundary, "eps={eps}: expected interior convergence");
        assert_close(reml.fixed_terms[0].f_statistic.unwrap(), 0.375, 1e-6, &format!("REML F eps={eps}"));
        assert_close(reml.fixed_terms[0].p_value.unwrap(), 0.573_399, 1e-4, &format!("REML P eps={eps}"));

        let ns = DataSheet::new("nest-eps", "Nested Eps", TableArchetype::Nested(nt)).unwrap();
        let nspec = NestedAnalysisSpec::default();
        let n_res = run_nested_analysis(&ns, &AnalysisSpec::new("rn", ns.id.clone(), AnalysisKind::NestedComparison(nspec.clone())).unwrap(), &nspec)
            .unwrap();
        assert_close(scalar(&n_res, "F (among groups)"), 0.375, 1e-6, &format!("Nested F eps={eps}"));
        assert_close(scalar(&n_res, "P value (groups)"), 0.573_399, 1e-4, &format!("Nested P eps={eps}"));
    }

    // Zero cluster variance boundary: constant cluster means with nonzero replicate noise -> sigma2_random == 0, at_boundary == true, converged == true.
    let mut nt_zero = NestedTable::new(2, 3, 3).unwrap();
    let mut x_z = Vec::with_capacity(18 * 2);
    let mut y_z = Vec::with_capacity(18);
    let mut cid_z = Vec::with_capacity(18);
    for g in 0..2 {
        for b in 0..3 {
            for (r, &v) in [-1.0, 0.0, 1.0].iter().enumerate() {
                let y = (g as f64) * 2.0 + v;
                nt_zero.groups[g].cells[r][b] = Cell::number(y);
                x_z.push(1.0);
                x_z.push(if g == 1 { 1.0 } else { 0.0 });
                y_z.push(y);
                cid_z.push(g * 3 + b);
            }
        }
    }
    let xz_mat = DenseMatrix::from_row_major(18, 2, x_z).unwrap();
    let reml_zero = fit_random_intercept_reml(&xz_mat, &y_z, &cid_z, &[("Group", 1, 1)]).unwrap();
    assert_close(reml_zero.sigma2_random, 0.0, 1e-12, "zero boundary sigma2_random");
    assert!(reml_zero.at_boundary && reml_zero.converged);
    let ns_zero = DataSheet::new("nz", "Nested Zero", TableArchetype::Nested(nt_zero)).unwrap();
    let nspec_z = NestedAnalysisSpec::default();
    let nz_res = run_nested_analysis(
        &ns_zero,
        &AnalysisSpec::new("rnz", ns_zero.id.clone(), AnalysisKind::NestedComparison(nspec_z.clone())).unwrap(),
        &nspec_z,
    )
    .unwrap();
    assert!(nz_res.checklist.iter().any(|c| c.assumption == "REML convergence and variance boundaries" && c.status == ChecklistStatus::Info));

    // Strictly zero replicate variance with nonzero cluster differences hits upper ceiling -> converged=false, at_boundary=true.
    let mut x_ceil = Vec::with_capacity(18 * 2);
    let mut y_ceil = Vec::with_capacity(18);
    let mut cid_ceil = Vec::with_capacity(18);
    for g in 0..2 {
        for b in 0..3 {
            let mu = [1.0, 2.0, 3.0][b] + 0.5 * (g as f64);
            for _ in 0..3 {
                x_ceil.push(1.0);
                x_ceil.push(if g == 1 { 1.0 } else { 0.0 });
                y_ceil.push(mu);
                cid_ceil.push(g * 3 + b);
            }
        }
    }
    let xc_mat = DenseMatrix::from_row_major(18, 2, x_ceil).unwrap();
    let reml_ceil = fit_random_intercept_reml(&xc_mat, &y_ceil, &cid_ceil, &[("Group", 1, 1)]).unwrap();
    assert!(reml_ceil.at_boundary && !reml_ceil.converged);
}

#[test]
fn finding_40_bky_two_stage_fdr_ten_discoveries_and_bky_def6() {
    // Benjamini, Krieger & Yekutieli (2006) Def. 6:
    // Stage 1 at q' = 0.05/1.05 rejects r1 = 9 of m = 10 hypotheses -> m0_hat = 1.
    // Stage 2 threshold scaling gives 0.2 * 1 * 1.05 / 10 = 0.021 <= 0.05 -> all 10 rejected.
    let raw = [0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.007, 0.008, 0.009, 0.2];
    let direct = adjust_two_stage_bky(&raw, 0.05);
    assert_eq!(direct.iter().filter(|&&q| q <= 0.05).count(), 10);
    assert_close(direct[9], 0.021, 1e-12, "BKY Def 6 10th adjusted value < raw_p");

    let (via_try, is_fdr) = try_adjust_p_values(&raw, MultiplicityProcedure::TwoStageBkyFdr, 0.05).unwrap();
    assert!(is_fdr && via_try == direct);

    let stack_res = analyze_pvalue_stack(&raw, PValueStackMethod::TwoStageBky, 0.05).unwrap();
    assert_eq!(stack_res.discovery_count, 10);
    assert_close(stack_res.rows[9].adjusted_p_or_q, 0.021, 1e-12, "pvalue_stack BKY 10th row");
}

#[test]
fn finding_81_try_adjust_p_values_rejects_anova_only_procedures() {
    let raw = [0.01, 0.04, 0.20];
    let anova_only = [
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::DunnRankSum,
        MultiplicityProcedure::LinearTrend,
        MultiplicityProcedure::TamhaneT2,
    ];
    for proc in anova_only {
        assert!(matches!(try_adjust_p_values(&raw, proc, 0.05), Err(StatsError::IncompatibleAnalysisSpec { .. })));
    }
    let valid_raw_p = [
        MultiplicityProcedure::None,
        MultiplicityProcedure::FisherLsd,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
        MultiplicityProcedure::TwoStageBkyFdr,
    ];
    for proc in valid_raw_p {
        let res = try_adjust_p_values(&raw, proc, 0.05).unwrap();
        assert_eq!(res, adjust_p_values(&raw, proc, 0.05));
    }
}

#[test]
fn finding_109_rm_two_way_honors_fit_reml_on_missing_false_and_records_estimator() {
    let build_rm = |missing_middle_cell: bool| -> DataSheet {
        let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 3).unwrap();
        let vals = [[[10.0, 11.0, 12.0], [15.0, 16.0, 17.5]], [[12.0, 13.5, 14.0], [18.0, 19.0, 20.5]], [[14.0, 15.0, 16.0], [21.0, 22.0, 23.0]]];
        for r in 0..3 {
            for c in 0..2 {
                for s in 0..3 {
                    if !(missing_middle_cell && r == 1 && c == 1 && s == 2) {
                        gt.data_sets[c].cells[r][s] = Cell::number(vals[r][c][s]);
                    }
                }
            }
        }
        DataSheet::new("rm2-109", "RM2", TableArchetype::Grouped(gt)).unwrap()
    };

    // Incomplete RM with fit_reml_on_missing = false returns typed error instead of silent REML.
    let miss_sheet = build_rm(true);
    let no_reml = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        fit_reml_on_missing: false,
        mixed_effects_reml: false,
        ..FactorialAnovaSpec::default()
    };
    let err = run_factorial_anova(
        &miss_sheet,
        &AnalysisSpec::new("r-err", miss_sheet.id.clone(), AnalysisKind::FactorialAnova(no_reml.clone())).unwrap(),
        &no_reml,
    );
    assert!(matches!(err, Err(StatsError::InsufficientSampleSize { .. })));

    // Incomplete RM with fit_reml_on_missing = true runs Compound Symmetry REML and records estimator.
    let yes_reml = FactorialAnovaSpec { fit_reml_on_missing: true, ..no_reml.clone() };
    let cs_res = run_factorial_anova(
        &miss_sheet,
        &AnalysisSpec::new("r-cs", miss_sheet.id.clone(), AnalysisKind::FactorialAnova(yes_reml.clone())).unwrap(),
        &yes_reml,
    )
    .unwrap();
    let cs_item = cs_res.checklist.iter().find(|c| c.assumption == "Repeated-measures estimator").unwrap();
    assert!(cs_item.diagnostic_metric.as_deref().unwrap().contains("Compound Symmetry"));
    let cs_tab = cs_res.tabs.iter().find(|t| t.id == "factorial-anova").unwrap();
    assert!(cs_tab.footnotes.iter().any(|f| f.contains("Compound Symmetry")));

    // Unstructured covariance REML records Unstructured estimator.
    let un_reml = FactorialAnovaSpec { residual_covariance: ResidualCovariance::Unstructured, ..yes_reml };
    let un_res = run_factorial_anova(
        &miss_sheet,
        &AnalysisSpec::new("r-un", miss_sheet.id.clone(), AnalysisKind::FactorialAnova(un_reml.clone())).unwrap(),
        &un_reml,
    )
    .unwrap();
    let un_item = un_res.checklist.iter().find(|c| c.assumption == "Repeated-measures estimator").unwrap();
    assert!(un_item.diagnostic_metric.as_deref().unwrap().contains("Unstructured"));

    // Complete RM with fit_reml_on_missing = false records Classical balanced RM ANOVA (OLS).
    let comp_sheet = build_rm(false);
    let bal_res = run_factorial_anova(
        &comp_sheet,
        &AnalysisSpec::new("r-bal", comp_sheet.id.clone(), AnalysisKind::FactorialAnova(no_reml.clone())).unwrap(),
        &no_reml,
    )
    .unwrap();
    let bal_item = bal_res.checklist.iter().find(|c| c.assumption == "Repeated-measures estimator").unwrap();
    assert!(bal_item.diagnostic_metric.as_deref().unwrap().contains("Classical balanced"));
    let bal_tab = bal_res.tabs.iter().find(|t| t.id == "factorial-anova").unwrap();
    assert!(bal_tab.footnotes.iter().any(|f| f.contains("Classical balanced")));
    assert!(bal_res.tabs[0].footnotes.iter().any(|f| f.contains("Classical balanced")));
}
