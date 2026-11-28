//! D1 reference oracles (R 4.5.2 `hist()`, `nclass.Sturges`, `nclass.scott`, `nclass.FD`, `ecdf`,
//! `dnorm`, `pnorm`) and degenerate edge-case coverage for standalone Frequency Distribution &
//! Histogram binning (`curvebook_stats::frequency`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, FrequencyBinningRule, FrequencyDistributionSpec,
    FrequencyOutputMode, GroupedTable, ResultStatus, SubcolumnAggregation, SubcolumnMode, TableArchetype,
};
use curvebook_stats::frequency::{
    MAX_HISTOGRAM_BINS, compute_frequency_distribution, compute_histogram, nclass_freedman_diaconis, nclass_scott, nclass_sturges,
};
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!((actual - expected).abs() <= tol, "{label}: actual={actual:.16}, expected={expected:.16}, diff={:.3e}", (actual - expected).abs());
}

#[test]
fn d1_r452_bin_counts_edges_densities_and_ecdf_oracles() {
    // Vector x1 validated against local R 4.5.2 (`nclass.Sturges`, `nclass.scott`, `nclass.FD`, `hist`, `ecdf`).
    let x1 = [1.2, 2.5, 2.8, 3.1, 3.9, 4.0, 4.4, 5.2, 6.1, 7.8, 8.3, 9.5, 10.2, 11.1, 12.0, 12.4, 13.7, 15.0, 16.2, 18.5];
    assert_eq!(nclass_sturges(x1.len()), 6);

    let spec_fd = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::AutoFreedmanDiaconis,
        output_mode: FrequencyOutputMode::Counts,
        ..FrequencyDistributionSpec::default()
    };
    let res_fd = compute_histogram(&x1, &spec_fd).expect("FD histogram");
    assert_eq!(res_fd.series.len(), 1);
    let s1 = &res_fd.series[0];
    assert_eq!(s1.bins.len(), 3);
    assert_close(s1.mean, 8.395, 1e-12, "x1 mean");
    assert_close(s1.sd.unwrap(), 5.092_871_695_346_66, 1e-12, "x1 sd");
    assert_close(s1.iqr, 8.125, 1e-12, "x1 IQR");
    assert_eq!(nclass_scott(x1.len(), s1.sd.unwrap(), 18.5 - 1.2), Some(3));
    assert_eq!(nclass_freedman_diaconis(x1.len(), s1.iqr, 18.5 - 1.2), Some(3));

    let expected_fd_edges = [1.2, 6.966_666_666_666_667, 12.733_333_333_333_333, 18.5];
    for (i, (&actual, &exp)) in res_fd.edges.iter().zip(expected_fd_edges.iter()).enumerate() {
        assert_close(actual, exp, 1e-12, &format!("x1 FD edge[{i}]"));
    }
    let expected_fd_counts = [9_usize, 7, 4];
    let expected_fd_density = [0.078_034_682_080_924_86, 0.060_693_641_618_497_12, 0.034_682_080_924_855_49];
    let expected_fd_ecdf = [0.45, 0.80, 1.0];
    for (j, bin) in s1.bins.iter().enumerate() {
        assert_eq!(bin.count, expected_fd_counts[j], "x1 FD count[{j}]");
        assert_close(bin.density, expected_fd_density[j], 1e-12, &format!("x1 FD density[{j}]"));
        assert_close(bin.cumulative_frequency, expected_fd_ecdf[j], 1e-12, &format!("x1 FD ecdf[{j}]"));
        assert_close(bin.percentage, 100.0 * bin.relative_frequency, 1e-12, &format!("x1 FD pct[{j}]"));
        assert_close(bin.cumulative_percentage, 100.0 * expected_fd_ecdf[j], 1e-12, &format!("x1 FD cumpct[{j}]"));
    }

    // Explicit bin width = 5.0 with first_bin_center = 2.5, last_bin_center = 17.5 -> edges [0, 5, 10, 15, 20].
    let spec_exp = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 5.0 },
        first_bin_center: Some(2.5),
        last_bin_center: Some(17.5),
        output_mode: FrequencyOutputMode::Density,
        ..FrequencyDistributionSpec::default()
    };
    let res_exp = compute_histogram(&x1, &spec_exp).expect("explicit width histogram");
    assert_eq!(res_exp.edges, vec![0.0, 5.0, 10.0, 15.0, 20.0]);
    let exp_counts = [7_usize, 5, 6, 2];
    let exp_dens = [0.07, 0.05, 0.06, 0.02];
    let exp_ecdf = [0.35, 0.60, 0.90, 1.0];
    for (j, bin) in res_exp.series[0].bins.iter().enumerate() {
        assert_eq!(bin.count, exp_counts[j]);
        assert_close(bin.density, exp_dens[j], 1e-12, &format!("x1 exp density[{j}]"));
        assert_close(bin.cumulative_frequency, exp_ecdf[j], 1e-12, &format!("x1 exp ecdf[{j}]"));
    }

    // Vector x2 validated against R 4.5.2 Scott, Sturges, dnorm(0), pnorm(0).
    let x2 = [-3.4, -2.1, -1.8, -1.5, -0.9, -0.4, -0.2, 0.0, 0.1, 0.3, 0.5, 0.7, 0.8, 1.1, 1.4, 1.9, 2.2, 2.8, 3.5, 4.9, 5.2, 6.0, 7.1, 8.4, 10.0];
    let res_scott = compute_histogram(
        &x2,
        &FrequencyDistributionSpec {
            binning_rule: FrequencyBinningRule::AutoScott,
            output_mode: FrequencyOutputMode::Density,
            overlay_gaussian: true,
            ..FrequencyDistributionSpec::default()
        },
    )
    .expect("x2 Scott histogram");
    assert_eq!(res_scott.series[0].bins.len(), 4);
    let scott_counts = [7_usize, 11, 4, 3];
    let scott_dens = [0.083_582_089_552_238_81, 0.131_343_283_582_089_55, 0.047_761_194_029_850_75, 0.035_820_895_522_388_07];
    let scott_ecdf = [0.28, 0.72, 0.88, 1.0];
    for (j, bin) in res_scott.series[0].bins.iter().enumerate() {
        assert_eq!(bin.count, scott_counts[j]);
        assert_close(bin.density, scott_dens[j], 1e-12, &format!("x2 Scott density[{j}]"));
        assert_close(bin.cumulative_frequency, scott_ecdf[j], 1e-12, &format!("x2 Scott ecdf[{j}]"));
    }
    // Check Gaussian density overlay against R dnorm(0, mean(x2), sd(x2)) = 0.10104292189232744.
    let gauss = res_scott.series[0].gaussian_overlay.as_ref().expect("Gaussian overlay present");
    assert_eq!(gauss.len(), 201);
    let eval_at_zero = curvebook_stats::frequency::evaluate_gaussian_overlay(
        res_scott.series[0].mean,
        res_scott.series[0].sd.unwrap(),
        x2.len(),
        res_scott.bin_width,
        FrequencyOutputMode::Density,
        (0.0, 1.0),
        2,
    )
    .unwrap();
    assert_close(eval_at_zero[0].1, 0.101_042_921_892_327_44, 1e-12, "x2 dnorm(0)");

    let eval_cdf_zero = curvebook_stats::frequency::evaluate_gaussian_overlay(
        res_scott.series[0].mean,
        res_scott.series[0].sd.unwrap(),
        x2.len(),
        res_scott.bin_width,
        FrequencyOutputMode::CumulativeFrequency,
        (0.0, 1.0),
        2,
    )
    .unwrap();
    assert_close(eval_cdf_zero[0].1, 0.291_553_758_511_709_5, 1e-12, "x2 pnorm(0)");

    // Sturges on x2 -> 6 bins: counts [4, 9, 5, 3, 2, 2].
    let res_sturges = compute_histogram(
        &x2,
        &FrequencyDistributionSpec { binning_rule: FrequencyBinningRule::AutoSturges, ..FrequencyDistributionSpec::default() },
    )
    .expect("x2 Sturges histogram");
    let st_counts: Vec<usize> = res_sturges.series[0].bins.iter().map(|b| b.count).collect();
    assert_eq!(st_counts, vec![4, 9, 5, 3, 2, 2]);
    let st_dens = [
        0.071_641_791_044_776_11,
        0.161_194_029_850_746_27,
        0.089_552_238_805_970_14,
        0.053_731_343_283_582_1,
        0.035_820_895_522_388_05,
        0.035_820_895_522_388_07,
    ];
    for (j, bin) in res_sturges.series[0].bins.iter().enumerate() {
        assert_close(bin.density, st_dens[j], 1e-12, &format!("x2 Sturges density[{j}]"));
    }

    // Exact boundary ties: right-closed (e_j, e_{j+1}] with leftmost [e_0, e_1] matches R hist().
    let x3 = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
    let res_b = compute_histogram(
        &x3,
        &FrequencyDistributionSpec {
            binning_rule: FrequencyBinningRule::ExplicitWidth { width: 2.0 },
            first_bin_center: Some(1.0),
            last_bin_center: Some(9.0),
            ..FrequencyDistributionSpec::default()
        },
    )
    .expect("boundary ties histogram");
    let b_counts: Vec<usize> = res_b.series[0].bins.iter().map(|b| b.count).collect();
    assert_eq!(b_counts, vec![2, 1, 1, 1, 1]);
    let b_ecdf: Vec<f64> = res_b.series[0].bins.iter().map(|b| b.cumulative_frequency).collect();
    for (actual, exp) in b_ecdf.iter().zip([2.0 / 6.0, 3.0 / 6.0, 4.0 / 6.0, 5.0 / 6.0, 1.0]) {
        assert_close(*actual, exp, 1e-12, "boundary ecdf");
    }
}

#[test]
fn degenerate_edge_cases_never_panic_and_return_typed_results_or_errors() {
    let default_spec = FrequencyDistributionSpec::default();

    // Empty input -> EmptyInput.
    assert!(matches!(compute_histogram(&[], &default_spec), Err(StatsError::EmptyInput)));

    // NaN / +Inf / -Inf -> Math(DomainError).
    assert!(matches!(compute_histogram(&[1.0, f64::NAN, 2.0], &default_spec), Err(StatsError::Math(_))));
    assert!(matches!(compute_histogram(&[1.0, f64::INFINITY, 2.0], &default_spec), Err(StatsError::Math(_))));
    assert!(matches!(compute_histogram(&[f64::NEG_INFINITY, 1.0], &default_spec), Err(StatsError::Math(_))));

    // Single point -> single unit-width bin centered on value.
    let single = compute_histogram(&[42.0], &default_spec).expect("single point");
    assert_eq!(single.edges, vec![41.5, 42.5]);
    assert_eq!(single.series[0].bins[0].count, 1);
    assert_eq!(single.series[0].sd, None);
    assert!(single.effective_rule_note.is_some());

    // Constant data -> single unit-width bin with count = n and Gaussian overlay skipped safely.
    let constant =
        compute_histogram(&[7.0, 7.0, 7.0, 7.0, 7.0], &FrequencyDistributionSpec { overlay_gaussian: true, ..FrequencyDistributionSpec::default() })
            .expect("constant data");
    assert_eq!(constant.edges, vec![6.5, 7.5]);
    assert_eq!(constant.series[0].bins[0].count, 5);
    assert_eq!(constant.series[0].gaussian_overlay, None);

    // Zero IQR with positive range under FD -> falls back to Scott cleanly.
    let zero_iqr = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0];
    let z_res = compute_histogram(&zero_iqr, &default_spec).expect("zero IQR fallback");
    assert!(z_res.effective_rule_note.as_deref().is_some_and(|n| n.contains("Interquartile range is zero")));
    assert_eq!(z_res.series[0].bins.iter().map(|b| b.count).sum::<usize>(), zero_iqr.len());

    // Invalid explicit bin widths and bin-count ceiling (> MAX_HISTOGRAM_BINS).
    for bad_w in [0.0, -2.0, f64::NAN, f64::INFINITY, 1e-6] {
        let spec =
            FrequencyDistributionSpec { binning_rule: FrequencyBinningRule::ExplicitWidth { width: bad_w }, ..FrequencyDistributionSpec::default() };
        let err = compute_histogram(&[0.0, 100.0], &spec);
        assert!(matches!(err, Err(StatsError::InvalidParameter { .. })), "expected InvalidParameter for width={bad_w}, got {err:?}");
    }
    assert_eq!(MAX_HISTOGRAM_BINS, 10_000);

    // first_bin_center > last_bin_center -> InvalidParameter.
    let bad_bounds = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 1.0 },
        first_bin_center: Some(10.0),
        last_bin_center: Some(5.0),
        ..FrequencyDistributionSpec::default()
    };
    assert!(matches!(compute_histogram(&[1.0, 2.0, 3.0], &bad_bounds), Err(StatsError::InvalidParameter { .. })));

    // Underflow and overflow accounting when explicit range is narrower than data.
    let narrow = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 2.0 },
        first_bin_center: Some(3.0),
        last_bin_center: Some(5.0),
        ..FrequencyDistributionSpec::default()
    };
    let narrow_res = compute_histogram(&[0.0, 1.0, 2.5, 3.5, 5.5, 9.0, 10.0], &narrow).expect("narrow bins");
    // Bins are [2, 4] and (4, 6]. Below 2: [0, 1] (2); in [2,4]: [2.5, 3.5] (2); in (4,6]: [5.5] (1); above 6: [9, 10] (2).
    assert_eq!(narrow_res.series[0].underflow_count, 2);
    assert_eq!(narrow_res.series[0].overflow_count, 2);
    assert_eq!(narrow_res.series[0].bins[0].count, 2);
    assert_eq!(narrow_res.series[0].bins[1].count, 1);
    assert_eq!(narrow_res.series[0].bins[0].cumulative_count, 4);
    assert_eq!(narrow_res.series[0].bins[1].cumulative_count, 5);
}

#[test]
fn result_sheet_pipeline_handles_multi_series_subcolumn_aggregation_and_exclusions() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap();
    grp.data_sets[0].title = String::from("Control");
    grp.data_sets[1].title = String::from("Treated");
    let mut sheet = DataSheet::new("grp-1", "Dose Assay", TableArchetype::Grouped(grp)).unwrap();
    let ctrl = [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0]];
    let trt = [[2.0, 3.0], [4.0, 5.0], [6.0, 7.0], [8.0, 9.0]];
    for r in 0..4 {
        for sc in 0..2 {
            sheet.set_cell(CellCoord::new(r, 0, sc), Cell::number(ctrl[r][sc])).unwrap();
            sheet.set_cell(CellCoord::new(r, 1, sc), Cell::number(trt[r][sc])).unwrap();
        }
    }
    sheet.set_excluded(CellCoord::new(0, 0, 0), true).unwrap();

    // PooledGrandSample pools all active replicate cells (Control has 7, Treated has 8).
    let fd_pooled = FrequencyDistributionSpec {
        binning_rule: FrequencyBinningRule::ExplicitWidth { width: 2.0 },
        first_bin_center: Some(2.0),
        last_bin_center: Some(8.0),
        output_mode: FrequencyOutputMode::Percentages,
        overlay_gaussian: true,
        aggregation: SubcolumnAggregation::PooledGrandSample,
        create_histogram_graph: true,
    };
    let spec = AnalysisSpec::new("res-freq", "grp-1", AnalysisKind::FrequencyDistribution(fd_pooled)).unwrap();
    let res = run_analysis(&sheet, &spec).expect("run frequency distribution");
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert!(res.validate().is_ok());
    assert_eq!(res.tabs.len(), 3);
    assert_eq!(res.fitted_curves.len(), 2);
    assert!(res.analyzable_data.is_some());
    let scalar_of = |r: &curvebook_doc::ResultSheet, key: &str, dsi: Option<usize>| -> Option<f64> {
        r.scalars.iter().find(|s| s.key == key && s.data_set_index == dsi).map(|s| s.value)
    };
    assert_eq!(scalar_of(&res, "Bin count", None), Some(4.0));
    assert_eq!(scalar_of(&res, "N", Some(0)), Some(7.0));
    assert_eq!(scalar_of(&res, "N", Some(1)), Some(8.0));

    // RowMeans collapses each row to 1 mean -> N = 4 per series.
    let fd_means = FrequencyDistributionSpec {
        aggregation: SubcolumnAggregation::RowMeans,
        output_mode: FrequencyOutputMode::CumulativeFrequency,
        overlay_gaussian: true,
        ..FrequencyDistributionSpec::default()
    };
    let spec_means = AnalysisSpec::new("res-freq-m", "grp-1", AnalysisKind::FrequencyDistribution(fd_means)).unwrap();
    let res_means = run_analysis(&sheet, &spec_means).expect("run row means");
    assert_eq!(scalar_of(&res_means, "N", Some(0)), Some(4.0));
    assert_eq!(scalar_of(&res_means, "N", Some(1)), Some(4.0));
    assert_eq!(res_means.fitted_curves[0].equation_id, "histogram_gaussian_cdf");

    // Pre-summarized Column table must be refused with RequiresRawReplicates.
    let sum_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 1, 2).unwrap();
    let sum_sheet = DataSheet::new("sum-1", "Summarized", TableArchetype::Column(sum_col)).unwrap();
    let spec_sum = AnalysisSpec::new("res-sum", "sum-1", AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec::default())).unwrap();
    assert!(matches!(run_analysis(&sum_sheet, &spec_sum), Err(StatsError::RequiresRawReplicates)));

    // Multi-series shared edges check via compute_frequency_distribution.
    let multi =
        compute_frequency_distribution(&[(0, "A", &[1.0, 2.0, 3.0], 0), (1, "B", &[5.0, 6.0, 7.0], 0)], &FrequencyDistributionSpec::default())
            .unwrap();
    assert_eq!(multi.series.len(), 2);
    assert_eq!(multi.series[0].bins.len(), multi.series[1].bins.len());
    assert_close(multi.edges[0], 1.0, 1e-12, "multi min edge");
    assert_close(*multi.edges.last().unwrap(), 7.0, 1e-12, "multi max edge");
}

#[test]
fn explicit_width_half_open_bounds_avoid_ulp_underflow_and_overflow() {
    // 3 * 0.3 = 0.8999999999999999 < 0.9 and 0.9 - 3 * 0.3 = 1.11e-16 > 0.0 in IEEE-754 f64.
    // Every unbounded-end combination must cover [0.0, 0.9] cleanly with zero underflow/overflow.
    let values = [0.0, 0.3, 0.6, 0.9];
    for (label, first, last) in [
        ("lower bounded, upper unbounded", Some(0.15), None),
        ("lower unbounded, upper bounded", None, Some(0.75)),
        ("both ends unbounded", None, None),
        ("both ends bounded exact", Some(0.15), Some(0.75)),
    ] {
        let spec = FrequencyDistributionSpec {
            binning_rule: FrequencyBinningRule::ExplicitWidth { width: 0.3 },
            first_bin_center: first,
            last_bin_center: last,
            ..FrequencyDistributionSpec::default()
        };
        let res = compute_histogram(&values, &spec).unwrap_or_else(|e| panic!("{label}: {e}"));
        let s = &res.series[0];
        assert_eq!(s.underflow_count, 0, "{label}: unexpected underflow");
        assert_eq!(s.overflow_count, 0, "{label}: unexpected overflow");
        assert_eq!(s.bins.len(), 3, "{label}: expected 3 bins, got edges={:?}", res.edges);
        assert_eq!(res.edges[0], 0.0, "{label}: first edge must be 0.0");
        assert_eq!(res.edges[3], 0.9, "{label}: last edge must be 0.9");
        let counts: Vec<usize> = s.bins.iter().map(|b| b.count).collect();
        assert_eq!(counts, vec![2, 1, 1], "{label}: expected counts [2, 1, 1]");
    }
}
