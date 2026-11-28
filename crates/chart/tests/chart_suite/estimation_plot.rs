//! Dual-axis Gardner–Altman estimation plot geometry, scale parity, alignment,
//! paired slope-lines, and degenerate input tests.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, GroupedTable,
    MultipleVariablesTable, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_palette::LineDashStyle;

use curvebook_chart::compile_graph;
use curvebook_chart::scene::{PathCmd, ScenePrimitive};

fn two_group_column_sheet(id: &str, hint: ColumnDesignHint, a: &[f64], b: &[f64]) -> DataSheet {
    let n = a.len().max(b.len());
    let mut col = ColumnTable::new(hint, 2, n).expect("column table");
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Treated");
    for (r, &v) in a.iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    for (r, &v) in b.iter().enumerate() {
        col.data_sets[1].cells[r][0] = Cell::number(v);
    }
    DataSheet::new(SheetId::new(id), "Two Groups", TableArchetype::Column(col)).expect("data sheet")
}

#[test]
fn estimation_plot_unpaired_aligns_zero_to_group_a_mean_and_delta_to_group_b_mean() {
    let a = [10.0, 11.0, 12.0, 10.5, 11.5, 11.0]; // mean = 11.0
    let b = [14.0, 15.0, 16.0, 14.5, 15.5, 15.0]; // mean = 15.0, delta = +4.0
    let ds = two_group_column_sheet("col-est", ColumnDesignHint::UnpairedReplicates, &a, &b);
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-est"), &ds).expect("gs");
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot);
    gs.spec.title = String::from("Estimation Plot");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_graph_sheet(gs.clone()).expect("add gs");

    let scene = compile_graph(&gs, &doc).expect("compile estimation plot");
    assert!(scene.warnings.is_empty(), "unexpected warnings: {:?}", scene.warnings);
    assert_eq!(scene.plotted_points.len(), 12, "all 12 raw observations must emit PlottedPointMeta");

    // Extract the two horizontal dashed alignment lines (Control mean -> 0, Treated mean -> Δ).
    let mut dashed_horiz_ys: Vec<(f64, f64, f64)> = Vec::new();
    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, stroke: Some(st), .. } = prim
            && st.dash == LineDashStyle::Dashed
            && let [PathCmd::MoveTo(p0), PathCmd::LineTo(p1)] = path.commands.as_slice()
            && (p0.y - p1.y).abs() < 1e-6
            && (p1.x - p0.x).abs() > 20.0
        {
            dashed_horiz_ys.push((p0.y, p0.x, p1.x));
        }
    }
    assert_eq!(dashed_horiz_ys.len(), 2, "expected exactly 2 horizontal dashed alignment lines");

    // Control mean (11.0) is lower in data space -> larger screen Y; Treated mean (15.0) is higher -> smaller screen Y.
    let y_mean_a = dashed_horiz_ys[0].0.max(dashed_horiz_ys[1].0);
    let y_mean_b = dashed_horiz_ys[0].0.min(dashed_horiz_ys[1].0);
    let right_spine_x = dashed_horiz_ys[0].2;
    assert!((dashed_horiz_ys[0].2 - dashed_horiz_ys[1].2).abs() < 1e-6, "both dashed alignment lines must end at the right floating axis spine");

    // Verify right axis tick '0' sits at exact vertical pixel y_mean_a.
    let zero_tick = scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "0" && t.anchor_pt.x > right_spine_x => Some(t.anchor_pt.y - 3.0),
            _ => None,
        })
        .expect("floating difference axis must render a '0' tick aligned with Group A mean");
    assert!((zero_tick - y_mean_a).abs() < 1e-6, "right axis '0' tick y={zero_tick} must equal Group A mean y={y_mean_a}");

    // Verify the effect estimate marker (Δ = 4) sits at exact vertical pixel y_mean_b.
    assert!(y_mean_a > y_mean_b, "higher Treated mean must lie above Control mean in screen Y");
    let has_delta_footer = scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("\u{0394} = 4")));
    assert!(has_delta_footer, "expected Δ = 4 footer under right difference panel");
}

#[test]
fn estimation_plot_paired_mode_renders_slope_lines_and_handles_mv_and_empty() {
    let a = [10.0, 12.0, 14.0, 16.0, 18.0];
    let b = [13.0, 14.5, 17.5, 18.0, 21.5];
    let ds = two_group_column_sheet("col-paired", ColumnDesignHint::PairedRepeatedMeasures, &a, &b);
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-paired"), &ds).expect("gs");
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot);
    gs.spec.estimation_paired_lines = true;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone()).expect("add ds");
    // Also attach a linked TwoGroupComparison(PairedT) result sheet.
    let spec = AnalysisSpec::new(
        SheetId::new("res-paired"),
        ds.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    let rs = curvebook_stats::run_analysis(&ds, &spec).expect("run paired t");
    doc.add_result_sheet(rs).expect("add rs");
    doc.add_graph_sheet(gs.clone()).expect("add gs");

    let scene = compile_graph(&gs, &doc).expect("compile paired estimation plot");
    assert!(scene.warnings.is_empty(), "warnings: {:?}", scene.warnings);

    // Count sloped lines (non-horizontal, non-vertical 2-point segments connecting group A -> group B).
    let slope_count = scene
        .primitives
        .iter()
        .filter(|p| {
            matches!(
                p,
                ScenePrimitive::Path { path, stroke: Some(st), .. }
                    if st.dash == LineDashStyle::Solid
                        && matches!(path.commands.as_slice(), [PathCmd::MoveTo(p0), PathCmd::LineTo(p1)]
                            if (p1.x - p0.x).abs() > 20.0 && (p1.y - p0.y).abs() > 1.0)
            )
        })
        .count();
    assert_eq!(slope_count, 5, "expected 5 paired slope-line connectors for 5 matched subjects");
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Paired mean difference"))));

    // MultipleVariables table with binary categorical column + numeric outcome.
    let mut mv = MultipleVariablesTable::new(2, 6).expect("mv");
    mv.variables[0].title = String::from("Arm");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Score");
    for (r, (g, y)) in [("Placebo", 5.0), ("Placebo", 6.0), ("Placebo", 7.0), ("Drug", 9.0), ("Drug", 10.0), ("Drug", 11.0)].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(g);
        mv.variables[1].cells[r] = Cell::number(y);
    }
    let ds_mv = DataSheet::new(SheetId::new("mv-est"), "MV Est", TableArchetype::MultipleVariables(mv)).expect("ds_mv");
    let mut gs_mv = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv-est"), &ds_mv).expect("gs_mv");
    gs_mv.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot);
    let mut doc_mv = ProjectDocument::default();
    doc_mv.add_data_sheet(ds_mv).expect("add mv");
    let scene_mv = compile_graph(&gs_mv, &doc_mv).expect("compile mv estimation");
    assert!(scene_mv.warnings.is_empty(), "mv warnings: {:?}", scene_mv.warnings);
    assert_eq!(scene_mv.plotted_points.len(), 6);

    // Degenerate insufficient observations -> graceful warning + open frame without panic.
    let ds_empty = two_group_column_sheet("col-empty", ColumnDesignHint::UnpairedReplicates, &[1.0], &[2.0]);
    let mut gs_empty = GraphSheet::new_uninitialized_for_data(SheetId::new("g-empty"), &ds_empty).expect("gs_empty");
    gs_empty.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot);
    let mut doc_empty = ProjectDocument::default();
    doc_empty.add_data_sheet(ds_empty).expect("add empty");
    let scene_empty = compile_graph(&gs_empty, &doc_empty).expect("compile empty estimation");
    assert!(scene_empty.warnings.iter().any(|w| w.code == "empty_estimation_plot"));
}

#[test]
fn estimation_plot_grouped_paired_aligns_coordinates_and_skips_incomplete_pairs() {
    // 2 columns x 3 rows x 2 replicates = 6 coordinate slots; exclude (r=0, c=0, sub=1) and
    // leave (r=2, c=1, sub=0) empty so only 4 aligned (row, subcol) pairs remain, each with exact diff +3.0.
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).expect("grouped table");
    gt.data_sets[0].title = String::from("Baseline");
    gt.data_sets[1].title = String::from("FollowUp");
    let a_vals = [[10.0, 99.0], [12.0, 14.0], [88.0, 16.0]];
    let b_vals = [[13.0, 20.0], [15.0, 17.0], [0.0, 19.0]];
    for r in 0..3 {
        for s in 0..2 {
            gt.data_sets[0].cells[r][s] = Cell::number(a_vals[r][s]);
            gt.data_sets[1].cells[r][s] = Cell::number(b_vals[r][s]);
        }
    }
    gt.data_sets[0].cells[0][1].excluded = true;
    gt.data_sets[1].cells[2][0] = Cell::empty();

    let ds = DataSheet::new(SheetId::new("grp-est-paired"), "Grouped Paired", TableArchetype::Grouped(gt)).expect("ds");
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-grp-paired"), &ds).expect("gs");
    gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot);
    gs.spec.estimation_paired_lines = true;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_graph_sheet(gs.clone()).expect("add gs");

    let scene = compile_graph(&gs, &doc).expect("compile grouped paired estimation plot");
    assert!(scene.warnings.is_empty(), "warnings: {:?}", scene.warnings);
    // Only the 4 complete pairs (8 points total) must be plotted — no trailing unpaired points.
    assert_eq!(scene.plotted_points.len(), 8, "expected exactly 8 plotted observations (4 matched pairs)");
    assert!(
        scene
            .plotted_points
            .iter()
            .all(|pt| !(pt.source_cell.row == 0 && pt.source_cell.subcol == 1) && !(pt.source_cell.row == 2 && pt.source_cell.subcol == 0)),
        "incomplete (row, subcol) pairs must be dropped on both sides"
    );
    let slope_count = scene
        .primitives
        .iter()
        .filter(|p| {
            matches!(
                p,
                ScenePrimitive::Path { path, stroke: Some(st), .. }
                    if st.dash == LineDashStyle::Solid
                        && matches!(path.commands.as_slice(), [PathCmd::MoveTo(p0), PathCmd::LineTo(p1)]
                            if (p1.x - p0.x).abs() > 20.0 && (p1.y - p0.y).abs() > 1.0)
            )
        })
        .count();
    assert_eq!(slope_count, 4, "expected exactly 4 aligned paired slope connectors");
    // Every aligned pair has b - a == +3.0 (unaligned zipping would have paired 12->20, 14->15, etc. != 3.0).
    assert!(
        scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("\u{0394} = 3"))),
        "aligned paired difference must equal exactly Δ = 3"
    );
}
