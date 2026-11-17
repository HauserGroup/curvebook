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
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet, DescriptiveSpec, GroupedTable,
    LevelOrderKind, LevelOrderSpec, MultipleRowTestsSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec,
    OneWayMethod, PassingBablokSpec, ResultCell, ResultTabKind, SheetId, SubcolumnMode, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus,
    SurvivalTable, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_two_sided_isf, student_t_two_sided_p, studentized_range_sf};
use curvebook_stats::column::qq_expected_quantiles;
use curvebook_stats::multiplicity::{MultiplicityProcedure, adjust_benjamini_hochberg};
use curvebook_stats::survival::{compare_survival_curves, kaplan_meier_curve};
use curvebook_stats::two_group::{bland_altman, mann_whitney_u, paired_t_test, unpaired_t_test, welch_t_test};
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::assert_approx_eq;

fn spec_for(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(src), kind).unwrap()
}

fn scalar_by_key(sheet: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    assert!(sheet.scalars.iter().any(|s| s.key == key), "missing scalar {key}");
    sheet.scalars.iter().find(|s| s.key == key).map_or(f64::NAN, |s| s.value)
}

fn tab_by_kind(sheet: &curvebook_doc::ResultSheet, kind: ResultTabKind) -> &curvebook_doc::ResultTab {
    assert!(sheet.tabs.iter().any(|t| t.kind == kind), "missing expected tab kind {kind:?}");
    sheet.tabs.iter().find(|t| t.kind == kind).unwrap()
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => {
            assert!(matches!(other, ResultCell::Number { .. }), "expected Number cell, got {other:?}");
            f64::NAN
        }
    }
}

fn cell_p(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => {
            assert!(matches!(other, ResultCell::PValue { .. }), "expected PValue cell, got {other:?}");
            f64::NAN
        }
    }
}

// ============================================================================
// 1. Multiplicity: Newman-Keuls & Fisher LSD
// ============================================================================

#[test]
fn multiplicity_newman_keuls_and_fisher_lsd_run_end_to_end() {
    assert_eq!(MultiplicityProcedure::from_spec(MultiplicityMethod::NewmanKeuls), MultiplicityProcedure::NewmanKeuls);
    assert_eq!(MultiplicityProcedure::from_spec(MultiplicityMethod::FisherLsd), MultiplicityProcedure::FisherLsd);
    assert_eq!(MultiplicityProcedure::from(MultiplicityMethod::NewmanKeuls), MultiplicityProcedure::NewmanKeuls);
    assert_eq!(MultiplicityProcedure::from(MultiplicityMethod::FisherLsd), MultiplicityProcedure::FisherLsd);

    // Balanced 4-group layout with exact analytical ANOVA quantities:
    // means = [11.5, 15.5, 19.5, 31.5], n = 4 per group, ss_within = 20, ms_within = 20 / 12 = 5/3.
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 4).unwrap();
    let cols =
        [("G1", [10.0, 11.0, 12.0, 13.0]), ("G2", [14.0, 15.0, 16.0, 17.0]), ("G3", [18.0, 19.0, 20.0, 21.0]), ("G4", [30.0, 31.0, 32.0, 33.0])];
    for (c, (title, vals)) in cols.iter().enumerate() {
        table.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let sheet = DataSheet::new(SheetId::new("anova-src"), "Balanced 4-group", TableArchetype::Column(table)).unwrap();
    let ms_within: f64 = 20.0 / 12.0;
    let df_within: f64 = 12.0;
    let se_diff = (ms_within * (1.0 / 4.0 + 1.0 / 4.0)).sqrt();

    let lsd_spec = spec_for(
        "r-lsd",
        "anova-src",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::FisherLsd, threshold: 0.05, report_confidence_intervals: true },
        }),
    );
    let lsd_res = run_analysis(&sheet, &lsd_spec).unwrap();
    assert_eq!(lsd_res.pairwise_comparisons.len(), 6);
    let expected_adj_p = student_t_two_sided_p(4.0 / se_diff, df_within).unwrap();
    let g1_g2 = lsd_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G2").unwrap();
    assert_approx_eq(g1_g2.raw_p_or_q, expected_adj_p, 1e-12, 1e-12, "Fisher LSD matches unadjusted Student-t");
    assert!(!g1_g2.is_fdr_q);
    assert!(g1_g2.significant);

    let lsd_tab = tab_by_kind(&lsd_res, ResultTabKind::MultipleComparisons);
    // Confidence interval column (col 6) should be populated when report_confidence_intervals is true.
    assert!(matches!(lsd_tab.rows[0][6], ResultCell::ConfidenceInterval { .. }));

    let snk_spec = spec_for(
        "r-snk",
        "anova-src",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::NewmanKeuls, threshold: 0.05, report_confidence_intervals: true },
        }),
    );
    let snk_res = run_analysis(&sheet, &snk_spec).unwrap();
    assert_eq!(snk_res.pairwise_comparisons.len(), 6);

    let tukey_spec = spec_for(
        "r-tukey",
        "anova-src",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
        }),
    );
    let tukey_res = run_analysis(&sheet, &tukey_spec).unwrap();
    let tukey_g1_g2 = tukey_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G2").unwrap();

    // Adjacent pair (G1 vs G2) has span r = 2 in SNK -> uses studentized_range_sf(q, 2, df), strictly smaller than Tukey k = 4!
    let snk_g1_g2 = snk_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G2").unwrap();
    let snk_g1_g4 = snk_res.pairwise_comparisons.iter().find(|p| p.label_a == "G1" && p.label_b == "G4").unwrap();
    let q_12 = 4.0 / (0.5 * ms_within * 0.5).sqrt();
    let q_14 = 20.0 / (0.5 * ms_within * 0.5).sqrt();
    let expected_snk_14 = studentized_range_sf(q_14, 4, df_within).unwrap();
    let expected_snk_12_span2 = studentized_range_sf(q_12, 2, df_within).unwrap();
    assert_approx_eq(snk_g1_g4.raw_p_or_q, expected_snk_14, 1e-12, 1e-12, "SNK max-span p equals q(span=4)");
    assert_approx_eq(snk_g1_g2.raw_p_or_q, expected_snk_12_span2, 1e-12, 1e-12, "SNK span-2 p");
    assert_approx_eq(snk_g1_g2.raw_p_or_q, expected_adj_p, 1e-10, 1e-10, "SNK span=2 matches 2-group Student-t");
    assert!(snk_g1_g2.raw_p_or_q < tukey_g1_g2.raw_p_or_q, "SNK step-down span=2 should be less conservative than Tukey k=4");
}

// ============================================================================
// 2. Descriptive Analysis: NormalQq Tab
// ============================================================================

#[test]
fn descriptive_analysis_emits_normal_and_lognormal_qq_tab() {
    let pos = [1.2, 2.4, 3.8, 5.1, 7.9, 12.5, 19.3, 30.0];
    let mixed = [-3.0, -1.0, 0.0, 1.5, 2.5, 4.0, 5.0, 8.0];
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 8).unwrap();
    table.data_sets[0].title = String::from("Positive");
    table.data_sets[1].title = String::from("Signed");
    for (r, (&a, &b)) in pos.iter().zip(mixed.iter()).enumerate() {
        table.data_sets[0].cells[r][0] = Cell::number(a);
        table.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let sheet = DataSheet::new(SheetId::new("desc-src"), "Q-Q", TableArchetype::Column(table)).unwrap();
    let spec = spec_for(
        "r-desc-qq",
        "desc-src",
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: true, ..DescriptiveSpec::default() }),
    );

    let res = run_analysis(&sheet, &spec).unwrap();
    // Existing tabs remain intact!
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::DescriptiveSummary));
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::Diagnostics));
    let qq_tab = tab_by_kind(&res, ResultTabKind::NormalQq);
    assert_eq!(qq_tab.rows.len(), pos.len() + mixed.len());

    let exp_norm_pos = qq_expected_quantiles(&pos, false).unwrap();
    let exp_log_pos = qq_expected_quantiles(&pos, true).unwrap();
    for (i, ((&(en, on), &(el, ol)), row)) in exp_norm_pos.iter().zip(exp_log_pos.iter()).zip(&qq_tab.rows[..pos.len()]).enumerate() {
        assert_eq!(row[0], ResultCell::Text(String::from("Positive")));
        assert_approx_eq(cell_num(&row[1]), (i + 1) as f64, 1e-12, 0.0, "order index");
        assert_approx_eq(cell_num(&row[2]), en, 1e-12, 1e-12, "normal expected quantile");
        assert_approx_eq(cell_num(&row[3]), on, 1e-12, 1e-12, "normal observed value");
        assert_approx_eq(cell_num(&row[4]), el, 1e-12, 1e-12, "lognormal expected quantile");
        assert_approx_eq(cell_num(&row[5]), ol, 1e-12, 1e-12, "lognormal observed ln(Y)");
    }

    // Non-positive dataset keeps Normal coordinates and safely emits Empty for lognormal cells.
    let exp_norm_mix = qq_expected_quantiles(&mixed, false).unwrap();
    for (&(en, on), row) in exp_norm_mix.iter().zip(&qq_tab.rows[pos.len()..]) {
        assert_eq!(row[0], ResultCell::Text(String::from("Signed")));
        assert_approx_eq(cell_num(&row[2]), en, 1e-12, 1e-12, "signed normal expected");
        assert_approx_eq(cell_num(&row[3]), on, 1e-12, 1e-12, "signed normal observed");
        assert_eq!(row[4], ResultCell::Empty);
        assert_eq!(row[5], ResultCell::Empty);
    }
    assert!(!qq_tab.footnotes.is_empty());
}

// ============================================================================
// 3. TwoGroupSpec on MultipleVariables (Categorical Grouping + Validation)
// ============================================================================

#[test]
fn two_group_mv_categorical_grouping_and_validation() {
    let mut mv = MultipleVariablesTable::new(2, 14).unwrap();
    mv.variables[0].title = String::from("Arm");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[0].level_order = LevelOrderSpec { order: LevelOrderKind::EntryOrder, reversed: false };
    mv.variables[1].title = String::from("Response");
    mv.variables[1].lock_role(VariableRole::Continuous);

    let ctrl_vals = [10.1, 11.4, 9.8, 10.7, 11.0];
    let trt_vals = [15.2, 16.0, 14.8, 15.6, 16.4];
    for (r, &v) in ctrl_vals.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text("Control");
        mv.variables[1].cells[r] = Cell::number(v);
    }
    for (r, &v) in trt_vals.iter().enumerate() {
        mv.variables[0].cells[5 + r] = Cell::text("Treatment");
        mv.variables[1].cells[5 + r] = Cell::number(v);
    }
    // Missingness & non-destructive exclusions:
    mv.variables[0].cells[10] = Cell::text("Control");
    mv.variables[1].cells[10] = Cell::empty(); // missing response
    mv.variables[0].cells[11] = Cell::empty(); // missing group
    mv.variables[1].cells[11] = Cell::number(99.0);
    mv.variables[0].cells[12] = Cell::excluded_text("ThirdExcluded");
    mv.variables[1].cells[12] = Cell::number(50.0);
    mv.variables[0].cells[13] = Cell::text("Treatment");
    mv.variables[1].cells[13] = Cell::excluded_number(100.0);

    let sheet = DataSheet::new(SheetId::new("mv-2g"), "MV 2-group", TableArchetype::MultipleVariables(mv.clone())).unwrap();
    let expected_unpaired = unpaired_t_test(&ctrl_vals, &trt_vals, true, ConfidenceLevel::DEFAULT_95).unwrap();
    let expected_welch = welch_t_test(&ctrl_vals, &trt_vals, true, ConfidenceLevel::DEFAULT_95).unwrap();
    let expected_mw = mann_whitney_u(&ctrl_vals, &trt_vals, true, ConfidenceLevel::DEFAULT_95).unwrap();

    // Explicit MV column specification + auto-detection when mv_*_column omitted.
    for (mv_g, mv_r) in [(Some(0), Some(1)), (None, None)] {
        let sp_u = spec_for(
            "r-mv-u",
            "mv-2g",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::UnpairedT,
                two_tailed: true,
                compute_effect_sizes: true,
                mv_group_column: mv_g,
                mv_response_column: mv_r,
            }),
        );
        let ru = run_analysis(&sheet, &sp_u).unwrap();
        assert_approx_eq(scalar_by_key(&ru, "t"), expected_unpaired.t_statistic, 1e-12, 1e-12, "MV unpaired t");
        assert_approx_eq(scalar_by_key(&ru, "PValue"), expected_unpaired.p_value, 1e-12, 1e-12, "MV unpaired p");
        assert_eq!(ru.pairwise_comparisons[0].label_a, "Control");
        assert_eq!(ru.pairwise_comparisons[0].label_b, "Treatment");
        assert!(ru.checklist.iter().any(|c| c.assumption.contains("Complete-case")));
    }

    let rw = run_analysis(
        &sheet,
        &spec_for(
            "r-mv-w",
            "mv-2g",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::WelchT,
                two_tailed: true,
                compute_effect_sizes: false,
                mv_group_column: Some(0),
                mv_response_column: Some(1),
            }),
        ),
    )
    .unwrap();
    assert_approx_eq(scalar_by_key(&rw, "PValue"), expected_welch.p_value, 1e-12, 1e-12, "MV Welch p");

    let rmw = run_analysis(
        &sheet,
        &spec_for(
            "r-mv-mw",
            "mv-2g",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::MannWhitneyU,
                two_tailed: true,
                compute_effect_sizes: false,
                mv_group_column: Some(0),
                mv_response_column: Some(1),
            }),
        ),
    )
    .unwrap();
    assert_approx_eq(scalar_by_key(&rmw, "PValue"), expected_mw.p_value, 1e-12, 1e-12, "MV Mann-Whitney p");

    // Validation errors: paired test on categorical MV, 3 levels, 1 level, empty level responses.
    let err_paired = run_analysis(
        &sheet,
        &spec_for(
            "r-mv-paired",
            "mv-2g",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::PairedT,
                two_tailed: true,
                compute_effect_sizes: false,
                mv_group_column: Some(0),
                mv_response_column: Some(1),
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_paired, StatsError::InvalidParameter { .. }));

    let mut mv_3lvl = mv.clone();
    mv_3lvl.variables[0].cells[12] = Cell::text("DoseHigh");
    let sheet_3 = DataSheet::new(SheetId::new("mv-3g"), "MV 3-lvl", TableArchetype::MultipleVariables(mv_3lvl)).unwrap();
    let err_3 = run_analysis(
        &sheet_3,
        &spec_for(
            "r-mv-3",
            "mv-3g",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::UnpairedT,
                two_tailed: true,
                compute_effect_sizes: false,
                mv_group_column: Some(0),
                mv_response_column: Some(1),
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_3, StatsError::DimensionMismatch { expected: 2, actual: 3 }));

    let mut mv_1lvl = MultipleVariablesTable::new(2, 4).unwrap();
    mv_1lvl.variables[0].lock_role(VariableRole::Categorical);
    for r in 0..4 {
        mv_1lvl.variables[0].cells[r] = Cell::text("OnlyOne");
        mv_1lvl.variables[1].cells[r] = Cell::number((r + 1) as f64);
    }
    let sheet_1 = DataSheet::new(SheetId::new("mv-1g"), "MV 1-lvl", TableArchetype::MultipleVariables(mv_1lvl)).unwrap();
    let err_1 = run_analysis(
        &sheet_1,
        &spec_for(
            "r-mv-1",
            "mv-1g",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::UnpairedT,
                two_tailed: true,
                compute_effect_sizes: false,
                mv_group_column: Some(0),
                mv_response_column: Some(1),
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_1, StatsError::DimensionMismatch { expected: 2, actual: 1 }));
}

// ============================================================================
// 4. MultipleRowTTests -> Tabs, Scalars, and Volcano PairwiseComparisonRecords
// ============================================================================

#[test]
fn multiple_row_t_tests_emits_tabs_scalars_and_volcano_records() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 5).unwrap();
    grp.data_sets[0].title = String::from("Vehicle");
    grp.data_sets[1].title = String::from("Drug");
    let features = ["GeneA", "GeneB", "GeneC", "GeneD", "GeneE"];
    let data_a = [[10.0, 10.2, 9.9, 10.1], [20.0, 20.5, 19.8, 20.1], [5.0, 5.4, 4.9, 5.3], [12.0, 13.0, 11.5, 12.5], [8.0, 7.5, 8.5, 8.0]];
    let data_b = [
        [2.0, 2.1, 1.9, 2.2],     // huge diff (mean_a - mean_b ~ +8.0)
        [12.0, 12.2, 11.9, 12.3], // huge diff (~ +8.0)
        [3.8, 3.9, 4.1, 3.7],     // moderate diff
        [12.1, 12.9, 11.6, 12.4], // null
        [8.2, 7.6, 8.4, 7.8],     // null
    ];
    for r in 0..5 {
        grp.row_titles[r] = features[r].to_owned();
        for s in 0..4 {
            grp.data_sets[0].cells[r][s] = Cell::number(data_a[r][s]);
            grp.data_sets[1].cells[r][s] = Cell::number(data_b[r][s]);
        }
    }

    let sheet = DataSheet::new(SheetId::new("mr-src"), "Row Tests", TableArchetype::Grouped(grp)).unwrap();
    let spec = spec_for(
        "r-mr-bh",
        "mr-src",
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: false,
            nonparametric: false,
            assume_equal_sd: false,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
        }),
    );

    let res = run_analysis(&sheet, &spec).unwrap();
    let sum_tab = tab_by_kind(&res, ResultTabKind::MainSummary);
    let cmp_tab = tab_by_kind(&res, ResultTabKind::MultipleComparisons);
    assert_eq!(cell_num(&sum_tab.rows[0][0]), 5.0);
    assert_eq!(cmp_tab.rows.len(), 5);
    assert_eq!(res.pairwise_comparisons.len(), 5);

    // Cross-check exact Welch t-test raw_p and BH step-up FDR q-values row by row.
    let mut raw_ps = Vec::new();
    let mut diffs = Vec::new();
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
        assert_eq!(rec.label_a, features[r]);
        assert!(rec.is_fdr_q);
        assert_approx_eq(rec.estimate_diff.unwrap(), diffs[r], 1e-12, 1e-12, "volcano estimate_diff");
        assert_approx_eq(rec.raw_p_or_q, bh_q[r], 1e-12, 1e-12, "volcano FDR q");
        assert_eq!(rec.significant, bh_q[r] <= 0.05);
    }

    // Also verify paired & nonparametric & MV split variants cleanly execute.
    let paired_res = run_analysis(
        &sheet,
        &spec_for(
            "r-mr-paired",
            "mr-src",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                paired: true,
                nonparametric: false,
                assume_equal_sd: false,
                multiplicity: MultiplicitySpec::default(),
            }),
        ),
    )
    .unwrap();
    let expected_row0_paired = paired_t_test(&data_a[0], &data_b[0], true, ConfidenceLevel::DEFAULT_95).unwrap();
    let p_cmp = tab_by_kind(&paired_res, ResultTabKind::MultipleComparisons);
    assert_approx_eq(cell_p(&p_cmp.rows[0][5]), expected_row0_paired.p_value, 1e-12, 1e-12, "paired row 0 p");
}

// ============================================================================
// 5. Bland-Altman ResultSheet Wiring
// ============================================================================

#[test]
fn bland_altman_absolute_and_percent_difference_sheets() {
    // Method A vs Method B method comparison dataset (10 matched clinical pairs).
    let a = [120.0, 132.0, 145.0, 160.0, 175.0, 190.0, 205.0, 220.0, 235.0, 250.0];
    let b = [118.0, 129.0, 146.0, 155.0, 172.0, 184.0, 201.0, 217.0, 230.0, 244.0];

    let mut table = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, a.len()).unwrap();
    table.data_sets[0].title = String::from("ReferenceMeter");
    table.data_sets[1].title = String::from("TestMeter");
    for i in 0..a.len() {
        table.data_sets[0].cells[i][0] = Cell::number(a[i]);
        table.data_sets[1].cells[i][0] = Cell::number(b[i]);
    }
    let sheet = DataSheet::new(SheetId::new("ba-src"), "Bland-Altman", TableArchetype::Column(table)).unwrap();

    for percent_difference in [false, true] {
        let direct = bland_altman(&a, &b, percent_difference, ConfidenceLevel::DEFAULT_95).unwrap();
        let z_95 = norm_two_sided_isf(0.05).unwrap();
        assert_approx_eq(direct.loa_lower, direct.bias - z_95 * direct.sd_bias, 1e-12, 1e-12, "LoA lower formula");
        assert_approx_eq(direct.loa_upper, direct.bias + z_95 * direct.sd_bias, 1e-12, 1e-12, "LoA upper formula");

        let spec = spec_for(
            if percent_difference { "r-ba-pct" } else { "r-ba-abs" },
            "ba-src",
            AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference }),
        );
        let res = run_analysis(&sheet, &spec).unwrap();
        let summary = tab_by_kind(&res, ResultTabKind::MainSummary);
        assert_eq!(summary.row_labels[2], "Bias");
        assert_eq!(summary.row_labels[3], "SD of bias");
        assert_eq!(summary.row_labels[6], "Lower 95% LoA");
        assert_eq!(summary.row_labels[8], "Upper 95% LoA");
        assert_approx_eq(cell_num(&summary.rows[2][0]), direct.bias, 1e-12, 1e-12, "MainSummary Bias");
        assert_approx_eq(cell_num(&summary.rows[3][0]), direct.sd_bias, 1e-12, 1e-12, "MainSummary SD of bias");
        assert_approx_eq(cell_num(&summary.rows[6][0]), direct.loa_lower, 1e-12, 1e-12, "MainSummary Lower 95% LoA");
        assert_approx_eq(cell_num(&summary.rows[8][0]), direct.loa_upper, 1e-12, 1e-12, "MainSummary Upper 95% LoA");

        assert_approx_eq(scalar_by_key(&res, "Bias"), direct.bias, 1e-12, 1e-12, "Bias scalar");
        assert_approx_eq(scalar_by_key(&res, "SDBias"), direct.sd_bias, 1e-12, 1e-12, "SDBias scalar");
        assert_approx_eq(scalar_by_key(&res, "LoALower"), direct.loa_lower, 1e-12, 1e-12, "LoALower scalar");
        assert_approx_eq(scalar_by_key(&res, "LoAUpper"), direct.loa_upper, 1e-12, 1e-12, "LoAUpper scalar");

        let coords = tab_by_kind(&res, ResultTabKind::Residuals);
        assert_eq!(coords.rows.len(), a.len());
        assert_eq!(res.fitted_curves[0].equation_id, "BlandAltman");
        assert_eq!(res.fitted_curves[0].points.len(), a.len());
        for (idx, ((pt, row), (&ai, &bi))) in direct.points.iter().zip(&coords.rows).zip(a.iter().zip(b.iter())).enumerate() {
            assert_approx_eq(cell_num(&row[0]), pt.0, 1e-12, 1e-12, &format!("coord avg[{idx}]"));
            assert_approx_eq(cell_num(&row[1]), pt.1, 1e-12, 1e-12, &format!("coord diff[{idx}]"));
            assert_approx_eq(cell_num(&row[2]), ai, 1e-12, 1e-12, "A coordinate");
            assert_approx_eq(cell_num(&row[3]), bi, 1e-12, 1e-12, "B coordinate");
        }
    }
}

// ============================================================================
// 6. Passing-Bablok Integration via run_analysis
// ============================================================================

#[test]
fn passing_bablok_dispatches_cleanly_via_run_analysis() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 9).unwrap();
    xy.data_sets[0].title = String::from("AssayY");
    // Exact line y = 2*x + 1 with symmetric small perturbations so Passing-Bablok recovers slope ~ 2.0, intercept ~ 1.0.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let ys = [3.0, 5.1, 6.9, 9.0, 11.0, 13.0, 15.1, 16.9, 19.0];
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new(SheetId::new("pb-src"), "Passing-Bablok", TableArchetype::Xy(xy)).unwrap();
    let spec = spec_for("r-pb-1", "pb-src", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 }));

    let res = run_analysis(&sheet, &spec).unwrap();
    assert_eq!(res.analysis.id, SheetId::new("r-pb-1"));
    assert!(!res.tabs.is_empty());
    assert!(!res.fitted_curves.is_empty());
    assert_approx_eq(scalar_by_key(&res, "Slope"), 2.0, 0.05, 0.05, "Passing-Bablok slope near 2.0");
    assert_approx_eq(scalar_by_key(&res, "Intercept"), 1.0, 0.25, 0.25, "Passing-Bablok intercept near 1.0");
}

// ============================================================================
// 7. Kaplan-Meier on MultipleVariables (Explicit, Single Cohort, Auto-Detect)
// ============================================================================

#[test]
fn kaplan_meier_mv_explicit_single_and_autodetect() {
    let mut mv = MultipleVariablesTable::new(3, 13).unwrap();
    mv.variables[0].title = String::from("FollowUpMonths");
    mv.variables[0].lock_role(VariableRole::Continuous);
    mv.variables[1].title = String::from("EventStatus");
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = String::from("Arm");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[2].level_order = LevelOrderSpec { order: LevelOrderKind::EntryOrder, reversed: false };

    let arm0 = [(2.0, 1.0), (4.0, 1.0), (6.0, 0.0), (7.0, 1.0), (9.0, 1.0), (12.0, 0.0)];
    let arm1 = [(5.0, 0.0), (8.0, 1.0), (11.0, 0.0), (14.0, 1.0), (16.0, 1.0), (20.0, 0.0)];
    for (r, &(t, e)) in arm0.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(e);
        mv.variables[2].cells[r] = Cell::text("Placebo");
    }
    for (r, &(t, e)) in arm1.iter().enumerate() {
        mv.variables[0].cells[6 + r] = Cell::number(t);
        mv.variables[1].cells[6 + r] = Cell::number(e);
        mv.variables[2].cells[6 + r] = Cell::text("Active");
    }
    // Excluded row 12 should be skipped cleanly.
    mv.variables[0].cells[12] = Cell::excluded_number(1.0);
    mv.variables[1].cells[12] = Cell::number(1.0);
    mv.variables[2].cells[12] = Cell::text("Placebo");

    let mv_sheet = DataSheet::new(SheetId::new("mv-km"), "MV KM", TableArchetype::MultipleVariables(mv.clone())).unwrap();

    // Equivalent SurvivalTable reference for cross-checking log-rank and medians.
    let mut surv = SurvivalTable::new(2, 12).unwrap();
    surv.cohorts[0].title = String::from("Placebo");
    surv.cohorts[1].title = String::from("Active");
    for (r, &(t, e)) in arm0.iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(e);
    }
    for (r, &(t, e)) in arm1.iter().enumerate() {
        surv.times[6 + r] = Cell::number(t);
        surv.cohorts[1].cells[6 + r][0] = Cell::number(e);
    }
    let surv_sheet = DataSheet::new(SheetId::new("st-km"), "Surv KM", TableArchetype::Survival(surv)).unwrap();

    let st_res = run_analysis(
        &surv_sheet,
        &spec_for(
            "r-st",
            "st-km",
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: true,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec::default(),
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        ),
    )
    .unwrap();

    // Explicit MV columns and full auto-detection should match SurvivalTable identically.
    for (t_c, s_c, g_c, id) in [(Some(0), Some(1), Some(2), "r-mv-exp"), (None, None, None, "r-mv-auto")] {
        let mv_res = run_analysis(
            &mv_sheet,
            &spec_for(
                id,
                "mv-km",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                    log_rank_mantel_cox: true,
                    gehan_breslow_wilcoxon: true,
                    trend_test: false,
                    asymmetrical_greenwood_ci: true,
                    multiplicity: MultiplicitySpec::default(),
                    mv_time_column: t_c,
                    mv_status_column: s_c,
                    mv_group_column: g_c,
                }),
            ),
        )
        .unwrap();
        assert_eq!(mv_res.fitted_curves.len(), 2);
        assert_approx_eq(
            scalar_by_key(&mv_res, "Log-rank chi-square"),
            scalar_by_key(&st_res, "Log-rank chi-square"),
            1e-12,
            1e-12,
            "MV vs Survival log-rank chi2",
        );
        assert_approx_eq(
            scalar_by_key(&mv_res, "Log-rank P value"),
            scalar_by_key(&st_res, "Log-rank P value"),
            1e-12,
            1e-12,
            "MV vs Survival log-rank P",
        );
        assert_approx_eq(
            scalar_by_key(&mv_res, "Gehan-Breslow-Wilcoxon chi-square"),
            scalar_by_key(&st_res, "Gehan-Breslow-Wilcoxon chi-square"),
            1e-12,
            1e-12,
            "MV vs Survival GBW chi2",
        );
    }

    // Unstratified single-cohort MV KM (explicit time + status, group = None).
    let single_res = run_analysis(
        &mv_sheet,
        &spec_for(
            "r-mv-1cohort",
            "mv-km",
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
        ),
    )
    .unwrap();
    assert_eq!(single_res.fitted_curves.len(), 1);
    assert!(single_res.tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk));
    assert!(!single_res.tabs.iter().any(|t| t.kind == ResultTabKind::ModelComparison));

    // Direct survival curve parity check on pooled observations.
    let obs_a: Vec<SurvivalObservation> = arm0
        .iter()
        .enumerate()
        .map(|(row, &(time, e))| SurvivalObservation { row, time, status: if e == 1.0 { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect();
    let obs_b: Vec<SurvivalObservation> = arm1
        .iter()
        .enumerate()
        .map(|(row, &(time, e))| SurvivalObservation { row, time, status: if e == 1.0 { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect();
    let cmp_direct = compare_survival_curves(&[&obs_a, &obs_b], false).unwrap();
    let km_a_direct = kaplan_meier_curve("Placebo", &obs_a, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(scalar_by_key(&st_res, "Log-rank chi-square"), cmp_direct.chi_square, 1e-12, 1e-12, "direct log-rank parity");
    assert_approx_eq(scalar_by_key(&st_res, "Median survival (Placebo)"), km_a_direct.median_survival.unwrap(), 1e-12, 1e-12, "direct median");

    // Invalid status value != 0/1 must error cleanly.
    let mut mv_bad = mv;
    mv_bad.variables[1].cells[0] = Cell::number(2.0);
    let bad_sheet = DataSheet::new(SheetId::new("mv-bad"), "Bad status", TableArchetype::MultipleVariables(mv_bad)).unwrap();
    let bad_err = run_analysis(
        &bad_sheet,
        &spec_for(
            "r-mv-bad",
            "mv-bad",
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec::default(),
                mv_time_column: Some(0),
                mv_status_column: Some(1),
                mv_group_column: Some(2),
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(bad_err, StatsError::InvalidParameter { name: "survival_status", .. }));
}

// ============================================================================
// 8. Cox Proportional Hazards: BaselineSurvival & LogMinusLog Tabs
// ============================================================================

#[test]
fn cox_ph_emits_baseline_survival_and_log_minus_log_tabs() {
    let mut mv = MultipleVariablesTable::new(4, 12).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("AgeCentered");
    mv.variables[3].title = String::from("Biomarker");

    let rows = [
        (3.0, 1.0, 0.2, -0.5),
        (5.0, 1.0, 1.1, 0.3),
        (6.0, 0.0, -0.4, 0.9),
        (7.0, 1.0, 0.8, -0.2),
        (9.0, 1.0, -1.0, 0.7),
        (11.0, 1.0, 0.5, -0.8),
        (13.0, 0.0, 0.1, 0.4),
        (15.0, 1.0, -0.7, 0.1),
        (18.0, 1.0, 0.3, -0.6),
        (20.0, 0.0, -1.2, 0.8),
        (22.0, 1.0, 0.6, -0.4),
        (25.0, 0.0, -0.3, 0.2),
    ];
    for (r, &(t, st, x1, x2)) in rows.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(st);
        mv.variables[2].cells[r] = Cell::number(x1);
        mv.variables[3].cells[r] = Cell::number(x2);
    }
    let sheet = DataSheet::new(SheetId::new("cox-src"), "Cox PH", TableArchetype::MultipleVariables(mv)).unwrap();

    for tie_method in [CoxTieMethod::Breslow, CoxTieMethod::Efron, CoxTieMethod::Exact] {
        let spec = spec_for(
            "r-cox",
            "cox-src",
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2, 3],
                reference_levels: vec![],
                tie_method,
            }),
        );
        let res = run_analysis(&sheet, &spec).unwrap();
        assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals));
        let base_tab = tab_by_kind(&res, ResultTabKind::BaselineSurvival);
        let lml_tab = tab_by_kind(&res, ResultTabKind::LogMinusLog);

        // Expect t = 0 + 8 distinct event times = 9 rows in BaselineSurvival, and 8 rows in LogMinusLog.
        assert_eq!(base_tab.rows.len(), 9);
        assert_eq!(lml_tab.rows.len(), 8);
        assert_approx_eq(cell_num(&base_tab.rows[0][0]), 0.0, 1e-12, 0.0, "t0 = 0");
        assert_approx_eq(cell_num(&base_tab.rows[0][1]), 0.0, 1e-12, 0.0, "H0(0) = 0");
        assert_approx_eq(cell_num(&base_tab.rows[0][2]), 1.0, 1e-12, 0.0, "S0(0) = 1");

        let mut prev_s0 = 1.0_f64;
        for (i, row) in base_tab.rows[1..].iter().enumerate() {
            let h0 = cell_num(&row[1]);
            let s0 = cell_num(&row[2]);
            assert!(h0 > 0.0, "H0(t) strictly positive after first event");
            assert!(s0 > 0.0 && s0 < 1.0, "S0(t) bounded in (0, 1)");
            assert!(s0 <= prev_s0 + 1e-12, "S0(t) monotonically non-increasing");
            assert_approx_eq(s0, (-h0).exp(), 1e-12, 1e-12, "S0(t) = exp(-H0(t))");
            prev_s0 = s0;

            let lml_row = &lml_tab.rows[i];
            let lml_val = cell_num(&lml_row[3]);
            assert_approx_eq(lml_val, h0.ln(), 1e-12, 1e-12, "ln(-ln S0(t)) == ln(H0(t))");
            assert_approx_eq(cell_num(&lml_row[1]), cell_num(&row[0]).ln(), 1e-12, 1e-12, "ln(Time) column");
        }
    }
}

#[test]
fn multiple_row_tests_student_nonparametric_and_mv_archetypes() {
    let mut mv = MultipleVariablesTable::new(5, 3).unwrap();
    mv.variables[0].title = String::from("Feature");
    mv.variables[0].lock_role(VariableRole::Categorical);
    for (c, title) in ["A1", "A2", "B1", "B2"].iter().enumerate() {
        mv.variables[c + 1].title = (*title).to_owned();
        mv.variables[c + 1].lock_role(VariableRole::Continuous);
    }
    let a_vals = [[10.0, 10.5, 11.0], [5.0, 5.2, 5.1], [20.0, 20.4, 19.8]];
    let b_vals = [[2.0, 2.2, 2.1], [5.1, 5.0, 5.2], [19.9, 20.1, 20.0]];
    for r in 0..3 {
        mv.variables[0].cells[r] = Cell::text(format!("Feat{}", r + 1));
        mv.variables[1].cells[r] = Cell::number(a_vals[r][0]);
        mv.variables[2].cells[r] = Cell::number(a_vals[r][1]);
        mv.variables[3].cells[r] = Cell::number(b_vals[r][0]);
        mv.variables[4].cells[r] = Cell::number(b_vals[r][1]);
    }
    let mv_sheet = DataSheet::new(SheetId::new("mr-mv"), "MV Row", TableArchetype::MultipleVariables(mv)).unwrap();

    let stu = run_analysis(
        &mv_sheet,
        &spec_for(
            "r-mr-stu",
            "mr-mv",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                paired: false,
                nonparametric: false,
                assume_equal_sd: true,
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::TwoStageBkyFdr, threshold: 0.05, report_confidence_intervals: false },
            }),
        ),
    )
    .unwrap();
    let cmp = tab_by_kind(&stu, ResultTabKind::MultipleComparisons);
    let direct_t = unpaired_t_test(&[a_vals[0][0], a_vals[0][1]], &[b_vals[0][0], b_vals[0][1]], true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(cmp.row_labels[0], "Feat1");
    assert_approx_eq(cell_p(&cmp.rows[0][5]), direct_t.p_value, 1e-12, 1e-12, "Student pooled row p");

    let mw = run_analysis(
        &mv_sheet,
        &spec_for(
            "r-mr-mw",
            "mr-mv",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                paired: false,
                nonparametric: true,
                assume_equal_sd: false,
                multiplicity: MultiplicitySpec::default(),
            }),
        ),
    )
    .unwrap();
    let direct_mw = mann_whitney_u(&[a_vals[0][0], a_vals[0][1]], &[b_vals[0][0], b_vals[0][1]], true, ConfidenceLevel::DEFAULT_95).unwrap();
    let mw_cmp = tab_by_kind(&mw, ResultTabKind::MultipleComparisons);
    assert_approx_eq(cell_p(&mw_cmp.rows[0][5]), direct_mw.p_value, 1e-12, 1e-12, "Mann-Whitney row p");

    let wil = run_analysis(
        &mv_sheet,
        &spec_for(
            "r-mr-wil",
            "mr-mv",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                paired: true,
                nonparametric: true,
                assume_equal_sd: false,
                multiplicity: MultiplicitySpec::default(),
            }),
        ),
    )
    .unwrap();
    assert_eq!(tab_by_kind(&wil, ResultTabKind::MultipleComparisons).rows.len(), 3);
}

#[test]
fn bland_altman_on_mv_and_xy_archetypes() {
    let a = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let b = [10.5, 19.8, 31.0, 39.2, 51.0, 59.5];

    let mut mv = MultipleVariablesTable::new(2, a.len()).unwrap();
    mv.variables[0].title = String::from("Ref");
    mv.variables[1].title = String::from("Cand");
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, a.len()).unwrap();
    xy.x_title = String::from("Ref");
    xy.data_sets[0].title = String::from("Cand");
    for (i, (&xa, &yb)) in a.iter().zip(b.iter()).enumerate() {
        mv.variables[0].cells[i] = Cell::number(xa);
        mv.variables[1].cells[i] = Cell::number(yb);
        xy.set_x_cell(i, Cell::number(xa)).unwrap();
        xy.data_sets[0].cells[i][0] = Cell::number(yb);
    }

    let direct = bland_altman(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    for (id, arch) in [("ba-mv", TableArchetype::MultipleVariables(mv)), ("ba-xy", TableArchetype::Xy(xy))] {
        let sheet = DataSheet::new(SheetId::new(id), id, arch).unwrap();
        let res = run_analysis(
            &sheet,
            &spec_for("r-ba-other", id, AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false })),
        )
        .unwrap();
        assert_approx_eq(scalar_by_key(&res, "Bias"), direct.bias, 1e-12, 1e-12, id);
        assert_approx_eq(scalar_by_key(&res, "LoALower"), direct.loa_lower, 1e-12, 1e-12, id);
        assert_approx_eq(scalar_by_key(&res, "LoAUpper"), direct.loa_upper, 1e-12, 1e-12, id);
    }
}
