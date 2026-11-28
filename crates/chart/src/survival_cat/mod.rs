//! Survival Kaplan-Meier step curves, contingency bars, and parts-of-whole geometry builders,
//! plus shared axis, marker, annotation, and table-grid primitives for `curvebook-chart`.

mod contingency;
mod parts_of_whole;
mod shared;
mod survival;

use curvebook_doc::{GraphKind, GraphSheet, ProjectDocument};

pub(crate) use shared::{
    LegendEntry, PlotMarginConfig, SegmentedFrameSpec, SheetGrid, alpha_fill, draw_colorbar, draw_empty_open_frame, draw_graph_annotations,
    draw_legend_entries, extract_sheet_grid, format_result_cell_with_sig_figs, layout_and_render_segmented_chrome, path_close, path_line, path_move,
    polygon_path, resolve_graph_text, sanitize_range, solid_stroke, text_metrics, update_plot_rect_for_labels,
};

use crate::scene::{ChartError, ChartScene, ChartWarning};

/// Compile a survival, contingency, or parts-of-whole graph sheet into `scene`.
pub fn compile_survival_or_categorical_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    if doc.data_sheet(&graph.spec.primary_source_sheet_id).is_none() && doc.result_sheet(&graph.spec.primary_source_sheet_id).is_none() {
        scene
            .warnings
            .push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{}' not found", graph.spec.primary_source_sheet_id)));
    }

    match graph.spec.kind {
        GraphKind::Survival => survival::compile_survival(graph, doc, scene),
        GraphKind::Contingency => contingency::compile_contingency(graph, doc, scene),
        GraphKind::PartsOfWhole => parts_of_whole::compile_parts_of_whole(graph, doc, scene),
        _ => {
            scene.warnings.push(ChartWarning::new("unexpected_graph_kind", "non-categorical kind dispatched to survival_cat"));
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
        ContingencyTable, DataSheet, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, PartsOfWholeLayout,
        PartsOfWholeTable, ProjectDocument, SheetId, SurvivalTable, TableArchetype,
    };

    use super::compile_survival_or_categorical_graph;
    use crate::scene::{ChartScene, ScenePrimitive};

    #[test]
    fn survival_compiles_curves_censors_ci_and_number_at_risk() {
        let mut surv = SurvivalTable::new(2, 6).expect("surv");
        surv.cohorts[0].title = String::from("Vehicle");
        surv.cohorts[1].title = String::from("Drug");
        for (r, (t, s0, s1)) in
            [(2.0, 1.0, 0.0), (4.0, 1.0, 1.0), (6.0, 0.0, 0.0), (8.0, 1.0, 1.0), (10.0, 1.0, 0.0), (12.0, 1.0, 1.0)].into_iter().enumerate()
        {
            surv.times[r] = curvebook_doc::Cell::number(t);
            surv.cohorts[0].cells[r][0] = curvebook_doc::Cell::number(s0);
            surv.cohorts[1].cells[r][0] = curvebook_doc::Cell::number(s1);
        }
        let ds = DataSheet::new(SheetId::new("surv-1"), "OS", TableArchetype::Survival(surv)).expect("ds");
        let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds).expect("gs");
        for series in &mut graph.spec.series {
            if let GraphicalDataObject::KaplanMeierStep { show_greenwood_ci, ci_dashed, .. } = &mut series.gdo {
                *show_greenwood_ci = true;
                *ci_dashed = false;
            }
        }
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add");
        let mut scene = ChartScene::new("g-1", "OS", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&graph, &doc, &mut scene).expect("compile");

        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Number at risk"))));
        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), stroke: None, .. })));
    }

    #[test]
    fn parts_of_whole_donut_pie_and_stacked_bar_compile() {
        let mut pow = PartsOfWholeTable::new(1, 3).expect("pow");
        pow.slice_labels = vec![String::from("Alpha"), String::from("Beta"), String::from("Gamma")];
        for (r, v) in [50.0, 30.0, 20.0].into_iter().enumerate() {
            pow.data_sets[0].cells[r][0] = curvebook_doc::Cell::number(v);
        }
        let ds = DataSheet::new(SheetId::new("pow-1"), "Share", TableArchetype::PartsOfWhole(pow)).expect("ds");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add");

        let donut_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-d"), &ds).expect("gs");
        let mut d_scene = ChartScene::new("g-d", "Donut", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&donut_gs, &doc, &mut d_scene).expect("compile donut");
        assert!(d_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Total: 100"))));

        let mut pie_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-p"), &ds).expect("gs");
        pie_gs.spec.series[0].gdo = GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Pie, show_center_total: true };
        let mut p_scene = ChartScene::new("g-p", "Pie", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&pie_gs, &doc, &mut p_scene).expect("compile pie");
        assert!(p_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), .. })));
        assert!(!p_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Total: 100"))));

        let mut bar_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-b"), &ds).expect("gs");
        bar_gs.spec.series[0].gdo = GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::StackedFractionBar, show_center_total: false };
        let mut b_scene = ChartScene::new("g-b", "StackedBar", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&bar_gs, &doc, &mut b_scene).expect("compile stacked bar");
        assert!(b_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "100%")));

        let fractions = ds.archetype_fractions_test();
        assert_eq!(fractions.len(), 3);
        let sum: f64 = fractions.iter().map(|s| s.fraction).sum();
        assert!((sum - 1.0).abs() < 1e-12);
    }

    trait PowHelper {
        fn archetype_fractions_test(&self) -> Vec<curvebook_doc::SliceFraction>;
    }
    impl PowHelper for DataSheet {
        fn archetype_fractions_test(&self) -> Vec<curvebook_doc::SliceFraction> {
            match &self.archetype {
                TableArchetype::PartsOfWhole(p) => p.fractions_for_series(0).expect("fractions"),
                _ => Vec::new(),
            }
        }
    }

    #[test]
    fn contingency_stacked_and_proportion_compile() {
        let mut cont = ContingencyTable::new(2, 2).expect("cont");
        cont.row_titles = vec![String::from("Control"), String::from("Treated")];
        cont.data_sets[0].title = String::from("Responder");
        cont.data_sets[1].title = String::from("NonResponder");
        cont.data_sets[0].cells[0][0] = curvebook_doc::Cell::number(10.0);
        cont.data_sets[1].cells[0][0] = curvebook_doc::Cell::number(30.0);
        cont.data_sets[0].cells[1][0] = curvebook_doc::Cell::number(24.0);
        cont.data_sets[1].cells[1][0] = curvebook_doc::Cell::number(16.0);
        let ds = DataSheet::new(SheetId::new("c-1"), "Contingency", TableArchetype::Contingency(cont)).expect("ds");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-c"), &ds).expect("gs");
        gs.spec.kind = GraphKind::Contingency;
        gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
        gs.spec.y_axis = GraphSpec::default_for_data_sheet(&ds).y_axis;
        gs.spec.y_axis.title = String::from("Percent (%)");

        let mut scene = ChartScene::new("g-c", "Contingency", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&gs, &doc, &mut scene).expect("compile");
        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "100")));
    }

    #[test]
    fn narrow_and_tiny_canvases_do_not_panic() {
        assert!((crate::clamp_f64_safe(5.0, 11.0, 6.4) - 6.4).abs() < 1e-12);
        assert!((crate::clamp_f64_safe(f64::NAN, 11.0, 6.4) - 6.4).abs() < 1e-12);
        assert!(crate::clamp_f64_safe(f64::NAN, f64::NAN, f64::NAN).abs() < 1e-12);

        let mut cont = ContingencyTable::new(2, 2).expect("cont");
        cont.row_titles = vec![String::from("Control"), String::from("Treated")];
        cont.data_sets[0].title = String::from("A");
        cont.data_sets[1].title = String::from("B");
        cont.data_sets[0].cells[0][0] = curvebook_doc::Cell::number(10.0);
        cont.data_sets[1].cells[0][0] = curvebook_doc::Cell::number(20.0);
        cont.data_sets[0].cells[1][0] = curvebook_doc::Cell::number(15.0);
        cont.data_sets[1].cells[1][0] = curvebook_doc::Cell::number(25.0);
        let ds = DataSheet::new(SheetId::new("c-narrow"), "Contingency", TableArchetype::Contingency(cont)).expect("ds");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-narrow"), &ds).expect("gs");
        gs.spec.kind = GraphKind::Contingency;
        gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
        gs.spec.y_axis = GraphSpec::default_for_data_sheet(&ds).y_axis;
        gs.spec.y_axis.title = String::from("Count");

        for (w, h) in [(30.0, 200.0), (10.0, 200.0), (200.0, 10.0), (1.0, 1.0)] {
            gs.initialized = true;
            gs.spec = GraphSpec::default_for_data_sheet(&ds);
            gs.spec.y_axis.title = String::from("Count");
            gs.spec.canvas_size_pt = curvebook_doc::SizePt::new(w, h);
            let default_scene = crate::compile_graph(&gs, &doc).expect("narrow default canvas should compile cleanly");
            assert!(!default_scene.primitives.is_empty());

            gs.spec.kind = GraphKind::Contingency;
            gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
            let stacked_scene = crate::compile_graph(&gs, &doc).expect("narrow stacked canvas should compile cleanly");
            assert!(!stacked_scene.primitives.is_empty());
        }
    }

    #[test]
    fn rv31_survival_chart_filters_non_finite_times_before_kaplan_meier() {
        use curvebook_doc::{SurvivalObservation, SurvivalStatus};

        let mixed = [
            SurvivalObservation { row: 0, time: f64::NAN, status: SurvivalStatus::Event },
            SurvivalObservation { row: 1, time: 2.0, status: SurvivalStatus::Event },
            SurvivalObservation { row: 2, time: f64::INFINITY, status: SurvivalStatus::Censored },
            SurvivalObservation { row: 3, time: f64::NEG_INFINITY, status: SurvivalStatus::Event },
            SurvivalObservation { row: 4, time: 5.0, status: SurvivalStatus::Censored },
        ];
        let (n_total, steps) = super::survival::compute_km_steps_guarded("Cohort A", &mixed).expect("valid finite subset");
        assert_eq!(n_total, 2);
        assert!(steps.iter().all(|s| s.time.is_finite() && s.survival.is_finite()));

        let all_non_finite = [
            SurvivalObservation { row: 0, time: f64::NAN, status: SurvivalStatus::Event },
            SurvivalObservation { row: 1, time: f64::INFINITY, status: SurvivalStatus::Censored },
        ];
        assert!(super::survival::compute_km_steps_guarded("Bad", &all_non_finite).is_none());
    }
}
