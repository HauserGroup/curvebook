//! Integration tests for Wave 22c-2 chart compilation parity:
//! - WF-39: Cox proportional hazards baseline + covariate survival curves via `build_km_from_result_sheet`.
//! - WF-40 / WF-42: `GeneralizedLinearModel` (`Linear`, `Poisson`, `Logistic`) residuals & Q-Q diagnostic plots.
//! - WF-37: Heatmap row clustering linked to `MultivariateSpec::Clustering` (`distance` + `linkage`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::compile_graph;
use curvebook_chart::scene::{ChartScene, PathCmd, ScenePrimitive};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, CoxTieMethod, DataSheet, DiagnosticPlotKind, GlmFamily, GlmSpec, GraphKind,
    GraphSheet, MultipleVariablesTable, MultivariateSpec, ProjectDocument, SheetId, SurvivalAnalysisSpec, TableArchetype, VariableRole,
};
use curvebook_palette::LineDashStyle;
use curvebook_stats::multivariate::run_multivariate_analysis;
use curvebook_stats::regression::run_glm_analysis;
use curvebook_stats::survival::run_survival_analysis;

use super::common::scene_texts;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn cox_ph_baseline_and_covariate_survival_curves_compile() -> TestResult {
    let mut mv = MultipleVariablesTable::new(4, 10)?;
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Dose");
    mv.variables[3].title = String::from("Arm");
    mv.variables[3].lock_role(VariableRole::Categorical);

    let data = [
        (2.0_f64, 1.0_f64, 10.0_f64, "Ctrl"),
        (4.0, 1.0, 12.0, "Ctrl"),
        (6.0, 0.0, 14.0, "Ctrl"),
        (8.0, 1.0, 18.0, "Ctrl"),
        (10.0, 1.0, 20.0, "Ctrl"),
        (5.0, 1.0, 22.0, "Drug"),
        (9.0, 0.0, 25.0, "Drug"),
        (13.0, 1.0, 28.0, "Drug"),
        (16.0, 1.0, 32.0, "Drug"),
        (21.0, 0.0, 35.0, "Drug"),
    ];
    for (r, (t, st, dose, arm)) in data.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(st);
        mv.variables[2].cells[r] = Cell::number(dose);
        mv.variables[3].cells[r] = Cell::text(arm);
    }

    let ds = DataSheet::new(SheetId::new("surv-mv"), "Cox Trial", TableArchetype::MultipleVariables(mv))?;
    let cox_spec = SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column: 0,
        status_column: 1,
        predictor_columns: vec![2, 3],
        reference_levels: vec![(3, String::from("Ctrl"))],
        tie_method: CoxTieMethod::Breslow,
        formula: None,
        covariate_profiles: Vec::new(),
        nested_comparison: None,
        emit_cox_snell_residuals: false,
        ci_method: Default::default(),
        event_code: None,
        censored_code: None,
        treat_other_as: Default::default(),
    };
    let analysis_spec = AnalysisSpec::new(SheetId::new("res-cox"), ds.id.clone(), AnalysisKind::Survival(cox_spec.clone()))?;
    let cox_res = run_survival_analysis(&ds, &analysis_spec, &cox_spec)?;
    assert!(cox_res.fitted_curves.len() >= 3, "expected baseline + 2 covariate curves");

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(cox_res.clone())?;

    for (gid, source_id) in [("g-cox-ds", ds.id.clone()), ("g-cox-res", cox_res.id.clone())] {
        let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new(gid), &ds)?;
        graph.initialized = true;
        graph.spec.primary_source_sheet_id = source_id;
        graph.spec.kind = GraphKind::Survival;
        graph.spec.survival_number_at_risk = true;

        let scene = compile_graph(&graph, &doc)?;
        assert!(
            !scene.warnings.iter().any(|w| w.code == "empty_survival_data"),
            "unexpected empty_survival_data warning for `{gid}`: {:?}",
            scene.warnings
        );

        let texts = scene_texts(&scene);
        assert!(texts.iter().any(|t| t == "Baseline"), "missing Baseline legend label in {texts:?}");
        assert!(texts.iter().any(|t| t == "Dose"), "missing Dose covariate legend label in {texts:?}");
        assert!(texts.iter().any(|t| t.contains("Arm[Drug vs Ctrl]")), "missing categorical covariate label in {texts:?}");
        assert!(!texts.iter().any(|t| t.contains("Number at risk")), "Cox overlay should not draw zeroed KM risk table: {texts:?}");

        let step_paths = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() >= 4))
            .count();
        assert!(step_paths >= 3, "expected >= 3 Cox step curves for `{gid}`, got {step_paths}");
    }
    Ok(())
}

#[test]
fn glm_linear_poisson_logistic_residuals_and_qq_compile() -> TestResult {
    let mut mv = MultipleVariablesTable::new(4, 12)?;
    mv.variables[0].title = String::from("X1");
    mv.variables[1].title = String::from("Y_lin");
    mv.variables[2].title = String::from("Y_pois");
    mv.variables[3].title = String::from("Y_bin");

    let rows = [
        (0.5_f64, 2.2_f64, 2.0_f64, 0.0_f64),
        (1.0, 3.1, 3.0, 0.0),
        (1.5, 4.4, 3.0, 0.0),
        (2.0, 4.9, 5.0, 1.0),
        (2.5, 6.2, 6.0, 0.0),
        (3.0, 6.8, 7.0, 0.0),
        (3.5, 8.3, 9.0, 1.0),
        (4.0, 9.1, 10.0, 1.0),
        (4.5, 9.8, 12.0, 0.0),
        (5.0, 11.4, 15.0, 1.0),
        (5.5, 12.1, 17.0, 1.0),
        (6.0, 13.5, 20.0, 1.0),
    ];
    for (r, (x, y_l, y_p, y_b)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y_l);
        mv.variables[2].cells[r] = Cell::number(y_p);
        mv.variables[3].cells[r] = Cell::number(y_b);
    }

    let ds = DataSheet::new(SheetId::new("glm-mv"), "GLM Diagnostics", TableArchetype::MultipleVariables(mv))?;

    for (suffix, family, resp_col, expect_tab_id) in [
        ("lin", GlmFamily::Linear, 1usize, "glm-linear-residuals"),
        ("pois", GlmFamily::Poisson, 2usize, "glm-poisson-residuals"),
        ("log", GlmFamily::Logistic, 3usize, "glm-logistic-predictions"),
    ] {
        let glm_spec = GlmSpec {
            family,
            response_column: resp_col,
            predictor_columns: vec![0],
            reference_levels: Vec::new(),
            compute_vif: false,
            ..Default::default()
        };
        let spec =
            AnalysisSpec::new(SheetId::new(format!("res-glm-{suffix}")), ds.id.clone(), AnalysisKind::GeneralizedLinearModel(glm_spec.clone()))?;
        let res = run_glm_analysis(&ds, &spec, &glm_spec)?;
        assert!(res.tabs.iter().any(|t| t.id == expect_tab_id), "missing expected tab `{expect_tab_id}`");

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone())?;
        doc.add_result_sheet(res.clone())?;

        for kind in [DiagnosticPlotKind::ResidualPlot, DiagnosticPlotKind::NormalQqPlot] {
            for (link_tag, source_id) in [("res", res.id.clone()), ("ds", ds.id.clone())] {
                let mut g = GraphSheet::new_uninitialized_for_data(SheetId::new(format!("g-{suffix}-{link_tag}")), &ds)?;
                g.initialized = true;
                g.spec.primary_source_sheet_id = source_id;
                g.spec.kind = GraphKind::Diagnostic(kind);

                let scene = compile_graph(&g, &doc)?;
                assert!(!scene.warnings.iter().any(|w| w.code.starts_with("empty_")), "unexpected empty warning on {suffix}/{kind:?}/{link_tag}");

                let point_count = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: Some(_), .. })).count();
                assert!(point_count >= 12, "expected >= 12 plotted points for {suffix}/{kind:?}/{link_tag}, got {point_count}");

                let has_dashed_ref = scene.primitives.iter().any(|p| {
                    matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if path.commands.len() == 2 && st.dash == LineDashStyle::Dashed)
                });
                assert!(has_dashed_ref, "expected dashed reference line on {suffix}/{kind:?}/{link_tag}");

                if family == GlmFamily::Logistic && kind == DiagnosticPlotKind::ResidualPlot {
                    // Response residuals (Observed Y - Predicted probability) must span negative values too,
                    // while fitted probabilities stay within [0, 1].
                    let has_negative_y_tick =
                        scene_texts(&scene).iter().any(|t| t.starts_with('-') && t.trim_start_matches('-').parse::<f64>().is_ok());
                    assert!(has_negative_y_tick, "expected negative response residual ticks for logistic GLM residual plot");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn heatmap_uses_linked_clustering_distance_and_linkage() -> TestResult {
    // Craft 4 rows where Euclidean/Ward pairs R_small_pos ([1,2,3]) with R_small_neg ([3,2,1]),
    // whereas Pearson/Complete pairs R_small_pos ([1,2,3]) with R_scaled_pos ([100,200,300]) (r = +1.0).
    let mut mv = MultipleVariablesTable::new(4, 4)?;
    mv.variables[0].title = String::from("Sample");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("F1");
    mv.variables[2].title = String::from("F2");
    mv.variables[3].title = String::from("F3");

    let matrix = [
        ("RowSmallPos", [1.0_f64, 2.0_f64, 3.0_f64]),
        ("RowMid", [10.0, 15.0, 12.0]),
        ("RowSmallNeg", [3.0, 2.0, 1.0]),
        ("RowScaledPos", [100.0, 200.0, 300.0]),
    ];
    for (r, (lbl, vals)) in matrix.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(lbl);
        mv.variables[1].cells[r] = Cell::number(vals[0]);
        mv.variables[2].cells[r] = Cell::number(vals[1]);
        mv.variables[3].cells[r] = Cell::number(vals[2]);
    }

    let ds = DataSheet::new(SheetId::new("hm-clust-ds"), "Patterns", TableArchetype::MultipleVariables(mv))?;

    let row_y_order = |scene: &ChartScene| -> Vec<String> {
        let mut labels: Vec<(f64, String)> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => {
                    let s = t.rich.plain_text();
                    s.starts_with("Row").then_some((t.anchor_pt.y, s))
                }
                _ => None,
            })
            .collect();
        labels.sort_by(|a, b| a.0.total_cmp(&b.0));
        labels.into_iter().map(|(_, s)| s).collect()
    };

    // 1. Default fallback (no Clustering ResultSheet): Euclidean / Ward.
    let mut doc_default = ProjectDocument::default();
    doc_default.add_data_sheet(ds.clone())?;
    let mut g_default = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm-default"), &ds)?;
    g_default.initialized = true;
    g_default.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    let scene_default = compile_graph(&g_default, &doc_default)?;
    let order_default = row_y_order(&scene_default);
    assert_eq!(order_default.len(), 4);
    let idx = |ord: &[String], name: &str| ord.iter().position(|s| s == name).unwrap() as isize;
    assert_eq!(
        (idx(&order_default, "RowSmallPos") - idx(&order_default, "RowSmallNeg")).abs(),
        1,
        "Euclidean/Ward fallback should pair RowSmallPos & RowSmallNeg: {order_default:?}"
    );

    // 2. Linked Clustering ResultSheet (Pearson / Complete): RowSmallPos & RowScaledPos must become adjacent.
    let clust_spec = MultivariateSpec::Clustering {
        k_means_k: Some(2),
        distance: ClusterDistance::Pearson,
        linkage: ClusterLinkage::Complete,
        k_sweep_max: None,
        consensus_optimal_k: false,
        kmeans_algorithm: Default::default(),
        cluster_direction: Default::default(),
        cut_height: None,
        emit_anova_tab: false,
    };
    let mut analysis = AnalysisSpec::new(SheetId::new("res-clust-pearson"), ds.id.clone(), AnalysisKind::Multivariate(clust_spec.clone()))?;
    analysis.selected_data_sets = vec![1, 2, 3];
    let clust_res = run_multivariate_analysis(&ds, &analysis, &clust_spec)?;

    let mut doc_clust = ProjectDocument::default();
    doc_clust.add_data_sheet(ds.clone())?;
    doc_clust.add_result_sheet(clust_res.clone())?;

    for (gid, src) in [("g-hm-upstream", ds.id.clone()), ("g-hm-direct", clust_res.id.clone())] {
        let mut g = GraphSheet::new_uninitialized_for_data(SheetId::new(gid), &ds)?;
        g.initialized = true;
        g.spec.primary_source_sheet_id = src;
        g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
        let scene = compile_graph(&g, &doc_clust)?;
        let order = row_y_order(&scene);
        assert_eq!(
            (idx(&order, "RowSmallPos") - idx(&order, "RowScaledPos")).abs(),
            1,
            "Pearson/Complete clustering ({gid}) must pair RowSmallPos & RowScaledPos: {order:?}"
        );
        let texts = scene_texts(&scene);
        assert!(texts.iter().any(|t| t == "Clustered: Pearson / Complete"), "missing clustering cue for {gid}: {texts:?}");
        let dendro_segments = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 4 && matches!(path.commands.first(), Some(PathCmd::MoveTo(_)))))
            .count();
        assert!(dendro_segments >= 3, "expected >= 3 dendrogram merge branches for {gid}, got {dendro_segments}");
    }
    Ok(())
}
