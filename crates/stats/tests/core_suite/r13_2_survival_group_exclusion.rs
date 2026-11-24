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
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, ConfidenceLevel, DataSheet, MultiplicitySpec, ResultCell, ResultSheet, ResultTabKind,
    SheetId, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, SurvivalTable, TableArchetype,
};
use curvebook_stats::survival::two_cohort_hazard_ratio;
use curvebook_stats::{StatsError, run_analysis};

const TOL: f64 = 1e-12;

/// Arm A: (time, event?)
const ARM_A: [(f64, bool); 6] = [(1.5, true), (3.0, true), (4.0, false), (6.5, true), (9.0, true), (12.0, false)];
/// Arm B
const ARM_B: [(f64, bool); 6] = [(2.0, false), (5.0, true), (7.5, true), (10.0, false), (13.0, true), (16.0, false)];
/// Arm C (the arm that gets excluded or left blank)
const ARM_C: [(f64, bool); 4] = [(0.5, true), (2.5, true), (3.5, true), (8.0, false)];

#[derive(Clone, Copy)]
enum ArmC {
    Absent,
    Present,
    AllExcluded,
    Blank,
}

fn status(ev: bool) -> f64 {
    if ev { 1.0 } else { 0.0 }
}

/// Builds a survival sheet with arms A and B in cohorts 0 and 1, and arm C in cohort 2 unless absent.
/// Arm C rows are placed between A and B so its omission also exercises row gaps.
fn sheet(arm_c: ArmC) -> DataSheet {
    let with_c = !matches!(arm_c, ArmC::Absent);
    let n_cohorts = if with_c { 3 } else { 2 };
    let rows = ARM_A.len() + ARM_C.len() + ARM_B.len();
    let mut t = SurvivalTable::new(n_cohorts, rows).unwrap();
    let mut r = 0;
    for &(time, ev) in &ARM_A {
        t.times[r] = Cell::number(time);
        t.cohorts[0].cells[r][0] = Cell::number(status(ev));
        r += 1;
    }
    for &(time, ev) in &ARM_C {
        match arm_c {
            ArmC::Present => {
                t.times[r] = Cell::number(time);
                t.cohorts[2].cells[r][0] = Cell::number(status(ev));
            }
            ArmC::AllExcluded => {
                t.times[r] = Cell::number(time);
                t.cohorts[2].cells[r][0] = Cell::excluded_number(status(ev));
            }
            // Blank spacer rows: no time and no status in any arm.
            ArmC::Blank | ArmC::Absent => {}
        }
        r += 1;
    }
    for &(time, ev) in &ARM_B {
        t.times[r] = Cell::number(time);
        t.cohorts[1].cells[r][0] = Cell::number(status(ev));
        r += 1;
    }
    t.cohorts[0].title = "Arm A".into();
    t.cohorts[1].title = "Arm B".into();
    if with_c {
        t.cohorts[2].title = "Arm C".into();
    }
    DataSheet::new(SheetId::new("km-excl"), "KM exclusion", TableArchetype::Survival(t)).unwrap()
}

fn km_spec() -> AnalysisSpec {
    AnalysisSpec::new(
        SheetId::new("r-km-excl"),
        SheetId::new("km-excl"),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: true,
            trend_test: true,
            asymmetrical_greenwood_ci: false,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )
    .unwrap()
}

fn scalar(rs: &ResultSheet, key: &str) -> f64 {
    rs.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar {key}")).value
}

fn obs(arm: &[(f64, bool)]) -> Vec<SurvivalObservation> {
    arm.iter()
        .enumerate()
        .map(|(row, &(time, ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect()
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
    let summary = reduced.tabs.iter().find(|t| t.id == "km-summary").unwrap();
    assert_eq!(summary.row_labels, vec!["Arm A".to_owned(), "Arm B".to_owned()], "{label}: summary rows");
    assert!(reduced.tabs.iter().all(|t| t.id != "survival-pairwise"), "{label}: no 3-arm pairwise tab");
    let hr_reduced = reduced.tabs.iter().find(|t| t.id == "hazard-ratio").expect("two-arm KM must emit hazard-ratio tab");
    let hr_two = two_arm.tabs.iter().find(|t| t.id == "hazard-ratio").expect("two-arm reference hazard-ratio tab");
    assert_eq!(hr_reduced.kind, ResultTabKind::ParametersAndConfidenceIntervals);
    assert_eq!(hr_reduced.rows, hr_two.rows, "{label}: hazard-ratio tab parity");
    let cmp = reduced.tabs.iter().find(|t| t.id == "curve-comparison").unwrap();
    assert_eq!(cmp.rows.len(), 2, "{label}: log-rank + Gehan only; trend needs 3 arms");
    assert_eq!(reduced.fitted_curves.len(), 2, "{label}: two KM curves");
    let mut idx: Vec<usize> = reduced.fitted_curves.iter().map(|c| c.data_set_index).collect();
    idx.sort_unstable();
    assert_eq!(idx, vec![0, 1], "{label}: curves keep their original data-set indices");
}

#[test]
fn fully_excluded_arm_reduces_to_two_arm_comparison() {
    let two_arm = run_analysis(&sheet(ArmC::Absent), &km_spec()).unwrap();
    let reduced = run_analysis(&sheet(ArmC::AllExcluded), &km_spec()).unwrap();
    assert_two_arm_equivalent(&reduced, &two_arm, "all excluded");

    // Independent path: the published-formula hazard ratios on the raw observations.
    let hr = two_cohort_hazard_ratio(&obs(&ARM_A), &obs(&ARM_B), ConfidenceLevel::new(95.0).unwrap()).unwrap();
    assert!((scalar(&reduced, "Hazard ratio (Mantel-Haenszel)") - hr.mantel_haenszel_hr).abs() <= TOL);
    assert!((scalar(&reduced, "Hazard ratio (log-rank)") - hr.log_rank_hr).abs() <= TOL);

    let note = reduced.checklist.iter().find(|c| c.assumption == "Groups without data").expect("omission disclosed");
    assert_eq!(note.status, ChecklistStatus::Info);
    assert!(note.explanation.contains("Arm C"), "names the omitted group: {}", note.explanation);
    assert!(reduced.tabs.first().unwrap().footnotes.iter().any(|f| f.contains("Arm C")), "footnote names the omitted group");
    assert!(two_arm.checklist.iter().all(|c| c.assumption != "Groups without data"), "no note when nothing is omitted");
}

#[test]
fn blank_arm_and_spacer_rows_reduce_to_two_arm_comparison() {
    let two_arm = run_analysis(&sheet(ArmC::Absent), &km_spec()).unwrap();
    let reduced = run_analysis(&sheet(ArmC::Blank), &km_spec()).unwrap();
    assert_two_arm_equivalent(&reduced, &two_arm, "blank arm");
    // Blank spacer rows must not change the number at risk.
    let nar = |rs: &ResultSheet| rs.tabs.iter().find(|t| t.id == "number-at-risk").unwrap().rows.clone();
    assert_eq!(nar(&reduced), nar(&two_arm));
}

#[test]
fn three_arm_table_still_runs_three_arm_comparison() {
    let rs = run_analysis(&sheet(ArmC::Present), &km_spec()).unwrap();
    assert!(rs.tabs.iter().any(|t| t.id == "survival-pairwise"), "pairwise tab for 3 arms");
    assert_eq!(rs.fitted_curves.len(), 3);
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
    let rs = run_analysis(&s, &km_spec()).unwrap();
    assert_eq!(rs.fitted_curves.len(), 1);
    assert!(rs.tabs.iter().all(|t| t.id != "curve-comparison"));
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
    assert!(matches!(run_analysis(&s, &km_spec()), Err(StatsError::EmptyInput)));
}

#[test]
fn two_arm_emits_hazard_ratio_tab_with_95ci() {
    let rs = run_analysis(&sheet(ArmC::Absent), &km_spec()).unwrap();
    let hr = two_cohort_hazard_ratio(&obs(&ARM_A), &obs(&ARM_B), ConfidenceLevel::new(95.0).unwrap()).unwrap();
    let hr_tab = rs.tabs.iter().find(|t| t.id == "hazard-ratio").expect("two-arm KM must emit hazard-ratio tab");
    assert_eq!(hr_tab.kind, ResultTabKind::ParametersAndConfidenceIntervals);
    assert_eq!(hr_tab.column_headers, vec!["Hazard ratio".to_owned(), "95% CI".to_owned()]);
    assert_eq!(hr_tab.row_labels, vec!["Log-rank".to_owned(), "Mantel-Haenszel".to_owned(), "Peto one-step".to_owned()]);
    assert_eq!(hr_tab.rows.len(), 3);

    let check_row = |row_idx: usize, expected_hr: f64, expected_ci: (f64, f64)| {
        let ResultCell::Number { value, .. } = hr_tab.rows[row_idx][0] else {
            panic!("expected finite HR number in row {row_idx}: {:?}", hr_tab.rows[row_idx][0]);
        };
        assert!((value - expected_hr).abs() <= TOL, "row {row_idx} HR mismatch: {value} vs {expected_hr}");
        let ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } = hr_tab.rows[row_idx][1] else {
            panic!("expected finite HR 95% CI in row {row_idx}: {:?}", hr_tab.rows[row_idx][1]);
        };
        assert!((lo - expected_ci.0).abs() <= TOL && (hi - expected_ci.1).abs() <= TOL, "row {row_idx} CI mismatch");
    };

    check_row(0, hr.log_rank_hr, hr.log_rank_ci.unwrap());
    check_row(1, hr.mantel_haenszel_hr, hr.mantel_haenszel_ci.unwrap());
    check_row(2, hr.peto_one_step_hr, hr.peto_one_step_ci.unwrap());
}

#[test]
fn hundred_percent_censored_arm_produces_flat_survival_and_valid_nar() {
    // Arm 0 has events; Arm 1 is 100% censored across follow-up times with blank spacer rows.
    let mut t = SurvivalTable::new(2, 10).unwrap();
    t.cohorts[0].title = "Treated".into();
    t.cohorts[1].title = "All-Censored".into();

    let arm_events = [(1.0, 1.0), (4.0, 1.0), (8.0, 0.0), (10.0, 1.0)];
    let mut r = 0;
    for &(time, st) in &arm_events {
        t.times[r] = Cell::number(time);
        t.cohorts[0].cells[r][0] = Cell::number(st);
        r += 1;
    }
    // Two blank spacer rows between cohorts.
    r += 2;
    for &time in &[2.0, 5.0, 9.0, 14.0] {
        t.times[r] = Cell::number(time);
        t.cohorts[1].cells[r][0] = Cell::number(0.0);
        r += 1;
    }

    let ds = DataSheet::new(SheetId::new("km-censored"), "All-censored arm", TableArchetype::Survival(t)).unwrap();
    let mut spec = km_spec();
    spec.source_sheet_id = SheetId::new("km-censored");
    let rs = run_analysis(&ds, &spec).unwrap();

    let summary = rs.tabs.iter().find(|t| t.id == "km-summary").unwrap();
    assert_eq!(summary.row_labels, vec!["Treated".to_owned(), "All-Censored".to_owned()]);
    assert!(matches!(summary.rows[1][1], ResultCell::Number { value, .. } if value == 0.0), "expected 0 events in all-censored arm");
    assert!(matches!(summary.rows[1][2], ResultCell::Number { value, .. } if value == 4.0), "expected 4 censored observations");
    assert!(matches!(summary.rows[1][3], ResultCell::Empty), "all-censored arm has undefined median");

    let cens_curve = rs.fitted_curves.iter().find(|c| c.data_set_index == 1).expect("curve for 100%-censored arm");
    assert!(cens_curve.points.iter().all(|p| p.y.is_finite() && (p.y - 1.0).abs() <= TOL));
    assert!((cens_curve.points.last().unwrap().x - 14.0).abs() <= TOL);

    let nar = rs.tabs.iter().find(|t| t.id == "number-at-risk").unwrap();
    let cens_nar: Vec<_> = nar.rows.iter().filter(|row| matches!(&row[0], ResultCell::Text(s) if s == "All-Censored")).collect();
    assert_eq!(cens_nar.len(), 4);
    assert!(matches!(cens_nar[0][2], ResultCell::Number { value, .. } if (value - 4.0).abs() <= TOL));
    assert!(matches!(cens_nar[3][2], ResultCell::Number { value, .. } if (value - 1.0).abs() <= TOL));
}
