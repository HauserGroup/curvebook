//! Typed edge-case contract for the standalone Levene / Brown–Forsythe analysis (`R19.2`,
//! `curvebook_stats::variance_homogeneity`): every degenerate input returns a specific
//! [`StatsError`] variant and nothing panics. Numerical `D1` oracles live in
//! `d1_extended_stats_oracles.rs` (stats level) and `crates/engine/tests/r19_2_levene_engine.rs`.

use curvebook_doc::{
    AnalysisKind, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, LeveneCenter, LeveneTestSpec, SubcolumnMode, TableArchetype, TableKind,
};
use curvebook_stats::StatsError;
use curvebook_stats::two_group::levene_variance_test;
use curvebook_stats::variance_homogeneity::levene_test_groups;

use crate::common::{col_sheet, run_kind_sel};

fn groups<'a>(gs: &'a [&'a [f64]]) -> Vec<(usize, &'a str, &'a [f64])> {
    gs.iter().enumerate().map(|(i, g)| (i, "g", *g)).collect()
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
            assert!(matches!(levene_test_groups(&groups(&[&[1.0, 2.0, 3.0], &[4.0, bad, 6.0]]), center), Err(StatsError::Math(_))));
        }
        for zero_spread in
            [&[&[5.0, 5.0, 5.0][..], &[7.0, 7.0]][..], &[&[1.0, 3.0], &[2.0, 8.0], &[4.0, 4.5]], &[&[0.1, 0.3], &[1e6 + 0.1, 1e6 + 0.3]]]
        {
            assert!(matches!(levene_test_groups(&groups(zero_spread), center), Err(StatsError::ZeroVariance { .. })));
        }
    }
}

#[test]
fn levene_slice_api_matches_legacy_routine_on_regular_data() {
    let a = [23.1, 25.4, 21.8, 24.0, 26.2, 22.7];
    let b = [18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6];
    for (center, use_median) in [(LeveneCenter::Median, true), (LeveneCenter::Mean, false)] {
        let new = levene_test_groups(&groups(&[&a, &b]), center).unwrap();
        let old = levene_variance_test(&[&a, &b], use_median).unwrap();
        assert_eq!((new.test, new.groups.len(), new.groups[1].n), (old, 2, 7));
    }
    let legacy = levene_variance_test(&[&[5.0, 5.0, 5.0], &[7.0, 7.0]], true).unwrap();
    assert_eq!((legacy.f_statistic, legacy.p_value), (0.0, 1.0));
}

#[test]
fn levene_sheet_api_rejects_incompatible_and_invalid_inputs() {
    let kind = AnalysisKind::LeveneTest(LeveneTestSpec::default());
    let het = col_sheet("src", &[("A", &[1.0, 2.0, 4.0]), ("B", &[2.0, 5.0, 9.0, 1.0]), ("C", &[3.0, 3.5, 2.5])]);
    assert!(run_kind_sel(&het, kind.clone(), Vec::new()).is_ok());
    assert!(matches!(run_kind_sel(&het, kind.clone(), vec![1]), Err(StatsError::InsufficientSampleSize { actual: 1, .. })));
    assert!(matches!(run_kind_sel(&het, kind.clone(), vec![0, 7]), Err(StatsError::MissingColumn { index: 7 })));
    assert!(matches!(run_kind_sel(&het, kind.clone(), vec![0, 0]), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let grouped = DataSheet::new("src", "src", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap())).unwrap();
    assert!(matches!(run_kind_sel(&grouped, kind.clone(), Vec::new()), Err(StatsError::IncompatibleTable { actual: TableKind::Grouped, .. })));

    let summary_sheet =
        DataSheet::new("src", "src", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 2).unwrap())).unwrap();
    assert!(matches!(run_kind_sel(&summary_sheet, kind, Vec::new()), Err(StatsError::RequiresRawReplicates)));
}
