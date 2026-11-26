//! Integration tests for `R10.19` bullet 2 in `curvebook-chart`:
//! bit-exact 2D equation typesetting determinism (`ChartScene` + auto-sized hit bounds)
//! and live refresh on upstream `XyTable`, fit, scalar, mode, and status mutations.

use curvebook_chart::equation_box::{
    ModelEquationParams, render_model_equation_annotation, render_tex_equation_box, resolve_model_equation_hit_bounds,
};
use curvebook_chart::{ChartScene, PointPt, ScenePrimitive, compile_graph, compile_layout, preflight_check_scene};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, ErrorCode, FittedCurveOverlay, GraphAnnotation, GraphSheet, LayoutItem,
    LayoutSheet, LayoutSpec, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument, RectPt as DocRectPt, ResultScalarEntry, ResultSheet,
    ResultStatus, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_palette::Rgba8;
use curvebook_text::{BUNDLED_FONTS, RichText, has_bundled_fonts_feature, measure_text_pt, parse_markup};

use super::common::{assert_rect_bits_eq, assert_scenes_bit_identical, scene_texts};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn assert_cleanroom_font_baseline() {
    if !has_bundled_fonts_feature() {
        assert!(BUNDLED_FONTS.is_empty(), "cleanroom build without bundled-fonts must not link host/bundled font blobs");
    }
}

fn joined_scene_text(scene: &ChartScene) -> String {
    scene_texts(scene).join(" | ")
}

fn assert_equation_scene_clean(scene: &ChartScene) {
    let joined = joined_scene_text(scene);
    for raw in ["\\frac", "\\binom", "\\sqrt", "\\sum", "\\int", "\\text", "\\cdot", "\\alpha", "\\Delta", "\\left", "\\right"] {
        assert!(!joined.contains(raw), "unexpanded TeX escape `{raw}` in `{joined}`");
    }
    for prim in &scene.primitives {
        if let ScenePrimitive::Text(t) = prim {
            assert!(t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite());
            assert!(t.font_size_pt.is_finite() && t.font_size_pt > 0.0);
        }
    }
}

fn build_nonlinear_result_sheet(
    id: &str,
    data_id: &str,
    equation_id: &str,
    custom_dsl: Option<&str>,
    scalars: Vec<ResultScalarEntry>,
) -> Result<ResultSheet, Box<dyn std::error::Error>> {
    let mut nl = NonlinearFitSpec::new(equation_id);
    nl.custom_dsl = custom_dsl.map(str::to_owned);
    let spec = AnalysisSpec::new(id, data_id, AnalysisKind::NonlinearRegression(nl))?;
    let mut sheet = ResultSheet::new(spec)?;
    sheet.status = ResultStatus::UpToDate;
    sheet.scalars = scalars;
    sheet.fitted_curves =
        vec![FittedCurveOverlay { data_set_index: 0, equation_id: equation_id.to_owned(), converged: true, ambiguous: false, points: Vec::new() }];
    Ok(sheet)
}

struct EquationProjectFixture {
    doc: ProjectDocument,
    graph: GraphSheet,
    layout: LayoutSheet,
    nl_spec: AnalysisSpec,
    lin_spec: AnalysisSpec,
}

fn build_4pl_and_linear_doc() -> Result<EquationProjectFixture, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("ChartDetLiveRefresh")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8)?;
    let pts: [(f64, f64); 8] = [(-9.0, 12.0), (-8.0, 15.0), (-7.0, 31.0), (-6.0, 62.0), (-5.0, 88.0), (-4.0, 96.0), (-3.0, 99.0), (-2.0, 100.0)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("xy-4pl", "Dose Response", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds.clone())?;

    let nl_spec = AnalysisSpec::new("res-4pl", "xy-4pl", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")))?;
    let nl_res = execute_analysis(&nl_spec, &ds, &HookContext::default())?;
    assert_eq!(nl_res.status, ResultStatus::UpToDate);
    doc.add_result_sheet(nl_res)?;

    let lin_spec = AnalysisSpec::new("res-lin", "xy-4pl", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let lin_res = execute_analysis(&lin_spec, &ds, &HookContext::default())?;
    assert_eq!(lin_res.status, ResultStatus::UpToDate);
    doc.add_result_sheet(lin_res)?;

    let interval_sheet = build_nonlinear_result_sheet(
        "res-interval",
        "xy-4pl",
        "beta_growth_then_decay",
        None,
        vec![
            ResultScalarEntry { key: "Ym".to_owned(), data_set_index: Some(0), value: 42.5, units: None },
            ResultScalarEntry { key: "te".to_owned(), data_set_index: Some(0), value: 12.0, units: None },
            ResultScalarEntry { key: "tm".to_owned(), data_set_index: Some(0), value: 5.5, units: None },
        ],
    )?;
    doc.add_result_sheet(interval_sheet)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-main", doc.data_sheet(&SheetId::new("xy-4pl")).ok_or("missing xy-4pl")?)?;
    graph.spec.title = String::from("<b>IC<sub>50</sub></b> &amp; Linear Fit (α = 0.05)");
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-4pl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(72.0, 26.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(72.0, 92.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-interval"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(72.0, 128.0, 260.0, 44.0),
        font_size_pt: 10.0,
        bordered: false,
    });
    graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: DocRectPt::new(72.0, 178.0, 180.0, 22.0),
        markdown: String::from("<b>EC<sub>50</sub></b> ∈ [10<sup>-7</sup>, 10<sup>-5</sup>] M"),
        bordered: true,
    });
    doc.add_graph_sheet(graph.clone())?;

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("g-main"),
        bounds_pt: DocRectPt::new(16.0, 16.0, 320.0, 220.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("note-a"),
        markdown: String::from("<i>Y</i><sub>obs</sub> ± SEM (μM)"),
        bounds_pt: DocRectPt::new(348.0, 24.0, 140.0, 48.0),
        font_size_pt: 10.0,
    });
    let layout = LayoutSheet::new(SheetId::new("layout-main"), "Panel Layout", layout_spec)?;
    doc.add_layout_sheet(layout.clone())?;

    Ok(EquationProjectFixture { doc, graph, layout, nl_spec, lin_spec })
}

#[test]
fn knuth_2d_math_formulas_and_hit_bounds_are_bit_identical_across_runs() -> TestResult {
    assert_cleanroom_font_baseline();

    let formulas = [
        ("nested_frac_binom", r"Y = \frac{1}{1 + \frac{a}{b}} + \binom{n}{k}"),
        ("stacked_scripts_exp", r"S^n_j + x_i^2 + e^{-\frac{X}{2}} + 10^{(\text{LogEC}_{50} - X) \cdot h}"),
        ("roots_and_nth_root", r"Y = \sqrt{a^2 + b^2} + \sqrt[3]{X + \Delta}"),
        (
            "summation_and_integral",
            r"Y = \sum_{i=1}^{n} \frac{\alpha_i \cdot X_i}{\sigma_i^2} + \int_{0}^{\infty} \lambda \cdot \exp(-\lambda \cdot t) \, dt",
        ),
        (
            "scaled_delims_and_intervals",
            r"Y = \left[\frac{X - \mu}{\sigma}\right] \cdot \left(0 \le X \le t_e\right) + \left|\frac{\Delta G}{R \cdot T}\right|",
        ),
        ("negative_coeffs_and_sci", r"Y = -2.5 \cdot X - 1.23 \times 10^{-5} \pm 0.045"),
    ];

    for (label, tex) in formulas {
        for (bounds, bordered) in [
            (DocRectPt::new(24.0, 24.0, 0.0, 0.0), true),
            (DocRectPt::new(24.0, 24.0, 0.0, 0.0), false),
            (DocRectPt::new(24.0, 24.0, 165.0, 72.0), true),
        ] {
            let mut s1 = ChartScene::new("eq-det", label, 520.0, 240.0, Some(Rgba8::WHITE));
            render_tex_equation_box(&mut s1, bounds, tex, 12.0, bordered);
            let mut s2 = ChartScene::new("eq-det", label, 520.0, 240.0, Some(Rgba8::WHITE));
            render_tex_equation_box(&mut s2, bounds, tex, 12.0, bordered);

            assert_scenes_bit_identical(&s1, &s2);
            assert_equation_scene_clean(&s1);
            assert!(
                preflight_check_scene(&s1).is_empty(),
                "preflight warnings for `{label}` (bordered={bordered}): {:?}",
                preflight_check_scene(&s1)
            );

            let hb1 = resolve_model_equation_hit_bounds(&s1, bounds, 12.0, bordered).ok_or("missing hit bounds s1")?;
            let hb2 = resolve_model_equation_hit_bounds(&s2, bounds, 12.0, bordered).ok_or("missing hit bounds s2")?;
            assert_rect_bits_eq(hb1, hb2, label);
            assert!(hb1.width > 12.0 && hb1.height > 12.0, "degenerate hit bounds for `{label}`: {hb1:?}");

            if bounds.width == 0.0 && bounds.height == 0.0 && bordered {
                for prim in &s1.primitives {
                    if let ScenePrimitive::Text(t) = prim {
                        let m = measure_text_pt(&t.rich, t.font_size_pt);
                        assert!(
                            t.anchor_pt.x >= hb1.x && t.anchor_pt.x + m.width_pt <= hb1.right() + 0.5,
                            "text `{}` overflows auto-sized width for `{label}`",
                            t.rich.plain_text()
                        );
                    }
                }
            }
        }
    }

    let mut rich_scene_a = ChartScene::new("rich-eq", "Rich", 360.0, 140.0, Some(Rgba8::WHITE));
    let rich = parse_markup("<b>IC<sub>50</sub></b> = 2.4 ± 0.3 µM · [10<sup>-6</sup>, 10<sup>-3</sup>]");
    rich_scene_a.primitives.push(ScenePrimitive::Text(curvebook_chart::PositionedText {
        rich: rich.clone(),
        anchor_pt: PointPt::new(24.0, 40.0),
        anchor: curvebook_chart::TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let mut rich_scene_b = ChartScene::new("rich-eq", "Rich", 360.0, 140.0, Some(Rgba8::WHITE));
    rich_scene_b.primitives.push(ScenePrimitive::Text(curvebook_chart::PositionedText {
        rich,
        anchor_pt: PointPt::new(24.0, 40.0),
        anchor: curvebook_chart::TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    assert_scenes_bit_identical(&rich_scene_a, &rich_scene_b);

    Ok(())
}

#[test]
fn compiled_graph_and_layout_with_model_equations_are_bit_identical() -> TestResult {
    assert_cleanroom_font_baseline();
    let EquationProjectFixture { doc, graph, layout, .. } = build_4pl_and_linear_doc()?;

    let g1 = compile_graph(&graph, &doc)?;
    let g2 = compile_graph(&graph, &doc)?;
    assert_scenes_bit_identical(&g1, &g2);
    assert_equation_scene_clean(&g1);

    let l1 = compile_layout(&layout, &doc)?;
    let l2 = compile_layout(&layout, &doc)?;
    assert_scenes_bit_identical(&l1, &l2);
    assert_equation_scene_clean(&l1);

    let text = joined_scene_text(&g1);
    for expected in ["Y =", "10", "≤", "EC", "∈", "α"] {
        assert!(text.contains(expected), "missing expected equation/symbol `{expected}` in `{text}`");
    }
    let layout_text = joined_scene_text(&l1);
    assert!(layout_text.contains("μM"), "missing μM in layout `{layout_text}`");
    let _ = RichText::plain("ok");
    Ok(())
}

#[test]
fn upstream_xy_fit_and_scalar_edits_refresh_equations_and_hit_bounds_immediately() -> TestResult {
    assert_cleanroom_font_baseline();
    let EquationProjectFixture { mut doc, mut graph, layout, nl_spec, lin_spec } = build_4pl_and_linear_doc()?;

    let g_v1 = compile_graph(&graph, &doc)?;
    let g_v1_repeat = compile_graph(&graph, &doc)?;
    assert_scenes_bit_identical(&g_v1, &g_v1_repeat);
    let l_v1 = compile_layout(&layout, &doc)?;
    let text_v1 = joined_scene_text(&g_v1);
    let hb_lin_v1 = resolve_model_equation_hit_bounds(&g_v1, DocRectPt::new(72.0, 92.0, 0.0, 0.0), 10.0, true).ok_or("missing lin hit bounds v1")?;

    // 1) Mutate upstream XY table: invert slope and scale responses into 4-digit range.
    {
        let ds_mut = doc.data_sheet_mut(&SheetId::new("xy-4pl")).ok_or("missing xy-4pl")?;
        if let TableArchetype::Xy(xy) = &mut ds_mut.archetype {
            let inverted = [1200.0, 1140.0, 940.0, 620.0, 310.0, 180.0, 125.0, 110.0];
            for (r, y_val) in inverted.into_iter().enumerate() {
                xy.data_sets[0].cells[r][0] = Cell::number(y_val);
            }
        }
    }
    let ds_updated = doc.data_sheet(&SheetId::new("xy-4pl")).ok_or("missing xy-4pl after edit")?.clone();
    doc.result_sheets[0] = execute_analysis(&nl_spec, &ds_updated, &HookContext::default())?;
    doc.result_sheets[1] = execute_analysis(&lin_spec, &ds_updated, &HookContext::default())?;

    let g_v2 = compile_graph(&graph, &doc)?;
    let g_v2_repeat = compile_graph(&graph, &doc)?;
    assert_scenes_bit_identical(&g_v2, &g_v2_repeat);
    let l_v2 = compile_layout(&layout, &doc)?;
    let l_v2_repeat = compile_layout(&layout, &doc)?;
    assert_scenes_bit_identical(&l_v2, &l_v2_repeat);

    let text_v2 = joined_scene_text(&g_v2);
    assert_ne!(text_v1, text_v2, "equation text must refresh after upstream XY re-fit");
    assert_ne!(joined_scene_text(&l_v1), joined_scene_text(&l_v2), "layout equation text must refresh");
    assert!(text_v2.contains("- "), "inverted response curve must refresh linear equation to negative slope: {text_v2}");

    let hb_lin_v2 = resolve_model_equation_hit_bounds(&g_v2, DocRectPt::new(72.0, 92.0, 0.0, 0.0), 10.0, true).ok_or("missing lin hit bounds v2")?;
    assert_ne!(
        (hb_lin_v1.width.to_bits(), hb_lin_v1.height.to_bits()),
        (hb_lin_v2.width.to_bits(), hb_lin_v2.height.to_bits()),
        "auto-sized bounding box must refresh when coefficient width changes"
    );

    // 2) Mutate ResultStatus::Dirty -> Failed -> UpToDate with negative scalar & wider digits.
    doc.result_sheets[1].status = ResultStatus::Dirty;
    let g_dirty = compile_graph(&graph, &doc)?;
    let text_dirty = joined_scene_text(&g_dirty);
    assert!(text_dirty.contains("(Fit pending)"), "dirty result must suppress stale coefficients: {text_dirty}");

    doc.result_sheets[1].status = ResultStatus::Failed { code: ErrorCode::SolverFailed, message: String::from("Singular Jacobian") };
    let g_failed = compile_graph(&graph, &doc)?;
    let text_failed = joined_scene_text(&g_failed);
    assert!(text_failed.contains("(Fit failed"), "failed result must suppress stale coefficients: {text_failed}");

    doc.result_sheets[1].status = ResultStatus::UpToDate;
    doc.result_sheets[1].scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: -148.3, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -0.000_045_6, units: None },
    ];
    let g_v3 = compile_graph(&graph, &doc)?;
    let g_v3_repeat = compile_graph(&graph, &doc)?;
    assert_scenes_bit_identical(&g_v3, &g_v3_repeat);
    let text_v3 = joined_scene_text(&g_v3);
    assert!(text_v3.contains("-148.3"), "expected refreshed negative slope in `{text_v3}`");
    assert!(text_v3.contains("- 4.56"), "expected refreshed negative sci-notation intercept in `{text_v3}`");

    // 3) Toggle Symbolic <-> Substituted and update custom DSL formula.
    graph.spec.annotations[0] = GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-4pl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(72.0, 26.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    };
    let g_sym = compile_graph(&graph, &doc)?;
    let sym_text = joined_scene_text(&g_sym);
    assert!(sym_text.contains("HillSlope") && sym_text.contains("Bottom"), "symbolic mode refresh failed: {sym_text}");

    let mut dsl_sheet = build_nonlinear_result_sheet(
        "res-dsl",
        "xy-4pl",
        "custom",
        Some("Y = Delta * exp(-theta * X) + alpha * sqrt(X + 1)"),
        vec![
            ResultScalarEntry { key: "Delta".to_owned(), data_set_index: Some(0), value: 33.5, units: None },
            ResultScalarEntry { key: "theta".to_owned(), data_set_index: Some(0), value: 0.75, units: None },
            ResultScalarEntry { key: "alpha".to_owned(), data_set_index: Some(0), value: -2.25, units: None },
            ResultScalarEntry { key: "alpha".to_owned(), data_set_index: Some(1), value: 9.5, units: None },
        ],
    )?;
    dsl_sheet.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 1,
        equation_id: String::from("custom"),
        converged: true,
        ambiguous: false,
        points: Vec::new(),
    });
    let mut dsl_doc = ProjectDocument::new("DslIsolation")?;
    dsl_doc.add_result_sheet(dsl_sheet)?;

    let p_ds0 = ModelEquationParams {
        result_sheet_id: &SheetId::new("res-dsl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    };
    let p_ds1 = ModelEquationParams { data_set_index: 1, ..p_ds0 };

    let mut s_ds0_before = ChartScene::new("ds0", "DS0", 400.0, 140.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(&mut s_ds0_before, &dsl_doc, &p_ds0);
    let mut s_ds1_before = ChartScene::new("ds1", "DS1", 400.0, 140.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(&mut s_ds1_before, &dsl_doc, &p_ds1);

    // Mutate dataset 1 scalar only -> dataset 0 remains bit-identical while dataset 1 updates.
    dsl_doc.result_sheets[0].scalars[3].value = 88.25;
    let mut s_ds0_after = ChartScene::new("ds0", "DS0", 400.0, 140.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(&mut s_ds0_after, &dsl_doc, &p_ds0);
    let mut s_ds1_after = ChartScene::new("ds1", "DS1", 400.0, 140.0, Some(Rgba8::WHITE));
    render_model_equation_annotation(&mut s_ds1_after, &dsl_doc, &p_ds1);

    assert_scenes_bit_identical(&s_ds0_before, &s_ds0_after);
    assert_ne!(joined_scene_text(&s_ds1_before), joined_scene_text(&s_ds1_after));
    assert!(joined_scene_text(&s_ds1_after).contains("88.25"));

    Ok(())
}
