//! Integration tests for Wave 1 survival/categorical charts, 5-channel multivariate plots,
//! all seven diagnostic plot subtypes, and multi-panel origin-aligned publication layouts.

use curvebook_chart::scene::{ChartScene, PathCmd, ScenePrimitive};
use curvebook_chart::{compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSpec, AxisThreshold, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ConstantValue,
    ContingencyTable, DataSheet, DiagnosticPlotKind, EquationDisplayMode, ErrorBarSpec, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject,
    GroupedArrangement, InfoSheet, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LevelOrderKind, LevelOrderSpec, LinearRegressionSpec,
    MultipleVariablesTable, MultiplicitySpec, MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, PartsOfWholeLayout,
    PartsOfWholeTable, ProjectDocument, RectPt as DocRectPt, ReplicateOverlayStyle, ResultScalarEntry, ResultSheet, ResultStatus, RocCiMethod,
    RocSpec, SheetId, SizeChannelMapping, SubcolumnMode, SummaryMeasure, SurvivalAnalysisSpec, SurvivalTable, TableArchetype, VariableRole,
    WhiskerMethod, XyTable,
};
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, PaletteId, Rgba8};
use curvebook_stats::{multivariate::run_multivariate_analysis, roc::run_roc_analysis, survival::run_survival_analysis};
use curvebook_text::SpanWeight;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn build_survival_doc(with_result_sheet: bool) -> TestResult<(ProjectDocument, GraphSheet)> {
    let mut surv = SurvivalTable::new(2, 10)?;
    surv.time_title = String::from("Months");
    surv.cohorts[0].title = String::from("Placebo");
    surv.cohorts[1].title = String::from("Treatment");
    let rows = [(1.0, 1, 0), (2.0, 1, 1), (3.0, 0, 0), (4.0, 1, 1), (6.0, 1, 0), (8.0, 0, 1), (10.0, 1, 0), (12.0, 1, 1), (15.0, 0, 0), (18.0, 1, 0)];
    for (r, (t, s0, s1)) in rows.into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(s0 as f64);
        surv.cohorts[1].cells[r][0] = Cell::number(s1 as f64);
    }

    let ds = DataSheet::new(SheetId::new("surv-data"), "Overall Survival", TableArchetype::Survival(surv))?;
    let mut doc = ProjectDocument::default();
    if with_result_sheet {
        let surv_spec = SurvivalAnalysisSpec::KaplanMeier {
            asymmetrical_greenwood_ci: false,
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        };
        let mut spec = AnalysisSpec::new(SheetId::new("km-res"), ds.id.clone(), AnalysisKind::Survival(surv_spec.clone()))?;
        spec.selected_data_sets = vec![0, 1];
        let res = run_survival_analysis(&ds, &spec, &surv_spec)?;
        doc.add_result_sheet(res)?;
    }

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("surv-graph"), &ds)?;
    for s in &mut gs.spec.series {
        if let GraphicalDataObject::KaplanMeierStep { show_censored_ticks, show_greenwood_ci, ci_dashed, .. } = &mut s.gdo {
            *show_censored_ticks = true;
            *show_greenwood_ci = true;
            *ci_dashed = false;
        }
    }
    gs.spec.survival_number_at_risk = true;
    doc.add_data_sheet(ds)?;
    doc.add_graph_sheet(gs.clone())?;
    Ok((doc, gs))
}

#[test]
fn survival_direct_and_result_backed_emit_km_ci_censors_and_aligned_at_risk() -> TestResult {
    for backed in [false, true] {
        let (doc, gs) = build_survival_doc(backed)?;
        let scene = compile_graph(&gs, &doc)?;
        assert!(scene.warnings.is_empty(), "warnings={:?}", scene.warnings);

        // Greenwood shaded CI polygons (translucent fills inside plot rect)
        let ribbon_fills = scene
            .primitives
            .iter()
            .filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(f), stroke: None, clip: Some(_), .. } if f.color.a > 0 && f.color.a < 120))
            .count();
        assert!(ribbon_fills >= 2, "expected shaded CI ribbons, got {ribbon_fills}");

        // Censoring ticks are short 7pt vertical segments inside plot_rect
        let censor_ticks = scene
            .primitives
            .iter()
            .filter(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() == 2 => {
                    matches!((path.commands.first(), path.commands.get(1)), (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.x - b.x).abs() < 1e-6 && ((b.y - a.y).abs() - 7.0).abs() < 0.5)
                }
                _ => false,
            })
            .count();
        assert!(censor_ticks >= 3, "expected censoring ticks, got {censor_ticks}");

        // Number at risk label and aligned tick columns beneath X axis
        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Number at risk"))));
        let tick_xs: Vec<f64> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 2 => {
                    match (path.commands.first(), path.commands.get(1)) {
                        (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b)))
                            if (a.y - scene.plot_rect_pt.bottom()).abs() < 0.5 && (b.y - (a.y + 4.0)).abs() < 0.5 =>
                        {
                            Some(a.x)
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        let nar_xs: Vec<f64> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t)
                    if t.anchor_pt.y > scene.plot_rect_pt.bottom() + 45.0 && t.rich.plain_text().chars().all(|c| c.is_ascii_digit()) =>
                {
                    Some(t.anchor_pt.x)
                }
                _ => None,
            })
            .collect();
        assert!(!tick_xs.is_empty() && !nar_xs.is_empty());
        for nx in nar_xs {
            assert!(tick_xs.iter().any(|&tx| (tx - nx).abs() < 0.5), "at-risk column x={nx} did not align with any X tick");
        }
    }
    Ok(())
}

#[test]
fn parts_of_whole_all_four_layouts_and_contingency_modes() -> TestResult {
    let mut pow = PartsOfWholeTable::new(1, 4)?;
    pow.slice_labels = vec![String::from("CD4"), String::from("CD8"), String::from("NK"), String::from("B-Cell")];
    for (r, v) in [45.0, 25.0, 20.0, 10.0].into_iter().enumerate() {
        pow.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let ds = DataSheet::new(SheetId::new("pow"), "Immune Subset", TableArchetype::PartsOfWhole(pow))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    for layout in [PartsOfWholeLayout::Pie, PartsOfWholeLayout::Donut, PartsOfWholeLayout::StackedFractionBar, PartsOfWholeLayout::Waffle10x10] {
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-pow"), &ds)?;
        gs.spec.series[0].gdo = GraphicalDataObject::PartsOfWhole { layout, show_center_total: true };
        let scene = compile_graph(&gs, &doc)?;
        assert!(!scene.primitives.is_empty());
        match layout {
            PartsOfWholeLayout::Pie | PartsOfWholeLayout::Donut => {
                assert!(
                    scene.primitives.iter().any(
                        |p| matches!(p, ScenePrimitive::Path { path, .. } if path.commands.iter().any(|c| matches!(c, PathCmd::CubicTo { .. })))
                    )
                );
                if layout == PartsOfWholeLayout::Donut {
                    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Total: 100"))));
                }
            }
            PartsOfWholeLayout::StackedFractionBar => {
                assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "100%")));
            }
            PartsOfWholeLayout::Waffle10x10 => {
                let filled_tiles = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), clip: None, .. })).count();
                assert!(filled_tiles >= 100, "waffle grid should have >= 100 filled tiles");
            }
        }
    }

    let mut cont = ContingencyTable::new(2, 3)?;
    cont.row_titles = vec![String::from("Low"), String::from("Mid"), String::from("High")];
    cont.data_sets[0].title = String::from("Success");
    cont.data_sets[1].title = String::from("Failure");
    for (r, (a, b)) in [(12.0, 28.0), (20.0, 20.0), (35.0, 5.0)].into_iter().enumerate() {
        cont.data_sets[0].cells[r][0] = Cell::number(a);
        cont.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let cds = DataSheet::new(SheetId::new("cont"), "Dose Contingency", TableArchetype::Contingency(cont))?;
    doc.add_data_sheet(cds.clone())?;
    for (arr, title) in [(GroupedArrangement::Interleaved, "Count"), (GroupedArrangement::Stacked, "Percent (%)")] {
        let mut cgs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cont"), &cds)?;
        cgs.spec.grouped_arrangement = arr;
        cgs.spec.y_axis.title = String::from(title);
        let scene = compile_graph(&cgs, &doc)?;
        assert!(scene.primitives.len() >= 12);
        if title.contains('%') {
            assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "100")));
        }
    }
    Ok(())
}

#[test]
fn multivariate_five_channel_bubble_area_proportional_and_categorical_summary_g7() -> TestResult {
    let mut mv = MultipleVariablesTable::new(5, 8)?;
    mv.variables[0].title = String::from("Conc");
    mv.variables[1].title = String::from("Response");
    mv.variables[2].title = String::from("Subtype");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("Weight");
    mv.variables[4].title = String::from("SampleID");
    mv.variables[4].lock_role(VariableRole::Label);

    let data = [
        (1.0, 12.0, "Alpha", 4.0, "S1"),
        (2.0, 18.0, "Alpha", 9.0, "S2"),
        (2.5, 15.0, "Alpha", 16.0, "S3"),
        (1.5, 22.0, "Alpha", 4.0, "S4"),
        (6.0, 42.0, "Beta", 4.0, "S5"),
        (7.5, 48.0, "Beta", 16.0, "S6"),
        (8.0, 40.0, "Beta", 9.0, "S7"),
        (6.8, 52.0, "Beta", 16.0, "S8"),
    ];
    for (r, (x, y, grp, w, lbl)) in data.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y);
        mv.variables[2].cells[r] = Cell::text(grp);
        mv.variables[3].cells[r] = Cell::number(w);
        mv.variables[4].cells[r] = Cell::text(lbl);
    }
    let ds = DataSheet::new(SheetId::new("mv"), "MultiVar", TableArchetype::MultipleVariables(mv))?;
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;

    // 1. Categorical color + variable area-proportional size + labels + hulls + ellipses
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv"), &ds)?;
    gs.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Categorical { column: 2, palette: PaletteId::OkabeIto, level_order_override: None },
        size: SizeChannelMapping::Variable { column: 3, min_radius_pt: 3.0, max_radius_pt: 12.0 },
        label_column: Some(4),
        x_level_order_override: None,
        show_convex_hulls: true,
        show_confidence_ellipses: true,
    });
    let scene = compile_graph(&gs, &doc)?;
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "S1")));
    assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(s), .. } if s.dash == LineDashStyle::Dashed)));

    // Verify Alpha uses Circle (CubicTo) and Beta uses Square (4-line rect) + verify S4 (w=4 -> r=6) vs S3 (w=16 -> r=12) area ratio
    let has_squares = scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { path, clip: Some(_), .. } if path.commands.len() == 5));
    let circle_radii: Vec<f64> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), stroke: Some(st), clip: Some(_), .. }
                if path.commands.len() == 6 && st.dash == LineDashStyle::Solid =>
            {
                if let (Some(PathCmd::CubicTo { to: bottom, .. }), Some(PathCmd::CubicTo { to: top, .. })) =
                    (path.commands.get(1), path.commands.get(3))
                {
                    Some((bottom.y - top.y).abs() * 0.5)
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    assert!(has_squares, "expected Beta group to use Square marker shape");
    assert!(circle_radii.iter().any(|&r| (r - 6.0).abs() < 0.25) && circle_radii.iter().any(|&r| (r - 12.0).abs() < 0.25), "radii={circle_radii:?}");

    // 2. Continuous Viridis colorbar
    let mut gs_cont = gs.clone();
    if let Some(enc) = &mut gs_cont.spec.multivariate {
        enc.color = ColorChannelMapping::Continuous {
            column: 3,
            ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
            domain_clamp: Some((0.0, 20.0)),
        };
    }
    let scene_cont = compile_graph(&gs_cont, &doc)?;
    assert!(scene_cont.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "20")));

    // 3. CategoricalSummary with x_level_order_override = Explicit(["Beta", "Alpha"])
    for gdo in [
        GraphicalDataObject::BoxAndWhisker {
            whiskers: WhiskerMethod::Tukey1_5Iqr,
            fill_color: Rgba8::rgb(0, 114, 178),
            stroke_color: Rgba8::BLACK,
            show_outliers: true,
            show_all_points: true,
        },
        GraphicalDataObject::Violin {
            fill_color: Rgba8::rgb(0, 158, 115),
            stroke_color: Rgba8::BLACK,
            clip_to_observed_range: true,
            show_median_and_quartiles: true,
            half_violin_with_points: false,
        },
        GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: Rgba8::rgb(213, 94, 0),
            border_color: Rgba8::BLACK,
            border_width_pt: 1.0,
            replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
        },
    ] {
        let mut gs_cat = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cat"), &ds)?;
        gs_cat.spec.multivariate = Some(MultivariateGraphEncoding {
            plot_mode: MultivariatePlotMode::CategoricalSummary { gdo, y_error_bars: Some(ErrorBarSpec::default()) },
            x_column: 2,
            y_column: 1,
            color: ColorChannelMapping::Constant(Rgba8::BLACK),
            size: SizeChannelMapping::Constant { radius_pt: 4.5 },
            label_column: None,
            x_level_order_override: Some(LevelOrderSpec {
                order: LevelOrderKind::Explicit(vec![String::from("Beta"), String::from("Alpha")]),
                reversed: false,
            }),
            show_convex_hulls: false,
            show_confidence_ellipses: false,
        });
        let sc = compile_graph(&gs_cat, &doc)?;
        let beta_x = sc.primitives.iter().find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "Beta" => Some(t.anchor_pt.x),
            _ => None,
        });
        let alpha_x = sc.primitives.iter().find_map(|p| match p {
            ScenePrimitive::Text(t) if t.rich.plain_text() == "Alpha" => Some(t.anchor_pt.x),
            _ => None,
        });
        assert!(beta_x.is_some() && alpha_x.is_some() && beta_x < alpha_x, "explicit order Beta -> Alpha violated");
    }
    Ok(())
}

#[test]
fn all_seven_diagnostic_plot_kinds_compile_rich_geometry() -> TestResult {
    let mut doc = ProjectDocument::default();

    // 1. Heatmap (+ Dendrogram + Colorbar) backed by MultipleVariables AND CorrelationMatrix ResultSheet
    let mut hm_mv = MultipleVariablesTable::new(4, 5)?;
    for c in 0..4 {
        hm_mv.variables[c].title = format!("Gene{c}");
        for r in 0..5 {
            hm_mv.variables[c].cells[r] = Cell::number(((r * 4 + c) as f64) * 0.25 - 1.5);
        }
    }
    let hm_ds = DataSheet::new(SheetId::new("hm-ds"), "Expr", TableArchetype::MultipleVariables(hm_mv))?;
    let mv_spec = MultivariateSpec::CorrelationMatrix { spearman_rank: false, method: None };
    let corr_spec = AnalysisSpec::new(SheetId::new("hm-corr"), hm_ds.id.clone(), AnalysisKind::Multivariate(mv_spec.clone()))?;
    let corr_res = run_multivariate_analysis(&hm_ds, &corr_spec, &mv_spec)?;
    doc.add_data_sheet(hm_ds.clone())?;
    doc.add_result_sheet(corr_res)?;
    let mut hm_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-hm"), &hm_ds)?;
    hm_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    let hm_scene = compile_graph(&hm_gs, &doc)?;
    assert!(hm_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), .. })).count() >= 16);

    // 2. Volcano (-log10(p) vs log2FC + threshold crosshairs)
    let mut vol_mv = MultipleVariablesTable::new(3, 5)?;
    vol_mv.variables[0].title = String::from("log2FC");
    vol_mv.variables[1].title = String::from("p_value");
    vol_mv.variables[2].title = String::from("Target");
    vol_mv.variables[2].lock_role(VariableRole::Label);
    for (r, (fc, pv, lbl)) in
        [(-2.5, 0.0005, "TP53"), (0.2, 0.45, "ACTB"), (2.8, 0.0001, "EGFR"), (1.8, 0.02, "MYC"), (-1.4, 0.2, "GAPDH")].into_iter().enumerate()
    {
        vol_mv.variables[0].cells[r] = Cell::number(fc);
        vol_mv.variables[1].cells[r] = Cell::number(pv);
        vol_mv.variables[2].cells[r] = Cell::text(lbl);
    }
    let vol_ds = DataSheet::new(SheetId::new("vol-ds"), "DEGs", TableArchetype::MultipleVariables(vol_mv))?;
    doc.add_data_sheet(vol_ds.clone())?;
    let mut vol_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-vol"), &vol_ds)?;
    vol_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Volcano);
    if let Some(m) = &mut vol_gs.spec.multivariate {
        m.label_column = Some(2);
    }
    vol_gs.spec.x_axis.thresholds = vec![
        AxisThreshold { value: -1.0, shade_to: None, color: Rgba8::BLACK, dash_style: LineDashStyle::Dashed, stroke_width_pt: 0.9, label: None },
        AxisThreshold { value: 1.0, shade_to: None, color: Rgba8::BLACK, dash_style: LineDashStyle::Dashed, stroke_width_pt: 0.9, label: None },
    ];
    let vol_scene = compile_graph(&vol_gs, &doc)?;
    assert!(vol_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "EGFR")));

    // 3. Bland-Altman
    let mut ba_col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 6)?;
    ba_col.data_sets[0].title = String::from("Method A");
    ba_col.data_sets[1].title = String::from("Method B");
    for (r, (a, b)) in [(10.2, 9.9), (15.4, 15.0), (20.1, 20.3), (25.5, 24.9), (30.0, 29.4), (35.2, 34.8)].into_iter().enumerate() {
        ba_col.data_sets[0].cells[r][0] = Cell::number(a);
        ba_col.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ba_ds = DataSheet::new(SheetId::new("ba-ds"), "Assay Agreement", TableArchetype::Column(ba_col))?;
    doc.add_data_sheet(ba_ds.clone())?;
    let mut ba_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-ba"), &ba_ds)?;
    ba_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman);
    let ba_scene = compile_graph(&ba_gs, &doc)?;
    assert!(ba_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().starts_with("Bias:"))));
    assert!(ba_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("+1.96 SD:"))));

    // 4. ROC Curve backed by RocResult
    let mut roc_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5)?;
    roc_col.data_sets[0].title = String::from("Controls");
    roc_col.data_sets[1].title = String::from("Cases");
    for (r, (ctrl, case)) in [(1.0, 5.2), (1.8, 6.1), (2.2, 7.0), (2.9, 8.4), (3.4, 9.1)].into_iter().enumerate() {
        roc_col.data_sets[0].cells[r][0] = Cell::number(ctrl);
        roc_col.data_sets[1].cells[r][0] = Cell::number(case);
    }
    let roc_ds = DataSheet::new(SheetId::new("roc-ds"), "Biomarker", TableArchetype::Column(roc_col))?;
    let roc_cfg = RocSpec { ci_method: RocCiMethod::DeLong };
    let mut roc_spec = AnalysisSpec::new(SheetId::new("roc-res"), roc_ds.id.clone(), AnalysisKind::RocCurve(roc_cfg))?;
    roc_spec.selected_data_sets = vec![0, 1];
    let roc_res = run_roc_analysis(&roc_ds, &roc_spec, &roc_cfg)?;
    doc.add_data_sheet(roc_ds.clone())?;
    doc.add_result_sheet(roc_res)?;
    let mut roc_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-roc"), &roc_ds)?;
    roc_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
    let roc_scene = compile_graph(&roc_gs, &doc)?;
    assert!(roc_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("AUC = 1.000"))));

    // 5. Forest Plot on Log10 axis + pooled diamond
    let mut for_mv = MultipleVariablesTable::new(4, 3)?;
    for_mv.variables[0].title = String::from("HR");
    for_mv.variables[1].title = String::from("Lower");
    for_mv.variables[2].title = String::from("Upper");
    for_mv.variables[3].title = String::from("Cohort");
    for_mv.variables[3].lock_role(VariableRole::Label);
    for (r, (hr, lo, hi, lbl)) in
        [(0.55, 0.35, 0.86, "Trial A"), (0.70, 0.48, 1.02, "Trial B"), (0.62, 0.40, 0.96, "Trial C")].into_iter().enumerate()
    {
        for_mv.variables[0].cells[r] = Cell::number(hr);
        for_mv.variables[1].cells[r] = Cell::number(lo);
        for_mv.variables[2].cells[r] = Cell::number(hi);
        for_mv.variables[3].cells[r] = Cell::text(lbl);
    }
    let for_ds = DataSheet::new(SheetId::new("for-ds"), "Meta", TableArchetype::MultipleVariables(for_mv))?;
    doc.add_data_sheet(for_ds.clone())?;
    let mut for_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-for"), &for_ds)?;
    for_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot);
    for_gs.spec.x_axis.segments[0].scale = AxisScale::Log10;
    for_gs.spec.x_axis.title = String::from("Hazard Ratio");
    let for_scene = compile_graph(&for_gs, &doc)?;
    assert!(for_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Pooled"))));
    assert!(for_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { path, fill: Some(_), .. } if path.commands.len() == 5)));

    // 6. ResidualPlot & 7. NormalQqPlot
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8)?;
    for r in 0..8 {
        let x = (r + 1) as f64;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + if r % 2 == 0 { 0.4 } else { -0.4 });
    }
    let xy_ds = DataSheet::new(SheetId::new("xy-ds"), "Calibration", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(xy_ds.clone())?;
    let mut res_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-res"), &xy_ds)?;
    res_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot);
    let res_scene = compile_graph(&res_gs, &doc)?;
    assert!(
        res_scene
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(st), clip: Some(_), .. } if st.dash == LineDashStyle::Dashed))
    );

    let mut qq_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-qq"), &xy_ds)?;
    qq_gs.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
    let qq_scene = compile_graph(&qq_gs, &doc)?;
    assert!(
        qq_scene
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(st), clip: Some(_), .. } if st.dash == LineDashStyle::Dashed))
    );
    Ok(())
}

#[test]
fn multipanel_layout_aligns_plot_origins_and_renders_badges_tables_callouts() -> TestResult {
    let mut xy_small = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    let mut xy_wide = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for r in 0..4 {
        let x = (r + 1) as f64;
        xy_small.x_cells[r][0] = Cell::number(x);
        xy_small.data_sets[0].cells[r][0] = Cell::number(x);
        xy_wide.x_cells[r][0] = Cell::number(x);
        xy_wide.data_sets[0].cells[r][0] = Cell::number(x * 12_500.0);
    }
    let d1 = DataSheet::new(SheetId::new("d-small"), "Low Magnitude", TableArchetype::Xy(xy_small))?;
    let d2 = DataSheet::new(SheetId::new("d-wide"), "High Magnitude", TableArchetype::Xy(xy_wide))?;
    let mut g1 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-small"), &d1)?;
    let mut g2 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-wide"), &d2)?;
    g1.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
    g2.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
    g1.spec.y_axis = AxisSpec::linear("Conc (nM)");
    g2.spec.y_axis = AxisSpec::linear("Signal (cps)");

    let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Study Meta")?;
    info.set_constant("Lot", ConstantValue::Text(String::from("L-402")), None)?;

    let mut doc = ProjectDocument::default();
    doc.add_info_sheet(info)?;
    doc.add_data_sheet(d1)?;
    doc.add_data_sheet(d2.clone())?;
    doc.add_graph_sheet(g1.clone())?;
    doc.add_graph_sheet(g2.clone())?;

    // Note g1 has much narrower Y-tick labels than g2 -> sub_scene plot_rect_pt.x differs!
    let sg1 = compile_graph(&g1, &doc)?;
    let sg2 = compile_graph(&g2, &doc)?;
    assert!(sg2.plot_rect_pt.x > sg1.plot_rect_pt.x + 5.0, "expected wider left margin for g2: g1={} g2={}", sg1.plot_rect_pt.x, sg2.plot_rect_pt.x);

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    layout_spec.align_plot_origins = true;
    layout_spec.items = vec![
        LayoutItem::LiveGraph {
            id: String::from("panel-1"),
            graph_sheet_id: SheetId::new("g-small"),
            bounds_pt: DocRectPt::new(16.0, 12.0, 225.0, 155.0),
            scale_percent: 100.0,
            badge_override: None,
        },
        LayoutItem::LiveGraph {
            id: String::from("panel-2"),
            graph_sheet_id: SheetId::new("g-wide"),
            bounds_pt: DocRectPt::new(16.0, 180.0, 225.0, 155.0),
            scale_percent: 100.0,
            badge_override: None,
        },
        LayoutItem::LiveTable {
            id: String::from("table-1"),
            source_sheet_id: d2.id,
            tab_id: None,
            bounds_pt: DocRectPt::new(260.0, 18.0, 210.0, 90.0),
            selected_rows: None,
            selected_cols: None,
            style: curvebook_doc::EmbeddedTableStyle::default(),
        },
        LayoutItem::TextCallout {
            id: String::from("note-1"),
            markdown: String::from("Batch <b>{{info:Lot}}</b> validated."),
            bounds_pt: DocRectPt::new(260.0, 130.0, 210.0, 40.0),
            font_size_pt: 9.5,
        },
    ];
    let layout = LayoutSheet::new(SheetId::new("lay-fig1"), "Figure 1", layout_spec)?;
    doc.add_layout_sheet(layout.clone())?;

    let page = compile_layout(&layout, &doc)?;
    assert!(page.warnings.is_empty(), "warnings={:?}", page.warnings);

    // Check bold badges "A" and "B"
    let has_badge_a = page
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "A" && t.rich.spans.iter().any(|s| s.weight == SpanWeight::Bold)));
    let has_badge_b = page
        .primitives
        .iter()
        .any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "B" && t.rich.spans.iter().any(|s| s.weight == SpanWeight::Bold)));
    assert!(has_badge_a && has_badge_b);

    // Check aligned vertical Y-axis spine X coordinates inside panel 1 (y < 170) and panel 2 (y > 175)
    let spine_x_panel1 = find_vertical_spine_x(&page, 12.0, 167.0).ok_or("panel 1 spine missing")?;
    let spine_x_panel2 = find_vertical_spine_x(&page, 180.0, 335.0).ok_or("panel 2 spine missing")?;
    assert!((spine_x_panel1 - spine_x_panel2).abs() < 1e-4, "origins not aligned: p1={spine_x_panel1}, p2={spine_x_panel2}");

    // Check expanded info hook in TextCallout
    assert!(page.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "Batch L-402 validated.")));

    // Check graph annotation TextBox hook expansion too
    let mut ann_gs = g1;
    ann_gs.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: DocRectPt::new(60.0, 50.0, 120.0, 24.0),
        markdown: String::from("Lot: {{info:Lot}}"),
        bordered: true,
    });
    let ann_scene = compile_graph(&ann_gs, &doc)?;
    assert!(ann_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "Lot: L-402")));
    Ok(())
}

fn find_vertical_spine_x(scene: &ChartScene, y_min: f64, y_max: f64) -> Option<f64> {
    scene.primitives.iter().find_map(|p| match p {
        ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: None } if st.color == Rgba8::BLACK && path.commands.len() == 2 => {
            match (path.commands.first(), path.commands.get(1)) {
                (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b)))
                    if (a.x - b.x).abs() < 1e-6 && (b.y - a.y).abs() > 40.0 && a.y >= y_min && b.y <= y_max =>
                {
                    Some(a.x)
                }
                _ => None,
            }
        }
        _ => None,
    })
}

#[test]
fn survival_graph_renders_model_equation_annotation() -> TestResult {
    let (mut doc, mut g) = build_survival_doc(false)?;
    let lin_spec = AnalysisSpec::new("res-surv-lin", "surv-data", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let mut lin_res = ResultSheet::new(lin_spec)?;
    lin_res.status = ResultStatus::UpToDate;
    lin_res.scalars = vec![
        ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: -0.045, units: None },
        ResultScalarEntry { key: String::from("YIntercept"), data_set_index: Some(0), value: 0.98, units: None },
    ];
    doc.result_sheets.push(lin_res);
    g.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-surv-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(140.0, 50.0, 150.0, 26.0),
        font_size_pt: 9.0,
        bordered: true,
    });
    let scene = compile_graph(&g, &doc)?;
    let joined: String = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(joined.contains("0.045") && joined.contains("0.98"), "joined={joined}");
    Ok(())
}
