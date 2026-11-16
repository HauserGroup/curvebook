use std::error::Error;
use std::time::Instant;

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, DataSheet,
    DiagnosticPlotKind, ErrorCode, GraphKind, GraphSheet, GraphSpec, GroupedTable, MultipleVariablesTable, MultiplicitySpec, MultivariateSpec,
    OneWayAnovaSpec, OneWayMethod, PairwiseBracketSpec, PairwiseComparisonRecord, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SheetId, SubcolumnMode, TableArchetype, VariableRole,
};

type TestRes<T = ()> = Result<T, Box<dyn Error>>;

fn make_doc(name: &str) -> TestRes<ProjectDocument> {
    Ok(ProjectDocument::new(name)?)
}

fn collect_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn find_label_y(scene: &ChartScene, label: &str) -> Option<f64> {
    scene.primitives.iter().find_map(|p| match p {
        ScenePrimitive::Text(t) if t.rich.plain_text() == label => Some(t.anchor_pt.y),
        _ => None,
    })
}

fn solid_fill_path_count(scene: &ChartScene) -> usize {
    scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), .. })).count()
}

fn compile_beeswarm_n(n: usize, uniform_val: Option<f64>) -> TestRes<ChartScene> {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, n)?;
    table.data_sets[0].title = String::from("Group A");
    table.data_sets[1].title = String::from("Group B");
    for r in 0..n {
        let va = uniform_val.unwrap_or(10.0 + ((r % 37) as f64) * 0.4 + ((r % 7) as f64) * 0.05);
        let vb = uniform_val.unwrap_or(25.0 + ((r % 29) as f64) * 0.3);
        table.data_sets[0].cells[r][0] = Cell::number(va);
        table.data_sets[1].cells[r][0] = Cell::number(vb);
    }

    let ds = DataSheet::new(SheetId::new("col_bs"), "Beeswarm", TableArchetype::Column(table))?;
    let mut spec = GraphSpec::default_for_data_sheet(&ds);
    spec.kind = GraphKind::Column;
    let mut doc = make_doc("Beeswarm")?;
    doc.add_data_sheet(ds)?;
    let graph = GraphSheet {
        id: SheetId::new("g_bs"),
        title: String::from("Beeswarm"),
        initialized: true,
        spec,
        floating_notes: Vec::new(),
        color_tag: None,
    };
    doc.add_graph_sheet(graph.clone())?;
    Ok(compile_graph(&graph, &doc)?)
}

#[test]
fn r1_s08_03_beeswarm_scaling_is_subquadratic_and_bounded() -> TestRes {
    let _warm = compile_beeswarm_n(120, None)?;

    let t0 = Instant::now();
    for _ in 0..3 {
        let s = compile_beeswarm_n(500, None)?;
        assert_eq!(s.plotted_points.len(), 1000);
    }
    let dt_500 = t0.elapsed().as_secs_f64();

    let t1 = Instant::now();
    for _ in 0..3 {
        let s = compile_beeswarm_n(2000, None)?;
        assert_eq!(s.plotted_points.len(), 4000);
    }
    let dt_2000 = t1.elapsed().as_secs_f64();

    // 4x input scaling under quadratic O(N^2) yields ~16x runtime; sweep-line must stay < 8x.
    let ratio = dt_2000 / dt_500.max(1e-4);
    assert!(ratio < 8.0, "expected subquadratic scaling for beeswarm (dt_500={dt_500:.4}s, dt_2000={dt_2000:.4}s, ratio={ratio:.2})");

    // Dense identical-Y points exceeding marker cap must complete quickly, warn, and stay deterministic.
    let t2 = Instant::now();
    let capped_a = compile_beeswarm_n(10_000, Some(42.0))?;
    let elapsed_capped = t2.elapsed().as_secs_f64();
    let capped_b = compile_beeswarm_n(10_000, Some(42.0))?;
    assert!(elapsed_capped < 2.0, "capped beeswarm took too long: {elapsed_capped:.3}s");
    assert_eq!(capped_a.primitives, capped_b.primitives, "beeswarm layout must remain deterministic");
    assert_eq!(capped_a.plotted_points.len(), 4_096 * 2, "each group should be bounded cleanly to MAX_REPLICATE_MARKERS");
    assert_eq!(
        capped_a.warnings.iter().filter(|w| w.code == "replicate_markers_capped").count(),
        1,
        "replicate_markers_capped warning should be emitted once"
    );
    Ok(())
}

fn build_three_group_bracket_doc(status: ResultStatus) -> TestRes<(ProjectDocument, GraphSheet)> {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?;
    table.data_sets[0].title = String::from("Control");
    table.data_sets[1].title = String::from("Low Dose");
    table.data_sets[2].title = String::from("High Dose");
    let vals = [[10.0, 11.0, 9.5, 10.5], [14.0, 15.0, 13.5, 14.5], [22.0, 23.0, 21.5, 22.5]];
    for (c, col_vals) in vals.iter().enumerate() {
        for (r, &v) in col_vals.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ds = DataSheet::new(SheetId::new("col_1"), "Arms", TableArchetype::Column(table))?;
    let mut spec = GraphSpec::default_for_data_sheet(&ds);
    spec.kind = GraphKind::Column;

    let anova_spec = AnalysisSpec::new(
        SheetId::new("res_anova"),
        SheetId::new("col_1"),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: false,
            multiplicity: MultiplicitySpec::default(),
        }),
    )?;
    let mut res = ResultSheet::new(anova_spec)?;
    res.status = status;
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 2,
        label_a: String::from("Control"),
        label_b: String::from("High Dose"),
        estimate_diff: Some(-12.0),
        raw_p_or_q: 0.002,
        is_fdr_q: false,
        significant: true,
    });

    spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res_anova"),
        group_a: 0,
        group_b: 2,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::NumericPOrQ,
        hide_if_non_significant: true,
        stroke_width_pt: 1.0,
        font_size_pt: 8.0,
        bracket_step_pt: 14.0,
    });

    let mut doc = make_doc("Brackets")?;
    doc.add_data_sheet(ds)?;
    doc.add_result_sheet(res)?;
    let graph =
        GraphSheet { id: SheetId::new("g_1"), title: String::from("Arms"), initialized: true, spec, floating_notes: Vec::new(), color_tag: None };
    doc.add_graph_sheet(graph.clone())?;
    Ok((doc, graph))
}

#[test]
fn r1_s08_07_missing_comparison_group_warns_and_skips_instead_of_misattaching() -> TestRes {
    let (doc, mut graph) = build_three_group_bracket_doc(ResultStatus::UpToDate)?;
    let visible_scene = compile_graph(&graph, &doc)?;
    let visible_texts = collect_texts(&visible_scene);
    assert!(visible_texts.iter().any(|t| t.contains("0.002")), "expected p-value bracket label when both groups are plotted: {visible_texts:?}");
    assert!(!visible_scene.warnings.iter().any(|w| w.code == "missing_comparison_group"));

    // Hide the first group ("Control", X position 1.0) so group 0 is missing from plotted bounds.
    graph.spec.x_axis.segments[0].min = Some(1.5);
    graph.spec.x_axis.segments[0].max = Some(3.5);
    let hidden_scene = compile_graph(&graph, &doc)?;
    let hidden_texts = collect_texts(&hidden_scene);
    assert!(!hidden_texts.iter().any(|t| t.contains("0.002")), "bracket referencing hidden Control must not attach to Low Dose: {hidden_texts:?}");
    assert!(
        hidden_scene.warnings.iter().any(|w| w.code == "missing_comparison_group"),
        "expected missing_comparison_group warning, got {:?}",
        hidden_scene.warnings
    );

    // Hide ALL groups -> still warns cleanly without panicking.
    graph.spec.x_axis.segments[0].min = Some(10.0);
    graph.spec.x_axis.segments[0].max = Some(12.0);
    let all_hidden_scene = compile_graph(&graph, &doc)?;
    assert!(all_hidden_scene.warnings.iter().any(|w| w.code == "missing_comparison_group"));
    Ok(())
}

#[test]
fn r1_s08_07_failed_result_sheet_skips_stale_brackets_and_warns() -> TestRes {
    let (doc_failed, mut graph_failed) =
        build_three_group_bracket_doc(ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("singular design matrix") })?;

    // NumericPOrQ with hide_if_non_significant=true must skip stale record & warn.
    let scene_num = compile_graph(&graph_failed, &doc_failed)?;
    assert!(!collect_texts(&scene_num).iter().any(|t| t.contains("0.002")), "failed result sheet must not plot stale p-value bracket");
    assert!(
        scene_num.warnings.iter().any(|w| w.code == "failed_comparison_result"),
        "expected failed_comparison_result warning, got {:?}",
        scene_num.warnings
    );

    // Custom label with hide_if_non_significant=false must also skip stale failed sheet & warn.
    graph_failed.spec.comparisons[0].hide_if_non_significant = false;
    graph_failed.spec.comparisons[0].label_mode = BracketLabelMode::Custom(String::from("STALE_BRACKET"));
    let scene_custom = compile_graph(&graph_failed, &doc_failed)?;
    assert!(!collect_texts(&scene_custom).iter().any(|t| t == "STALE_BRACKET"));
    assert!(scene_custom.warnings.iter().any(|w| w.code == "failed_comparison_result"));

    // Dirty result sheet (normal post-load state) still renders bracket cleanly.
    let (doc_dirty, graph_dirty) = build_three_group_bracket_doc(ResultStatus::Dirty)?;
    let scene_dirty = compile_graph(&graph_dirty, &doc_dirty)?;
    assert!(collect_texts(&scene_dirty).iter().any(|t| t.contains("0.002")));
    assert!(!scene_dirty.warnings.iter().any(|w| w.code == "failed_comparison_result"));
    Ok(())
}

fn compile_column_heatmap(rows: &[[Option<Cell>; 3]]) -> TestRes<ChartScene> {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, rows.len())?;
    table.data_sets[0].title = String::from("C1");
    table.data_sets[1].title = String::from("C2");
    table.data_sets[2].title = String::from("C3");
    for (r, row) in rows.iter().enumerate() {
        for (c, cell_opt) in row.iter().enumerate() {
            if let Some(cell) = cell_opt {
                table.data_sets[c].cells[r][0] = cell.clone();
            }
        }
    }
    let ds = DataSheet::new(SheetId::new("col_hm"), "HM", TableArchetype::Column(table))?;
    let mut spec = GraphSpec::default_for_data_sheet(&ds);
    spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    let mut doc = make_doc("HM")?;
    doc.add_data_sheet(ds)?;
    let graph =
        GraphSheet { id: SheetId::new("g_hm"), title: String::from("HM"), initialized: true, spec, floating_notes: Vec::new(), color_tag: None };
    doc.add_graph_sheet(graph.clone())?;
    Ok(compile_graph(&graph, &doc)?)
}

#[test]
fn r1_x2_07_heatmap_missing_and_excluded_cells_not_coerced_to_zero() -> TestRes {
    let missing_rows = [
        [Some(Cell::number(10.0)), Some(Cell::number(20.0)), Some(Cell::number(30.0))],
        [Some(Cell::number(40.0)), Some(Cell::number(50.0)), Some(Cell::number(60.0))],
        [Some(Cell::number(70.0)), Some(Cell::number(80.0)), Some(Cell::number(90.0))],
        [Some(Cell::number(100.0)), None, Some(Cell::number(120.0))],
    ];
    let zero_rows = [
        [Some(Cell::number(10.0)), Some(Cell::number(20.0)), Some(Cell::number(30.0))],
        [Some(Cell::number(40.0)), Some(Cell::number(50.0)), Some(Cell::number(60.0))],
        [Some(Cell::number(70.0)), Some(Cell::number(80.0)), Some(Cell::number(90.0))],
        [Some(Cell::number(100.0)), Some(Cell::number(0.0)), Some(Cell::number(120.0))],
    ];
    let excluded_rows = [
        [Some(Cell::number(10.0)), Some(Cell::number(20.0)), Some(Cell::number(30.0))],
        [Some(Cell::number(40.0)), Some(Cell::number(50.0)), Some(Cell::number(60.0))],
        [Some(Cell::number(70.0)), Some(Cell::number(80.0)), Some(Cell::number(90.0))],
        [Some(Cell::number(100.0)), Some(Cell::excluded_number(0.0)), Some(Cell::number(120.0))],
    ];

    let missing_scene = compile_column_heatmap(&missing_rows)?;
    let zero_scene = compile_column_heatmap(&zero_rows)?;
    let excluded_scene = compile_column_heatmap(&excluded_rows)?;

    assert_ne!(missing_scene.primitives, zero_scene.primitives, "missing heatmap cell must differ from 0.0");
    assert_eq!(excluded_scene.primitives, missing_scene.primitives, "excluded cell must render like missing cell");
    assert!(solid_fill_path_count(&missing_scene) < solid_fill_path_count(&zero_scene), "missing cell must not emit a solid colormap fill rect");

    let missing_texts = collect_texts(&missing_scene);
    let zero_texts = collect_texts(&zero_scene);
    assert!(missing_texts.iter().any(|t| t == "10"), "colorbar min should stay 10, got {missing_texts:?}");
    assert!(!missing_texts.iter().any(|t| t == "0"), "missing cell must not stretch colorbar min to 0");
    assert!(zero_texts.iter().any(|t| t == "0"), "explicit 0.0 cell should set colorbar min to 0");

    // Diverging domain where 0.0 is strictly inside [-10, 10] must STILL render missing cells distinctly.
    let diverging_missing =
        [[Some(Cell::number(-10.0)), Some(Cell::number(5.0)), Some(Cell::number(10.0))], [Some(Cell::number(2.0)), None, Some(Cell::number(-4.0))]];
    let diverging_zero = [
        [Some(Cell::number(-10.0)), Some(Cell::number(5.0)), Some(Cell::number(10.0))],
        [Some(Cell::number(2.0)), Some(Cell::number(0.0)), Some(Cell::number(-4.0))],
    ];
    let div_miss_scene = compile_column_heatmap(&diverging_missing)?;
    let div_zero_scene = compile_column_heatmap(&diverging_zero)?;
    assert_ne!(
        div_miss_scene.primitives, div_zero_scene.primitives,
        "missing cell must remain distinct from 0.0 even inside symmetric diverging range"
    );

    // All-missing heatmap must warn cleanly and emit empty open frame.
    let all_missing_scene = compile_column_heatmap(&[[None, None, None], [None, None, None]])?;
    assert!(all_missing_scene.warnings.iter().any(|w| w.code == "empty_heatmap"));
    Ok(())
}

#[test]
fn r1_x2_07_heatmap_pairwise_complete_clustering_and_grouped_and_correlation_missing() -> TestRes {
    // 1. MultipleVariables table with Euclidean/Ward clustering where row "A2" has one missing value.
    // Pairwise-complete Euclidean puts "A2" (100, None, 100) next to "A1" (100, 100, 100), NOT "C5" (45, 45, 45).
    let mut mv = MultipleVariablesTable::new(4, 4)?;
    mv.variables[0].title = String::from("Label");
    mv.variables[0].role = VariableRole::Label;
    for (i, &lbl) in ["A1", "B0", "A2", "C5"].iter().enumerate() {
        mv.variables[0].cells[i] = Cell::text(lbl);
    }
    for c in 1..=3 {
        mv.variables[c].title = format!("F{c}");
        mv.variables[c].role = VariableRole::Continuous;
        mv.variables[c].cells[0] = Cell::number(100.0);
        mv.variables[c].cells[1] = Cell::number(0.0);
        mv.variables[c].cells[3] = Cell::number(45.0);
    }
    mv.variables[1].cells[2] = Cell::number(100.0);
    mv.variables[2].cells[2] = Cell::empty();
    mv.variables[3].cells[2] = Cell::number(100.0);

    let ds_mv = DataSheet::new(SheetId::new("mv_hm"), "MV HM", TableArchetype::MultipleVariables(mv))?;
    let mut spec_mv = GraphSpec::default_for_data_sheet(&ds_mv);
    spec_mv.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    let cl_spec = AnalysisSpec::new(
        SheetId::new("res_cluster"),
        SheetId::new("mv_hm"),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: None,
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: None,
        }),
    )?;
    let cl_res = ResultSheet::new(cl_spec)?;

    let mut doc_mv = make_doc("MV Cluster HM")?;
    doc_mv.add_data_sheet(ds_mv)?;
    doc_mv.add_result_sheet(cl_res)?;
    let graph_mv = GraphSheet {
        id: SheetId::new("g_mv"),
        title: String::from("MV Cluster"),
        initialized: true,
        spec: spec_mv,
        floating_notes: Vec::new(),
        color_tag: None,
    };
    doc_mv.add_graph_sheet(graph_mv.clone())?;
    let scene_mv = compile_graph(&graph_mv, &doc_mv)?;
    let mut ordered = [
        ("A1", find_label_y(&scene_mv, "A1").ok_or("missing A1 row label")?),
        ("B0", find_label_y(&scene_mv, "B0").ok_or("missing B0 row label")?),
        ("A2", find_label_y(&scene_mv, "A2").ok_or("missing A2 row label")?),
        ("C5", find_label_y(&scene_mv, "C5").ok_or("missing C5 row label")?),
    ];
    ordered.sort_by(|a, b| a.1.total_cmp(&b.1));
    let names: Vec<&str> = ordered.iter().map(|&(k, _)| k).collect();
    let pos_a1 = names.iter().position(|&k| k == "A1").ok_or("pos A1")?;
    let pos_a2 = names.iter().position(|&k| k == "A2").ok_or("pos A2")?;
    assert_eq!(pos_a1 / 2, pos_a2 / 2, "pairwise-complete clustering should pair A2 into the same first clade as A1 (order={names:?})");
    assert_eq!(pos_a1.abs_diff(pos_a2), 1, "A1 and A2 should be adjacent in dendrogram order (order={names:?})");

    // 2. Disjoint missing rows must skip clustering cleanly with a warning instead of failing/coercing.
    let disjoint_scene = compile_column_heatmap(&[[Some(Cell::number(10.0)), None, None], [None, Some(Cell::number(20.0)), None]])?;
    assert!(
        disjoint_scene.warnings.iter().any(|w| w.code == "clustering_skipped_missing_data"),
        "expected clustering_skipped_missing_data warning, got {:?}",
        disjoint_scene.warnings
    );

    // 3. Grouped table with all-excluded replicates in one cell must differ from 0.0.
    let mut g_miss = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    let mut g_zero = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2)?;
    for g in [&mut g_miss, &mut g_zero] {
        g.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(12.0)];
        g.data_sets[1].cells[0] = vec![Cell::number(20.0), Cell::number(22.0)];
        g.data_sets[0].cells[1] = vec![Cell::number(30.0), Cell::number(32.0)];
    }
    g_miss.data_sets[1].cells[1] = vec![Cell::excluded_number(0.0), Cell::empty()];
    g_zero.data_sets[1].cells[1] = vec![Cell::number(0.0), Cell::number(0.0)];

    let compile_grouped = |gt: GroupedTable| -> TestRes<ChartScene> {
        let ds = DataSheet::new(SheetId::new("g_ds"), "Grp", TableArchetype::Grouped(gt))?;
        let mut spec = GraphSpec::default_for_data_sheet(&ds);
        spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
        let mut doc = make_doc("GrpHM")?;
        doc.add_data_sheet(ds)?;
        let gs =
            GraphSheet { id: SheetId::new("g_hm"), title: String::from("Grp"), initialized: true, spec, floating_notes: Vec::new(), color_tag: None };
        doc.add_graph_sheet(gs.clone())?;
        Ok(compile_graph(&gs, &doc)?)
    };
    let grp_miss_scene = compile_grouped(g_miss)?;
    let grp_zero_scene = compile_grouped(g_zero)?;
    assert_ne!(grp_miss_scene.primitives, grp_zero_scene.primitives);

    // 4. CorrelationMatrix ResultSheet with missing ResultCell::Empty must differ from 0.0.
    let compile_corr = |missing_cell: ResultCell| -> TestRes<ChartScene> {
        let mut mv_src = MultipleVariablesTable::new(2, 2)?;
        mv_src.variables[0].cells[0] = Cell::number(1.0);
        mv_src.variables[1].cells[1] = Cell::number(2.0);
        let ds = DataSheet::new(SheetId::new("src"), "Src", TableArchetype::MultipleVariables(mv_src))?;
        let corr_spec = AnalysisSpec::new(
            SheetId::new("res_corr"),
            SheetId::new("src"),
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
        )?;
        let mut res = ResultSheet::new(corr_spec)?;
        res.status = ResultStatus::UpToDate;
        res.tabs.push(ResultTab {
            id: String::from("corr"),
            title: String::from("Correlation"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("V1"), String::from("V2")],
            row_labels: vec![String::from("V1"), String::from("V2")],
            rows: vec![
                vec![ResultCell::Number { value: 1.0, prefix: None, units: None }, missing_cell.clone()],
                vec![missing_cell, ResultCell::Number { value: 1.0, prefix: None, units: None }],
            ],
            footnotes: Vec::new(),
        });
        let mut spec = GraphSpec::default_for_data_sheet(&ds);
        spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
        spec.primary_source_sheet_id = SheetId::new("res_corr");
        let mut doc = make_doc("CorrHM")?;
        doc.add_data_sheet(ds)?;
        doc.add_result_sheet(res)?;
        let gs = GraphSheet {
            id: SheetId::new("g_corr"),
            title: String::from("Corr"),
            initialized: true,
            spec,
            floating_notes: Vec::new(),
            color_tag: None,
        };
        doc.add_graph_sheet(gs.clone())?;
        Ok(compile_graph(&gs, &doc)?)
    };
    let corr_miss_scene = compile_corr(ResultCell::Empty)?;
    let corr_zero_scene = compile_corr(ResultCell::Number { value: 0.0, prefix: None, units: None })?;
    assert_ne!(corr_miss_scene.primitives, corr_zero_scene.primitives);
    Ok(())
}
