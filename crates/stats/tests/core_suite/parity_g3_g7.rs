//! Wave 2 (`L2` Stats) analytical & black-box reference oracle tests for parity deliverables:
//! - Multiplicity mapping (`MultiplicityMethod::{NewmanKeuls, FisherLsd}` via `OneWayAnova` & `from_spec`)
//! - Descriptive analysis `ResultTabKind::NormalQq` (normal & lognormal quantiles + non-positive handling)
//! - `TwoGroupSpec` on `MultipleVariables` categorical 2-level grouping + missingness & level count validation
//! - `AnalysisKind::MultipleRowTTests` -> `MainSummary`, `MultipleComparisons`, scalars & Volcano `PairwiseComparisonRecord`s
//! - `AnalysisKind::BlandAltman` -> structured `MainSummary`, pair coordinates, scalars & `% difference` mode
//! - `AnalysisKind::PassingBablok` dispatch through `run_analysis`
//! - `SurvivalAnalysisSpec::KaplanMeier` on `MultipleVariables` (explicit, single-cohort & role/title auto-detection)
//! - `SurvivalAnalysisSpec::CoxProportionalHazards` `ResultTabKind::{BaselineSurvival, LogMinusLog}` tabs
//!
//! Reference provenance: Fisher (1935) unprotected LSD; Keuls (1952) / Newman (1939) studentized range step-down;
//! Blom (1958) normal plotting positions; Bland & Altman (*Lancet* 1986) Limits of Agreement;
//! Benjamini & Hochberg (1995) & Benjamini, Krieger & Yekutieli (2006) FDR procedures;
//! Passing & Bablok (*J Clin Chem Clin Biochem* 1983); Kaplan & Meier (1958); Breslow (1972) / Cox (1972) baseline hazard.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, BlandAltmanSpec, Cell, ColumnDesignHint, CoxTieMethod, DescriptiveSpec, LevelOrderKind, LevelOrderSpec, MultipleRowTestsSpec,
    MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec, OneWayMethod, PassingBablokSpec, ResultCell, ResultTabKind, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_two_sided_isf, student_t_two_sided_p, studentized_range_sf};
use curvebook_stats::column::qq_expected_quantiles;
use curvebook_stats::multiplicity::{MultiplicityProcedure, adjust_benjamini_hochberg};
use curvebook_stats::survival::{compare_survival_curves, kaplan_meier_curve};
use curvebook_stats::two_group::{bland_altman, mann_whitney_u, paired_t_test, unpaired_t_test, welch_t_test};
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    cell_num, cell_p, col_sheet, col_sheet_hint, grouped_sheet, mv_cells_sheet, mv_num_sheet, obs, run_kind, scalar as scalar_by_key, spec_for,
    survival_sheet, tab_by_kind, xy_sheet,
};

#[test]
fn multiplicity_newman_keuls_and_fisher_lsd_run_end_to_end() {
    assert_eq!(MultiplicityProcedure::from_spec(MultiplicityMethod::NewmanKeuls), MultiplicityProcedure::NewmanKeuls);
    assert_eq!(MultiplicityProcedure::from_spec(MultiplicityMethod::FisherLsd), MultiplicityProcedure::FisherLsd);
    assert_eq!(MultiplicityProcedure::from(MultiplicityMethod::NewmanKeuls), MultiplicityProcedure::NewmanKeuls);
    assert_eq!(MultiplicityProcedure::from(MultiplicityMethod::FisherLsd), MultiplicityProcedure::FisherLsd);

    let sheet = col_sheet(
        "anova-src",
        &[("G1", &[10.0, 11.0, 12.0, 13.0]), ("G2", &[14.0, 15.0, 16.0, 17.0]), ("G3", &[18.0, 19.0, 20.0, 21.0]), ("G4", &[30.0, 31.0, 32.0, 33.0])],
    );
    let (ms_within, df_within): (f64, f64) = (20.0 / 12.0, 12.0);
    let se_diff = (ms_within * 0.5).sqrt();
    let run_aov = |method| {
        run_kind(
            &sheet,
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::OrdinaryAnova,
                compute_effect_sizes: false,
                multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
                ..Default::default()
            }),
        )
        .unwrap()
    };

    let lsd_res = run_aov(MultiplicityMethod::FisherLsd);
    assert_eq!(lsd_res.pairwise_comparisons.len(), 6);
    let expected_adj_p = student_t_two_sided_p(4.0 / se_diff, df_within).unwrap();
    let g1_g2 = lsd_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G2").unwrap();
    assert_approx_eq(g1_g2.raw_p_or_q, expected_adj_p, 1e-12, 1e-12, "Fisher LSD matches unadjusted Student-t");
    assert!(!g1_g2.is_fdr_q && g1_g2.significant);
    assert!(matches!(tab_by_kind(&lsd_res, ResultTabKind::MultipleComparisons).rows[0][6], ResultCell::ConfidenceInterval { .. }));

    let snk_res = run_aov(MultiplicityMethod::NewmanKeuls);
    let tukey_res = run_aov(MultiplicityMethod::TukeyHsd);
    assert_eq!(snk_res.pairwise_comparisons.len(), 6);
    let tukey_g1_g2 = tukey_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G2").unwrap();
    let snk_g1_g2 = snk_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G2").unwrap();
    let snk_g1_g4 = snk_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G4").unwrap();
    let (q_12, q_14) = (4.0 / (0.5 * ms_within * 0.5).sqrt(), 20.0 / (0.5 * ms_within * 0.5).sqrt());
    assert_approx_eq(snk_g1_g4.raw_p_or_q, studentized_range_sf(q_14, 4, df_within).unwrap(), 1e-12, 1e-12, "SNK max-span p equals q(span=4)");
    assert_approx_eq(snk_g1_g2.raw_p_or_q, studentized_range_sf(q_12, 2, df_within).unwrap(), 1e-12, 1e-12, "SNK span-2 p");
    assert_approx_eq(snk_g1_g2.raw_p_or_q, expected_adj_p, 1e-10, 1e-10, "SNK span=2 matches 2-group Student-t");
    assert!(snk_g1_g2.raw_p_or_q < tukey_g1_g2.raw_p_or_q);
}

#[test]
fn descriptive_analysis_emits_normal_and_lognormal_qq_tab() {
    let pos = [1.2, 2.4, 3.8, 5.1, 7.9, 12.5, 19.3, 30.0];
    let mixed = [-3.0, -1.0, 0.0, 1.5, 2.5, 4.0, 5.0, 8.0];
    let res = run_kind(
        &col_sheet("desc-src", &[("Positive", &pos), ("Signed", &mixed)]),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: true, ..Default::default() }),
    )
    .unwrap();
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::DescriptiveSummary) && res.tabs.iter().any(|t| t.kind == ResultTabKind::Diagnostics));
    let qq_tab = tab_by_kind(&res, ResultTabKind::NormalQq);
    assert_eq!(qq_tab.rows.len(), pos.len() + mixed.len());

    let (exp_norm_pos, exp_log_pos) = (qq_expected_quantiles(&pos, false).unwrap(), qq_expected_quantiles(&pos, true).unwrap());
    for (i, ((&(en, on), &(el, ol)), row)) in exp_norm_pos.iter().zip(exp_log_pos.iter()).zip(&qq_tab.rows[..pos.len()]).enumerate() {
        assert_eq!(row[0], ResultCell::Text(String::from("Positive")));
        assert_approx_eq(cell_num(&row[1]), (i + 1) as f64, 1e-12, 0.0, "order index");
        assert_approx_eq(cell_num(&row[2]), en, 1e-12, 1e-12, "normal expected quantile");
        assert_approx_eq(cell_num(&row[3]), on, 1e-12, 1e-12, "normal observed value");
        assert_approx_eq(cell_num(&row[4]), el, 1e-12, 1e-12, "lognormal expected quantile");
        assert_approx_eq(cell_num(&row[5]), ol, 1e-12, 1e-12, "lognormal observed ln(Y)");
    }
    for (&(en, on), row) in qq_expected_quantiles(&mixed, false).unwrap().iter().zip(&qq_tab.rows[pos.len()..]) {
        assert_eq!((&row[0], &row[4], &row[5]), (&ResultCell::Text(String::from("Signed")), &ResultCell::Empty, &ResultCell::Empty));
        assert_approx_eq(cell_num(&row[2]), en, 1e-12, 1e-12, "signed normal expected");
        assert_approx_eq(cell_num(&row[3]), on, 1e-12, 1e-12, "signed normal observed");
    }
    assert!(!qq_tab.footnotes.is_empty());
}

#[test]
fn two_group_mv_categorical_grouping_and_validation() {
    let ctrl_vals = [10.1, 11.4, 9.8, 10.7, 11.0];
    let trt_vals = [15.2, 16.0, 14.8, 15.6, 16.4];
    let mut g_cells: Vec<Cell> = ctrl_vals.iter().map(|_| Cell::text("Control")).chain(trt_vals.iter().map(|_| Cell::text("Treatment"))).collect();
    g_cells.extend([Cell::text("Control"), Cell::empty(), Cell::excluded_text("ThirdExcluded"), Cell::text("Treatment")]);
    let mut r_cells: Vec<Cell> = ctrl_vals.iter().chain(&trt_vals).map(|&v| Cell::number(v)).collect();
    r_cells.extend([Cell::empty(), Cell::number(99.0), Cell::number(50.0), Cell::excluded_number(100.0)]);
    let mut sheet =
        mv_cells_sheet("mv-2g", &[("Arm", Some(VariableRole::Categorical), &g_cells), ("Response", Some(VariableRole::Continuous), &r_cells)]);
    if let TableArchetype::MultipleVariables(mv) = &mut sheet.archetype {
        mv.variables[0].level_order = LevelOrderSpec { order: LevelOrderKind::EntryOrder, reversed: false };
    }

    let conf = ConfidenceLevel::DEFAULT_95;
    let exp_u = unpaired_t_test(&ctrl_vals, &trt_vals, true, conf).unwrap();
    let exp_w = welch_t_test(&ctrl_vals, &trt_vals, true, conf).unwrap();
    let exp_mw = mann_whitney_u(&ctrl_vals, &trt_vals, true, conf).unwrap();
    let tg = |method, mv_g, mv_r, es| {
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method,
            two_tailed: true,
            compute_effect_sizes: es,
            mv_group_column: mv_g,
            mv_response_column: mv_r,
        })
    };

    for (mv_g, mv_r) in [(Some(0), Some(1)), (None, None)] {
        let ru = run_kind(&sheet, tg(TwoGroupMethod::UnpairedT, mv_g, mv_r, true)).unwrap();
        assert_approx_eq(scalar_by_key(&ru, "t"), exp_u.t_statistic, 1e-12, 1e-12, "MV unpaired t");
        assert_approx_eq(scalar_by_key(&ru, "PValue"), exp_u.p_value, 1e-12, 1e-12, "MV unpaired p");
        assert_eq!((ru.pairwise_comparisons[0].label_a.as_str(), ru.pairwise_comparisons[0].label_b.as_str()), ("Control", "Treatment"));
        assert!(ru.checklist.iter().any(|c| c.assumption.contains("Complete-case")));
    }
    assert_approx_eq(
        scalar_by_key(&run_kind(&sheet, tg(TwoGroupMethod::WelchT, Some(0), Some(1), false)).unwrap(), "PValue"),
        exp_w.p_value,
        1e-12,
        1e-12,
        "MV Welch p",
    );
    assert_approx_eq(
        scalar_by_key(&run_kind(&sheet, tg(TwoGroupMethod::MannWhitneyU, Some(0), Some(1), false)).unwrap(), "PValue"),
        exp_mw.p_value,
        1e-12,
        1e-12,
        "MV Mann-Whitney p",
    );
    assert!(matches!(run_kind(&sheet, tg(TwoGroupMethod::PairedT, Some(0), Some(1), false)), Err(StatsError::InvalidParameter { .. })));

    let mut sheet_3 = sheet.clone();
    if let TableArchetype::MultipleVariables(mv) = &mut sheet_3.archetype {
        mv.variables[0].cells[12] = Cell::text("DoseHigh");
    }
    assert!(matches!(
        run_kind(&sheet_3, tg(TwoGroupMethod::UnpairedT, Some(0), Some(1), false)),
        Err(StatsError::DimensionMismatch { expected: 2, actual: 3 })
    ));

    let one_g = [Cell::text("OnlyOne"), Cell::text("OnlyOne"), Cell::text("OnlyOne"), Cell::text("OnlyOne")];
    let one_r = [Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)];
    let sheet_1 = mv_cells_sheet("mv-1g", &[("Arm", Some(VariableRole::Categorical), &one_g), ("Y", Some(VariableRole::Continuous), &one_r)]);
    assert!(matches!(
        run_kind(&sheet_1, tg(TwoGroupMethod::UnpairedT, Some(0), Some(1), false)),
        Err(StatsError::DimensionMismatch { expected: 2, actual: 1 })
    ));
}

#[test]
fn multiple_row_t_tests_emits_tabs_scalars_and_volcano_records() {
    let features = ["GeneA", "GeneB", "GeneC", "GeneD", "GeneE"];
    let data_a = [[10.0, 10.2, 9.9, 10.1], [20.0, 20.5, 19.8, 20.1], [5.0, 5.4, 4.9, 5.3], [12.0, 13.0, 11.5, 12.5], [8.0, 7.5, 8.5, 8.0]];
    let data_b = [[2.0, 2.1, 1.9, 2.2], [12.0, 12.2, 11.9, 12.3], [3.8, 3.9, 4.1, 3.7], [12.1, 12.9, 11.6, 12.4], [8.2, 7.6, 8.4, 7.8]];
    let a_rows: Vec<&[f64]> = data_a.iter().map(AsRef::as_ref).collect();
    let b_rows: Vec<&[f64]> = data_b.iter().map(AsRef::as_ref).collect();
    let sheet = grouped_sheet("mr-src", SubcolumnMode::RawReplicates(4), &features, &[("Vehicle", &a_rows), ("Drug", &b_rows)]);

    let res = run_kind(
        &sheet,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: false,
            nonparametric: false,
            assume_equal_sd: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
        }),
    )
    .unwrap();
    let (sum_tab, cmp_tab) = (tab_by_kind(&res, ResultTabKind::MainSummary), tab_by_kind(&res, ResultTabKind::MultipleComparisons));
    assert_eq!((cell_num(&sum_tab.rows[0][0]), cmp_tab.rows.len(), res.pairwise_comparisons.len()), (5.0, 5, 5));

    let (mut raw_ps, mut diffs) = (Vec::new(), Vec::new());
    for r in 0..5 {
        let w = welch_t_test(&data_a[r], &data_b[r], true, ConfidenceLevel::DEFAULT_95).unwrap();
        raw_ps.push(w.p_value);
        diffs.push(w.difference);
    }
    let bh_q = adjust_benjamini_hochberg(&raw_ps);
    let expected_disc = bh_q.iter().filter(|&&q| q <= 0.05).count();
    assert_approx_eq(scalar_by_key(&res, "RowsTested"), 5.0, 1e-12, 0.0, "RowsTested scalar");
    assert_approx_eq(scalar_by_key(&res, "Discoveries"), expected_disc as f64, 1e-12, 0.0, "Discoveries scalar");
    assert_approx_eq(scalar_by_key(&res, "Threshold"), 0.05, 1e-12, 0.0, "Threshold scalar");
    for r in 0..5 {
        assert_eq!(cmp_tab.row_labels[r], features[r]);
        assert_approx_eq(cell_p(&cmp_tab.rows[r][5]), raw_ps[r], 1e-12, 1e-12, "row raw p");
        assert_approx_eq(cell_p(&cmp_tab.rows[r][6]), bh_q[r], 1e-12, 1e-12, "row BH q");
        let rec = &res.pairwise_comparisons[r];
        assert!(rec.label_a == features[r] && rec.is_fdr_q && rec.significant == (bh_q[r] <= 0.05));
        assert_approx_eq(rec.estimate_diff.unwrap(), diffs[r], 1e-12, 1e-12, "volcano estimate_diff");
        assert_approx_eq(rec.raw_p_or_q, bh_q[r], 1e-12, 1e-12, "volcano FDR q");
    }

    let paired_res = run_kind(
        &sheet,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: true,
            nonparametric: false,
            assume_equal_sd: false,
            multiplicity: MultiplicitySpec::default(),
        }),
    )
    .unwrap();
    let expected_row0_paired = paired_t_test(&data_a[0], &data_b[0], true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(
        cell_p(&tab_by_kind(&paired_res, ResultTabKind::MultipleComparisons).rows[0][5]),
        expected_row0_paired.p_value,
        1e-12,
        1e-12,
        "paired row 0 p",
    );
}

#[test]
fn bland_altman_absolute_and_percent_difference_sheets() {
    let a = [120.0, 132.0, 145.0, 160.0, 175.0, 190.0, 205.0, 220.0, 235.0, 250.0];
    let b = [118.0, 129.0, 146.0, 155.0, 172.0, 184.0, 201.0, 217.0, 230.0, 244.0];
    let sheet = col_sheet_hint("ba-src", ColumnDesignHint::PairedRepeatedMeasures, &[("ReferenceMeter", &a), ("TestMeter", &b)]);

    for percent_difference in [false, true] {
        let direct = bland_altman(&a, &b, percent_difference, ConfidenceLevel::DEFAULT_95).unwrap();
        let z_95 = norm_two_sided_isf(0.05).unwrap();
        assert_approx_eq(direct.loa_lower, direct.bias - z_95 * direct.sd_bias, 1e-12, 1e-12, "LoA lower formula");
        assert_approx_eq(direct.loa_upper, direct.bias + z_95 * direct.sd_bias, 1e-12, 1e-12, "LoA upper formula");

        let res = run_kind(&sheet, AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference, ..Default::default() }))
            .unwrap();
        let summary = tab_by_kind(&res, ResultTabKind::MainSummary);
        for (idx, lbl, exp) in [
            (2, "Bias", direct.bias),
            (3, "SD of bias", direct.sd_bias),
            (6, "Lower 95% LoA", direct.loa_lower),
            (8, "Upper 95% LoA", direct.loa_upper),
        ] {
            assert_eq!(summary.row_labels[idx], lbl);
            assert_approx_eq(cell_num(&summary.rows[idx][0]), exp, 1e-12, 1e-12, lbl);
        }
        for (k, exp) in [("Bias", direct.bias), ("SDBias", direct.sd_bias), ("LoALower", direct.loa_lower), ("LoAUpper", direct.loa_upper)] {
            assert_approx_eq(scalar_by_key(&res, k), exp, 1e-12, 1e-12, k);
        }

        let coords = tab_by_kind(&res, ResultTabKind::Residuals);
        assert_eq!(
            (coords.rows.len(), res.fitted_curves[0].equation_id.as_str(), res.fitted_curves[0].points.len()),
            (a.len(), "BlandAltman", a.len())
        );
        for (idx, ((pt, row), (&ai, &bi))) in direct.points.iter().zip(&coords.rows).zip(a.iter().zip(b.iter())).enumerate() {
            assert_approx_eq(cell_num(&row[0]), pt.0, 1e-12, 1e-12, &format!("coord avg[{idx}]"));
            assert_approx_eq(cell_num(&row[1]), pt.1, 1e-12, 1e-12, &format!("coord diff[{idx}]"));
            assert_approx_eq(cell_num(&row[2]), ai, 1e-12, 1e-12, "A coordinate");
            assert_approx_eq(cell_num(&row[3]), bi, 1e-12, 1e-12, "B coordinate");
        }
    }
}

#[test]
fn passing_bablok_dispatches_cleanly_via_run_analysis() {
    let sheet = xy_sheet("pb-src", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], &[3.0, 5.1, 6.9, 9.0, 11.0, 13.0, 15.1, 16.9, 19.0]);
    let res =
        run_analysis(&sheet, &spec_for("r-pb-1", "pb-src", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 }))).unwrap();
    assert_eq!(res.analysis.id, SheetId::new("r-pb-1"));
    assert!(!res.tabs.is_empty() && !res.fitted_curves.is_empty());
    assert_approx_eq(scalar_by_key(&res, "Slope"), 2.0, 0.05, 0.05, "Passing-Bablok slope near 2.0");
    assert_approx_eq(scalar_by_key(&res, "Intercept"), 1.0, 0.25, 0.25, "Passing-Bablok intercept near 1.0");
}

#[test]
fn kaplan_meier_mv_explicit_single_and_autodetect() {
    let arm0 = [(2.0, true), (4.0, true), (6.0, false), (7.0, true), (9.0, true), (12.0, false)];
    let arm1 = [(5.0, false), (8.0, true), (11.0, false), (14.0, true), (16.0, true), (20.0, false)];
    let mut t_cells: Vec<Cell> = arm0.iter().chain(&arm1).map(|&(t, _)| Cell::number(t)).collect();
    let mut e_cells: Vec<Cell> = arm0.iter().chain(&arm1).map(|&(_, e)| Cell::number(if e { 1.0 } else { 0.0 })).collect();
    let mut a_cells: Vec<Cell> = arm0.iter().map(|_| Cell::text("Placebo")).chain(arm1.iter().map(|_| Cell::text("Active"))).collect();
    t_cells.push(Cell::excluded_number(1.0));
    e_cells.push(Cell::number(1.0));
    a_cells.push(Cell::text("Placebo"));
    let mut mv_sheet = mv_cells_sheet(
        "mv-km",
        &[
            ("FollowUpMonths", Some(VariableRole::Continuous), &t_cells),
            ("EventStatus", Some(VariableRole::Continuous), &e_cells),
            ("Arm", Some(VariableRole::Categorical), &a_cells),
        ],
    );
    if let TableArchetype::MultipleVariables(mv) = &mut mv_sheet.archetype {
        mv.variables[2].level_order = LevelOrderSpec { order: LevelOrderKind::EntryOrder, reversed: false };
    }
    let surv_sheet = survival_sheet("st-km", &[("Placebo", &arm0), ("Active", &arm1)]);
    let km = |gbw, t_c, s_c, g_c| {
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: gbw,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: t_c,
            mv_status_column: s_c,
            mv_group_column: g_c,
        })
    };

    let st_res = run_kind(&surv_sheet, km(true, None, None, None)).unwrap();
    for (t_c, s_c, g_c) in [(Some(0), Some(1), Some(2)), (None, None, None)] {
        let mv_res = run_kind(&mv_sheet, km(true, t_c, s_c, g_c)).unwrap();
        assert_eq!(mv_res.fitted_curves.len(), 2);
        for key in ["Log-rank chi-square", "Log-rank P value", "Gehan-Breslow-Wilcoxon chi-square"] {
            assert_approx_eq(scalar_by_key(&mv_res, key), scalar_by_key(&st_res, key), 1e-12, 1e-12, key);
        }
    }

    let single_res = run_kind(&mv_sheet, km(false, Some(0), Some(1), None)).unwrap();
    assert_eq!(single_res.fitted_curves.len(), 1);
    assert!(single_res.tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk));
    assert!(!single_res.tabs.iter().any(|t| t.kind == ResultTabKind::ModelComparison));

    let (obs_a, obs_b) = (obs(&arm0), obs(&arm1));
    assert_approx_eq(
        scalar_by_key(&st_res, "Log-rank chi-square"),
        compare_survival_curves(&[&obs_a, &obs_b], false).unwrap().chi_square,
        1e-12,
        1e-12,
        "direct log-rank parity",
    );
    assert_approx_eq(
        scalar_by_key(&st_res, "Median survival (Placebo)"),
        kaplan_meier_curve("Placebo", &obs_a, true, ConfidenceLevel::DEFAULT_95).unwrap().median_survival.unwrap(),
        1e-12,
        1e-12,
        "direct median",
    );

    let mut bad_sheet = mv_sheet;
    if let TableArchetype::MultipleVariables(mv) = &mut bad_sheet.archetype {
        mv.variables[1].cells[0] = Cell::number(2.0);
    }
    assert!(matches!(run_kind(&bad_sheet, km(false, Some(0), Some(1), Some(2))), Err(StatsError::InvalidParameter { name: "survival_status", .. })));
}

#[test]
fn cox_ph_emits_baseline_survival_and_log_minus_log_tabs() {
    let sheet = mv_num_sheet(
        "cox-src",
        &[
            ("Time", &[3.0, 5.0, 6.0, 7.0, 9.0, 11.0, 13.0, 15.0, 18.0, 20.0, 22.0, 25.0]),
            ("Status", &[1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0]),
            ("AgeCentered", &[0.2, 1.1, -0.4, 0.8, -1.0, 0.5, 0.1, -0.7, 0.3, -1.2, 0.6, -0.3]),
            ("Biomarker", &[-0.5, 0.3, 0.9, -0.2, 0.7, -0.8, 0.4, 0.1, -0.6, 0.8, -0.4, 0.2]),
        ],
    );

    for tie_method in [CoxTieMethod::Breslow, CoxTieMethod::Efron, CoxTieMethod::Exact] {
        let res = run_kind(
            &sheet,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2, 3],
                reference_levels: vec![],
                tie_method,
            }),
        )
        .unwrap();
        assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals));
        let (base_tab, lml_tab) = (tab_by_kind(&res, ResultTabKind::BaselineSurvival), tab_by_kind(&res, ResultTabKind::LogMinusLog));
        assert_eq!((base_tab.rows.len(), lml_tab.rows.len()), (9, 8));
        assert_approx_eq(cell_num(&base_tab.rows[0][0]), 0.0, 1e-12, 0.0, "t0 = 0");
        assert_approx_eq(cell_num(&base_tab.rows[0][1]), 0.0, 1e-12, 0.0, "H0(0) = 0");
        assert_approx_eq(cell_num(&base_tab.rows[0][2]), 1.0, 1e-12, 0.0, "S0(0) = 1");

        let mut prev_s0 = 1.0_f64;
        for (i, row) in base_tab.rows[1..].iter().enumerate() {
            let (h0, s0) = (cell_num(&row[1]), cell_num(&row[2]));
            assert!(h0 > 0.0 && s0 > 0.0 && s0 < 1.0 && s0 <= prev_s0 + 1e-12);
            assert_approx_eq(s0, (-h0).exp(), 1e-12, 1e-12, "S0(t) = exp(-H0(t))");
            prev_s0 = s0;
            let lml_row = &lml_tab.rows[i];
            assert_approx_eq(cell_num(&lml_row[3]), h0.ln(), 1e-12, 1e-12, "ln(-ln S0(t)) == ln(H0(t))");
            assert_approx_eq(cell_num(&lml_row[1]), cell_num(&row[0]).ln(), 1e-12, 1e-12, "ln(Time) column");
        }
    }
}

#[test]
fn multiple_row_tests_student_nonparametric_and_mv_archetypes() {
    let mv_sheet = mv_cells_sheet(
        "mr-mv",
        &[
            ("Feature", Some(VariableRole::Categorical), &[Cell::text("Feat1"), Cell::text("Feat2"), Cell::text("Feat3")]),
            ("A1", Some(VariableRole::Continuous), &[Cell::number(10.0), Cell::number(5.0), Cell::number(20.0)]),
            ("A2", Some(VariableRole::Continuous), &[Cell::number(10.5), Cell::number(5.2), Cell::number(20.4)]),
            ("B1", Some(VariableRole::Continuous), &[Cell::number(2.0), Cell::number(5.1), Cell::number(19.9)]),
            ("B2", Some(VariableRole::Continuous), &[Cell::number(2.2), Cell::number(5.0), Cell::number(20.1)]),
        ],
    );
    let conf = ConfidenceLevel::DEFAULT_95;
    let stu = run_kind(
        &mv_sheet,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: false,
            nonparametric: false,
            assume_equal_sd: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TwoStageBkyFdr, threshold: 0.05, report_confidence_intervals: false },
        }),
    )
    .unwrap();
    let cmp = tab_by_kind(&stu, ResultTabKind::MultipleComparisons);
    let direct_t = unpaired_t_test(&[10.0, 10.5], &[2.0, 2.2], true, conf).unwrap();
    assert_eq!(cmp.row_labels[0], "Feat1");
    assert_approx_eq(cell_p(&cmp.rows[0][5]), direct_t.p_value, 1e-12, 1e-12, "Student pooled row p");

    let mw = run_kind(
        &mv_sheet,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: false,
            nonparametric: true,
            assume_equal_sd: false,
            multiplicity: MultiplicitySpec::default(),
        }),
    )
    .unwrap();
    let direct_mw = mann_whitney_u(&[10.0, 10.5], &[2.0, 2.2], true, conf).unwrap();
    assert_approx_eq(cell_p(&tab_by_kind(&mw, ResultTabKind::MultipleComparisons).rows[0][5]), direct_mw.p_value, 1e-12, 1e-12, "Mann-Whitney row p");

    let wil = run_kind(
        &mv_sheet,
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: true,
            nonparametric: true,
            assume_equal_sd: false,
            multiplicity: MultiplicitySpec::default(),
        }),
    )
    .unwrap();
    assert_eq!(tab_by_kind(&wil, ResultTabKind::MultipleComparisons).rows.len(), 3);
}

#[test]
fn bland_altman_on_mv_and_xy_archetypes() {
    let (a, b) = ([10.0, 20.0, 30.0, 40.0, 50.0, 60.0], [10.5, 19.8, 31.0, 39.2, 51.0, 59.5]);
    let direct = bland_altman(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    for sheet in [mv_num_sheet("ba-mv", &[("Ref", &a), ("Cand", &b)]), xy_sheet("ba-xy", &a, &b)] {
        let res = run_kind(
            &sheet,
            AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, ..Default::default() }),
        )
        .unwrap();
        assert_approx_eq(scalar_by_key(&res, "Bias"), direct.bias, 1e-12, 1e-12, sheet.id.as_str());
        assert_approx_eq(scalar_by_key(&res, "LoALower"), direct.loa_lower, 1e-12, 1e-12, sheet.id.as_str());
        assert_approx_eq(scalar_by_key(&res, "LoAUpper"), direct.loa_upper, 1e-12, 1e-12, sheet.id.as_str());
    }
}
