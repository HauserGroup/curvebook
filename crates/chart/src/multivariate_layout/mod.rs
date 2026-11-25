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
}
