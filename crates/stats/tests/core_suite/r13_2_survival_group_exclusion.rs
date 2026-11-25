#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! R13.2 (WF-53) Kaplan–Meier behaviour when a whole group has no included observations.
//!
//! Excluding every cell of one arm (the effect of selecting a column and excluding it) must leave
//! the analysis of the remaining arms identical to analysing those arms alone, instead of failing
//! with `StatsError::EmptyInput`. The reference is therefore the two-arm run on the same
//! observations, whose hazard ratios are themselves checked against R 4.5.2 in
//! `r35_survival_twogroup_oracles.rs`; here the equivalence must hold to `1e-12` (identical
//! arithmetic on identical inputs). The datasets are original synthetic values.

use curvebook_doc::{
    AnalysisKind, Cell, ChecklistStatus, ConfidenceLevel, DataSheet, MultiplicitySpec, ResultCell, ResultSheet, ResultTabKind, SheetId,
    SurvivalAnalysisSpec, SurvivalTable, TableArchetype,
};
use curvebook_stats::StatsError;
use curvebook_stats::survival::two_cohort_hazard_ratio;

use crate::common::{cell_ci_finite, cell_num, obs, run_kind, scalar, tab_by_id};

const TOL: f64 = 1e-12;
const ARM_A: [(f64, bool); 6] = [(1.5, true), (3.0, true), (4.0, false), (6.5, true), (9.0, true), (12.0, false)];
const ARM_B: [(f64, bool); 6] = [(2.0, false), (5.0, true), (7.5, true), (10.0, false), (13.0, true), (16.0, false)];
const ARM_C: [(f64, bool); 4] = [(0.5, true), (2.5, true), (3.5, true), (8.0, false)];

#[derive(Clone, Copy)]
enum ArmC {
    Absent,
    Present,
    AllExcluded,
    Blank,
}

fn sheet(arm_c: ArmC) -> DataSheet {
    let with_c = !matches!(arm_c, ArmC::Absent);
    let mut t = SurvivalTable::new(if with_c { 3 } else { 2 }, ARM_A.len() + ARM_C.len() + ARM_B.len()).unwrap();
    let mut r = 0;
    for &(time, ev) in &ARM_A {
        (t.times[r], t.cohorts[0].cells[r][0]) = (Cell::number(time), Cell::number(if ev { 1.0 } else { 0.0 }));
        r += 1;
    }
    for &(time, ev) in &ARM_C {
        let s = if ev { 1.0 } else { 0.0 };
        match arm_c {
            ArmC::Present => (t.times[r], t.cohorts[2].cells[r][0]) = (Cell::number(time), Cell::number(s)),
            ArmC::AllExcluded => (t.times[r], t.cohorts[2].cells[r][0]) = (Cell::number(time), Cell::excluded_number(s)),
            ArmC::Blank | ArmC::Absent => {}
        }
        r += 1;
    }
    for &(time, ev) in &ARM_B {
        (t.times[r], t.cohorts[1].cells[r][0]) = (Cell::number(time), Cell::number(if ev { 1.0 } else { 0.0 }));
        r += 1;
    }
    t.cohorts[0].title = "Arm A".into();
    t.cohorts[1].title = "Arm B".into();
    if with_c {
        t.cohorts[2].title = "Arm C".into();
    }
    DataSheet::new(SheetId::new("km-excl"), "KM exclusion", TableArchetype::Survival(t)).unwrap()
}

fn km_kind() -> AnalysisKind {
    AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: true,
        trend_test: true,
        asymmetrical_greenwood_ci: false,
        multiplicity: MultiplicitySpec::default(),
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
    })
}

fn assert_two_arm_equivalent(reduced: &ResultSheet, two_arm: &ResultSheet, label: &str) {
    for key in [
        "Log-rank chi-square",
        "Log-rank P value",
        "Gehan-Breslow-Wilcoxon chi-square",
        "Gehan-Breslow-Wilcoxon P",
        "Hazard ratio (log-rank)",
        "Hazard ratio (Mantel-Haenszel)",
        "Hazard ratio (Peto one-step)",
    ] {
        let (a, b) = (scalar(reduced, key), scalar(two_arm, key));
        assert!((a - b).abs() <= TOL, "{label}: {key} reduced={a} two-arm={b}");
    }
    assert_eq!(tab_by_id(reduced, "km-summary").row_labels, vec!["Arm A", "Arm B"]);
    assert!(reduced.tabs.iter().all(|t| t.id != "survival-pairwise"));
    let (hr_red, hr_two) = (tab_by_id(reduced, "hazard-ratio"), tab_by_id(two_arm, "hazard-ratio"));
    assert_eq!((hr_red.kind, &hr_red.rows), (ResultTabKind::ParametersAndConfidenceIntervals, &hr_two.rows));
    assert_eq!(tab_by_id(reduced, "curve-comparison").rows.len(), 2);
    assert_eq!(reduced.fitted_curves.len(), 2);
    let mut idx: Vec<usize> = reduced.fitted_curves.iter().map(|c| c.data_set_index).collect();
    idx.sort_unstable();
    assert_eq!(idx, vec![0, 1]);
}

#[test]
fn fully_excluded_arm_reduces_to_two_arm_comparison() {
    let (two_arm, reduced) = (run_kind(&sheet(ArmC::Absent), km_kind()).unwrap(), run_kind(&sheet(ArmC::AllExcluded), km_kind()).unwrap());
    assert_two_arm_equivalent(&reduced, &two_arm, "all excluded");

    let hr = two_cohort_hazard_ratio(&obs(&ARM_A), &obs(&ARM_B), ConfidenceLevel::new(95.0).unwrap()).unwrap();
    assert!((scalar(&reduced, "Hazard ratio (Mantel-Haenszel)") - hr.mantel_haenszel_hr).abs() <= TOL);
    assert!((scalar(&reduced, "Hazard ratio (log-rank)") - hr.log_rank_hr).abs() <= TOL);

    let note = reduced.checklist.iter().find(|c| c.assumption == "Groups without data").unwrap();
    assert!(note.status == ChecklistStatus::Info && note.explanation.contains("Arm C"));
    assert!(reduced.tabs.first().unwrap().footnotes.iter().any(|f| f.contains("Arm C")));
    assert!(two_arm.checklist.iter().all(|c| c.assumption != "Groups without data"));
}

#[test]
fn blank_arm_and_spacer_rows_reduce_to_two_arm_comparison() {
    let (two_arm, reduced) = (run_kind(&sheet(ArmC::Absent), km_kind()).unwrap(), run_kind(&sheet(ArmC::Blank), km_kind()).unwrap());
    assert_two_arm_equivalent(&reduced, &two_arm, "blank arm");
    assert_eq!(tab_by_id(&reduced, "number-at-risk").rows, tab_by_id(&two_arm, "number-at-risk").rows);
}

#[test]
fn three_arm_table_still_runs_three_arm_comparison() {
    let rs = run_kind(&sheet(ArmC::Present), km_kind()).unwrap();
    assert!(rs.tabs.iter().any(|t| t.id == "survival-pairwise") && rs.fitted_curves.len() == 3);
    assert!(rs.checklist.iter().all(|c| c.assumption != "Groups without data"));
}

#[test]
fn single_remaining_arm_reports_curve_without_comparison() {
    let mut s = sheet(ArmC::AllExcluded);
    let TableArchetype::Survival(t) = &mut s.archetype else { panic!("survival sheet") };
    for row in &mut t.cohorts[1].cells {
        if let Some(c) = row.first_mut() {
            c.excluded = true;
        }
    }
    let rs = run_kind(&s, km_kind()).unwrap();
    assert!(rs.fitted_curves.len() == 1 && rs.tabs.iter().all(|t| t.id != "curve-comparison"));
    let note = rs.checklist.iter().find(|c| c.assumption == "Groups without data").unwrap();
    assert!(note.explanation.contains("Arm B") && note.explanation.contains("Arm C"));
}

#[test]
fn every_arm_without_data_is_empty_input() {
    let mut s = sheet(ArmC::AllExcluded);
    let TableArchetype::Survival(t) = &mut s.archetype else { panic!("survival sheet") };
    for cohort in &mut t.cohorts {
        for row in &mut cohort.cells {
            if let Some(c) = row.first_mut() {
                c.excluded = true;
            }
        }
    }
    assert!(matches!(run_kind(&s, km_kind()), Err(StatsError::EmptyInput)));
}

#[test]
fn two_arm_emits_hazard_ratio_tab_with_95ci() {
    let rs = run_kind(&sheet(ArmC::Absent), km_kind()).unwrap();
    let hr = two_cohort_hazard_ratio(&obs(&ARM_A), &obs(&ARM_B), ConfidenceLevel::new(95.0).unwrap()).unwrap();
    let hr_tab = tab_by_id(&rs, "hazard-ratio");
    assert_eq!(hr_tab.kind, ResultTabKind::ParametersAndConfidenceIntervals);
    assert_eq!(hr_tab.column_headers, vec!["Hazard ratio", "95% CI"]);
    assert_eq!(hr_tab.row_labels, vec!["Log-rank", "Mantel-Haenszel", "Peto one-step"]);
    for (row_idx, (exp_hr, exp_ci)) in [
        (hr.log_rank_hr, hr.log_rank_ci.unwrap()),
        (hr.mantel_haenszel_hr, hr.mantel_haenszel_ci.unwrap()),
        (hr.peto_one_step_hr, hr.peto_one_step_ci.unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        assert!((cell_num(&hr_tab.rows[row_idx][0]) - exp_hr).abs() <= TOL);
        let (lo, hi) = cell_ci_finite(&hr_tab.rows[row_idx][1]);
        assert!((lo - exp_ci.0).abs() <= TOL && (hi - exp_ci.1).abs() <= TOL);
    }
}

#[test]
fn hundred_percent_censored_arm_produces_flat_survival_and_valid_nar() {
    let mut t = SurvivalTable::new(2, 10).unwrap();
    (t.cohorts[0].title, t.cohorts[1].title) = ("Treated".into(), "All-Censored".into());
    for (r, &(time, st)) in [(1.0, 1.0), (4.0, 1.0), (8.0, 0.0), (10.0, 1.0)].iter().enumerate() {
        (t.times[r], t.cohorts[0].cells[r][0]) = (Cell::number(time), Cell::number(st));
    }
    for (i, &time) in [2.0, 5.0, 9.0, 14.0].iter().enumerate() {
        (t.times[6 + i], t.cohorts[1].cells[6 + i][0]) = (Cell::number(time), Cell::number(0.0));
    }
    let ds = DataSheet::new(SheetId::new("km-censored"), "All-censored arm", TableArchetype::Survival(t)).unwrap();
    let rs = run_kind(&ds, km_kind()).unwrap();

    let summary = tab_by_id(&rs, "km-summary");
    assert_eq!(summary.row_labels, vec!["Treated", "All-Censored"]);
    assert_eq!((cell_num(&summary.rows[1][1]), cell_num(&summary.rows[1][2]), &summary.rows[1][3]), (0.0, 4.0, &ResultCell::Empty));

    let cens_curve = rs.fitted_curves.iter().find(|c| c.data_set_index == 1).unwrap();
    assert!(cens_curve.points.iter().all(|p| p.y.is_finite() && (p.y - 1.0).abs() <= TOL));
    assert!((cens_curve.points.last().unwrap().x - 14.0).abs() <= TOL);

    let cens_nar: Vec<_> =
        tab_by_id(&rs, "number-at-risk").rows.iter().filter(|row| matches!(&row[0], ResultCell::Text(s) if s == "All-Censored")).collect();
    assert_eq!(cens_nar.len(), 4);
    assert!((cell_num(&cens_nar[0][2]) - 4.0).abs() <= TOL && (cell_num(&cens_nar[3][2]) - 1.0).abs() <= TOL);
}
