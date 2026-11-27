//! Integration & determinism tests for R23 Wave 3B (`curvebook-chart`):
//! - All 6 new `DiagnosticPlotKind` compilers (`HomoscedasticityPlot`, `ActualVsPredictedPlot`,
//!   `ResidualVsRowPlot`, `GapStatisticPlot`, `ParameterCorrelationHeatmap`, `LognormalQqPlot`).
//! - Multiple Logistic diagnostic curves (`glm-logistic-roc`, `glm-logistic-calibration`) and
//!   Cox PH covariate-profile overlays (`cox-profile-survival`, `cox_profile:{idx}`).
//! - §6.2 Theoretical distribution plotting overlays (continuous `Normal`, `StudentT`, `FisherF`,
//!   `ChiSquare` with `LowerTail`/`UpperTail`/`TwoTailed`/`Between` shaded tail polygons, and
//!   discrete `Binomial`/`Poisson` integer-step PMF lollipop / spike / bar & CDF step overlays).
//! - Edge cases: empty tabs, single-row inputs, `NaN`/`Inf` cells, 1×1 correlation matrices,
//!   non-positive lognormal Q–Q samples, and `Dirty`/`Failed` result guards.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, PathCmd, PointPt, RectPt, ScenePrimitive, compile_graph, preflight_check_scene};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, CovariateProfile, CoxTieMethod, DataSheet,
    DescriptiveSpec, DiagnosticPlotKind, DistributionTailShade, FactorialAnovaSpec, GlmFamily, GlmSpec, GraphKind, GraphSheet, GraphicalDataObject,
    GroupedTable, MultipleVariablesTable, MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, ProjectDocument, RegressionOverlaySpec,
    ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SeriesSpec, SheetId, SizePt, SubcolumnMode, SummaryMeasure,
    SurvivalAnalysisSpec, TableArchetype, TheoreticalDistributionFamily, TheoreticalDistributionSpec, VariableRole, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis as execute_fit_analysis};
use curvebook_palette::{LineDashStyle, PaletteId, Rgba8};
use curvebook_stats::anova::{run_factorial_anova, run_one_way_anova};
use curvebook_stats::column::run_descriptive_analysis;
use curvebook_stats::multivariate::run_multivariate_analysis;
use curvebook_stats::regression::run_glm_analysis;
use curvebook_stats::survival::run_survival_analysis;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn assert_all_scene_coords_finite(scene: &ChartScene, label: &str) {
    assert!(scene.width_pt.is_finite() && scene.height_pt.is_finite(), "{label}: non-finite canvas size");
    assert!(
        scene.plot_rect_pt.x.is_finite()
            && scene.plot_rect_pt.y.is_finite()
            && scene.plot_rect_pt.width.is_finite()
            && scene.plot_rect_pt.height.is_finite(),
        "{label}: non-finite plot_rect_pt"
    );
    for (idx, prim) in scene.primitives.iter().enumerate() {
        match prim {
            ScenePrimitive::Path { path, stroke, clip, .. } => {
                if let Some(st) = stroke {
                    assert!(st.width_pt.is_finite() && st.width_pt >= 0.25, "{label}: invalid stroke width at {idx}: {}", st.width_pt);
                }
                if let Some(c) = clip {
                    assert!(c.x.is_finite() && c.y.is_finite() && c.width.is_finite() && c.height.is_finite(), "{label}: non-finite clip at {idx}");
                }
                for cmd in &path.commands {
                    match cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => {
                            assert!(p.x.is_finite() && p.y.is_finite(), "{label}: non-finite path pt at {idx}: {p:?}");
                        }
                        PathCmd::QuadTo { control, to } => {
                            assert!(
                                control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite(),
                                "{label}: non-finite quad at {idx}"
                            );
                        }
                        PathCmd::CubicTo { c1, c2, to } => {
                            assert!(
                                c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite(),
                                "{label}: non-finite cubic at {idx}"
                            );
                        }
                        PathCmd::Close => {}
                    }
                }
            }
            ScenePrimitive::Text(t) => {
                assert!(t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite(), "{label}: non-finite text anchor at {idx}");
                assert!(t.font_size_pt.is_finite() && t.font_size_pt >= 5.0, "{label}: font < 5pt at {idx}: {}", t.font_size_pt);
                assert!(t.rotation_deg.is_finite(), "{label}: non-finite text rotation at {idx}");
            }
            ScenePrimitive::PushClip { rect } => {
                assert!(
                    rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite(),
                    "{label}: non-finite PushClip at {idx}"
                );
            }
            ScenePrimitive::RasterImage(_) | ScenePrimitive::PopClip => {}
        }
    }
}

fn assert_point_bits_eq(a: PointPt, b: PointPt, ctx: &str) {
    assert_eq!(a.x.to_bits(), b.x.to_bits(), "{ctx}: x bit mismatch");
    assert_eq!(a.y.to_bits(), b.y.to_bits(), "{ctx}: y bit mismatch");
}

fn assert_rect_bits_eq(a: RectPt, b: RectPt, ctx: &str) {
    assert_eq!(a.x.to_bits(), b.x.to_bits(), "{ctx}: rect.x bit mismatch");
    assert_eq!(a.y.to_bits(), b.y.to_bits(), "{ctx}: rect.y bit mismatch");
    assert_eq!(a.width.to_bits(), b.width.to_bits(), "{ctx}: rect.width bit mismatch");
    assert_eq!(a.height.to_bits(), b.height.to_bits(), "{ctx}: rect.height bit mismatch");
}

fn assert_scenes_bit_identical(a: &ChartScene, b: &ChartScene, ctx: &str) {
    assert_eq!(a.sheet_id, b.sheet_id, "{ctx}: sheet_id");
    assert_eq!(a.width_pt.to_bits(), b.width_pt.to_bits(), "{ctx}: width");
    assert_eq!(a.height_pt.to_bits(), b.height_pt.to_bits(), "{ctx}: height");
    assert_rect_bits_eq(a.plot_rect_pt, b.plot_rect_pt, ctx);
    assert_eq!(a.primitives.len(), b.primitives.len(), "{ctx}: primitive count");
    assert_eq!(a.warnings, b.warnings, "{ctx}: warnings");
    for (idx, (pa, pb)) in a.primitives.iter().zip(&b.primitives).enumerate() {
        match (pa, pb) {
            (ScenePrimitive::Text(ta), ScenePrimitive::Text(tb)) => {
                assert_eq!(ta.rich, tb.rich, "{ctx}: text {idx}");
                assert_point_bits_eq(ta.anchor_pt, tb.anchor_pt, &format!("{ctx} text {idx}"));
                assert_eq!(ta.font_size_pt.to_bits(), tb.font_size_pt.to_bits(), "{ctx}: font {idx}");
                assert_eq!(ta.rotation_deg.to_bits(), tb.rotation_deg.to_bits(), "{ctx}: rot {idx}");
                assert_eq!(ta.anchor, tb.anchor, "{ctx}: anchor {idx}");
                assert_eq!(ta.color, tb.color, "{ctx}: color {idx}");
            }
            (
                ScenePrimitive::Path { path: p1, fill: f1, stroke: s1, clip: c1 },
                ScenePrimitive::Path { path: p2, fill: f2, stroke: s2, clip: c2 },
            ) => {
                assert_eq!(f1, f2, "{ctx}: fill {idx}");
                assert_eq!(c1.is_some(), c2.is_some(), "{ctx}: clip presence {idx}");
                if let (Some(ra), Some(rb)) = (c1, c2) {
                    assert_rect_bits_eq(*ra, *rb, &format!("{ctx} clip {idx}"));
                }
                assert_eq!(p1.commands.len(), p2.commands.len(), "{ctx}: path cmd count {idx}");
                for (ci, (ca, cb)) in p1.commands.iter().zip(&p2.commands).enumerate() {
                    match (ca, cb) {
                        (PathCmd::MoveTo(u), PathCmd::MoveTo(v)) | (PathCmd::LineTo(u), PathCmd::LineTo(v)) => {
                            assert_point_bits_eq(*u, *v, &format!("{ctx} p{idx} c{ci}"));
                        }
                        (PathCmd::QuadTo { control: c1a, to: t1 }, PathCmd::QuadTo { control: c1b, to: t2 }) => {
                            assert_point_bits_eq(*c1a, *c1b, &format!("{ctx} p{idx} c{ci}"));
                            assert_point_bits_eq(*t1, *t2, &format!("{ctx} p{idx} c{ci}"));
                        }
                        (PathCmd::CubicTo { c1: a1, c2: a2, to: t1 }, PathCmd::CubicTo { c1: b1, c2: b2, to: t2 }) => {
                            assert_point_bits_eq(*a1, *b1, &format!("{ctx} p{idx} c{ci}"));
                            assert_point_bits_eq(*a2, *b2, &format!("{ctx} p{idx} c{ci}"));
                            assert_point_bits_eq(*t1, *t2, &format!("{ctx} p{idx} c{ci}"));
                        }
                        (PathCmd::Close, PathCmd::Close) => {}
                        _ => panic!("{ctx}: mismatched PathCmd variants at prim {idx} cmd {ci}"),
                    }
                }
                match (s1, s2) {
                    (Some(sa), Some(sb)) => {
                        assert_eq!(sa.color, sb.color, "{ctx}: stroke color {idx}");
                        assert_eq!(sa.width_pt.to_bits(), sb.width_pt.to_bits(), "{ctx}: stroke width {idx}");
                        assert_eq!(sa.dash, sb.dash, "{ctx}: stroke dash {idx}");
                    }
                    (None, None) => {}
                    _ => panic!("{ctx}: stroke presence mismatch at {idx}"),
                }
            }
            (ScenePrimitive::PushClip { rect: r1 }, ScenePrimitive::PushClip { rect: r2 }) => {
                assert_rect_bits_eq(*r1, *r2, &format!("{ctx} pushclip {idx}"));
            }
            (ScenePrimitive::PopClip, ScenePrimitive::PopClip) => {}
            _ => panic!("{ctx}: primitive variant mismatch at {idx}"),
        }
    }
}

fn compile_checked_deterministic(graph: &GraphSheet, doc: &ProjectDocument, label: &str) -> TestResult<ChartScene> {
    let s1 = compile_graph(graph, doc)?;
    let s2 = compile_graph(graph, doc)?;
    assert_scenes_bit_identical(&s1, &s2, label);
    assert_all_scene_coords_finite(&s1, label);
    let preflight = preflight_check_scene(&s1);
    assert!(preflight.is_empty(), "{label}: preflight issues: {preflight:?}");
    Ok(s1)
}

fn diag_graph(id: &str, ds: &DataSheet, source_id: SheetId, kind: DiagnosticPlotKind) -> TestResult<GraphSheet> {
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new(id), ds)?;
    gs.initialized = true;
    gs.spec.primary_source_sheet_id = source_id;
    gs.spec.kind = GraphKind::Diagnostic(kind);
    gs.spec.canvas_size_pt = SizePt::new(420.0, 280.0);
    gs.spec.x_axis.title = String::from("X");
    gs.spec.y_axis.title = String::from("Y");
    Ok(gs)
}

fn build_glm_mv_sheet() -> TestResult<DataSheet> {
    let mut mv = MultipleVariablesTable::new(5, 16)?;
    mv.variables[0].title = String::from("X1");
    mv.variables[1].title = String::from("X2");
    mv.variables[2].title = String::from("Y_lin");
    mv.variables[3].title = String::from("Y_pois");
    mv.variables[4].title = String::from("Y_bin");

    let rows = [
        (0.5_f64, 1.2_f64, 3.1_f64, 2.0_f64, 0.0_f64),
        (1.0, 0.8, 3.8, 3.0, 0.0),
        (1.5, 2.1, 5.7, 4.0, 0.0),
        (2.0, 1.5, 6.4, 5.0, 0.0),
        (2.5, 3.0, 8.2, 6.0, 1.0),
        (3.0, 2.2, 8.9, 8.0, 0.0),
        (3.5, 3.8, 11.1, 9.0, 1.0),
        (4.0, 2.9, 11.4, 11.0, 1.0),
        (4.5, 4.1, 13.5, 13.0, 0.0),
        (5.0, 3.6, 14.2, 16.0, 1.0),
        (5.5, 4.9, 16.4, 18.0, 1.0),
        (6.0, 4.2, 16.9, 21.0, 1.0),
        (6.5, 5.4, 18.8, 25.0, 1.0),
        (7.0, 4.8, 19.6, 28.0, 1.0),
        (7.5, 6.0, 21.7, 33.0, 1.0),
        (8.0, 5.5, 22.5, 38.0, 1.0),
    ];
    for (r, (x1, x2, yl, yp, yb)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x1);
        mv.variables[1].cells[r] = Cell::number(x2);
        mv.variables[2].cells[r] = Cell::number(yl);
        mv.variables[3].cells[r] = Cell::number(yp);
        mv.variables[4].cells[r] = Cell::number(yb);
    }
    Ok(DataSheet::new(SheetId::new("mv-glm"), "GLM Cohort", TableArchetype::MultipleVariables(mv))?)
}

fn build_cox_mv_sheet() -> TestResult<DataSheet> {
    let mut mv = MultipleVariablesTable::new(4, 14)?;
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Age");
    mv.variables[3].title = String::from("Arm");
    mv.variables[3].lock_role(VariableRole::Categorical);

    let data = [
        (2.0_f64, 1.0_f64, 48.0_f64, "Ctrl"),
        (4.0, 1.0, 52.0, "Ctrl"),
        (6.0, 0.0, 55.0, "Ctrl"),
        (7.0, 1.0, 60.0, "Ctrl"),
        (9.0, 1.0, 64.0, "Ctrl"),
        (11.0, 1.0, 68.0, "Ctrl"),
        (14.0, 0.0, 71.0, "Ctrl"),
        (5.0, 1.0, 46.0, "Drug"),
        (8.0, 1.0, 51.0, "Drug"),
        (10.0, 0.0, 54.0, "Drug"),
        (13.0, 1.0, 59.0, "Drug"),
        (16.0, 1.0, 63.0, "Drug"),
        (19.0, 1.0, 67.0, "Drug"),
        (23.0, 0.0, 70.0, "Drug"),
    ];
    for (r, (t, st, age, arm)) in data.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(st);
        mv.variables[2].cells[r] = Cell::number(age);
        mv.variables[3].cells[r] = Cell::text(arm);
    }
    Ok(DataSheet::new(SheetId::new("mv-cox"), "Cox Trial", TableArchetype::MultipleVariables(mv))?)
}

#[test]
fn glm_homoscedasticity_actual_vs_predicted_residual_vs_row_and_param_corr_heatmap_compile() -> TestResult {
    let ds = build_glm_mv_sheet()?;
    for (tag, family, resp_col, expect_diag, expect_corr) in [
        ("lin", GlmFamily::Linear, 2_usize, "glm-linear-diagnostics", "glm-linear-correlation"),
        ("log", GlmFamily::Logistic, 4_usize, "glm-logistic-diagnostics", "glm-logistic-correlation"),
        ("pois", GlmFamily::Poisson, 3_usize, "glm-poisson-diagnostics", "glm-poisson-correlation"),
    ] {
        let glm_spec =
            GlmSpec { family, response_column: resp_col, predictor_columns: vec![0, 1], emit_influence_diagnostics: true, ..Default::default() };
        let an_spec = AnalysisSpec::new(SheetId::new(format!("res-{tag}")), ds.id.clone(), AnalysisKind::GeneralizedLinearModel(glm_spec.clone()))?;
        let res = run_glm_analysis(&ds, &an_spec, &glm_spec)?;
        assert!(res.tabs.iter().any(|t| t.id == expect_diag), "missing {expect_diag}");
        assert!(res.tabs.iter().any(|t| t.id == expect_corr), "missing {expect_corr}");

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone())?;
        doc.add_result_sheet(res.clone())?;

        // 1. HomoscedasticityPlot
        let g_homo = diag_graph(&format!("g-homo-{tag}"), &ds, res.id.clone(), DiagnosticPlotKind::HomoscedasticityPlot)?;
        let s_homo = compile_checked_deterministic(&g_homo, &doc, &format!("homo-{tag}"))?;
        assert!(s_homo.warnings.is_empty(), "homo-{tag} warnings: {:?}", s_homo.warnings);
        assert_eq!(s_homo.plotted_points.len(), 16, "homo-{tag} point count");
        let has_dashed = s_homo.primitives.iter().any(
            |p| matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if path.commands.len() == 2 && st.dash == LineDashStyle::Dashed),
        );
        assert!(has_dashed, "homo-{tag}: expected dashed reference line");

        // 2. ActualVsPredictedPlot
        let g_avp = diag_graph(&format!("g-avp-{tag}"), &ds, res.id.clone(), DiagnosticPlotKind::ActualVsPredictedPlot)?;
        let s_avp = compile_checked_deterministic(&g_avp, &doc, &format!("avp-{tag}"))?;
        assert!(s_avp.warnings.is_empty(), "avp-{tag} warnings: {:?}", s_avp.warnings);
        assert!(s_avp.plotted_points.len() >= 16, "avp-{tag} point count");
        let has_identity = s_avp.primitives.iter().any(
            |p| matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if path.commands.len() == 2 && st.dash == LineDashStyle::Dashed),
        );
        assert!(has_identity, "avp-{tag}: expected y=x identity reference line");
        if family == GlmFamily::Logistic {
            let texts = scene_texts(&s_avp);
            assert!(texts.iter().any(|t| t == "Calibration bins"), "logistic AvP should show calibration bins legend: {texts:?}");
            assert!(s_avp.plotted_points.len() > 16, "logistic AvP should include Hosmer-Lemeshow calibration bins");
        }

        // 3. ResidualVsRowPlot
        let g_rvr = diag_graph(&format!("g-rvr-{tag}"), &ds, res.id.clone(), DiagnosticPlotKind::ResidualVsRowPlot)?;
        let s_rvr = compile_checked_deterministic(&g_rvr, &doc, &format!("rvr-{tag}"))?;
        assert!(s_rvr.warnings.is_empty(), "rvr-{tag} warnings: {:?}", s_rvr.warnings);
        assert_eq!(s_rvr.plotted_points.len(), 16, "rvr-{tag} point count");
        let has_zero_line = s_rvr.primitives.iter().any(|p| {
            matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. }
                if st.dash == LineDashStyle::Dashed
                    && path.commands.len() == 2
                    && matches!((path.commands.first(), path.commands.get(1)), (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.y - b.y).abs() < 1e-6))
        });
        assert!(has_zero_line, "rvr-{tag}: expected horizontal y=0 dashed line");

        // 4. ParameterCorrelationHeatmap
        let g_corr = diag_graph(&format!("g-corr-{tag}"), &ds, res.id.clone(), DiagnosticPlotKind::ParameterCorrelationHeatmap)?;
        let s_corr = compile_checked_deterministic(&g_corr, &doc, &format!("corr-{tag}"))?;
        assert!(s_corr.warnings.is_empty(), "corr-{tag} warnings: {:?}", s_corr.warnings);
        let texts = scene_texts(&s_corr);
        assert!(texts.iter().any(|t| t == "1.00"), "corr-{tag}: expected diagonal 1.00 annotation in {texts:?}");
        assert!(texts.iter().any(|t| t == "r"), "corr-{tag}: expected colorbar title 'r' in {texts:?}");
        assert!(texts.iter().any(|t| t == "X1") && texts.iter().any(|t| t == "X2"), "corr-{tag}: missing axis labels in {texts:?}");
        assert!(texts.iter().any(|t| t == "-1") && texts.iter().any(|t| t == "1"), "corr-{tag}: missing [-1, 1] colorbar ticks in {texts:?}");
    }
    Ok(())
}

#[test]
fn anova_homoscedasticity_plot_handles_one_way_and_factorial_spearman_tabs() -> TestResult {
    // 1. One-Way ANOVA (ordinary & lognormal)
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6)?;
    col.data_sets[0].title = String::from("Low");
    col.data_sets[1].title = String::from("Mid");
    col.data_sets[2].title = String::from("High");
    for (r, (a, b, c)) in
        [(5.1, 14.8, 30.2), (5.5, 16.0, 35.5), (4.9, 13.5, 27.1), (5.3, 15.2, 33.8), (5.0, 17.1, 41.0), (5.4, 14.3, 25.9)].into_iter().enumerate()
    {
        col.data_sets[0].cells[r][0] = Cell::number(a);
        col.data_sets[1].cells[r][0] = Cell::number(b);
        col.data_sets[2].cells[r][0] = Cell::number(c);
    }
    let col_ds = DataSheet::new(SheetId::new("col-ow"), "Dose Groups", TableArchetype::Column(col))?;
    for (tag, method) in [("ord", OneWayMethod::OrdinaryAnova), ("ln", OneWayMethod::LognormalOrdinary)] {
        let ow_spec = OneWayAnovaSpec { method, ..Default::default() };
        let an_spec = AnalysisSpec::new(SheetId::new(format!("res-ow-{tag}")), col_ds.id.clone(), AnalysisKind::OneWayAnova(ow_spec.clone()))?;
        let res = run_one_way_anova(&col_ds, &an_spec, &ow_spec)?;
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(col_ds.clone())?;
        doc.add_result_sheet(res.clone())?;

        let gs = diag_graph(&format!("g-ow-{tag}"), &col_ds, res.id.clone(), DiagnosticPlotKind::HomoscedasticityPlot)?;
        let sc = compile_checked_deterministic(&gs, &doc, &format!("ow-homo-{tag}"))?;
        assert!(sc.warnings.is_empty(), "ow-homo-{tag}: {:?}", sc.warnings);
        assert_eq!(sc.plotted_points.len(), 18);
    }

    // 2. Factorial ANOVA with Spearman heteroscedasticity test enabled
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2)?;
    grp.data_sets[0].title = String::from("Vehicle");
    grp.data_sets[1].title = String::from("Treated");
    grp.row_titles = vec![String::from("WT"), String::from("KO")];
    let vals = [([10.0, 11.0, 9.5], [20.0, 22.5, 18.0]), ([14.0, 15.2, 13.1], [40.0, 48.0, 33.0])];
    for (r, (v0, v1)) in vals.into_iter().enumerate() {
        for s in 0..3 {
            grp.data_sets[0].cells[r][s] = Cell::number(v0[s]);
            grp.data_sets[1].cells[r][s] = Cell::number(v1[s]);
        }
    }
    let grp_ds = DataSheet::new(SheetId::new("grp-fac"), "Two-Way Assay", TableArchetype::Grouped(grp))?;
    let fac_spec = FactorialAnovaSpec { heteroscedasticity_test: true, emit_consolidated_data_tab: true, ..Default::default() };
    let an_spec = AnalysisSpec::new(SheetId::new("res-fac"), grp_ds.id.clone(), AnalysisKind::FactorialAnova(fac_spec.clone()))?;
    let res = run_factorial_anova(&grp_ds, &an_spec, &fac_spec)?;
    assert!(res.tabs.iter().any(|t| t.id == "heteroscedasticity"));

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(grp_ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    let gs = diag_graph("g-fac-homo", &grp_ds, res.id.clone(), DiagnosticPlotKind::HomoscedasticityPlot)?;
    let sc = compile_checked_deterministic(&gs, &doc, "factorial-homo")?;
    assert!(sc.warnings.is_empty(), "factorial-homo warnings: {:?}", sc.warnings);
    assert_eq!(sc.plotted_points.len(), 12);
    let texts = scene_texts(&sc);
    assert!(texts.iter().any(|t| t.contains("Spearman")), "expected Spearman heteroscedasticity badge in {texts:?}");
    Ok(())
}

#[test]
fn gap_statistic_plot_renders_error_bars_and_optimal_k_marker() -> TestResult {
    let mut mv = MultipleVariablesTable::new(2, 18)?;
    mv.variables[0].title = String::from("Dim1");
    mv.variables[1].title = String::from("Dim2");
    let pts = [
        (0.0, 0.1),
        (0.2, -0.1),
        (-0.1, 0.2),
        (0.1, 0.0),
        (-0.2, -0.1),
        (0.05, 0.15),
        (10.0, 10.1),
        (10.2, 9.9),
        (9.8, 10.2),
        (10.1, 10.0),
        (9.9, 9.8),
        (10.15, 10.05),
        (20.0, 0.1),
        (20.2, -0.2),
        (19.9, 0.2),
        (20.1, 0.0),
        (19.8, -0.1),
        (20.05, 0.15),
    ];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(y);
    }
    let ds = DataSheet::new(SheetId::new("mv-clust"), "Three Clusters", TableArchetype::MultipleVariables(mv))?;
    let clust_spec = MultivariateSpec::Clustering {
        k_means_k: None,
        distance: ClusterDistance::Euclidean,
        linkage: ClusterLinkage::Ward,
        k_sweep_max: Some(5),
        consensus_optimal_k: true,
        kmeans_algorithm: Default::default(),
        cluster_direction: Default::default(),
        cut_height: None,
        emit_anova_tab: false,
    };
    let an_spec = AnalysisSpec::new(SheetId::new("res-gap"), ds.id.clone(), AnalysisKind::Multivariate(clust_spec.clone()))?;
    let res = run_multivariate_analysis(&ds, &an_spec, &clust_spec)?;
    assert!(res.tabs.iter().any(|t| t.id == "gap-statistic"));

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    let gs = diag_graph("g-gap", &ds, res.id.clone(), DiagnosticPlotKind::GapStatisticPlot)?;
    let sc = compile_checked_deterministic(&gs, &doc, "gap-statistic")?;
    assert!(sc.warnings.is_empty(), "gap warnings: {:?}", sc.warnings);
    assert_eq!(sc.plotted_points.len(), 5, "expected k=1..=5 Gap points");
    let texts = scene_texts(&sc);
    assert!(texts.iter().any(|t| t.starts_with("k* = ")), "expected optimal-k callout in {texts:?}");

    let has_opt_v_dashed = sc.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. }
            if st.dash == LineDashStyle::Dashed
                && path.commands.len() == 2
                && matches!((path.commands.first(), path.commands.get(1)), (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.x - b.x).abs() < 1e-6))
    });
    assert!(has_opt_v_dashed, "expected vertical dashed optimal-k line");
    Ok(())
}

#[test]
fn lognormal_qq_plot_consumes_lognormal_qq_tab_and_rejects_non_positive() -> TestResult {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 8)?;
    col.data_sets[0].title = String::from("Positive");
    col.data_sets[1].title = String::from("HasZero");
    let pos = [1.2, 2.5, 4.1, 6.8, 11.2, 18.5, 31.0, 54.0];
    let bad = [0.0, 2.5, 4.1, 6.8, 11.2, 18.5, 31.0, 54.0];
    for r in 0..8 {
        col.data_sets[0].cells[r][0] = Cell::number(pos[r]);
        col.data_sets[1].cells[r][0] = Cell::number(bad[r]);
    }
    let ds = DataSheet::new(SheetId::new("col-lnqq"), "Lognormal Sample", TableArchetype::Column(col))?;
    let desc_spec = DescriptiveSpec { emit_qq_tabs: true, ..Default::default() };
    let mut an_spec = AnalysisSpec::new(SheetId::new("res-lnqq"), ds.id.clone(), AnalysisKind::DescriptiveStats(desc_spec.clone()))?;
    an_spec.selected_data_sets = vec![0, 1];
    let res = run_descriptive_analysis(&ds, &an_spec, &desc_spec)?;
    assert!(res.tabs.iter().any(|t| t.id == "lognormal-qq"));

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds.clone())?;
    doc.add_result_sheet(res.clone())?;

    // Dataset 0 (strictly positive) compiles cleanly with 8 Blom lognormal Q-Q points + dashed line.
    let g_pos = diag_graph("g-lnqq-pos", &ds, res.id.clone(), DiagnosticPlotKind::LognormalQqPlot)?;
    let s_pos = compile_checked_deterministic(&g_pos, &doc, "lognormal-qq-pos")?;
    assert!(s_pos.warnings.is_empty(), "lognormal-qq-pos warnings: {:?}", s_pos.warnings);
    assert_eq!(s_pos.plotted_points.len(), 8);
    let has_ref_line = s_pos.primitives.iter().any(
        |p| matches!(p, ScenePrimitive::Path { path, stroke: Some(st), clip: Some(_), .. } if path.commands.len() == 2 && st.dash == LineDashStyle::Dashed),
    );
    assert!(has_ref_line, "expected dashed Blom reference line on LognormalQqPlot");

    // Raw dataset 1 (contains 0.0) without result sheet warns cleanly and draws empty open frame.
    let mut doc_raw = ProjectDocument::default();
    doc_raw.add_data_sheet(ds.clone())?;
    let mut g_bad = diag_graph("g-lnqq-bad", &ds, ds.id.clone(), DiagnosticPlotKind::LognormalQqPlot)?;
    g_bad.spec.series[0].data_set_index = 1;
    let s_bad = compile_checked_deterministic(&g_bad, &doc_raw, "lognormal-qq-nonpositive")?;
    assert!(s_bad.warnings.iter().any(|w| w.code == "non_positive_lognormal_qq"));
    assert!(s_bad.plotted_points.is_empty());
    Ok(())
}

#[test]
fn logistic_roc_tab_calibration_and_cox_profiles_and_residuals_compile_cleanly() -> TestResult {
    // 1. Multiple Logistic ROC (both FittedCurveOverlay and tab-only `glm-logistic-roc` fallback) + calibration-only `glm-logistic-calibration`.
    let glm_ds = build_glm_mv_sheet()?;
    let log_spec = GlmSpec {
        family: GlmFamily::Logistic,
        response_column: 4,
        predictor_columns: vec![0, 1],
        emit_influence_diagnostics: true,
        ..Default::default()
    };
    let log_an = AnalysisSpec::new(SheetId::new("res-log"), glm_ds.id.clone(), AnalysisKind::GeneralizedLinearModel(log_spec.clone()))?;
    let log_res = run_glm_analysis(&glm_ds, &log_an, &log_spec)?;
    assert!(log_res.tabs.iter().any(|t| t.id == "glm-logistic-roc"));
    assert!(log_res.tabs.iter().any(|t| t.id == "glm-logistic-calibration"));

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(glm_ds.clone())?;
    doc.add_result_sheet(log_res.clone())?;

    let g_roc = diag_graph("g-log-roc", &glm_ds, log_res.id.clone(), DiagnosticPlotKind::RocCurve)?;
    let s_roc = compile_checked_deterministic(&g_roc, &doc, "logistic-roc-full")?;
    assert!(s_roc.warnings.is_empty(), "logistic ROC warnings: {:?}", s_roc.warnings);
    assert!(scene_texts(&s_roc).iter().any(|t| t.contains("AUC =")));

    // Tab-only ROC (`fitted_curves` stripped, leaving only `glm-logistic-roc` tab).
    let mut tab_only_roc = log_res.clone();
    tab_only_roc.id = SheetId::new("res-log-tab-roc");
    tab_only_roc.fitted_curves.clear();
    doc.add_result_sheet(tab_only_roc.clone())?;
    let g_roc_tab = diag_graph("g-log-roc-tab", &glm_ds, tab_only_roc.id.clone(), DiagnosticPlotKind::RocCurve)?;
    let s_roc_tab = compile_checked_deterministic(&g_roc_tab, &doc, "logistic-roc-tab-only")?;
    assert!(s_roc_tab.warnings.is_empty(), "tab-only ROC warnings: {:?}", s_roc_tab.warnings);
    assert!(scene_texts(&s_roc_tab).iter().any(|t| t.contains("AUC =")));

    // Calibration-only (`glm-logistic-calibration` retained, row diagnostics stripped).
    let mut cal_only = log_res;
    cal_only.id = SheetId::new("res-log-cal-only");
    cal_only.tabs.retain(|t| t.id == "glm-logistic-calibration");
    doc.add_result_sheet(cal_only.clone())?;
    let g_cal = diag_graph("g-log-cal", &glm_ds, cal_only.id.clone(), DiagnosticPlotKind::ActualVsPredictedPlot)?;
    let s_cal = compile_checked_deterministic(&g_cal, &doc, "logistic-calibration-only")?;
    assert!(s_cal.warnings.is_empty(), "calibration-only warnings: {:?}", s_cal.warnings);
    assert!(!s_cal.plotted_points.is_empty(), "expected Hosmer-Lemeshow bin markers");

    // 2. Cox PH covariate-profile overlays (`cox_profile:0`, `cox_profile:1`, `cox-profile-survival`)
    //    + Cox residuals (`cox-snell-residuals`, `cox-covariate-residuals`) + `cox-covariance`.
    let cox_ds = build_cox_mv_sheet()?;
    let cox_spec = SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column: 0,
        status_column: 1,
        predictor_columns: vec![2, 3],
        reference_levels: vec![(3, String::from("Ctrl"))],
        tie_method: CoxTieMethod::Efron,
        formula: None,
        covariate_profiles: vec![
            CovariateProfile {
                label: String::from("Young Drug"),
                numeric_values: vec![(2, 48.0)],
                categorical_values: vec![(3, String::from("Drug"))],
            },
            CovariateProfile {
                label: String::from("Older Ctrl"),
                numeric_values: vec![(2, 68.0)],
                categorical_values: vec![(3, String::from("Ctrl"))],
            },
        ],
        nested_comparison: None,
        emit_cox_snell_residuals: true,
        ci_method: Default::default(),
        event_code: None,
        censored_code: None,
        treat_other_as: Default::default(),
    };
    let cox_an = AnalysisSpec::new(SheetId::new("res-cox"), cox_ds.id.clone(), AnalysisKind::Survival(cox_spec.clone()))?;
    let cox_res = run_survival_analysis(&cox_ds, &cox_an, &cox_spec)?;
    assert!(cox_res.fitted_curves.iter().any(|c| c.equation_id == "cox_profile:0"));
    assert!(cox_res.fitted_curves.iter().any(|c| c.equation_id == "cox_profile:1"));
    assert!(cox_res.tabs.iter().any(|t| t.id == "cox-profile-survival"));
    assert!(cox_res.tabs.iter().any(|t| t.id == "cox-snell-residuals"));
    assert!(cox_res.tabs.iter().any(|t| t.id == "cox-covariate-residuals"));
    assert!(cox_res.tabs.iter().any(|t| t.id == "cox-covariance"));

    let mut cox_doc = ProjectDocument::default();
    cox_doc.add_data_sheet(cox_ds.clone())?;
    cox_doc.add_result_sheet(cox_res.clone())?;

    let mut g_surv = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cox-prof"), &cox_ds)?;
    g_surv.initialized = true;
    g_surv.spec.primary_source_sheet_id = cox_res.id.clone();
    g_surv.spec.kind = GraphKind::Survival;
    g_surv.spec.canvas_size_pt = SizePt::new(440.0, 290.0);
    let s_surv = compile_checked_deterministic(&g_surv, &cox_doc, "cox-profile-survival")?;
    let surv_texts = scene_texts(&s_surv);
    assert!(surv_texts.iter().any(|t| t == "Baseline"), "missing Baseline in {surv_texts:?}");
    assert!(surv_texts.iter().any(|t| t == "Young Drug"), "missing 'Young Drug' profile label in {surv_texts:?}");
    assert!(surv_texts.iter().any(|t| t == "Older Ctrl"), "missing 'Older Ctrl' profile label in {surv_texts:?}");

    // Tab-only `cox-profile-survival` fallback when `fitted_curves` is empty.
    let mut cox_tab_only = cox_res.clone();
    cox_tab_only.id = SheetId::new("res-cox-tab-only");
    cox_tab_only.fitted_curves.clear();
    cox_doc.add_result_sheet(cox_tab_only.clone())?;
    let mut g_surv_tab = g_surv.clone();
    g_surv_tab.spec.primary_source_sheet_id = cox_tab_only.id.clone();
    let s_surv_tab = compile_checked_deterministic(&g_surv_tab, &cox_doc, "cox-profile-tab-only")?;
    let surv_tab_texts = scene_texts(&s_surv_tab);
    assert!(surv_tab_texts.iter().any(|t| t == "Young Drug"), "missing 'Young Drug' from tab fallback: {surv_tab_texts:?}");
    assert!(surv_tab_texts.iter().any(|t| t == "Older Ctrl"), "missing 'Older Ctrl' from tab fallback: {surv_tab_texts:?}");

    // Cox ResidualVsRowPlot (both `cox-snell-residuals` and `cox-covariate-residuals`-only).
    let g_cox_rvr = diag_graph("g-cox-rvr", &cox_ds, cox_res.id.clone(), DiagnosticPlotKind::ResidualVsRowPlot)?;
    let s_cox_rvr = compile_checked_deterministic(&g_cox_rvr, &cox_doc, "cox-snell-rvr")?;
    assert_eq!(s_cox_rvr.plotted_points.len(), 14);

    let mut cox_cov_only = cox_res.clone();
    cox_cov_only.id = SheetId::new("res-cox-cov-resid");
    cox_cov_only.tabs.retain(|t| t.id == "cox-covariate-residuals");
    cox_doc.add_result_sheet(cox_cov_only.clone())?;
    let g_cox_cov = diag_graph("g-cox-cov", &cox_ds, cox_cov_only.id.clone(), DiagnosticPlotKind::ResidualVsRowPlot)?;
    let s_cox_cov = compile_checked_deterministic(&g_cox_cov, &cox_doc, "cox-covariate-rvr")?;
    assert_eq!(s_cox_cov.plotted_points.len(), 14);

    // Cox ParameterCorrelationHeatmap (`cox-covariance`).
    let g_cox_corr = diag_graph("g-cox-corr", &cox_ds, cox_res.id.clone(), DiagnosticPlotKind::ParameterCorrelationHeatmap)?;
    let s_cox_corr = compile_checked_deterministic(&g_cox_corr, &cox_doc, "cox-corr-heatmap")?;
    assert!(scene_texts(&s_cox_corr).iter().any(|t| t == "1.00"));
    Ok(())
}

#[test]
fn theoretical_distribution_continuous_tail_shading_and_discrete_pmf_overlays_render_deterministically() -> TestResult {
    // Minimal dummy source table for `execute_fit_analysis`.
    let mut dummy_xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    dummy_xy.x_cells[0][0] = Cell::number(0.0);
    dummy_xy.data_sets[0].cells[0][0] = Cell::number(0.0);
    dummy_xy.x_cells[1][0] = Cell::number(1.0);
    dummy_xy.data_sets[0].cells[1][0] = Cell::number(1.0);
    let dummy_ds = DataSheet::new(SheetId::new("xy-seed"), "Seed", TableArchetype::Xy(dummy_xy))?;

    // 1. All 4 continuous distributions (Normal, StudentT, FisherF, ChiSquare) across all 4 shading modes.
    let continuous_cases = [
        (
            "normal-lower",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::Normal,
                parameters: vec![0.0, 1.0],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::LowerTail { cutoff: -1.645 },
            },
            1_usize,
        ),
        (
            "normal-between",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::Normal,
                parameters: vec![10.0, 2.5],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::Between { lower: 7.5, upper: 12.5 },
            },
            1_usize,
        ),
        (
            "student-t-twotailed",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::StudentT,
                parameters: vec![12.0],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::TwoTailed { lower: -2.179, upper: 2.179 },
            },
            2_usize,
        ),
        (
            "fisher-f-upper",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::FisherF,
                parameters: vec![4.0, 20.0],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::UpperTail { cutoff: 2.866 },
            },
            1_usize,
        ),
        (
            "chisq-upper",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::ChiSquare,
                parameters: vec![5.0],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::UpperTail { cutoff: 11.07 },
            },
            1_usize,
        ),
    ];

    for (tag, dist_spec, expected_shaded_polys) in continuous_cases {
        let mut nl = NonlinearFitSpec::new("plot_distribution");
        nl.plot_distribution = Some(dist_spec);
        let an_spec = AnalysisSpec::new(SheetId::new(format!("res-{tag}")), dummy_ds.id.clone(), AnalysisKind::NonlinearRegression(nl))?;
        let res = execute_fit_analysis(&an_spec, &dummy_ds, &HookContext::default())?;

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(dummy_ds.clone())?;
        doc.add_result_sheet(res.clone())?;

        // Pattern A: Single-series XY graph with `regression_overlay` (auto-renders tail polygons).
        let mut g_ov = GraphSheet::new_uninitialized_for_data(SheetId::new(format!("g-ov-{tag}")), &dummy_ds)?;
        g_ov.initialized = true;
        g_ov.spec.canvas_size_pt = SizePt::new(420.0, 280.0);
        g_ov.spec.series[0].show_raw_data = false;
        g_ov.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: res.id.clone(),
            sample_segments: 256,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: true,
        });
        let s_ov = compile_checked_deterministic(&g_ov, &doc, &format!("dist-ov-{tag}"))?;
        assert!(s_ov.warnings.is_empty(), "dist-ov-{tag} warnings: {:?}", s_ov.warnings);

        let shaded_polys = s_ov
            .primitives
            .iter()
            .filter(|p| {
                matches!(p, ScenePrimitive::Path { path, fill: Some(f), stroke: None, clip: Some(_) }
                    if f.color.a > 0 && f.color.a < 255 && path.commands.len() >= 5 && matches!(path.commands.last(), Some(PathCmd::Close)))
            })
            .count();
        assert_eq!(shaded_polys, expected_shaded_polys, "dist-ov-{tag}: expected {expected_shaded_polys} shaded tail polygon(s), got {shaded_polys}");

        // Pattern B: Direct XY graph sourced from `res.id` (`analyzable_data`) with multi-dataset series.
        let analyzable = res.analyzable_data.as_ref().expect("analyzable_data");
        let mut g_dir = GraphSheet::new_uninitialized_for_data(SheetId::new(format!("g-dir-{tag}")), analyzable)?;
        g_dir.initialized = true;
        g_dir.spec.canvas_size_pt = SizePt::new(420.0, 280.0);
        g_dir.spec.primary_source_sheet_id = res.id.clone();
        for s in &mut g_dir.spec.series {
            s.source_sheet_id = res.id.clone();
        }
        let s_dir = compile_checked_deterministic(&g_dir, &doc, &format!("dist-dir-{tag}"))?;
        assert!(s_dir.warnings.is_empty(), "dist-dir-{tag} warnings: {:?}", s_dir.warnings);
        let dir_shaded_polys = s_dir
            .primitives
            .iter()
            .filter(|p| {
                matches!(p, ScenePrimitive::Path { path, fill: Some(f), stroke: None, clip: Some(_) }
                    if f.color.a > 0 && f.color.a < 255 && path.commands.len() >= 5 && matches!(path.commands.last(), Some(PathCmd::Close)))
            })
            .count();
        assert_eq!(dir_shaded_polys, expected_shaded_polys, "dist-dir-{tag}: expected {expected_shaded_polys} shaded tail polygon(s)");
    }

    // 2. Discrete distributions (`Binomial` & `Poisson`): integer-step PMF lollipop / spike, bar, and CDF step modes.
    for (tag, dist_spec, bar_mode) in [
        (
            "binom-lollipop",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::Binomial,
                parameters: vec![10.0, 0.4],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::Between { lower: 3.0, upper: 6.0 },
            },
            false,
        ),
        (
            "binom-bars",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::Binomial,
                parameters: vec![8.0, 0.5],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::None,
            },
            true,
        ),
        (
            "poisson-upper-lollipop",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::Poisson,
                parameters: vec![4.0],
                parameter_hooks: Vec::new(),
                cumulative: false,
                tail_shade: DistributionTailShade::UpperTail { cutoff: 6.0 },
            },
            false,
        ),
        (
            "poisson-cdf-step",
            TheoreticalDistributionSpec {
                family: TheoreticalDistributionFamily::Poisson,
                parameters: vec![3.0],
                parameter_hooks: Vec::new(),
                cumulative: true,
                tail_shade: DistributionTailShade::LowerTail { cutoff: 4.0 },
            },
            false,
        ),
    ] {
        let mut nl = NonlinearFitSpec::new("plot_distribution");
        nl.plot_distribution = Some(dist_spec.clone());
        let an_spec = AnalysisSpec::new(SheetId::new(format!("res-{tag}")), dummy_ds.id.clone(), AnalysisKind::NonlinearRegression(nl))?;
        let res = execute_fit_analysis(&an_spec, &dummy_ds, &HookContext::default())?;

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(dummy_ds.clone())?;
        doc.add_result_sheet(res.clone())?;

        let mut g = GraphSheet::new_uninitialized_for_data(SheetId::new(format!("g-{tag}")), &dummy_ds)?;
        g.initialized = true;
        g.spec.canvas_size_pt = SizePt::new(420.0, 280.0);
        g.spec.series[0].show_raw_data = false;
        if bar_mode {
            g.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
                summary: SummaryMeasure::Mean,
                fill_color: PaletteId::OkabeIto.color_at(0),
                border_color: Rgba8::BLACK,
                border_width_pt: 0.75,
                replicate_overlay: None,
            };
        }
        g.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: res.id.clone(),
            sample_segments: 256,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: true,
        });
        let sc = compile_checked_deterministic(&g, &doc, tag)?;
        assert!(sc.warnings.is_empty(), "{tag} warnings: {:?}", sc.warnings);

        if bar_mode {
            let bars = sc
                .primitives
                .iter()
                .filter(|p| matches!(p, ScenePrimitive::Path { path, fill: Some(_), clip: Some(_), .. } if path.commands.len() == 5))
                .count();
            assert_eq!(bars, 9, "{tag}: expected 9 discrete Binomial(8, 0.5) PMF bars (k=0..=8), got {bars}");
        } else if !dist_spec.cumulative {
            let vertical_stems = sc
                .primitives
                .iter()
                .filter(|p| {
                    matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) }
                        if path.commands.len() == 2
                            && matches!((path.commands.first(), path.commands.get(1)), (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() > 0.3))
                })
                .count();
            assert!(vertical_stems >= 5, "{tag}: expected discrete integer-step PMF spikes, got {vertical_stems}");
        } else {
            let has_step_curve = sc
                .primitives
                .iter()
                .any(|p| matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: Some(_) } if path.commands.len() >= 10));
            assert!(has_step_curve, "{tag}: expected discrete CDF staircase curve");
        }
    }
    Ok(())
}

#[test]
fn wave3b_edge_cases_empty_single_row_nan_1x1_corr_and_dirty_failed_guards() -> TestResult {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 1)?;
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.data_sets[0].cells[0][0] = Cell::number(2.0);
    let seed_ds = DataSheet::new(SheetId::new("xy-edge"), "Edge", TableArchetype::Xy(xy))?;

    let make_res = |id: &str, tab: ResultTab, status: ResultStatus| -> TestResult<ResultSheet> {
        let spec = AnalysisSpec::new(SheetId::new(id), seed_ds.id.clone(), AnalysisKind::GeneralizedLinearModel(GlmSpec::default()))?;
        let mut r = ResultSheet::new(spec)?;
        r.status = status;
        r.tabs.push(tab);
        Ok(r)
    };

    // 1. Empty tabs for all 6 new DiagnosticPlotKinds -> clean warning + empty open frame, zero panics.
    let empty_cases = [
        (
            DiagnosticPlotKind::HomoscedasticityPlot,
            "glm-linear-diagnostics",
            ResultTabKind::Diagnostics,
            vec!["Observed Y", "Predicted Y", "Raw residual"],
        ),
        (
            DiagnosticPlotKind::ActualVsPredictedPlot,
            "glm-linear-diagnostics",
            ResultTabKind::Diagnostics,
            vec!["Observed Y", "Predicted Y", "Raw residual"],
        ),
        (
            DiagnosticPlotKind::ResidualVsRowPlot,
            "glm-linear-diagnostics",
            ResultTabKind::Diagnostics,
            vec!["Observed Y", "Predicted Y", "Raw residual"],
        ),
        (
            DiagnosticPlotKind::GapStatisticPlot,
            "gap-statistic",
            ResultTabKind::CurveCoordinates,
            vec!["Clusters (k)", "log(W)", "E[log(W*)]", "Gap(k)", "SE(sim)"],
        ),
        (DiagnosticPlotKind::ParameterCorrelationHeatmap, "glm-linear-correlation", ResultTabKind::CovarianceMatrix, vec!["Intercept"]),
        (
            DiagnosticPlotKind::LognormalQqPlot,
            "lognormal-qq",
            ResultTabKind::NormalQq,
            vec!["Data Set", "Order Index", "Lognormal Expected (ln)", "Lognormal Observed ln(Y)"],
        ),
    ];
    for (idx, (kind, tab_id, tab_kind, hdrs)) in empty_cases.into_iter().enumerate() {
        let tab = ResultTab {
            id: String::from(tab_id),
            title: String::from("Empty"),
            kind: tab_kind,
            column_headers: hdrs.into_iter().map(String::from).collect(),
            row_labels: Vec::new(),
            rows: Vec::new(),
            footnotes: Vec::new(),
        };
        let res = make_res(&format!("res-empty-{idx}"), tab, ResultStatus::UpToDate)?;
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(seed_ds.clone())?;
        doc.add_result_sheet(res.clone())?;
        let gs = diag_graph(&format!("g-empty-{idx}"), &seed_ds, res.id.clone(), kind)?;
        let sc = compile_checked_deterministic(&gs, &doc, &format!("empty-{kind:?}"))?;
        assert!(!sc.warnings.is_empty(), "empty-{kind:?} must emit empty/insufficient warning");
    }

    // 2. Single-row + NaN-injected tabs across Homoscedasticity, ActualVsPredicted, ResidualVsRow, GapStatistic.
    let single_diag_tab = ResultTab {
        id: String::from("glm-linear-diagnostics"),
        title: String::from("Single + NaN"),
        kind: ResultTabKind::Diagnostics,
        column_headers: vec![String::from("Observed Y"), String::from("Predicted Y"), String::from("Raw residual")],
        row_labels: vec![String::from("Row 1"), String::from("Row 2")],
        rows: vec![
            vec![
                ResultCell::Number { value: 5.5, prefix: None, units: None },
                ResultCell::Number { value: 5.0, prefix: None, units: None },
                ResultCell::Number { value: 0.5, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                ResultCell::Number { value: f64::INFINITY, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
            ],
        ],
        footnotes: Vec::new(),
    };
    let single_res = make_res("res-single", single_diag_tab, ResultStatus::UpToDate)?;
    let mut doc_s = ProjectDocument::default();
    doc_s.add_data_sheet(seed_ds.clone())?;
    doc_s.add_result_sheet(single_res.clone())?;
    for kind in [DiagnosticPlotKind::HomoscedasticityPlot, DiagnosticPlotKind::ActualVsPredictedPlot, DiagnosticPlotKind::ResidualVsRowPlot] {
        let gs = diag_graph("g-single", &seed_ds, single_res.id.clone(), kind)?;
        let sc = compile_checked_deterministic(&gs, &doc_s, &format!("single-{kind:?}"))?;
        assert!(sc.warnings.is_empty(), "single-{kind:?}: {:?}", sc.warnings);
        assert_eq!(sc.plotted_points.len(), 1);
    }

    // Single-row + NaN SE GapStatisticPlot
    let single_gap_tab = ResultTab {
        id: String::from("gap-statistic"),
        title: String::from("Single Gap"),
        kind: ResultTabKind::CurveCoordinates,
        column_headers: vec![
            String::from("Clusters (k)"),
            String::from("log(W)"),
            String::from("E[log(W*)]"),
            String::from("Gap(k)"),
            String::from("SE(sim)"),
        ],
        row_labels: vec![String::from("k = 1"), String::from("k = 2")],
        rows: vec![
            vec![
                ResultCell::Number { value: 1.0, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
                ResultCell::Number { value: 2.4, prefix: None, units: None },
                ResultCell::Number { value: 0.4, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: 2.0, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                ResultCell::Number { value: 0.05, prefix: None, units: None },
            ],
        ],
        footnotes: Vec::new(),
    };
    let single_gap_res = make_res("res-single-gap", single_gap_tab, ResultStatus::UpToDate)?;
    doc_s.add_result_sheet(single_gap_res.clone())?;
    let g_sgap = diag_graph("g-sgap", &seed_ds, single_gap_res.id.clone(), DiagnosticPlotKind::GapStatisticPlot)?;
    let s_sgap = compile_checked_deterministic(&g_sgap, &doc_s, "single-gap-nan")?;
    assert_eq!(s_sgap.plotted_points.len(), 1);

    // 3. 1x1 correlation matrix + 2x2 matrix with NaN off-diagonal
    let corr_1x1_tab = ResultTab {
        id: String::from("glm-linear-correlation"),
        title: String::from("1x1 Correlation"),
        kind: ResultTabKind::CovarianceMatrix,
        column_headers: vec![String::from("Intercept")],
        row_labels: vec![String::from("Intercept")],
        rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }]],
        footnotes: Vec::new(),
    };
    let corr_1x1_res = make_res("res-corr-1x1", corr_1x1_tab, ResultStatus::UpToDate)?;
    doc_s.add_result_sheet(corr_1x1_res.clone())?;
    let g_c1 = diag_graph("g-c1", &seed_ds, corr_1x1_res.id.clone(), DiagnosticPlotKind::ParameterCorrelationHeatmap)?;
    let s_c1 = compile_checked_deterministic(&g_c1, &doc_s, "corr-1x1")?;
    assert!(s_c1.warnings.is_empty());
    let c1_texts = scene_texts(&s_c1);
    assert!(c1_texts.iter().any(|t| t == "Intercept") && c1_texts.iter().any(|t| t == "1.00"));

    let corr_nan_tab = ResultTab {
        id: String::from("glm-linear-correlation"),
        title: String::from("NaN Off-Diag Correlation"),
        kind: ResultTabKind::CovarianceMatrix,
        column_headers: vec![String::from("B0"), String::from("B1")],
        row_labels: vec![String::from("B0"), String::from("B1")],
        rows: vec![
            vec![ResultCell::Number { value: 1.0, prefix: None, units: None }, ResultCell::Number { value: f64::NAN, prefix: None, units: None }],
            vec![ResultCell::Number { value: f64::NAN, prefix: None, units: None }, ResultCell::Number { value: 1.0, prefix: None, units: None }],
        ],
        footnotes: Vec::new(),
    };
    let corr_nan_res = make_res("res-corr-nan", corr_nan_tab, ResultStatus::UpToDate)?;
    doc_s.add_result_sheet(corr_nan_res.clone())?;
    let g_cnan = diag_graph("g-cnan", &seed_ds, corr_nan_res.id.clone(), DiagnosticPlotKind::ParameterCorrelationHeatmap)?;
    let s_cnan = compile_checked_deterministic(&g_cnan, &doc_s, "corr-nan-offdiag")?;
    assert!(s_cnan.warnings.is_empty());

    // 4. Dirty / Failed result status blocks stale data on new diagnostic plots.
    let mut dirty_res = single_res.clone();
    dirty_res.id = SheetId::new("res-dirty");
    dirty_res.status = ResultStatus::Dirty;
    doc_s.add_result_sheet(dirty_res.clone())?;
    let g_dirty = diag_graph("g-dirty", &seed_ds, dirty_res.id.clone(), DiagnosticPlotKind::ActualVsPredictedPlot)?;
    let s_dirty = compile_checked_deterministic(&g_dirty, &doc_s, "dirty-avp")?;
    assert!(s_dirty.plotted_points.is_empty());
    assert!(s_dirty.warnings.iter().any(|w| w.code.starts_with("pending_")));

    let mut failed_res = single_res.clone();
    failed_res.id = SheetId::new("res-failed");
    failed_res.status = ResultStatus::Failed { code: curvebook_doc::ErrorCode::BadInput, message: String::from("convergence failure") };
    doc_s.add_result_sheet(failed_res.clone())?;
    let g_failed = diag_graph("g-failed", &seed_ds, failed_res.id.clone(), DiagnosticPlotKind::ResidualVsRowPlot)?;
    let s_failed = compile_checked_deterministic(&g_failed, &doc_s, "failed-rvr")?;
    assert!(s_failed.plotted_points.is_empty());
    assert!(s_failed.warnings.iter().any(|w| w.code.starts_with("failed_")));

    // 5. Explicit multi-series RegressionOverlaySpec on TwoTailed Normal distribution (series 0, 1, 2).
    let mut nl_two = NonlinearFitSpec::new("plot_distribution");
    nl_two.plot_distribution = Some(TheoreticalDistributionSpec {
        family: TheoreticalDistributionFamily::Normal,
        parameters: vec![0.0, 1.0],
        parameter_hooks: Vec::new(),
        cumulative: false,
        tail_shade: DistributionTailShade::TwoTailed { lower: -1.96, upper: 1.96 },
    });
    let an_two = AnalysisSpec::new(SheetId::new("res-norm-multi"), seed_ds.id.clone(), AnalysisKind::NonlinearRegression(nl_two))?;
    let res_two = execute_fit_analysis(&an_two, &seed_ds, &HookContext::default())?;
    doc_s.add_result_sheet(res_two.clone())?;

    let mut g_multi = GraphSheet::new_uninitialized_for_data(SheetId::new("g-norm-multi"), &seed_ds)?;
    g_multi.initialized = true;
    g_multi.spec.canvas_size_pt = SizePt::new(420.0, 280.0);
    let base_series = g_multi.spec.series[0].clone();
    g_multi.spec.series = (0..3)
        .map(|idx| SeriesSpec {
            data_set_index: idx,
            show_raw_data: false,
            legend_label: Some(match idx {
                0 => String::from("PDF"),
                1 => String::from("Lower 2.5%"),
                _ => String::from("Upper 2.5%"),
            }),
            regression_overlay: Some(RegressionOverlaySpec {
                result_sheet_id: res_two.id.clone(),
                sample_segments: 256,
                show_confidence_band: false,
                show_prediction_band: false,
                shaded_ribbon: true,
            }),
            ..base_series.clone()
        })
        .collect();
    let s_multi = compile_checked_deterministic(&g_multi, &doc_s, "dist-multi-series-twotailed")?;
    assert!(s_multi.warnings.is_empty(), "dist-multi-series warnings: {:?}", s_multi.warnings);
    let multi_polys = s_multi
        .primitives
        .iter()
        .filter(|p| {
            matches!(p, ScenePrimitive::Path { path, fill: Some(f), stroke: None, clip: Some(_) }
                if f.color.a > 0 && f.color.a < 255 && path.commands.len() >= 5 && matches!(path.commands.last(), Some(PathCmd::Close)))
        })
        .count();
    assert_eq!(multi_polys, 2, "expected 2 explicit shaded tail polygons, got {multi_polys}");

    // 6. Unstandardized PCA covariance matrix normalized into [-1, 1] ParameterCorrelationHeatmap.
    let mut mv_pca = MultipleVariablesTable::new(3, 6)?;
    mv_pca.variables[0].title = String::from("V1");
    mv_pca.variables[1].title = String::from("V2");
    mv_pca.variables[2].title = String::from("V3");
    for (r, (a, b, c)) in [(10.0, 200.0, 5.0), (20.0, 410.0, 4.0), (30.0, 590.0, 6.0), (40.0, 810.0, 3.0), (50.0, 990.0, 7.0), (60.0, 1210.0, 2.0)]
        .into_iter()
        .enumerate()
    {
        mv_pca.variables[0].cells[r] = Cell::number(a);
        mv_pca.variables[1].cells[r] = Cell::number(b);
        mv_pca.variables[2].cells[r] = Cell::number(c);
    }
    let pca_ds = DataSheet::new(SheetId::new("mv-pca"), "PCA", TableArchetype::MultipleVariables(mv_pca))?;
    let pca_spec = MultivariateSpec::Pca {
        standardize: false,
        parallel_analysis_seed: None,
        pcr_outcome_column: None,
        retention_rule: Default::default(),
        emit_extra_matrices: true,
    };
    let pca_an = AnalysisSpec::new(SheetId::new("res-pca"), pca_ds.id.clone(), AnalysisKind::Multivariate(pca_spec.clone()))?;
    let pca_res = run_multivariate_analysis(&pca_ds, &pca_an, &pca_spec)?;
    assert!(pca_res.tabs.iter().any(|t| t.id == "pca-cov-corr-matrix"));
    let mut doc_pca = ProjectDocument::default();
    doc_pca.add_data_sheet(pca_ds.clone())?;
    doc_pca.add_result_sheet(pca_res.clone())?;
    let g_pca = diag_graph("g-pca-corr", &pca_ds, pca_res.id.clone(), DiagnosticPlotKind::ParameterCorrelationHeatmap)?;
    let s_pca = compile_checked_deterministic(&g_pca, &doc_pca, "pca-cov-normalized-heatmap")?;
    assert!(s_pca.warnings.is_empty());
    assert!(scene_texts(&s_pca).iter().any(|t| t == "1.00"));
    Ok(())
}

#[test]
fn wave3b_regression_lognormal_qq_normal_only_tab_and_mv_anova_homoscedasticity() -> TestResult {
    let num = |value: f64| ResultCell::Number { value, prefix: None, units: None };

    // Regression 1: LognormalQqPlot must NOT silently plot linear Normal Q-Q coordinates when a
    // linked ResultSheet has only a Normal-only `ResultTabKind::NormalQq` tab (no "lognormal"/"ln(" headers).
    // Case 1A: source has negative values -> falls through to raw lognormal validation and rejects with `non_positive_lognormal_qq`.
    let mut col_neg = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5)?;
    col_neg.data_sets[0].title = String::from("Signed");
    for (r, v) in [-2.0, -0.5, 1.0, 2.5, 4.0].into_iter().enumerate() {
        col_neg.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let ds_neg = DataSheet::new(SheetId::new("col-neg"), "Signed", TableArchetype::Column(col_neg))?;
    let normal_only_tab = ResultTab {
        id: String::from("normal-qq"),
        title: String::from("Normal Q-Q"),
        kind: ResultTabKind::NormalQq,
        column_headers: vec![String::from("Data Set"), String::from("Order Index"), String::from("Normal Expected"), String::from("Normal Observed")],
        row_labels: Vec::new(),
        rows: [(-1.2, -2.0), (-0.5, -0.5), (0.0, 1.0), (0.5, 2.5), (1.2, 4.0)]
            .into_iter()
            .enumerate()
            .map(|(i, (e, o))| vec![ResultCell::Text(String::from("Signed")), num((i + 1) as f64), num(e), num(o)])
            .collect(),
        footnotes: Vec::new(),
    };
    let an_neg = AnalysisSpec::new(
        SheetId::new("res-norm-only-neg"),
        ds_neg.id.clone(),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { emit_qq_tabs: true, ..Default::default() }),
    )?;
    let mut res_neg = ResultSheet::new(an_neg)?;
    res_neg.status = ResultStatus::UpToDate;
    res_neg.tabs.push(normal_only_tab);
    let mut doc_r1 = ProjectDocument::default();
    doc_r1.add_data_sheet(ds_neg.clone())?;
    doc_r1.add_result_sheet(res_neg.clone())?;
    let g_r1a = diag_graph("g-lnqq-neg-normal-tab", &ds_neg, res_neg.id.clone(), DiagnosticPlotKind::LognormalQqPlot)?;
    let s_r1a = compile_checked_deterministic(&g_r1a, &doc_r1, "lnqq-normal-only-tab-neg")?;
    assert!(s_r1a.plotted_points.is_empty(), "must not plot linear Normal Q-Q rows for negative data");
    assert!(
        s_r1a.warnings.iter().any(|w| w.code == "non_positive_lognormal_qq"),
        "expected non_positive_lognormal_qq warning, got {:?}",
        s_r1a.warnings
    );

    // Case 1B: strictly positive skewed data with a Normal-only Q-Q tab -> falls through to true ln(Y) Blom quantiles.
    let mut col_pos = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5)?;
    col_pos.data_sets[0].title = String::from("PosSkew");
    for (r, v) in [1.0, 2.5, 6.0, 18.0, 55.0].into_iter().enumerate() {
        col_pos.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let ds_pos = DataSheet::new(SheetId::new("col-pos"), "PosSkew", TableArchetype::Column(col_pos))?;
    let normal_only_pos_tab = ResultTab {
        id: String::from("normal-qq"),
        title: String::from("Normal Q-Q"),
        kind: ResultTabKind::NormalQq,
        column_headers: vec![String::from("Data Set"), String::from("Order Index"), String::from("Normal Expected"), String::from("Normal Observed")],
        row_labels: Vec::new(),
        rows: [(0.0, 1.0), (10.0, 2.5), (20.0, 6.0), (35.0, 18.0), (50.0, 55.0)]
            .into_iter()
            .enumerate()
            .map(|(i, (e, o))| vec![ResultCell::Text(String::from("PosSkew")), num((i + 1) as f64), num(e), num(o)])
            .collect(),
        footnotes: Vec::new(),
    };
    let an_pos = AnalysisSpec::new(
        SheetId::new("res-norm-only-pos"),
        ds_pos.id.clone(),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { emit_qq_tabs: true, ..Default::default() }),
    )?;
    let mut res_pos = ResultSheet::new(an_pos)?;
    res_pos.status = ResultStatus::UpToDate;
    res_pos.tabs.push(normal_only_pos_tab);
    doc_r1.add_data_sheet(ds_pos.clone())?;
    doc_r1.add_result_sheet(res_pos.clone())?;
    let g_r1b = diag_graph("g-lnqq-pos-normal-tab", &ds_pos, res_pos.id.clone(), DiagnosticPlotKind::LognormalQqPlot)?;
    let s_r1b = compile_checked_deterministic(&g_r1b, &doc_r1, "lnqq-normal-only-tab-pos")?;
    assert!(s_r1b.warnings.is_empty(), "lnqq pos warnings: {:?}", s_r1b.warnings);
    assert_eq!(s_r1b.plotted_points.len(), 5);
    assert!(
        s_r1b.plotted_points.iter().all(|p| !p.value_summary.contains("55")),
        "expected ln(Y) quantiles (~4.01), not untransformed linear 55.0: {:?}",
        s_r1b.plotted_points.iter().map(|p| &p.value_summary).collect::<Vec<_>>()
    );

    // Regression 2: HomoscedasticityPlot on OneWayAnova with MultipleVariablesTable treats each
    // selected numeric column as a group (mirroring `extract_numeric_columns`), skipping Categorical/Label.
    let mut mv_anova = MultipleVariablesTable::new(4, 5)?;
    mv_anova.variables[0].title = String::from("GroupA");
    mv_anova.variables[0].lock_role(VariableRole::Continuous);
    mv_anova.variables[1].title = String::from("GroupB");
    mv_anova.variables[1].lock_role(VariableRole::Continuous);
    mv_anova.variables[2].title = String::from("GroupC");
    mv_anova.variables[2].lock_role(VariableRole::Continuous);
    mv_anova.variables[3].title = String::from("BatchTag");
    mv_anova.variables[3].lock_role(VariableRole::Categorical);
    for r in 0..5 {
        mv_anova.variables[0].cells[r] = Cell::number(10.0 + r as f64 * 0.5);
        mv_anova.variables[1].cells[r] = Cell::number(20.0 + r as f64 * 1.5);
        mv_anova.variables[2].cells[r] = Cell::number(35.0 + r as f64 * 3.0);
        mv_anova.variables[3].cells[r] = Cell::text(if r % 2 == 0 { "X" } else { "Y" });
    }
    let ds_mv_anova = DataSheet::new(SheetId::new("mv-oneway"), "MV One-Way", TableArchetype::MultipleVariables(mv_anova))?;
    let ow_ord = OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..Default::default() };
    let an_mv_ord = AnalysisSpec::new(SheetId::new("res-mv-ord"), ds_mv_anova.id.clone(), AnalysisKind::OneWayAnova(ow_ord.clone()))?;
    let res_mv_ord = run_one_way_anova(&ds_mv_anova, &an_mv_ord, &ow_ord)?;

    let ow_log = OneWayAnovaSpec { method: OneWayMethod::LognormalOrdinary, ..Default::default() };
    let mut an_mv_log = AnalysisSpec::new(SheetId::new("res-mv-log"), ds_mv_anova.id.clone(), AnalysisKind::OneWayAnova(ow_log.clone()))?;
    an_mv_log.selected_data_sets = vec![0, 2];
    let res_mv_log = run_one_way_anova(&ds_mv_anova, &an_mv_log, &ow_log)?;

    let mut doc_r2 = ProjectDocument::default();
    doc_r2.add_data_sheet(ds_mv_anova.clone())?;
    doc_r2.add_result_sheet(res_mv_ord.clone())?;
    doc_r2.add_result_sheet(res_mv_log.clone())?;

    let g_mv_ord = diag_graph("g-mv-homo-ord", &ds_mv_anova, res_mv_ord.id.clone(), DiagnosticPlotKind::HomoscedasticityPlot)?;
    let s_mv_ord = compile_checked_deterministic(&g_mv_ord, &doc_r2, "mv-oneway-ordinary-homoscedasticity")?;
    assert!(s_mv_ord.warnings.is_empty(), "mv one-way ordinary warnings: {:?}", s_mv_ord.warnings);
    assert_eq!(s_mv_ord.plotted_points.len(), 15, "expected 3 numeric columns x 5 rows = 15 points");
    assert!(s_mv_ord.plotted_points.iter().any(|p| p.series_label == "GroupA"));
    assert!(s_mv_ord.plotted_points.iter().any(|p| p.series_label == "GroupB"));
    assert!(s_mv_ord.plotted_points.iter().any(|p| p.series_label == "GroupC"));
    assert!(s_mv_ord.plotted_points.iter().all(|p| p.series_label != "X" && p.series_label != "Y"));

    let g_mv_log = diag_graph("g-mv-homo-log", &ds_mv_anova, res_mv_log.id.clone(), DiagnosticPlotKind::HomoscedasticityPlot)?;
    let s_mv_log = compile_checked_deterministic(&g_mv_log, &doc_r2, "mv-oneway-lognormal-homoscedasticity")?;
    assert!(s_mv_log.warnings.is_empty(), "mv one-way lognormal warnings: {:?}", s_mv_log.warnings);
    assert_eq!(s_mv_log.plotted_points.len(), 10, "expected 2 selected numeric columns x 5 rows = 10 points");
    assert!(s_mv_log.plotted_points.iter().all(|p| p.series_label == "GroupA" || p.series_label == "GroupC"));
    Ok(())
}

#[test]
fn wave3b_regression_param_corr_missing_diag_and_direct_distribution_legend() -> TestResult {
    // Regression 3: ParameterCorrelationHeatmap must NOT misclassify a k×k raw MV table with
    // all-None diagonal and values in [-1, 1] as an already-computed correlation matrix.
    let mut mv_miss_diag = MultipleVariablesTable::new(3, 3)?;
    mv_miss_diag.variables[0].title = String::from("X1");
    mv_miss_diag.variables[1].title = String::from("X2");
    mv_miss_diag.variables[2].title = String::from("X3");
    let raw_rows: [[Option<f64>; 3]; 3] = [[None, Some(0.2), Some(-0.4)], [Some(0.8), None, Some(0.5)], [Some(-0.1), Some(0.9), None]];
    for (r, row) in raw_rows.iter().enumerate() {
        for (c, val) in row.iter().enumerate() {
            if let Some(v) = val {
                mv_miss_diag.variables[c].cells[r] = Cell::number(*v);
            }
        }
    }
    let ds_miss = DataSheet::new(SheetId::new("mv-miss-diag"), "Missing Diag Raw", TableArchetype::MultipleVariables(mv_miss_diag))?;
    let mut doc_r3 = ProjectDocument::default();
    doc_r3.add_data_sheet(ds_miss.clone())?;
    let mut g_r3 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-corr-miss-diag"), &ds_miss)?;
    g_r3.initialized = true;
    g_r3.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::ParameterCorrelationHeatmap);
    g_r3.spec.canvas_size_pt = SizePt::new(380.0, 320.0);
    let s_r3 = compile_checked_deterministic(&g_r3, &doc_r3, "param-corr-missing-diag-raw")?;
    assert!(s_r3.warnings.is_empty(), "missing diag corr warnings: {:?}", s_r3.warnings);
    let r3_texts = scene_texts(&s_r3);
    let ones_count = r3_texts.iter().filter(|t| t.as_str() == "1.00").count();
    assert!(ones_count >= 3, "computed correlation matrix should have 1.00 diagonal cells, got texts: {r3_texts:?}");
    assert!(
        !r3_texts.iter().any(|t| t == "0.20" || t == "0.80"),
        "should compute correlation matrix rather than plotting raw asymmetric off-diagonals: {r3_texts:?}"
    );

    // Regression 4: XY §6.2 distribution legend swatch shares `shade_accent_color` (switching to
    // palette.color_at(2) when primary == palette.color_at(1)), and direct `analyzable_data`
    // continuous distributions show a solid line without marker in the legend matching the plot.
    let mut xy_seed = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    xy_seed.x_cells[0][0] = Cell::number(0.0);
    xy_seed.x_cells[1][0] = Cell::number(1.0);
    xy_seed.data_sets[0].cells[0][0] = Cell::number(0.1);
    xy_seed.data_sets[0].cells[1][0] = Cell::number(0.2);
    let ds_seed = DataSheet::new(SheetId::new("xy-r4-seed"), "Seed", TableArchetype::Xy(xy_seed))?;
    let mut nl_r4 = NonlinearFitSpec::new("plot_distribution");
    nl_r4.plot_distribution = Some(TheoreticalDistributionSpec {
        family: TheoreticalDistributionFamily::Normal,
        parameters: vec![0.0, 1.0],
        parameter_hooks: Vec::new(),
        cumulative: false,
        tail_shade: DistributionTailShade::UpperTail { cutoff: 1.645 },
    });
    let an_r4 = AnalysisSpec::new(SheetId::new("res-r4-norm"), ds_seed.id.clone(), AnalysisKind::NonlinearRegression(nl_r4))?;
    let res_r4 = execute_fit_analysis(&an_r4, &ds_seed, &HookContext::default())?;
    let mut doc_r4 = ProjectDocument::default();
    doc_r4.add_data_sheet(ds_seed.clone())?;
    doc_r4.add_result_sheet(res_r4.clone())?;

    let mut g_r4 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-r4-direct-dist"), &ds_seed)?;
    g_r4.initialized = true;
    g_r4.spec.canvas_size_pt = SizePt::new(440.0, 290.0);
    g_r4.spec.palette = PaletteId::OkabeIto;
    let c1 = PaletteId::OkabeIto.color_at(1);
    let c2 = PaletteId::OkabeIto.color_at(2);
    assert_ne!(c1, c2);
    g_r4.spec.primary_source_sheet_id = res_r4.id.clone();
    g_r4.spec.legend.visible = true;
    // Direct `analyzable_data` continuous distribution with default marker-only GDO whose color == palette.color_at(1).
    g_r4.spec.series = vec![SeriesSpec {
        source_sheet_id: res_r4.id.clone(),
        data_set_index: 0,
        gdo: GraphicalDataObject::ScatterAndLine {
            marker: Some(curvebook_palette::MarkerStyle {
                shape: curvebook_palette::MarkerShape::Circle,
                fill_mode: curvebook_palette::MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: c1,
                stroke_color: c1,
                stroke_width_pt: 0.75,
            }),
            line: None,
            area_fill: None,
            replicate_lines: curvebook_doc::ReplicateLineMode::default(),
        },
        show_raw_data: true,
        legend_label: Some(String::from("Normal PDF")),
        regression_overlay: None,
        ..g_r4.spec.series[0].clone()
    }];
    let s_r4 = compile_checked_deterministic(&g_r4, &doc_r4, "direct-dist-legend-accent-and-line")?;
    assert!(s_r4.warnings.is_empty(), "direct dist legend warnings: {:?}", s_r4.warnings);

    // Shaded tail polygon inside clip rect must use palette.color_at(2) (since primary == palette.color_at(1)).
    let rendered_tail_uses_c2 = s_r4.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { path, fill: Some(f), stroke: None, clip: Some(_) }
            if f.color.r == c2.r && f.color.g == c2.g && f.color.b == c2.b && f.color.a > 0 && f.color.a < 255
            && path.commands.len() >= 5 && matches!(path.commands.last(), Some(PathCmd::Close)))
    });
    assert!(rendered_tail_uses_c2, "rendered tail polygon should switch to palette.color_at(2)");

    // Unclipped legend swatch rect must also use palette.color_at(2) (matching the rendered tail polygon).
    let legend_swatch_uses_c2 = s_r4.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { fill: Some(f), stroke: Some(st), clip: None, .. }
            if f.color.r == c2.r && f.color.g == c2.g && f.color.b == c2.b
            && st.color.r == c2.r && st.color.g == c2.g && st.color.b == c2.b)
    });
    assert!(legend_swatch_uses_c2, "auto-shade legend swatch must match rendered shade_accent_color (palette.color_at(2))");

    // Unclipped legend sample for the continuous direct-distribution PDF must draw a solid line in `c1` and NO marker in `c1`.
    let legend_line_uses_c1 = s_r4.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: None }
            if st.color == c1 && path.commands.len() == 2)
    });
    assert!(legend_line_uses_c1, "direct continuous distribution legend must draw a line segment in curve color");
    let legend_marker_uses_c1 = s_r4.primitives.iter().any(|p| {
        matches!(p, ScenePrimitive::Path { fill: Some(f), clip: None, .. }
            if f.color == c1)
    });
    assert!(!legend_marker_uses_c1, "direct continuous distribution legend must not emit a scatter marker when no markers are drawn");

    Ok(())
}
