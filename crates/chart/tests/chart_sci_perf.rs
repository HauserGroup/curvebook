//! Wall-clock budgets for scientific chart compiles (`GAP-02-07`, `GAP-02-13`).
//!
//! Timing assertions live in this `*perf*` binary so `PERFORMANCE_FILTER` (`xtask/src/ci.rs`) runs
//! them in the performance stage only; the functional assertions for the same fixtures stay in
//! `tests/chart_suite/gap_chart_sci_fixes.rs`.
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    CartesianOrientation, Cell, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, GraphicalDataObject, GroupedArrangement, GroupedTable,
    HeatmapLabelSpec, MultipleVariablesTable, ProjectDocument, SheetId, SubcolumnMode, TableArchetype,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Generous wall-clock ceiling for a single compile; the quadratic regressions these guard took minutes.
const COMPILE_BUDGET_SECS: f64 = 5.0;

#[test]
fn gap_02_07_perf_superimposed_layout_compiles_within_budget() -> TestResult {
    let nrows = 2_500;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, nrows)?;
    for r in 0..nrows {
        grp.data_sets[0].cells[r][0] = Cell::number(((r % 50) + 10) as f64);
        grp.data_sets[1].cells[r][0] = Cell::number(((r % 30) + 5) as f64);
    }
    let ds = DataSheet::new(SheetId::new("sup-big"), "SupBig", TableArchetype::Grouped(grp))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    for orient in [CartesianOrientation::Vertical, CartesianOrientation::Horizontal] {
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-sup"), &ds)?;
        gs.spec.kind = GraphKind::Grouped;
        gs.spec.grouped_arrangement = GroupedArrangement::Superimposed;
        gs.spec.orientation = orient;
        for s in &mut gs.spec.series {
            s.y_error_bars = None;
            if let GraphicalDataObject::BarWithReplicates { replicate_overlay, .. } = &mut s.gdo {
                *replicate_overlay = None;
            }
        }
        let t0 = Instant::now();
        let _scene = compile_graph(&gs, &doc)?;
        let elapsed = t0.elapsed();
        assert!(elapsed.as_secs_f64() < COMPILE_BUDGET_SECS, "5000-slot {orient:?} superimposed compile took {elapsed:?}");
    }
    Ok(())
}

#[test]
fn gap_02_13_perf_capped_heatmap_compiles_within_budget() -> TestResult {
    let nrows = 4_000;
    let ncols = 10;
    let mut mv = MultipleVariablesTable::new(ncols, nrows)?;
    for c in 0..ncols {
        mv.variables[c].title = format!("V{c}");
        for r in 0..nrows {
            mv.variables[c].cells[r] = Cell::number(((r + c) % 100) as f64);
        }
    }
    let ds = DataSheet::new(SheetId::new("hm-huge"), "HugeHM", TableArchetype::MultipleVariables(mv))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm-huge"), &ds)?;
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    gs.spec.heatmap_labels = HeatmapLabelSpec { annotate_cells: true, ..HeatmapLabelSpec::default() };

    let t0 = Instant::now();
    let _scene = compile_graph(&gs, &doc)?;
    let elapsed = t0.elapsed();
    assert!(elapsed.as_secs_f64() < COMPILE_BUDGET_SECS, "capped heatmap took too long: {elapsed:?}");
    Ok(())
}
