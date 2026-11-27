//! Wall-clock performance benchmarks for large post-test comparison families in `curvebook-stats`.
//! Selected by `PERFORMANCE_FILTER` (`binary(*perf*)`), kept out of the general `core_suite` so
//! timing noise never flakes parallel correctness gates.
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, FactorialAnovaSpec, GroupedTable, MultiplicityMethod, MultiplicitySpec, ResultStatus, SheetId,
    SubcolumnMode, TableArchetype,
};
use curvebook_stats::run_analysis;

fn build_48x2_grouped_sheet() -> DataSheet {
    let n_rows = 48_usize;
    let n_cols = 2_usize;
    let n_reps = 3_usize;
    let mut tbl = GroupedTable::new(SubcolumnMode::RawReplicates(n_reps), n_cols, n_rows).unwrap();
    tbl.row_factor_title = String::from("Treatment");
    tbl.column_factor_title = String::from("Condition");
    for c in 0..n_cols {
        tbl.data_sets[c].title = format!("Slice {}", c + 1);
    }
    for r in 0..n_rows {
        tbl.row_titles[r] = format!("Level {:02}", r + 1);
        for c in 0..n_cols {
            let base = 10.0 + 0.35 * (r as f64) + 1.8 * (c as f64) + 0.12 * ((r % 7) as f64) * (c as f64);
            for (s, jit) in [-0.45, 0.05, 0.40].into_iter().enumerate() {
                tbl.data_sets[c].cells[r][s] = Cell::number(base + jit + 0.03 * ((r + s + c) % 5) as f64);
            }
        }
    }
    DataSheet::new(SheetId::new("anova_48x2"), "Two-Way 48x2 Post-Test Perf", TableArchetype::Grouped(tbl)).unwrap()
}

#[test]
fn posttest_two_way_48_levels_by_2_slices_tukey_perf() {
    let sheet = build_48x2_grouped_sheet();
    let spec = AnalysisSpec::new(
        SheetId::new("res_48x2"),
        sheet.id.clone(),
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..FactorialAnovaSpec::default()
        }),
    )
    .unwrap();

    let t0 = Instant::now();
    let res = run_analysis(&sheet, &spec).unwrap();
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1_000.0;

    // Also time the pure math batch of 2,256 pairs at (r = 2, c = 48, df = 192.0) directly.
    let qs: Vec<f64> = (0..2_256).map(|i| 0.25 + 14.0 * (i as f64) / 2_255.0).collect();
    let t1 = Instant::now();
    let ps = curvebook_math::dist::max_studentized_range_sf_batch(&qs, 2, 48, 192.0).unwrap();
    let batch_ms = t1.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(ps.len(), qs.len());

    // Measure 5 scalar calls for apples-to-apples per-pair comparison.
    let t2 = Instant::now();
    for &q in &[1.5, 4.0, 6.0, 8.5, 11.0] {
        let _ = curvebook_math::dist::max_studentized_range_sf(q, 2, 48, 192.0).unwrap();
    }
    let scalar5_ms = t2.elapsed().as_secs_f64() * 1_000.0;

    eprintln!(
        "Two-way ANOVA 48 levels x 2 slices (Tukey HSD, {} pairwise rows): {elapsed_ms:.2} ms | batch 2256 q (r=2,c=48,df=192): {batch_ms:.2} ms | 5 scalar calls: {scalar5_ms:.2} ms ({:.2} ms/pair)",
        res.pairwise_comparisons.len(),
        scalar5_ms / 5.0
    );

    assert_eq!(res.status, ResultStatus::UpToDate);
    // 1128 (marginal A) + 1 (marginal B) + 2256 (simple A within B) + 48 (simple B within A) = 3433.
    assert_eq!(res.pairwise_comparisons.len(), 3_433);
    if !cfg!(debug_assertions) {
        assert!(elapsed_ms < 5_000.0, "48x2 two-way Tukey ANOVA regressed: took {elapsed_ms:.2} ms (budget < 5000 ms)");
        assert!(batch_ms < 500.0, "batched 2256-pair max_studentized_range_sf_batch regressed: took {batch_ms:.2} ms (budget < 500 ms)");
    }
}
