//! Consolidated regression & D1 oracle tests for two-group, column, descriptive,
//! normality, outlier, Bland-Altman, and multiple-row analyses.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, BlandAltmanSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, MultipleRowTestsSpec,
    MultipleVariablesTable, MultiplicitySpec, OutlierSpec, ResultCell, ResultTabKind, SequenceProgression, SubcolumnAggregation, SubcolumnMode,
    TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableColumn, VariableRole, XColumnMode, XyTable,
};
use curvebook_math::dist::norm_sf;
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::column::{
    anderson_darling, one_sample_ks_normal, one_sample_ks_normal_at_alpha, one_sample_t_test, one_sample_wilcoxon, rout_outliers, shapiro_wilk,
    summarize_column,
};
use curvebook_stats::error::StatsError;
use curvebook_stats::multiplicity::MultiplicityProcedure;
use curvebook_stats::two_group::{
    bland_altman, levene_variance_test, mann_whitney_u, multiple_row_t_tests, one_sample_t_test_from_summary, paired_sign_test, paired_t_test,
    ratio_paired_t_test, unpaired_t_test, welch_t_test, wilcoxon_matched_pairs,
};
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_all_sheet_numbers_finite, assert_close, cell_ci_finite, cell_num, cell_p_stars, col_sheet, col_sheet_hint, grouped_sheet_fill,
    mv_cells_sheet, mv_num_sheet, presum_col_sheet, row_cell, run_kind, run_kind_sel, scalar, scalar_ds, scalar_opt, tab_by_id, tab_by_kind,
    xy_multi_sheet, xy_sheet,
};

#[test]
fn audit_04_104_bland_altman_xy_modes_replicates_percent_and_scale() {
    let xs = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let y1 = [11.0, 19.0, 32.0, 39.0, 51.0, 58.0];
    let y2 = [100.0, 200.0, 300.0, 400.0, 500.0, 600.0];
    let one_y = xy_multi_sheet("ba-1y", &xs, &[("Method B", &y1)]);
    let two_y = xy_multi_sheet("ba-2y", &xs, &[("Method B", &y1), ("Method C", &y2)]);
    let exp_xy = bland_altman(&xs, &y1, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let exp_yy = bland_altman(&y1, &y2, false, ConfidenceLevel::DEFAULT_95).unwrap();

    let ba = |a, b, pct, xy_cmp| {
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: a, column_b: b, percent_difference: pct, xy_compare_x_vs_y: xy_cmp })
    };

    // Valid XY mode matrix (Some(true) stays X vs Y[0], Some(false) compares Y[a] vs Y[b], None legacy).
    for (label, sheet, kind, exp_bias) in [
        ("locked 1y", &one_y, ba(0, 1, false, Some(true)), exp_xy.bias),
        ("locked 2y", &two_y, ba(0, 1, false, Some(true)), exp_xy.bias),
        ("y-vs-y 2y", &two_y, ba(0, 1, false, Some(false)), exp_yy.bias),
        ("legacy 1y", &one_y, ba(0, 1, false, None), exp_xy.bias),
        ("legacy 2y", &two_y, ba(0, 1, false, None), exp_yy.bias),
    ] {
        let res = run_kind(sheet, kind).unwrap();
        assert_close(scalar_ds(&res, "Bias", None), exp_bias, 1e-12, label);
        if label == "locked 2y" {
            assert_eq!(tab_by_id(&res, "bland-altman-summary").column_headers, vec![String::from("X vs. Method B")]);
        }
    }

    // Invalid / equal XY column index matrix.
    for (sheet, kind, expect_missing) in [
        (&two_y, ba(0, 2, false, Some(true)), true),
        (&one_y, ba(0, 1, false, Some(false)), true),
        (&one_y, ba(99, 98, false, None), true),
        (&one_y, ba(0, 98, false, None), true),
        (&two_y, ba(0, 5, false, None), true),
        (&one_y, ba(0, 0, false, None), false),
        (&two_y, ba(0, 0, false, None), false),
    ] {
        let err = run_kind(sheet, kind).unwrap_err();
        if expect_missing {
            assert!(matches!(err, StatsError::MissingColumn { .. }), "expected MissingColumn, got {err:?}");
        } else {
            assert!(matches!(err, StatsError::InvalidParameter { name: "column_b", .. }), "expected equal column_b err, got {err:?}");
        }
    }

    // XY RawReplicates(2) Y[A] vs Y[B], X vs Y[0] text/missing, and GeneratedSequence X tracking.
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap();
    for r in 0..4 {
        let base = ((r + 1) * 10) as f64;
        xy_rep.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy_rep.data_sets[0].cells[r] = vec![Cell::number(base), if r == 2 { Cell::text("ND") } else { Cell::number(base + 2.0) }];
        xy_rep.data_sets[1].cells[r] = vec![Cell::number(base + 1.0), Cell::number(base + 3.0)];
    }
    let sh_rep = DataSheet::new("xy-ba-rep", "XYRep", TableArchetype::Xy(xy_rep)).unwrap();
    let res_rep = run_kind(&sh_rep, ba(0, 1, false, Some(false))).unwrap();
    assert_close(scalar(&res_rep, "N"), 7.0, 1e-12, "XY replicates paired N");
    assert_close(scalar(&res_rep, "Bias"), -1.0, 1e-12, "XY replicates bias");
    assert!(res_rep.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(res_rep.checklist.iter().any(|c| c.assumption == "Complete-case row alignment"));

    let mut xy_x = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    for (r, (xc, yc)) in [
        (Cell::number(10.0), Cell::number(10.5)),
        (Cell::text("ND"), Cell::number(20.5)),
        (Cell::number(30.0), Cell::empty()),
        (Cell::number(40.0), Cell::number(41.0)),
        (Cell::number(50.0), Cell::number(50.5)),
    ]
    .into_iter()
    .enumerate()
    {
        xy_x.set_x_cell(r, xc).unwrap();
        xy_x.data_sets[0].cells[r][0] = yc;
    }
    let res_x = run_kind(&DataSheet::new("xy-ba-x", "XYX", TableArchetype::Xy(xy_x)).unwrap(), ba(0, 1, false, Some(true))).unwrap();
    assert_close(scalar(&res_x, "N"), 3.0, 1e-12, "X vs Y[0] N");
    assert!(
        res_x
            .checklist
            .iter()
            .any(|c| c.assumption == "Complete-case row alignment" && c.diagnostic_metric.as_deref() == Some("2 row(s) omitted; 3 retained"))
    );

    let mut xy_gen = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 10.0, step: 10.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        4,
    )
    .unwrap();
    xy_gen.data_sets[0].cells = vec![vec![Cell::number(10.5)], vec![Cell::text("BAD")], vec![Cell::number(31.0)], vec![Cell::number(40.5)]];
    let res_gen = run_kind(&DataSheet::new("xy-ba-gen", "XYGen", TableArchetype::Xy(xy_gen)).unwrap(), ba(0, 1, false, Some(true))).unwrap();
    assert_close(scalar(&res_gen, "N"), 3.0, 1e-12, "GeneratedSequence N");
    assert!(res_gen.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));

    // Percent-mode non-positive / sign-crossing / zero / cancelling / non-finite guards + absolute mode ok.
    let neg_a = [-3.0, -5.0, -2.0, -4.0];
    let neg_b = [-2.0, -4.5, -1.0, -3.6];
    for (a, b, expected_name) in [
        (&neg_a[..], &neg_b[..], Some("average")),
        (&[2.0, 4.0, -3.0, 5.0][..], &[2.0, 4.0, 3.0, 5.0][..], Some("average")),
        (&[f64::MAX, 1.0, 2.0, 3.0][..], &[-f64::MAX, 1.0, 2.0, 3.0][..], Some("average")),
        (&[-1.0, 10.0][..], &[1.5, 11.0][..], Some("percent_difference_measurement")),
        (&[0.0, 10.0][..], &[1.5, 11.0][..], Some("percent_difference_measurement")),
        (&[f64::NAN, 1.0][..], &[1.0, 2.0][..], None),
        (&[f64::INFINITY, 1.0][..], &[1.0, 2.0][..], None),
    ] {
        let err = bland_altman(a, b, true, ConfidenceLevel::DEFAULT_95).unwrap_err();
        if let Some(name) = expected_name {
            assert!(matches!(err, StatsError::InvalidParameter { name: n, .. } if n == name), "expected {name}, got {err:?}");
        } else {
            assert!(matches!(err, StatsError::Math(MathError::DomainError { .. })), "expected DomainError, got {err:?}");
        }
    }
    assert!(bland_altman(&neg_a, &neg_b, false, ConfidenceLevel::DEFAULT_95).is_ok());
    assert!(bland_altman(&[-1.0, 10.0], &[1.5, 11.0], false, ConfidenceLevel::DEFAULT_95).is_ok());
    assert!(matches!(
        run_kind(&col_sheet("ba-neg", &[("A", &neg_a), ("B", &neg_b)]), ba(0, 1, true, None)),
        Err(StatsError::InvalidParameter { name: "average", .. })
    ));

    // Scale resilience at 1e300 + overflow guard at 1e308.
    let big_a = [1.0e300, 1.1e300, 1.2e300];
    let big_b = [-1.0e300, 1.0e300, 1.5e300];
    let ba_big = bland_altman(&big_a, &big_b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let ba_unit = bland_altman(&[1.0, 1.1, 1.2], &[-1.0, 1.0, 1.5], false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(ba_big.bias / 1.0e300, ba_unit.bias, 1e-12, "BA 1e300 bias");
    assert_close(ba_big.sd_bias / 1.0e300, ba_unit.sd_bias, 1e-12, "BA 1e300 sd");
    assert!(bland_altman(&[1.0e308, 1.0e308], &[-1.0e308, 0.0], false, ConfidenceLevel::DEFAULT_95).is_err());
    let res_big = run_kind(&col_sheet("ba-1e300", &[("A", &big_a), ("B", &big_b)]), AnalysisKind::BlandAltman(BlandAltmanSpec::default())).unwrap();
    assert_all_sheet_numbers_finite(&res_big, "BA 1e300");
}

#[test]
fn audit_07_08_multiple_row_parity_nonparametric_df_and_df1_t_tests() {
    let paired = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
        paired: true,
        nonparametric: false,
        assume_equal_sd: true,
        multiplicity: MultiplicitySpec::default(),
    });
    let unpaired = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: true,
        multiplicity: MultiplicitySpec::default(),
    });

    // Odd column selections on Column (5, 7, 5-of-8) and MV (5 numeric, Label+3) reject with paired_selected_columns.
    let col5 = col_sheet("c5", &[("C1", &[1.0, 2.0]), ("C2", &[1.2, 2.1]), ("C3", &[2.0, 3.0]), ("C4", &[2.2, 3.1]), ("C5", &[3.0, 4.0])]);
    for sel in [vec![0], vec![0, 1, 2], vec![0, 1, 2, 3, 4]] {
        let err = run_kind_sel(&col5, paired.clone(), sel.clone()).unwrap_err();
        assert!(matches!(err, StatsError::InvalidParameter { name: "paired_selected_columns", value } if (value - sel.len() as f64).abs() < 1e-9));
    }
    let col7 = col_sheet_hint(
        "c7",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("0", &[0.0, 1.0]), ("1", &[1.0, 2.0]), ("2", &[2.0, 3.0]), ("3", &[3.0, 4.0]), ("4", &[4.0, 5.0]), ("5", &[5.0, 6.0]), ("6", &[6.0, 7.0])],
    );
    assert!(
        matches!(run_kind(&col7, paired.clone()), Err(StatsError::InvalidParameter { name: "paired_selected_columns", value }) if (value - 7.0).abs() < 1e-9)
    );
    let mv5 = mv_num_sheet("mv5", &[("V0", &[1.0, 2.0]), ("V1", &[2.0, 3.0]), ("V2", &[3.0, 4.0]), ("V3", &[4.0, 5.0]), ("V4", &[5.0, 6.0])]);
    assert!(
        matches!(run_kind(&mv5, paired.clone()), Err(StatsError::InvalidParameter { name: "paired_selected_columns", value }) if (value - 5.0).abs() < 1e-9)
    );
    let mv_lbl3 = mv_cells_sheet(
        "mv-l3",
        &[
            ("Row", Some(VariableRole::Label), &[Cell::text("G1"), Cell::text("G2")]),
            ("A1", None, &[Cell::number(1.0), Cell::number(2.0)]),
            ("A2", None, &[Cell::number(1.1), Cell::number(2.1)]),
            ("B1", None, &[Cell::number(2.0), Cell::number(3.0)]),
        ],
    );
    assert!(
        matches!(run_kind(&mv_lbl3, paired.clone()), Err(StatsError::InvalidParameter { name: "paired_selected_columns", value }) if (value - 3.0).abs() < 1e-9)
    );

    // Grouped & XY 3-dataset parity + replicate width mismatch + blank-cell gap alignment.
    let grp3 = grouped_sheet_fill("g3", 3, 2, 3, |c, r, s| Some((c + 1) as f64 + 0.1 * (r as f64) + 0.05 * (s as f64)));
    let mut xy3_tbl = XyTable::new(SubcolumnMode::RawReplicates(3), 3, 2).unwrap();
    for r in 0..2 {
        xy3_tbl.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        for c in 0..3 {
            for s in 0..3 {
                xy3_tbl.data_sets[c].cells[r][s] = Cell::number((c + r + 1) as f64 + 0.01 * (s as f64));
            }
        }
    }
    let xy3 = DataSheet::new("xy3", "XY3", TableArchetype::Xy(xy3_tbl)).unwrap();
    for sh in [&grp3, &xy3] {
        assert!(
            matches!(run_kind(sh, paired.clone()), Err(StatsError::InvalidParameter { name: "paired_selected_columns", value }) if (value - 3.0).abs() < 1e-9)
        );
        for sel in [vec![0], vec![0, 1, 2]] {
            assert!(matches!(
                run_kind_sel(sh, paired.clone(), sel.clone()),
                Err(StatsError::InvalidParameter { name: "paired_selected_columns", .. })
            ));
            assert!(matches!(run_kind_sel(sh, unpaired.clone(), sel), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
        }
    }
    let mut asym = grouped_sheet_fill("asym", 2, 2, 3, |c, _, s| Some((c + 1 + s) as f64));
    if let TableArchetype::Grouped(g) = &mut asym.archetype {
        for r in 0..2 {
            g.data_sets[1].cells[r].pop();
        }
    }
    assert!(matches!(run_kind(&asym, paired.clone()), Err(StatsError::DimensionMismatch { expected: 3, actual: 2 })));

    let grp_gap = grouped_sheet_fill("gap", 2, 1, 3, |c, _, s| match (c, s) {
        (0, 1) => Some(10.0),
        (0, 2) => Some(20.0),
        (1, 0) => Some(999.0),
        (1, 1) => Some(12.0),
        (1, 2) => Some(23.0),
        _ => None,
    });
    let res_gap = run_kind(&grp_gap, paired).unwrap();
    assert_close(cell_num(row_cell(tab_by_id(&res_gap, "multiple-row-comparisons"), "Row 1", 2)), -2.5, 1e-12, "gap alignment");
}

#[test]
fn audit_multiple_row_nonparametric_empty_df_and_underpopulated_skip() {
    let unpaired = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: true,
        multiplicity: MultiplicitySpec::default(),
    });
    // Nonparametric multiple-row tests emit Empty df; parametric keeps finite df; under-populated row skipped cleanly.
    let grp_np =
        grouped_sheet_fill("grp-np", 2, 2, 6, |c, r, s| Some(if c == 0 { (r * 10 + s + 1) as f64 } else { (r * 10 + s * (r + 1) + 7) as f64 }));
    for is_paired in [false, true] {
        let r =
            run_kind(&grp_np, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired: is_paired, nonparametric: true, ..Default::default() }))
                .unwrap();
        let tab = tab_by_id(&r, "multiple-row-comparisons");
        assert_eq!(tab.column_headers[4], "df");
        assert!(tab.rows.iter().all(|row| row[4] == ResultCell::Empty));
        assert!(tab.footnotes.iter().any(|f| f.contains("df is omitted")));
    }
    let r_param = run_kind(&grp_np, unpaired).unwrap();
    assert_close(cell_num(&tab_by_id(&r_param, "multiple-row-comparisons").rows[0][4]), 10.0, 1e-12, "parametric df");

    let mr_skip = multiple_row_t_tests(
        &["Row1".into(), "Degen".into(), "Row3".into()],
        &[vec![10.0, 10.2, 9.8, 10.1], vec![5.0], vec![10.0, 10.5, 9.5, 10.0]],
        &[vec![14.0, 14.2, 13.9, 14.1], vec![6.0], vec![10.1, 10.4, 9.6, 10.0]],
        false,
        MultiplicityProcedure::HolmSidak,
        0.05,
    )
    .unwrap();
    assert_eq!(mr_skip.len(), 2);
    assert!(mr_skip.iter().all(|r| r.adjusted_p_or_q.is_finite()));
}

#[test]
fn audit_08_df1_paired_ratio_one_sample_and_multiple_row_t_tests() {
    // Audit #08: df = 1 paired, ratio paired, one-sample, summary, sheet, and MultipleRowTTests succeed with Hedges' g None.
    let (a2, b2) = ([10.0, 12.0], [7.0, 11.0]);
    let pt = paired_t_test(&a2, &b2, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pt.df, 1.0, 1e-12, 1e-12, "paired df=1");
    assert_approx_eq(pt.t_statistic, 2.0, 1e-12, 1e-12, "paired t df=1");
    assert_eq!((pt.hedges_gz, pt.hedges_gz_ci), (None, None));
    let rpt = ratio_paired_t_test(&a2, &b2, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(rpt.df, 1.0, 1e-12, 1e-12, "ratio df=1");
    assert_eq!((rpt.hedges_gz, rpt.hedges_gz_ci), (None, None));
    let os = one_sample_t_test(&[3.0, 1.0], 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    let os_sum = one_sample_t_test_from_summary(2.0, 2.0_f64.sqrt(), 2.0, 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(os_sum.p_value, os.p_value, 1e-12, 1e-12, "summary df=1");
    assert_eq!((os.hedges_g, os_sum.hedges_g), (None, None));

    let sh2 = col_sheet_hint("c2p", ColumnDesignHint::PairedRepeatedMeasures, &[("Before", &a2), ("After", &b2)]);
    for compute_effect_sizes in [false, true] {
        let res = run_kind(
            &sh2,
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, compute_effect_sizes, ..Default::default() }),
        )
        .unwrap();
        assert_approx_eq(scalar(&res, "t"), 2.0, 1e-12, 1e-12, "sheet df=1 t");
        if compute_effect_sizes {
            assert!(scalar_opt(&res, "Hedges g_z", None).is_none());
        }
    }
    let mut row_c = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, 1).unwrap();
    row_c.row_titles = vec![String::from("TwoPairRow")];
    for (c, &v) in [a2[0], b2[0], a2[1], b2[1]].iter().enumerate() {
        row_c.data_sets[c].cells[0][0] = Cell::number(v);
    }
    let row_res = run_kind(
        &DataSheet::new("r2p", "R2P", TableArchetype::Column(row_c)).unwrap(),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired: true, ..Default::default() }),
    )
    .unwrap();
    assert_eq!(tab_by_kind(&row_res, ResultTabKind::MultipleComparisons).row_labels, vec![String::from("TwoPairRow")]);
}

#[test]
fn audit_21_wilcoxon_pratt_hl_exact_ties_one_tailed_and_sign_overflow() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Exact small-sample HL CIs (one-sample n=4/5/10, paired n=4 sheet warning, MW 3x3 & 8x8).
    let w4 = one_sample_wilcoxon(&[1.0, 2.0, 4.0, 8.0], 0.0, true, conf).unwrap();
    assert_eq!(w4.pseudomedian_ci, Some((1.0, 8.0)));
    assert_close(w4.ci_actual_confidence_percent.unwrap(), 87.5, 1e-9, "w4 pct");
    let w5 = one_sample_wilcoxon(&[1.0, 2.0, 4.0, 8.0, 16.0], 0.0, true, conf).unwrap();
    assert_close(w5.ci_actual_confidence_percent.unwrap(), 93.75, 1e-9, "w5 pct");
    let w10 = one_sample_wilcoxon(&[1.1, 2.3, 3.7, 5.2, 8.1, 11.4, 15.8, 20.9, 27.3, 35.6], 0.0, true, conf).unwrap();
    assert_close(w10.ci_actual_confidence_percent.unwrap(), 100.0 * (1.0 - 50.0 / 1024.0), 1e-9, "w10 pct");

    let s_w4 = col_sheet("w4", &[("Before", &[10.0, 20.0, 40.0, 80.0]), ("After", &[9.0, 18.0, 36.0, 72.0])]);
    let res_w4 =
        run_kind(&s_w4, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WilcoxonSignedRank, ..Default::default() })).unwrap();
    let tab_w4 = tab_by_id(&res_w4, "wilcoxon-paired");
    assert_close(cell_num(row_cell(tab_w4, "HL CI actual confidence (%)", 0)), 87.5, 1e-9, "sheet w4 pct");
    assert!(tab_w4.row_labels.iter().any(|l| l.contains("87.50% CI") && l.contains("requested 95%")));
    assert!(res_w4.checklist.iter().any(|c| c.assumption == "Hodges-Lehmann CI achieved coverage" && c.status == ChecklistStatus::Warn));

    let mw33 = mann_whitney_u(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0], true, conf).unwrap();
    assert_eq!(mw33.hodges_lehmann_ci, Some((-29.0, -7.0)));
    assert_close(mw33.ci_actual_confidence_percent.unwrap(), 90.0, 1e-9, "mw33 pct");
    let (a8, b8) = ([1.1, 2.2, 3.4, 4.7, 6.1, 7.6, 9.2, 11.0], [13.5, 15.8, 18.2, 21.0, 24.3, 27.9, 31.4, 36.2]);
    let mw88 = mann_whitney_u(&a8, &b8, true, conf).unwrap();
    let pct88 = mw88.ci_actual_confidence_percent.unwrap();
    assert!((95.0..=96.0).contains(&pct88));
    let res_mw88 = run_kind(
        &col_sheet("mw88", &[("A", &a8), ("B", &b8)]),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..Default::default() }),
    )
    .unwrap();
    assert_close(cell_num(row_cell(tab_by_id(&res_mw88, "mann-whitney"), "HL CI actual confidence (%)", 0)), pct88, 1e-9, "sheet mw88 pct");
    assert!(!res_mw88.checklist.iter().any(|c| c.assumption == "Hodges-Lehmann CI achieved coverage" && c.status == ChecklistStatus::Warn));

    // 2. Pratt (1959) zero handling: sheet + MultipleRow + exact DP when zero-free + no zero double-counting + all-zero.
    let (ap, bp) = ([1.0, 2.0, 3.0, 4.0, 5.0], [1.0, 1.0, 1.0, 2.0, 3.0]);
    let pratt_d = wilcoxon_matched_pairs(&ap, &bp, true, conf, true).unwrap();
    let classic_d = wilcoxon_matched_pairs(&ap, &bp, true, conf, false).unwrap();
    assert_ne!(pratt_d.n_effective, classic_d.n_effective);
    let res_pr = run_kind(
        &col_sheet("wp", &[("A", &ap), ("B", &bp)]),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WilcoxonSignedRank, ..Default::default() }),
    )
    .unwrap();
    assert_close(scalar_ds(&res_pr, "PValue", None), pratt_d.p_value, 1e-12, "Wilcoxon sheet Pratt P");
    let tab_pr = tab_by_id(&res_pr, "wilcoxon-paired");
    assert_eq!(tab_pr.rows[1][0], ResultCell::Boolean(false));
    assert!(tab_pr.footnotes.iter().any(|f| f.contains("Pratt's (1959)")));

    let grp_pr = grouped_sheet_fill("grp-pr", 2, 1, 5, |c, _, s| Some(if c == 0 { ap[s] } else { bp[s] }));
    let mr_pr =
        run_kind(&grp_pr, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired: true, nonparametric: true, ..Default::default() })).unwrap();
    assert_close(cell_p_stars(&tab_by_id(&mr_pr, "multiple-row-comparisons").rows[0][5]).0, pratt_d.p_value, 1e-12, "MultipleRow Pratt P");
    assert!(tab_by_id(&mr_pr, "multiple-row-comparisons").footnotes.iter().any(|f| f.contains("Pratt (1959)")));

    let (anz, bnz) = ([10.1, 11.3, 12.5, 14.2, 15.8, 17.1, 18.9, 20.6], [9.0, 10.1, 11.1, 12.6, 14.0, 15.1, 16.7, 18.2]);
    let p_nz = wilcoxon_matched_pairs(&anz, &bnz, true, conf, true).unwrap();
    assert!(p_nz.exact_p_used);
    assert_close(p_nz.p_value, 1.0 / 128.0, 1e-15, "zero-free Pratt exact DP");

    // 14 pairs: 8 zeros + 6 nonzeros ([1, 2, 2, 3, -4, 5]) — exact Pratt variance without double-counting zero group.
    let p14 = wilcoxon_matched_pairs(
        &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 21.0, 22.0, 22.0, 23.0, 10.0, 25.0],
        &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 20.0, 20.0, 20.0, 20.0, 14.0, 20.0],
        true,
        conf,
        true,
    )
    .unwrap();
    let var_w: f64 = (14.0 * 15.0 * 29.0 - 8.0 * 9.0 * 17.0 - 3.0) / 24.0;
    assert_close(p14.w_plus, 56.0, 1e-12, "Pratt 14-pair W+");
    assert_close(p14.p_value, 2.0 * norm_sf(((56.0 - 34.5_f64).abs() - 0.5) / var_w.sqrt()), 1e-12, "Pratt 14-pair P");
    let all_z = wilcoxon_matched_pairs(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], true, conf, true).unwrap();
    assert_eq!((all_z.p_value, all_z.rank_biserial_r), (1.0, 0.0));

    // 3. Mann-Whitney exact ties permutation DP (brute-force C(10,5)) + all-identical normal approx + large-N HL.
    let (ta, tb) = ([1.0, 2.0, 2.0, 4.0, 6.0], [2.0, 3.0, 5.0, 6.0, 6.0]);
    let mw_t = mann_whitney_u(&ta, &tb, true, conf).unwrap();
    assert!(mw_t.exact_p_used);
    let ranks = [1.0, 3.0, 3.0, 6.0, 9.0, 3.0, 5.0, 7.0, 9.0, 9.0];
    let obs_sum: f64 = ranks[..5].iter().sum();
    let (mut tot, mut le, mut ge) = (0_u64, 0_u64, 0_u64);
    for mask in 0_u32..(1_u32 << 10) {
        if mask.count_ones() == 5 {
            tot += 1;
            let s: f64 = ranks.iter().enumerate().filter(|&(i, _)| (mask & (1 << i)) != 0).map(|(_, &r)| r).sum();
            if s <= obs_sum + 1e-9 {
                le += 1;
            }
            if s + 1e-9 >= obs_sum {
                ge += 1;
            }
        }
    }
    let exp_p = (2.0 * (le.min(ge) as f64) / (tot as f64)).clamp(0.0, 1.0);
    assert_close(mw_t.p_value, exp_p, 1e-12, "MW ties DP vs brute");
    assert_close(mann_whitney_u(&tb, &ta, true, conf).unwrap().p_value, mw_t.p_value, 1e-12, "MW ties swap");
    assert_close(2.0 * mann_whitney_u(&ta, &tb, false, conf).unwrap().p_value, mw_t.p_value, 1e-12, "MW ties 1-tail");
    assert!(
        !mann_whitney_u(&(0..10).map(|i| (i % 4) as f64).collect::<Vec<_>>(), &(0..9).map(|i| ((i % 4) + 1) as f64).collect::<Vec<_>>(), true, conf)
            .unwrap()
            .exact_p_used
    );
    for (na, nb) in [(9, 9), (10, 10), (10, 15), (50, 50)] {
        let one = mann_whitney_u(&vec![5.0; na], &vec![5.0; nb], false, conf).unwrap();
        let two = mann_whitney_u(&vec![5.0; na], &vec![5.0; nb], true, conf).unwrap();
        assert_close(one.p_value, 1.0, 1e-12, "all-identical 1-tail");
        assert_close(two.p_value, 1.0, 1e-12, "all-identical 2-tail");
    }

    // Large-N exact HL above legacy cutoffs (n_a*n_b = 40,200 and one-sample Walsh n = 2,005).
    let la: Vec<f64> = (0..201).map(|i| (i * i) as f64).collect();
    let lb: Vec<f64> = (0..200).map(|j| (3 * j) as f64).collect();
    let mw_l = mann_whitney_u(&la, &lb, true, conf).unwrap();
    let mut diffs: Vec<f64> = la.iter().flat_map(|&va| lb.iter().map(move |&vb| va - vb)).collect();
    diffs.sort_by(f64::total_cmp);
    assert_close(mw_l.hodges_lehmann_diff, 0.5 * (diffs[diffs.len() / 2 - 1] + diffs[diffs.len() / 2]), 1e-12, "2-sample large HL");
    let n_w = 2_005;
    let xw: Vec<f64> = (0..n_w).map(|i| ((i as f64) / ((n_w - 1) as f64)).powi(3) * 100.0).collect();
    let wil_l = wilcoxon_matched_pairs(&xw, &vec![0.0; n_w], true, conf, true).unwrap();
    let mut walsh = Vec::with_capacity(n_w * (n_w + 1) / 2);
    for i in 0..n_w {
        for j in i..n_w {
            walsh.push(0.5 * xw[i] + 0.5 * xw[j]);
        }
    }
    walsh.sort_by(f64::total_cmp);
    assert_close(wil_l.hodges_lehmann_pseudomedian, 0.5 * (walsh[(walsh.len() - 1) / 2] + walsh[walsh.len() / 2]), 1e-12, "1-sample Walsh HL");

    // 4. One-tailed rank direction symmetry (exact, tied, asymptotic) + paired_sign_test overflow.
    let (p_lo, p_hi) = ([1.0, 2.0, 3.0, 4.0, 5.0, 6.0], [10.0, 12.0, 15.0, 19.0, 24.0, 30.0]);
    let (a25, b25): (Vec<f64>, Vec<f64>) = ((1..=25).map(|i| i as f64).collect(), (101..=125).map(|i| i as f64).collect());
    assert_close(wilcoxon_matched_pairs(&p_lo, &p_hi, false, conf, false).unwrap().p_value, 1.0 / 64.0, 1e-12, "paired 1-tail lo");
    assert_close(wilcoxon_matched_pairs(&p_hi, &p_lo, false, conf, false).unwrap().p_value, 1.0 / 64.0, 1e-12, "paired 1-tail hi");
    let (w25_a, w25_b) =
        (wilcoxon_matched_pairs(&a25, &b25, false, conf, false).unwrap(), wilcoxon_matched_pairs(&b25, &a25, false, conf, false).unwrap());
    assert!(w25_a.p_value < 1e-4 && (w25_a.p_value - w25_b.p_value).abs() < 1e-12);
    assert_close(one_sample_wilcoxon(&p_lo, 50.0, false, conf).unwrap().p_value, 1.0 / 64.0, 1e-12, "1s wilc lo");
    assert_close(one_sample_wilcoxon(&p_hi, 0.0, false, conf).unwrap().p_value, 1.0 / 64.0, 1e-12, "1s wilc hi");
    let (o25_a, o25_b) = (one_sample_wilcoxon(&a25, 100.0, false, conf).unwrap(), one_sample_wilcoxon(&b25, 0.0, false, conf).unwrap());
    assert!(o25_a.p_value < 1e-4 && (o25_a.p_value - o25_b.p_value).abs() < 1e-12);

    assert!(matches!(paired_sign_test(&[1e308, 1e308], &[-1e308, -1e308], true, conf), Err(StatsError::Math(MathError::DomainError { .. }))));
    assert!(matches!(paired_sign_test(&[1e308, 2.0], &[-1e308, 1.0], true, conf), Err(StatsError::Math(MathError::DomainError { .. }))));
    let ovf_sh = col_sheet_hint("ovf", ColumnDesignHint::PairedRepeatedMeasures, &[("A", &[1e308, 1e308]), ("B", &[-1e308, -1e308])]);
    assert!(
        run_kind(&ovf_sh, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..Default::default() })).is_err()
    );
}

#[test]
fn audit_67_68_69_93_100_descriptive_normality_outliers_and_empty_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Lognormal unpaired t log-scale labels (#67).
    let ln_sh = col_sheet("ln", &[("Ctrl", &[2.0, 4.0, 8.0, 16.0]), ("Trt", &[4.0, 8.0, 16.0, 32.0])]);
    let ln_res =
        run_kind(&ln_sh, AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::LognormalUnpairedT, ..Default::default() }))
            .unwrap();
    let ln_tab = tab_by_id(&ln_res, "two-group");
    for label in [
        "Mean of ln(Ctrl)",
        "Mean of ln(Trt)",
        "Difference of ln means (ln A - ln B)",
        "SE of ln difference",
        "95% CI of ln difference",
        "F-test to compare ln variances (F)",
        "F-test P value (ln scale)",
        "Geometric mean ratio (A / B)",
        "95% CI of geometric ratio",
        "Cohen's d (ln scale)",
        "Hedges' g (ln scale)",
        "Glass's Delta (ln scale)",
        "Glass's Delta standardizing group",
        "R squared (ln scale)",
    ] {
        assert!(ln_tab.row_labels.iter().any(|l| l == label), "missing `{label}`");
    }
    assert_close(cell_num(row_cell(ln_tab, "Geometric mean ratio (A / B)", 0)), 0.5, 1e-12, "geo ratio");
    assert_eq!(row_cell(ln_tab, "Glass's Delta standardizing group", 0), &ResultCell::Text(String::from("ln(Ctrl)")));

    // 2. Custom percentiles Type 7 (#68) & Glass's Delta zero-SD-A no fallback (#69).
    let vals = [2.0, 4.0, 4.0, 8.0, 16.0, 32.0];
    let direct = summarize_column("X", &vals, 0, conf, Some(90.0)).unwrap();
    assert_eq!((direct.custom_lower_percentile, direct.custom_upper_percentile), (Some((10.0, 3.0)), Some((90.0, 24.0))));
    let p_sh = col_sheet("pct", &[("X", &vals)]);
    let r90 = run_kind(
        &p_sh,
        AnalysisKind::DescriptiveStats(DescriptiveSpec { custom_upper_percentile: Some(90.0), run_normality_battery: false, ..Default::default() }),
    )
    .unwrap();
    assert_close(cell_num(row_cell(tab_by_id(&r90, "desc-summary"), "10% Percentile (Type 7)", 0)), 3.0, 1e-12, "p10");
    assert_close(cell_num(row_cell(tab_by_id(&r90, "desc-summary"), "90% Percentile (Type 7)", 0)), 24.0, 1e-12, "p90");
    let r75 = run_kind(
        &p_sh,
        AnalysisKind::DescriptiveStats(DescriptiveSpec { custom_upper_percentile: Some(75.0), run_normality_battery: false, ..Default::default() }),
    )
    .unwrap();
    let t75 = tab_by_id(&r75, "desc-summary");
    for lbl in ["25% Percentile", "75% Percentile", "25% Percentile (Type 7)", "75% Percentile (Type 7)"] {
        assert!(t75.row_labels.iter().any(|l| l == lbl), "missing `{lbl}`");
    }

    let (z_a, v_b) = ([5.0, 5.0, 5.0], [1.0, 2.0, 3.0]);
    assert_eq!(unpaired_t_test(&z_a, &v_b, true, conf).unwrap().effect_sizes.glass_delta, None);
    assert_eq!(welch_t_test(&z_a, &v_b, true, conf).unwrap().effect_sizes.glass_delta, None);
    assert_close(unpaired_t_test(&v_b, &z_a, true, conf).unwrap().effect_sizes.glass_delta.unwrap(), -3.0, 1e-12, "rev Glass");
    let gl_res = run_kind(&col_sheet("gl", &[("Placebo", &z_a), ("Drug", &v_b)]), AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    assert_eq!(row_cell(tab_by_id(&gl_res, "two-group"), "Glass's Delta", 0), &ResultCell::Empty);
    assert_eq!(row_cell(tab_by_id(&gl_res, "two-group"), "Glass's Delta standardizing group", 0), &ResultCell::Text(String::from("Placebo")));

    // 3. MV persisted columns stable under Calculated column (#93) & row-descriptive Label exclusion (R2-S02-07).
    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[1].title = String::from("Baseline");
    mv.variables[2].title = String::from("Followup");
    for r in 0..6 {
        mv.variables[0].cells[r] = Cell::text(if r < 3 { "Ctrl" } else { "Trt" });
        mv.variables[1].cells[r] = Cell::number(if r < 3 { 10.0 + r as f64 } else { 20.0 + (r - 3) as f64 });
        mv.variables[2].cells[r] = Cell::number(100.0 + (r % 3) as f64);
    }
    mv.variables[0].refresh_inferred_role();
    let k_unp = AnalysisKind::TwoGroupComparison(TwoGroupSpec { mv_group_column: Some(0), mv_response_column: Some(1), ..Default::default() });
    let k_one = AnalysisKind::TwoGroupComparison(TwoGroupSpec {
        method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 },
        mv_response_column: Some(1),
        compute_effect_sizes: false,
        ..Default::default()
    });
    let sh_b = DataSheet::new("mv93", "MV93", TableArchetype::MultipleVariables(mv.clone())).unwrap();
    let (u_b, o_b) = (run_kind(&sh_b, k_unp.clone()).unwrap(), run_kind(&sh_b, k_one.clone()).unwrap());
    let mut delta = VariableColumn::new("Delta", 6).unwrap();
    delta.set_calculated_formula("=[Followup]-[Baseline]");
    delta.cells = vec![Cell::number(90.0), Cell::number(90.0), Cell::number(90.0), Cell::number(80.0), Cell::number(80.0), Cell::number(80.0)];
    mv.variables.push(delta);
    let sh_a = DataSheet::new("mv93", "MV93", TableArchetype::MultipleVariables(mv)).unwrap();
    let (u_a, o_a) = (run_kind(&sh_a, k_unp).unwrap(), run_kind(&sh_a, k_one).unwrap());
    assert_close(scalar_ds(&u_a, "t", None), scalar_ds(&u_b, "t", None), 1e-12, "persisted MV t");
    assert_close(cell_num(row_cell(tab_by_id(&o_a, "one-sample-t"), "Sample mean", 0)), 16.0, 1e-12, "persisted MV 1s");
    assert_eq!(tab_by_id(&o_a, "one-sample-t").column_headers, tab_by_id(&o_b, "one-sample-t").column_headers);

    let mv_row = mv_cells_sheet(
        "mv-rdesc",
        &[
            ("SubjectID", Some(VariableRole::Label), &[Cell::number(1001.0), Cell::number(1002.0)]),
            ("SysBP", None, &[Cell::number(120.0), Cell::number(130.0)]),
            ("DiaBP", None, &[Cell::number(80.0), Cell::number(90.0)]),
        ],
    );
    let r_rd = run_kind(&mv_row, AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, ..Default::default() })).unwrap();
    let t_rd = tab_by_id(&r_rd, "row-descriptive-stats");
    assert_close(cell_num(&t_rd.rows[0][0]), 2.0, 1e-12, "row desc N excludes Label");
    assert_close(cell_num(&t_rd.rows[0][1]), 100.0, 1e-12, "row desc mean");
}

#[test]
fn audit_100_normality_and_xy_tied_x_auc() {
    // 4. Normality: Anderson-Darling monotone past 153.5 (#100), KS alpha, Shapiro-Wilk strict >, XY tied-X AUC (R2-S02-06).
    let mut prev_p = 1.0;
    for n in [30_usize, 100, 300, 1000, 3000] {
        let mut skew = vec![0.0; n - 1];
        skew.push(if n == 1000 { 1.0 } else { 10.0 });
        let ad = anderson_darling(&skew, 0.05).unwrap();
        if n == 1000 {
            assert!(ad.statistic > 153.5);
        }
        assert!(ad.p_value <= prev_p + 1e-15 && ad.p_value < 1e-6 && !ad.passed);
        prev_p = ad.p_value;
    }
    let shifted = [0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 1.4, 1.6, 1.8, 2.0];
    let ks_05 = one_sample_ks_normal(&shifted, 0.0, 1.0).unwrap();
    let ks_001 = one_sample_ks_normal_at_alpha(&shifted, 0.0, 1.0, 0.001).unwrap();
    assert_close(ks_05.p_value, ks_001.p_value, 1e-15, "KS p alpha-invariant");
    assert!(ks_001.passed && !one_sample_ks_normal_at_alpha(&shifted, 0.0, 1.0, 0.50).unwrap().passed);
    assert!(matches!(one_sample_ks_normal_at_alpha(&shifted, 0.0, 1.0, 0.0), Err(StatsError::InvalidParameter { name: "alpha", .. })));
    assert!(matches!(one_sample_ks_normal_at_alpha(&shifted, f64::NAN, 1.0, 0.05), Err(StatsError::InvalidParameter { name: "mean", .. })));
    assert!(!shapiro_wilk(&[1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 5.0], 0.99).unwrap().passed);
    assert!(shapiro_wilk(&[1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 5.0], 1e-12).unwrap().passed);

    let xy_tie = xy_sheet("xy-tie", &[0.0, 1.0, 0.0, 1.0], &[0.0, 40.0, 10.0, 20.0]);
    assert_close(
        scalar_ds(&run_kind(&xy_tie, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap(), "Trapezoidal AUC", Some(0)),
        17.5,
        1e-12,
        "tied-X AUC",
    );
    let mut xy_pre = XyTable::new(SubcolumnMode::MeanSdN, 1, 3).unwrap();
    for (r, &(x, m, n)) in [(0.0, 4.0, 1.0), (0.0, 10.0, 2.0), (2.0, 12.0, 1.0)].iter().enumerate() {
        xy_pre.x_cells[r][0] = Cell::number(x);
        xy_pre.data_sets[0].cells[r] = vec![Cell::number(m), Cell::number(1.0), Cell::number(n)];
    }
    let r_xyp =
        run_kind(&DataSheet::new("xy-pt", "XYPT", TableArchetype::Xy(xy_pre)).unwrap(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))
            .unwrap();
    assert_close(scalar_ds(&r_xyp, "Trapezoidal AUC", Some(0)), 20.0, 1e-12, "MeanSdN tied-X AUC");
}

#[test]
fn audit_r1_s02_10_outliers_zero_rsdr_and_empty_input_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;
    // 5. Outliers: 1-based row & dataset scalar indices (R1-S02-10), ROUT zero-RSDR (R1-S02-11), empty guards (RV-03).
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 8).unwrap();
    tab.data_sets[0].title = String::from("AllEmpty");
    tab.data_sets[1].title = String::from("TreatA");
    tab.data_sets[2].title = String::from("TreatB");
    tab.data_sets[1].cells[1][0] = Cell::excluded_number(10.0);
    for (r, v) in [(2, 10.1), (3, 10.2), (5, 9.9), (6, 500.0), (7, 10.0)] {
        tab.data_sets[1].cells[r][0] = Cell::number(v);
    }
    for (r, v) in [1.0, 1.1, 0.9, 1.05, 0.95].into_iter().enumerate() {
        tab.data_sets[2].cells[r + 2][0] = Cell::number(v);
    }
    let idx_sh = DataSheet::new("col-idx", "Idx", TableArchetype::Column(tab)).unwrap();
    for out_spec in [OutlierSpec::Rout { fdr_q_percent: 1.0 }, OutlierSpec::Grubbs { alpha: 0.05, iterative: true }] {
        let res = run_kind(&idx_sh, AnalysisKind::OutlierDetection(out_spec)).unwrap();
        assert_eq!(
            (scalar_opt(&res, "OutlierCount", Some(0)), scalar_opt(&res, "OutlierCount", Some(1)), scalar_opt(&res, "OutlierCount", Some(2))),
            (None, Some(1.0), Some(0.0))
        );
        assert_eq!(res.tabs[0].rows[0][0], ResultCell::Text(String::from("TreatA")));
        assert_close(cell_num(&res.tabs[0].rows[0][1]), 7.0, 1e-12, "1-based outlier row");
    }
    let desc_idx = run_kind(
        &idx_sh,
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::PooledGrandSample,
            run_normality_battery: false,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(
        (scalar_opt(&desc_idx, "N", Some(0)), scalar_opt(&desc_idx, "N", Some(1)), scalar_opt(&desc_idx, "N", Some(2))),
        (None, Some(5.0), Some(5.0))
    );
    let sum_sh = presum_col_sheet("sum-idx", SubcolumnMode::MeanSdN, &[("E0", &[]), ("E1", &[]), ("Val", &[&[42.0, 2.0, 6.0]])]);
    let sum_r = run_kind(&sum_sh, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    assert_eq!((scalar_opt(&sum_r, "Mean", Some(0)), scalar_opt(&sum_r, "Mean", Some(2))), (None, Some(42.0)));

    let mut ties = vec![5.0; 8];
    ties.push(100.0);
    let rout = rout_outliers(&ties, 1.0).unwrap();
    assert_eq!((rout.rsdr, rout.outliers.len(), rout.outliers[0].index, rout.outliers[0].threshold_or_p), (0.0, 1, 8, 0.0));
    assert!(rout.outliers[0].score.is_infinite());
    assert!(rout_outliers(&[5.0; 10], 1.0).unwrap().outliers.is_empty());
    let rt_r = run_kind(&col_sheet("rt", &[("X", &ties)]), AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })).unwrap();
    assert_eq!(rt_r.tabs[0].rows[0][3], ResultCell::Text(String::from("∞")));

    assert!(matches!(mann_whitney_u(&[], &[1.0, 2.0, 3.0], true, conf), Err(StatsError::EmptyInput)));
    assert!(matches!(mann_whitney_u(&[1.0, 2.0, 3.0], &[], true, conf), Err(StatsError::EmptyInput)));
    assert!(matches!(levene_variance_test(&[&[], &[1.0, 2.0, 3.0]], true), Err(StatsError::EmptyInput)));
    assert!(matches!(rout_outliers(&[], 1.0), Err(StatsError::EmptyInput)));
    assert!(matches!(one_sample_wilcoxon(&[], 0.0, true, conf), Err(StatsError::EmptyInput)));
}

#[test]
fn descriptive_tab_emits_geometric_harmonic_quadratic_mean_cis() {
    let (pos, non_pos, single) = ([10.0, 12.0, 14.0, 16.0, 18.0], [-2.0, 0.0, 3.0, 5.0, 7.0], [42.0]);
    let direct_pos = summarize_column("Pos", &pos, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    let direct_np = summarize_column("NonPos", &non_pos, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    let sheet = col_sheet("gen-ci", &[("Pos", &pos), ("NonPos", &non_pos), ("Single", &single)]);
    let res = run_kind(&sheet, AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let tab = tab_by_id(&res, "desc-summary");

    for (row_lbl, expected) in [
        ("95% CI of Geometric mean", direct_pos.geometric_mean_ci.unwrap()),
        ("95% CI of Harmonic mean", direct_pos.harmonic_mean_ci.unwrap()),
        ("95% CI of Quadratic mean (RMS)", direct_pos.quadratic_mean_ci.unwrap()),
    ] {
        let (lo, hi) = cell_ci_finite(row_cell(tab, row_lbl, 0));
        assert_close(lo, expected.0, 1e-12, row_lbl);
        assert_close(hi, expected.1, 1e-12, row_lbl);
        assert_eq!(row_cell(tab, row_lbl, 2), &ResultCell::Empty);
    }
    assert_eq!(row_cell(tab, "95% CI of Geometric mean", 1), &ResultCell::Empty);
    assert_eq!(row_cell(tab, "95% CI of Harmonic mean", 1), &ResultCell::Empty);
    let (np_lo, np_hi) = cell_ci_finite(row_cell(tab, "95% CI of Quadratic mean (RMS)", 1));
    let exp_np = direct_np.quadratic_mean_ci.unwrap();
    assert_close(np_lo, exp_np.0, 1e-12, "NonPos RMS CI lo");
    assert_close(np_hi, exp_np.1, 1e-12, "NonPos RMS CI hi");
}
