//! Regression tests for 2-subcolumn `SubcolumnMode::MeanSd` and `SubcolumnMode::MeanSem`
//! across descriptive statistics (supported without fabricating N) and inferential / outlier
//! analyses requiring N (refused with typed errors) (schema v9 §3.5).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, GroupedTable, OneWayAnovaSpec, OutlierSpec,
    ResultStatus, SubcolumnMode, SummaryPointError, TableArchetype, TwoGroupSpec, XyTable,
};
use curvebook_stats::{StatsError, run_analysis};

#[test]
fn v9_mean_sd_and_mean_sem_support_descriptive_stats_and_refuse_analyses_needing_n() {
    for mode in [SubcolumnMode::MeanSd, SubcolumnMode::MeanSem] {
        // 1. Column table: descriptive stats succeeds without fabricating N; t-test, ANOVA, outliers refuse.
        let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 2, 1).unwrap();
        col.data_sets[0].title = String::from("Control");
        col.data_sets[1].title = String::from("Treated");
        col.data_sets[0].cells = vec![vec![Cell::number(12.0), Cell::number(1.5)]];
        col.data_sets[1].cells = vec![vec![Cell::number(18.0), Cell::number(2.0)]];
        let col_sheet = DataSheet::new("col-no-n", "Column Summary", TableArchetype::Column(col)).unwrap();

        let desc_spec = AnalysisSpec::new("desc-1", "col-no-n", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        let desc_res = run_analysis(&col_sheet, &desc_spec).unwrap();
        assert_eq!(desc_res.status, ResultStatus::UpToDate);
        assert_eq!(desc_res.lookup_scalar("Mean", Some(0)), Some(12.0));
        assert_eq!(desc_res.lookup_scalar("Mean", Some(1)), Some(18.0));
        let expected_spread_key = if mode == SubcolumnMode::MeanSd { "SD" } else { "SEM" };
        assert_eq!(desc_res.lookup_scalar(expected_spread_key, Some(0)), Some(1.5));
        assert_eq!(desc_res.lookup_scalar("N", Some(0)), None, "must never fabricate N for {mode:?}");

        let ttest_spec = AnalysisSpec::new("tt-1", "col-no-n", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
        assert!(run_analysis(&col_sheet, &ttest_spec).is_err(), "t-test must refuse {mode:?}");

        let anova_spec = AnalysisSpec::new("an-1", "col-no-n", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
        assert!(run_analysis(&col_sheet, &anova_spec).is_err(), "one-way ANOVA must refuse {mode:?}");

        let out_spec = AnalysisSpec::new("out-1", "col-no-n", AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })).unwrap();
        assert!(matches!(run_analysis(&col_sheet, &out_spec), Err(StatsError::RequiresRawReplicates)));

        // Negative spread refused with typed SummaryPointError::NegativeSpread.
        let mut bad_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 1, 1).unwrap();
        bad_col.data_sets[0].cells = vec![vec![Cell::number(12.0), Cell::number(-0.5)]];
        let bad_sheet = DataSheet::new("col-neg", "Neg Spread", TableArchetype::Column(bad_col)).unwrap();
        assert!(matches!(run_analysis(&bad_sheet, &desc_spec), Err(StatsError::SummaryPoint(SummaryPointError::NegativeSpread { .. }))));

        // 2. XY table with MeanSd/MeanSem: descriptive stats computes mean & trapezoidal AUC cleanly.
        let mut xy = XyTable::new(mode, 1, 3).unwrap();
        xy.x_cells = vec![vec![Cell::number(0.0)], vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
        xy.data_sets[0].cells =
            vec![vec![Cell::number(2.0), Cell::number(0.2)], vec![Cell::number(4.0), Cell::number(0.4)], vec![Cell::number(2.0), Cell::number(0.2)]];
        let xy_sheet = DataSheet::new("xy-no-n", "XY Summary", TableArchetype::Xy(xy)).unwrap();
        let xy_desc = AnalysisSpec::new("desc-xy", "xy-no-n", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        let xy_res = run_analysis(&xy_sheet, &xy_desc).unwrap();
        assert_eq!(xy_res.status, ResultStatus::UpToDate);
        assert!(xy_res.lookup_scalar("Trapezoidal AUC", Some(0)).is_some_and(|auc| (auc - 6.0).abs() < 1e-12));

        // 3. Grouped table with MeanSd/MeanSem: descriptive stats succeeds.
        let mut grp = GroupedTable::new(mode, 2, 2).unwrap();
        grp.data_sets[0].cells = vec![vec![Cell::number(5.0), Cell::number(0.5)], vec![Cell::number(7.0), Cell::number(0.7)]];
        grp.data_sets[1].cells = vec![vec![Cell::number(9.0), Cell::number(0.9)], vec![Cell::number(11.0), Cell::number(1.1)]];
        let grp_sheet = DataSheet::new("grp-no-n", "Grouped Summary", TableArchetype::Grouped(grp)).unwrap();
        let grp_desc = AnalysisSpec::new("desc-grp", "grp-no-n", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        assert_eq!(run_analysis(&grp_sheet, &grp_desc).unwrap().status, ResultStatus::UpToDate);
    }
}
