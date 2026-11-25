//! Typed edge-case contract for the standalone Levene / Brown–Forsythe analysis (`R19.2`,
//! `curvebook_stats::variance_homogeneity`): every degenerate input returns a specific
//! [`StatsError`] variant and nothing panics. Numerical `D1` oracles live in
//! `d1_extended_stats_oracles.rs` (stats level) and `crates/engine/tests/r19_2_levene_engine.rs`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, LeveneCenter, LeveneTestSpec, SubcolumnMode,
    TableArchetype, TableKind,
};
use curvebook_stats::StatsError;
use curvebook_stats::run_analysis;
use curvebook_stats::two_group::levene_variance_test;
use curvebook_stats::variance_homogeneity::levene_test_groups;

fn groups<'a>(gs: &'a [&'a [f64]]) -> Vec<(usize, &'a str, &'a [f64])> {
    gs.iter().enumerate().map(|(i, g)| (i, "g", *g)).collect()
}

fn column_sheet(gs: &[&[f64]]) -> DataSheet {
    let rows = gs.iter().map(|g| g.len()).max().unwrap_or(1).max(1);
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, gs.len(), rows).unwrap();
    for (ds, values) in t.data_sets.iter_mut().zip(gs.iter()) {
        for (r, &v) in values.iter().enumerate() {
            ds.cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new("src", "src", TableArchetype::Column(t)).unwrap()
}

fn spec(selected: Vec<usize>) -> AnalysisSpec {
    let mut s = AnalysisSpec::new("res", "src", AnalysisKind::LeveneTest(LeveneTestSpec::default())).unwrap();
    s.selected_data_sets = selected;
    s
}

#[test]
fn levene_slice_api_rejects_degenerate_inputs_with_typed_errors() {
    for center in LeveneCenter::ALL {
        assert!(matches!(
            levene_test_groups(&groups(&[&[1.0, 2.0, 3.0]]), center),
            Err(StatsError::InsufficientSampleSize { context: "Levene test", required: 2, actual: 1 })
        ));
        assert!(matches!(levene_test_groups(&[], center), Err(StatsError::InsufficientSampleSize { actual: 0, .. })));
        assert!(matches!(
            levene_test_groups(&groups(&[&[1.0, 2.0, 3.0], &[4.0]]), center),
            Err(StatsError::InsufficientSampleSize { context: "Levene group", required: 2, actual: 1 })
        ));
        assert!(matches!(levene_test_groups(&groups(&[&[1.0, 2.0, 3.0], &[]]), center), Err(StatsError::EmptyInput)));
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = levene_test_groups(&groups(&[&[1.0, 2.0, 3.0], &[4.0, bad, 6.0]]), center).unwrap_err();
            assert!(matches!(err, StatsError::Math(_)), "non-finite -> {err:?}");
        }
        // Zero spread of the absolute deviations: constant groups, and all-n = 2 groups.
        assert!(matches!(levene_test_groups(&groups(&[&[5.0, 5.0, 5.0], &[7.0, 7.0]]), center), Err(StatsError::ZeroVariance { .. })));
        assert!(matches!(levene_test_groups(&groups(&[&[1.0, 3.0], &[2.0, 8.0], &[4.0, 4.5]]), center), Err(StatsError::ZeroVariance { .. })));
        // Rounding-level spread (|x - c| equal up to floating-point noise) is treated as zero spread too.
        assert!(matches!(levene_test_groups(&groups(&[&[0.1, 0.3], &[1e6 + 0.1, 1e6 + 0.3]]), center), Err(StatsError::ZeroVariance { .. })));
    }
}

#[test]
fn levene_slice_api_matches_legacy_routine_on_regular_data() {
    let a = [23.1, 25.4, 21.8, 24.0, 26.2, 22.7];
    let b = [18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6];
    for (center, use_median) in [(LeveneCenter::Median, true), (LeveneCenter::Mean, false)] {
        let new = levene_test_groups(&groups(&[&a, &b]), center).unwrap();
        let old = levene_variance_test(&[&a, &b], use_median).unwrap();
        assert_eq!(new.test, old, "refactored core must reproduce levene_variance_test bit-for-bit");
        assert_eq!(new.groups.len(), 2);
        assert_eq!(new.groups[1].n, 7);
    }
    // The legacy routine keeps its historical F = 0 / p = 1 convention on constant groups.
    let legacy = levene_variance_test(&[&[5.0, 5.0, 5.0], &[7.0, 7.0]], true).unwrap();
    assert_eq!((legacy.f_statistic, legacy.p_value), (0.0, 1.0));
}

#[test]
fn levene_sheet_api_rejects_incompatible_and_invalid_inputs() {
    let het = column_sheet(&[&[1.0, 2.0, 4.0], &[2.0, 5.0, 9.0, 1.0], &[3.0, 3.5, 2.5]]);
    assert!(run_analysis(&het, &spec(Vec::new())).is_ok());
    assert!(matches!(run_analysis(&het, &spec(vec![1])), Err(StatsError::InsufficientSampleSize { actual: 1, .. })));
    assert!(matches!(run_analysis(&het, &spec(vec![0, 7])), Err(StatsError::MissingColumn { index: 7 })));
    assert!(matches!(run_analysis(&het, &spec(vec![0, 0])), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let grouped = DataSheet::new("src", "src", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap())).unwrap();
    assert!(matches!(run_analysis(&grouped, &spec(Vec::new())), Err(StatsError::IncompatibleTable { actual: TableKind::Grouped, .. })));

    let summary_tbl = ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 2).unwrap();
    let sheet = DataSheet::new("src", "src", TableArchetype::Column(summary_tbl)).unwrap();
    assert!(matches!(run_analysis(&sheet, &spec(Vec::new())), Err(StatsError::RequiresRawReplicates)));
}
