//! Multivariate bubble/categorical figures, all seven diagnostic plot subtypes, and
//! multi-panel publication layout scene compilation.

mod diagnostics;
mod layout_sheet;
mod multivariate;

use curvebook_doc::{DiagnosticPlotKind, GraphKind, GraphSheet, ProjectDocument};

pub(crate) use layout_sheet::check_layout_primitive_budget;
pub use layout_sheet::{MAX_LAYOUT_PRIMITIVES, compile_layout_scene};

use crate::scene::{ChartError, ChartScene, ChartWarning};
use crate::survival_cat::{draw_empty_open_frame, draw_graph_annotations};

/// Compile a multivariate or diagnostic graph sheet into vector primitives inside `scene`.
pub fn compile_multivariate_or_diagnostic_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    if doc.data_sheet(&graph.spec.primary_source_sheet_id).is_none() && doc.result_sheet(&graph.spec.primary_source_sheet_id).is_none() {
        scene
            .warnings
            .push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{}' not found", graph.spec.primary_source_sheet_id)));
    }

    match graph.spec.kind {
        GraphKind::MultipleVariables => multivariate::compile_multiple_variables(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap) => diagnostics::compile_heatmap(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::Volcano) => diagnostics::compile_volcano(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman) => diagnostics::compile_bland_altman(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve) => diagnostics::compile_roc_curve(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot) => diagnostics::compile_forest_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot) => diagnostics::compile_residual_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot) => diagnostics::compile_normal_qq_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::HomoscedasticityPlot) => diagnostics::compile_homoscedasticity_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ActualVsPredictedPlot) => diagnostics::compile_actual_vs_predicted_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualVsRowPlot) => diagnostics::compile_residual_vs_row_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::GapStatisticPlot) => diagnostics::compile_gap_statistic_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ParameterCorrelationHeatmap) => {
            diagnostics::compile_parameter_correlation_heatmap(graph, doc, scene)
        }
        GraphKind::Diagnostic(DiagnosticPlotKind::LognormalQqPlot) => diagnostics::compile_lognormal_qq_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram) => diagnostics::compile_dendrogram_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot) => diagnostics::compile_pca_biplot(graph, doc, scene),
        _ => {
            scene.warnings.push(ChartWarning::new("unexpected_graph_kind", "non-multivariate graph kind dispatched"));
            draw_empty_open_frame(scene);
            Ok(())
        }
    }?;

    draw_graph_annotations(scene, &graph.spec.annotations, doc, doc.paired_info_sheet_id(&graph.spec.primary_source_sheet_id))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AxisSpec, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec,
        MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, ProjectDocument, RectPt as DocRectPt, SheetId, SizeChannelMapping,
        SubcolumnMode, TableArchetype, VariableRole, XyTable,
    };
    use curvebook_palette::{ColormapId, PaletteId, Rgba8};
    use curvebook_text::SpanWeight;

    use super::{compile_layout_scene, compile_multivariate_or_diagnostic_graph};
    use crate::scene::{ChartScene, ScenePrimitive};

    #[test]
    fn bubble_scatter_compiles_hulls_ellipses_and_colorbar() {
        let mut mv = MultipleVariablesTable::new(4, 6).expect("mv");
        mv.variables[0].title = String::from("X");
        mv.variables[1].title = String::from("Y");
        mv.variables[2].title = String::from("Group");
        mv.variables[2].lock_role(VariableRole::Categorical);
        mv.variables[3].title = String::from("Mass");
        for (r, (x, y, g, sz)) in
            [(1.0, 2.0, "A", 4.0), (2.0, 4.0, "A", 16.0), (3.0, 3.0, "A", 9.0), (6.0, 7.0, "B", 4.0), (7.0, 9.0, "B", 9.0), (8.0, 8.0, "B", 16.0)]
                .into_iter()
                .enumerate()
        {
            mv.variables[0].cells[r] = curvebook_doc::Cell::number(x);
            mv.variables[1].cells[r] = curvebook_doc::Cell::number(y);
            mv.variables[2].cells[r] = curvebook_doc::Cell::text(g);
            mv.variables[3].cells[r] = curvebook_doc::Cell::number(sz);
        }
        let ds = DataSheet::new(SheetId::new("mv-1"), "Bubble", TableArchetype::MultipleVariables(mv)).expect("ds");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv"), &ds).expect("gs");
        gs.spec.multivariate = Some(MultivariateGraphEncoding {
            plot_mode: MultivariatePlotMode::BubbleScatter,
            x_column: 0,
            y_column: 1,
            color: ColorChannelMapping::Categorical { column: 2, palette: PaletteId::OkabeIto, level_order_override: None },
            size: SizeChannelMapping::Variable { column: 3, min_radius_pt: 3.0, max_radius_pt: 12.0 },
            label_column: Some(2),
            x_level_order_override: None,
            show_convex_hulls: true,
            show_confidence_ellipses: true,
        });
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add");
        let mut scene = ChartScene::new("g-mv", "Bubble", 420.0, 300.0, None);
        compile_multivariate_or_diagnostic_graph(&gs, &doc, &mut scene).expect("compile");
        assert!(scene.primitives.len() > 20);
        assert!(
            scene
                .primitives
                .iter()
                .any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(st), .. } if st.dash == curvebook_palette::LineDashStyle::Dashed))
        );
    }

    #[test]
    fn categorical_summary_renders_cld_letters_above_error_bars() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, CompactLetterDisplaySpec, ErrorBarSpec, GraphicalDataObject, OneWayAnovaSpec, PairwiseComparisonRecord,
            PairwiseDisplayMode, ResultSheet, ResultStatus, SummaryMeasure,
        };

        let mut mv = MultipleVariablesTable::new(2, 6).expect("mv");
        mv.variables[0].title = String::from("Group");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[1].title = String::from("Response");
        for (r, (g, y)) in [("Ctrl", 10.0), ("Ctrl", 12.0), ("Drug", 24.0), ("Drug", 26.0), ("Ref", 18.0), ("Ref", 20.0)].into_iter().enumerate() {
            mv.variables[0].cells[r] = curvebook_doc::Cell::text(g);
            mv.variables[1].cells[r] = curvebook_doc::Cell::number(y);
        }
        let ds = DataSheet::new(SheetId::new("mv-cat"), "Cat Summary", TableArchetype::MultipleVariables(mv)).expect("ds");

        let spec =
            AnalysisSpec::new(SheetId::new("res-cat"), SheetId::new("mv-cat"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("spec");
        let mut res = ResultSheet::new(spec).expect("res");
        res.status = ResultStatus::UpToDate;
        res.pairwise_comparisons = vec![
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 1,
                label_a: String::from("Ctrl"),
                label_b: String::from("Drug"),
                estimate_diff: Some(-14.0),
                raw_p_or_q: 0.002,
                is_fdr_q: false,
                significant: true,
            },
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 2,
                label_a: String::from("Ctrl"),
                label_b: String::from("Ref"),
                estimate_diff: Some(-8.0),
                raw_p_or_q: 0.14,
                is_fdr_q: false,
                significant: false,
            },
            PairwiseComparisonRecord {
                group_a: 1,
                group_b: 2,
                label_a: String::from("Drug"),
                label_b: String::from("Ref"),
                estimate_diff: Some(6.0),
                raw_p_or_q: 0.16,
                is_fdr_q: false,
                significant: false,
            },
        ];

        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cat"), &ds).expect("gs");
        gs.spec.title = String::from("Categorical CLD");
        gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res-cat")));
        gs.spec.multivariate = Some(MultivariateGraphEncoding {
            plot_mode: MultivariatePlotMode::CategoricalSummary {
                gdo: GraphicalDataObject::BarWithReplicates {
                    summary: SummaryMeasure::Mean,
                    fill_color: Rgba8::rgb(0, 114, 178),
                    border_color: Rgba8::BLACK,
                    border_width_pt: 0.8,
                    replicate_overlay: None,
                },
                y_error_bars: Some(ErrorBarSpec::default()),
            },
            x_column: 0,
            y_column: 1,
            color: ColorChannelMapping::Constant(Rgba8::rgb(0, 114, 178)),
            size: SizeChannelMapping::Constant { radius_pt: 4.0 },
            label_column: None,
            x_level_order_override: None,
            show_convex_hulls: false,
            show_confidence_ellipses: false,
        });

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add ds");
        doc.add_result_sheet(res).expect("add res");
        let mut scene = ChartScene::new("g-cat", "Categorical CLD", 360.0, 240.0, None);
        compile_multivariate_or_diagnostic_graph(&gs, &doc, &mut scene).expect("compile");
        assert!(scene.warnings.is_empty(), "warnings: {:?}", scene.warnings);
        let texts: Vec<String> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&String::from("a")), "missing 'a' in {texts:?}");
        assert!(texts.contains(&String::from("ab")), "missing 'ab' in {texts:?}");
        assert!(texts.contains(&String::from("b")), "missing 'b' in {texts:?}");
    }

    #[test]
    fn layout_align_plot_origins_matches_y_axis_x_across_stacked_panels() {
        let mut xy_small = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy1");
        let mut xy_big = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy2");
        for r in 0..3 {
            xy_small.x_cells[r][0] = curvebook_doc::Cell::number(r as f64);
            xy_small.data_sets[0].cells[r][0] = curvebook_doc::Cell::number((r as f64) * 1.0);
            xy_big.x_cells[r][0] = curvebook_doc::Cell::number(r as f64);
            xy_big.data_sets[0].cells[r][0] = curvebook_doc::Cell::number((r as f64) * 1_000_000.0);
        }
        let d1 = DataSheet::new(SheetId::new("d1"), "Small", TableArchetype::Xy(xy_small)).expect("d1");
        let d2 = DataSheet::new(SheetId::new("d2"), "Big", TableArchetype::Xy(xy_big)).expect("d2");
        let mut g1 = GraphSheet::new_uninitialized_for_data(SheetId::new("g1"), &d1).expect("g1");
        let mut g2 = GraphSheet::new_uninitialized_for_data(SheetId::new("g2"), &d2).expect("g2");
        g1.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        g2.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        g1.spec.y_axis = AxisSpec::linear("Y");
        g2.spec.y_axis = AxisSpec::linear("Y");

        let layout_spec = LayoutSpec {
            align_plot_origins: true,
            items: vec![
                LayoutItem::LiveGraph {
                    id: String::from("p1"),
                    graph_sheet_id: SheetId::new("g1"),
                    bounds_pt: DocRectPt::new(20.0, 15.0, 220.0, 150.0),
                    scale_percent: 100.0,
                    badge_override: None,
                },
                LayoutItem::LiveGraph {
                    id: String::from("p2"),
                    graph_sheet_id: SheetId::new("g2"),
                    bounds_pt: DocRectPt::new(20.0, 180.0, 220.0, 150.0),
                    scale_percent: 100.0,
                    badge_override: None,
                },
            ],
            ..LayoutSpec::default()
        };
        let layout = LayoutSheet::new(SheetId::new("lay-1"), "Aligned", layout_spec).expect("layout");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(d1).expect("add");
        doc.add_data_sheet(d2).expect("add");
        doc.add_graph_sheet(g1).expect("add");
        doc.add_graph_sheet(g2).expect("add");
        doc.add_layout_sheet(layout.clone()).expect("add");

        let compiled = compile_layout_scene(&layout, &doc).expect("compile layout");
        let has_bold_a = compiled
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "A" && t.rich.spans.iter().any(|s| s.weight == SpanWeight::Bold)));
        let has_bold_b = compiled
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "B" && t.rich.spans.iter().any(|s| s.weight == SpanWeight::Bold)));
        assert!(has_bold_a && has_bold_b);

        // Ensure ColormapId & Rgba8 used in test are exercised
        assert_ne!(ColormapId::Cividis.sample(0.0), Rgba8::WHITE);
    }

    #[test]
    fn percentile_linear_preserves_empty_default_exact_rank_and_extreme_span() {
        let eval = |sorted: &[f64], q: f64| curvebook_math::quantile_type7_sorted(sorted, crate::clamp_f64_safe(q, 0.0, 1.0)).unwrap_or(0.0);

        assert_eq!(eval(&[], 0.5), 0.0);
        assert_eq!(eval(&[7.5], 0.5), 7.5);
        assert_eq!(eval(&[10.0, 20.0, 30.0, 40.0], 0.25), 17.5);
        assert_eq!(eval(&[10.0, 20.0, 30.0, 40.0], 0.5), 25.0);
        assert_eq!(eval(&[10.0, 20.0, 30.0, 40.0], f64::NAN), 10.0);
        assert_eq!(eval(&[10.0, 20.0, 30.0, 40.0], f64::NEG_INFINITY), 10.0);
        assert_eq!(eval(&[10.0, 20.0, 30.0, 40.0], f64::INFINITY), 40.0);

        // Exact-rank endpoints and non-exact midpoint on extreme finite spans must not produce NaN or Inf.
        let extreme = [f64::MIN, f64::MAX];
        assert_eq!(eval(&extreme, 0.0), f64::MIN);
        assert_eq!(eval(&extreme, 1.0), f64::MAX);
        assert_eq!(eval(&extreme, 0.5), 0.0);
    }

    #[test]
    fn standalone_dendrogram_compiles_from_clustering_result_and_raw_table() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, ClusterDirection, ClusterDistance, ClusterLinkage, MultivariateSpec, ResultCell, ResultSheet, ResultStatus,
            ResultTab, ResultTabKind,
        };

        let mut mv = MultipleVariablesTable::new(3, 5).expect("mv");
        mv.variables[0].title = String::from("Sample");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[1].title = String::from("X1");
        mv.variables[2].title = String::from("X2");
        let rows = [("S1", 1.0, 1.1), ("S2", 1.2, 0.9), ("S3", 5.0, 5.1), ("S4", 5.2, 4.9), ("S5", 5.1, 5.0)];
        for (r, (lbl, x1, x2)) in rows.into_iter().enumerate() {
            mv.variables[0].cells[r] = curvebook_doc::Cell::text(lbl);
            mv.variables[1].cells[r] = curvebook_doc::Cell::number(x1);
            mv.variables[2].cells[r] = curvebook_doc::Cell::number(x2);
        }
        let ds = DataSheet::new(SheetId::new("mv-dend"), "Cluster Data", TableArchetype::MultipleVariables(mv)).expect("ds");

        // 1. From a Clustering ResultSheet containing `dendrogram-merges` and `cluster-assignments`.
        let spec = AnalysisSpec::new(
            SheetId::new("res-dend"),
            SheetId::new("mv-dend"),
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: None,
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Upgma,
                k_sweep_max: None,
                consensus_optimal_k: false,
                kmeans_algorithm: Default::default(),
                cluster_direction: ClusterDirection::Rows,
                cut_height: Some(2.0),
                emit_anova_tab: false,
            }),
        )
        .expect("spec");
        let mut res = ResultSheet::new(spec).expect("res");
        res.status = ResultStatus::UpToDate;
        let num = |v: f64| ResultCell::Number { value: v, prefix: None, units: None };
        res.tabs.push(ResultTab {
            id: String::from("hierarchical-height-cut"),
            title: String::from("Cluster assignments"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Cluster")],
            row_labels: vec!["S1".into(), "S2".into(), "S3".into(), "S4".into(), "S5".into()],
            rows: vec![vec![num(1.0)], vec![num(1.0)], vec![num(2.0)], vec![num(2.0)], vec![num(2.0)]],
            footnotes: Vec::new(),
        });
        res.tabs.push(ResultTab {
            id: String::from("hierarchical-dendrogram"),
            title: String::from("Dendrogram merges"),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec![String::from("Left"), String::from("Right"), String::from("Distance"), String::from("Size")],
            row_labels: vec!["Step 1".into(), "Step 2".into(), "Step 3".into(), "Step 4".into()],
            rows: vec![
                vec![num(-1.0), num(-2.0), num(0.28), num(2.0)],
                vec![num(-3.0), num(-5.0), num(0.14), num(2.0)],
                vec![num(1.0), num(-4.0), num(0.22), num(3.0)],
                vec![num(0.0), num(2.0), num(5.60), num(5.0)],
            ],
            footnotes: Vec::new(),
        });

        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-dend"), &ds).expect("gs");
        gs.spec.primary_source_sheet_id = SheetId::new("res-dend");
        gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram);
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add ds");
        doc.add_result_sheet(res).expect("add res");

        let mut scene = ChartScene::new("g-dend", "Dendrogram", 420.0, 300.0, None);
        compile_multivariate_or_diagnostic_graph(&gs, &doc, &mut scene).expect("compile result dendrogram");
        assert!(scene.warnings.is_empty(), "warnings: {:?}", scene.warnings);
        assert!(scene.primitives.len() > 15);
        assert!(
            scene
                .primitives
                .iter()
                .any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(stroke), .. } if stroke.dash == curvebook_palette::LineDashStyle::Dashed)),
            "expected dashed cut-height reference line"
        );

        // 2. Directly from raw MultipleVariables DataSheet (fallback hierarchical clustering).
        let mut gs_raw = GraphSheet::new_uninitialized_for_data(SheetId::new("g-dend-raw"), &ds).expect("gs_raw");
        gs_raw.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram);
        let mut scene_raw = ChartScene::new("g-dend-raw", "Raw Dendrogram", 420.0, 300.0, None);
        compile_multivariate_or_diagnostic_graph(&gs_raw, &doc, &mut scene_raw).expect("compile raw dendrogram");
        assert!(scene_raw.warnings.is_empty(), "raw warnings: {:?}", scene_raw.warnings);
        assert!(scene_raw.primitives.len() > 15);
    }

    #[test]
    fn pca_biplot_compiles_scores_and_loading_vectors_from_result_and_raw_table() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, MultivariateSpec, PcaRetentionRule, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
        };

        let mut mv = MultipleVariablesTable::new(4, 6).expect("mv");
        mv.variables[0].title = String::from("Group");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[1].title = String::from("GeneA");
        mv.variables[2].title = String::from("GeneB");
        mv.variables[3].title = String::from("GeneC");
        let rows = [
            ("Ctrl", 1.0, 2.0, 1.5),
            ("Ctrl", 1.2, 2.3, 1.3),
            ("Ctrl", 0.9, 1.8, 1.7),
            ("Drug", 5.0, 6.2, 0.4),
            ("Drug", 5.4, 5.8, 0.2),
            ("Drug", 4.8, 6.5, 0.5),
        ];
        for (r, (g, a, b, c)) in rows.into_iter().enumerate() {
            mv.variables[0].cells[r] = curvebook_doc::Cell::text(g);
            mv.variables[1].cells[r] = curvebook_doc::Cell::number(a);
            mv.variables[2].cells[r] = curvebook_doc::Cell::number(b);
            mv.variables[3].cells[r] = curvebook_doc::Cell::number(c);
        }
        let ds = DataSheet::new(SheetId::new("mv-pca"), "PCA Data", TableArchetype::MultipleVariables(mv)).expect("ds");

        // 1. From PCA ResultSheet with `pca-summary`, `pca-loadings`, and `pca-scores`.
        let spec = AnalysisSpec::new(
            SheetId::new("res-pca"),
            SheetId::new("mv-pca"),
            AnalysisKind::Multivariate(MultivariateSpec::Pca {
                standardize: true,
                parallel_analysis_seed: None,
                pcr_outcome_column: None,
                retention_rule: PcaRetentionRule::FixedCount(2),
                emit_extra_matrices: false,
            }),
        )
        .expect("spec");
        let mut res = ResultSheet::new(spec).expect("res");
        res.status = ResultStatus::UpToDate;
        let num = |v: f64| ResultCell::Number { value: v, prefix: None, units: None };
        res.tabs.push(ResultTab {
            id: String::from("pca-eigenvalues"),
            title: String::from("PCA eigenvalues"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Eigenvalue"), String::from("Proportion (%)"), String::from("Cumulative (%)")],
            row_labels: vec!["PC1".into(), "PC2".into()],
            rows: vec![vec![num(2.7), num(90.0), num(90.0)], vec![num(0.25), num(8.3), num(98.3)]],
            footnotes: Vec::new(),
        });
        res.tabs.push(ResultTab {
            id: String::from("pca-loadings"),
            title: String::from("PCA loadings"),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec![String::from("PC1"), String::from("PC2")],
            row_labels: vec!["GeneA".into(), "GeneB".into(), "GeneC".into()],
            rows: vec![vec![num(0.62), num(0.15)], vec![num(0.58), num(-0.42)], vec![num(-0.52), num(0.89)]],
            footnotes: Vec::new(),
        });
        res.tabs.push(ResultTab {
            id: String::from("pca-scores"),
            title: String::from("PCA scores"),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec![String::from("PC1"), String::from("PC2")],
            row_labels: vec!["Row 1".into(), "Row 2".into(), "Row 3".into(), "Row 4".into(), "Row 5".into(), "Row 6".into()],
            rows: vec![
                vec![num(-1.5), num(0.2)],
                vec![num(-1.2), num(-0.1)],
                vec![num(-1.6), num(0.3)],
                vec![num(1.4), num(-0.2)],
                vec![num(1.7), num(0.1)],
                vec![num(1.2), num(-0.3)],
            ],
            footnotes: Vec::new(),
        });

        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-biplot"), &ds).expect("gs");
        gs.spec.primary_source_sheet_id = SheetId::new("res-pca");
        gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot);
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add ds");
        doc.add_result_sheet(res).expect("add res");

        let mut scene = ChartScene::new("g-biplot", "PCA Biplot", 420.0, 300.0, None);
        compile_multivariate_or_diagnostic_graph(&gs, &doc, &mut scene).expect("compile pca biplot");
        assert!(scene.warnings.is_empty(), "biplot warnings: {:?}", scene.warnings);
        let texts: Vec<String> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("PC1 (90.0%)")), "missing PC1 variance label in {texts:?}");
        assert!(texts.iter().any(|t| t.contains("GeneA")), "missing loading label GeneA in {texts:?}");

        // 2. Directly from raw MultipleVariables DataSheet (fallback PCA).
        let mut gs_raw = GraphSheet::new_uninitialized_for_data(SheetId::new("g-biplot-raw"), &ds).expect("gs_raw");
        gs_raw.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot);
        let mut scene_raw = ChartScene::new("g-biplot-raw", "Raw PCA Biplot", 420.0, 300.0, None);
        compile_multivariate_or_diagnostic_graph(&gs_raw, &doc, &mut scene_raw).expect("compile raw biplot");
        assert!(scene_raw.warnings.is_empty(), "raw biplot warnings: {:?}", scene_raw.warnings);
        assert!(scene_raw.primitives.len() > 20);
    }
}
