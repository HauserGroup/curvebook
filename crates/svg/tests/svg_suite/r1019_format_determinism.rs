//! Integration tests for `R10.19` bullet 2 in `curvebook-svg`:
//! byte-identical `.svg` exports for 2D Knuth-boxes-and-glue `ModelEquation` / rich-text scenes
//! (editable `<text><tspan>` and outlined `<path>` modes) and live `.svg` refresh on upstream edits.

use curvebook_chart::equation_box::{render_tex_equation_box, resolve_model_equation_hit_bounds};
use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor, compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, FittedCurveOverlay, GraphAnnotation, GraphSheet, LayoutItem, LayoutSheet,
    LayoutSpec, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument, RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_palette::Rgba8;
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_testkit::sha256_bytes;
use curvebook_text::{BUNDLED_FONTS, has_bundled_fonts_feature, parse_markup};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn assert_cleanroom_font_baseline() {
    if !has_bundled_fonts_feature() {
        assert!(BUNDLED_FONTS.is_empty(), "cleanroom build without bundled-fonts must not link bundled font blobs");
    }
}

fn build_nonlinear_result(
    id: &str,
    data_id: &str,
    equation_id: &str,
    custom_dsl: Option<&str>,
    scalars: Vec<ResultScalarEntry>,
) -> Result<ResultSheet, Box<dyn std::error::Error>> {
    let mut nl = NonlinearFitSpec::new(equation_id);
    nl.custom_dsl = custom_dsl.map(str::to_owned);
    let spec = AnalysisSpec::new(id, data_id, AnalysisKind::NonlinearRegression(nl))?;
    let mut res = ResultSheet::new(spec)?;
    res.status = ResultStatus::UpToDate;
    res.scalars = scalars;
    res.fitted_curves =
        vec![FittedCurveOverlay { data_set_index: 0, equation_id: equation_id.to_owned(), converged: true, ambiguous: false, points: Vec::new() }];
    Ok(res)
}

fn build_svg_equation_project() -> Result<(ProjectDocument, GraphSheet, LayoutSheet), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("SvgDeterminism")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6)?;
    let pts = [(0.5, 16.0), (1.0, 28.0), (2.0, 49.0), (4.0, 76.0), (8.0, 92.0), (16.0, 98.0)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        *xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()).ok_or("missing xy cell")? = Cell::number(y);
    }
    let ds = DataSheet::new("xy-svg", "Dose-Response", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds.clone())?;

    let res_4pl = build_nonlinear_result(
        "res-4pl",
        "xy-svg",
        "log_agonist_vs_response_4pl",
        None,
        vec![
            ResultScalarEntry { key: "Bottom".to_owned(), data_set_index: Some(0), value: 11.4, units: None },
            ResultScalarEntry { key: "Top".to_owned(), data_set_index: Some(0), value: 98.2, units: None },
            ResultScalarEntry { key: "LogEC50".to_owned(), data_set_index: Some(0), value: -6.25, units: None },
            ResultScalarEntry { key: "HillSlope".to_owned(), data_set_index: Some(0), value: 1.45, units: None },
        ],
    )?;
    doc.add_result_sheet(res_4pl)?;

    let lin_spec = AnalysisSpec::new("res-lin", "xy-svg", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let mut lin_res = ResultSheet::new(lin_spec)?;
    lin_res.status = ResultStatus::UpToDate;
    lin_res.scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: 2.5, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -1.25, units: None },
    ];
    doc.add_result_sheet(lin_res)?;

    let res_interval = build_nonlinear_result(
        "res-interval",
        "xy-svg",
        "beta_growth_then_decay",
        None,
        vec![
            ResultScalarEntry { key: "Ym".to_owned(), data_set_index: Some(0), value: 42.5, units: None },
            ResultScalarEntry { key: "te".to_owned(), data_set_index: Some(0), value: 12.0, units: None },
            ResultScalarEntry { key: "tm".to_owned(), data_set_index: Some(0), value: 5.5, units: None },
        ],
    )?;
    doc.add_result_sheet(res_interval)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-svg", &ds)?;
    graph.spec.title = String::from("<b>EC<sub>50</sub></b> &amp; Interval (α = 0.05)");
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-4pl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(68.0, 28.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(68.0, 88.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-interval"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(68.0, 128.0, 260.0, 44.0),
        font_size_pt: 9.0,
        bordered: false,
    });
    graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: DocRectPt::new(68.0, 180.0, 190.0, 22.0),
        markdown: String::from("<b>IC<sub>50</sub></b> = 2.4 ± 0.3 µM"),
        bordered: true,
    });
    doc.add_graph_sheet(graph.clone())?;

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-svg"),
        graph_sheet_id: SheetId::new("g-svg"),
        bounds_pt: DocRectPt::new(16.0, 16.0, 320.0, 220.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("note-svg"),
        markdown: String::from("<i>ΔG</i> = -5.4 kJ/mol"),
        bounds_pt: DocRectPt::new(348.0, 24.0, 132.0, 40.0),
        font_size_pt: 10.0,
    });
    let layout = LayoutSheet::new(SheetId::new("layout-svg"), "SVG Equation Layout", layout_spec)?;
    doc.add_layout_sheet(layout.clone())?;

    Ok((doc, graph, layout))
}

#[test]
fn svg_exports_are_byte_identical_for_2d_math_model_equations_and_rich_text() -> TestResult {
    assert_cleanroom_font_baseline();

    let mut math_scene = ChartScene::new("svg-math", "2D TeX SVG & <Math>", 500.0, 340.0, Some(Rgba8::WHITE));
    for (rect, tex) in [
        (DocRectPt::new(20.0, 18.0, 0.0, 0.0), r"Y = \frac{1}{1 + \frac{\alpha}{\beta}} + \binom{n}{k}"),
        (DocRectPt::new(20.0, 74.0, 0.0, 0.0), r"S^n_j + \sqrt[3]{X + \Delta} - 2.5 \cdot 10^{-5}"),
        (
            DocRectPt::new(20.0, 134.0, 0.0, 0.0),
            r"\sum_{i=1}^{n} \frac{x_i}{\sigma_i^2} + \int_{0}^{\infty} \lambda \cdot e^{-\lambda \cdot t} \, dt",
        ),
        (DocRectPt::new(20.0, 200.0, 0.0, 0.0), r"\left[\frac{X - \mu}{\sigma}\right] \cdot \left(0 \le X \le t_e\right) \pm 0.05"),
    ] {
        render_tex_equation_box(&mut math_scene, rect, tex, 11.0, true);
    }
    math_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>IC<sub>50</sub></b> = 1.8 ± 0.2 µM · [10<sup>-6</sup>, 10<sup>-3</sup>]"),
        anchor_pt: PointPt::new(24.0, 286.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let (doc, graph, layout) = build_svg_equation_project()?;
    let g_scene = compile_graph(&graph, &doc)?;
    let l_scene = compile_layout(&layout, &doc)?;

    for (name, scene) in [("math-2d", &math_scene), ("graph", &g_scene), ("layout", &l_scene)] {
        for outline_text in [false, true] {
            for pretty in [true, false] {
                let opts = SvgExportOptions {
                    pretty,
                    include_xml_declaration: true,
                    include_metadata: true,
                    embed_fonts: false,
                    outline_text,
                    precision: 3,
                    transparent_bg: false,
                };
                let svg_1 = render_svg(scene, &opts)?;
                let svg_2 = render_svg(scene, &opts)?;
                assert_eq!(svg_1, svg_2, "SVG bytes differed across repeat exports ({name}, outline={outline_text}, pretty={pretty})");
                assert_eq!(sha256_bytes(svg_1.as_bytes()), sha256_bytes(svg_2.as_bytes()));
                validate_svg_structure(&svg_1)?;
                assert!(!svg_1.contains("-0.000"), "negative zero leaked into SVG ({name})");
                for raw in ["\\frac", "\\sqrt", "\\sum", "\\int", "\\binom"] {
                    assert!(!svg_1.contains(raw), "unexpanded TeX `{raw}` leaked into SVG ({name})");
                }
                if outline_text {
                    assert!(!svg_1.contains("<text"), "outlined SVG must emit paths rather than <text> ({name})");
                    assert!(!svg_1.contains("<tspan"), "outlined SVG must emit paths rather than <tspan> ({name})");
                    assert!(svg_1.contains("<path d=\""), "outlined SVG missing glyph paths ({name})");
                } else {
                    assert!(svg_1.contains("<text "), "editable SVG missing <text> ({name})");
                    assert!(svg_1.contains("<tspan"), "editable SVG missing <tspan> ({name})");
                    assert!(svg_1.contains("<path d=\""), "editable SVG missing fraction/surd/delimiter paths ({name})");
                }
            }
        }
    }

    let editable_math = render_svg(&math_scene, &SvgExportOptions::default())?;
    for expected in ["α", "β", "Δ", "∑", "∫", "λ", "μ", "σ", "≤", "∞", "±"] {
        assert!(editable_math.contains(expected), "missing symbol `{expected}` in editable SVG");
    }

    Ok(())
}

#[test]
fn upstream_coefficient_and_model_edits_refresh_svg_without_stale_content() -> TestResult {
    assert_cleanroom_font_baseline();
    let (mut doc, mut graph, layout) = build_svg_equation_project()?;

    let g_v1 = compile_graph(&graph, &doc)?;
    let l_v1 = compile_layout(&layout, &doc)?;
    let opts_edit = SvgExportOptions { outline_text: false, ..SvgExportOptions::default() };
    let opts_out = SvgExportOptions { outline_text: true, ..SvgExportOptions::default() };

    let svg_g1 = render_svg(&g_v1, &opts_edit)?;
    let svg_g1_out = render_svg(&g_v1, &opts_out)?;
    let svg_l1 = render_svg(&l_v1, &opts_edit)?;
    assert_eq!(svg_g1, render_svg(&g_v1, &opts_edit)?);
    assert!(svg_g1.contains("98.2") && svg_g1.contains("6.25") && svg_g1.contains("2.5"));
    let hb_4pl_v1 = resolve_model_equation_hit_bounds(&g_v1, DocRectPt::new(68.0, 28.0, 0.0, 0.0), 10.0, true).ok_or("missing 4PL hit bounds v1")?;

    // Mutate upstream coefficients + switch third equation to custom DSL with sqrt & Greek.
    doc.result_sheets[0].scalars[1].value = 1840.0;
    doc.result_sheets[0].scalars[2].value = -4.375;
    doc.result_sheets[0].scalars[3].value = 1.625;
    doc.result_sheets[1].scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: -3.75, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -7.25, units: None },
    ];
    doc.result_sheets[2].analysis.kind = AnalysisKind::NonlinearRegression(NonlinearFitSpec {
        custom_dsl: Some(String::from("Y = Delta * exp(-theta * X) + sqrt(X + 9)")),
        ..NonlinearFitSpec::new("custom")
    });
    doc.result_sheets[2].fitted_curves[0].equation_id = String::from("custom");
    doc.result_sheets[2].scalars = vec![
        ResultScalarEntry { key: "Delta".to_owned(), data_set_index: Some(0), value: 77.5, units: None },
        ResultScalarEntry { key: "theta".to_owned(), data_set_index: Some(0), value: 0.45, units: None },
    ];

    let g_v2 = compile_graph(&graph, &doc)?;
    let l_v2 = compile_layout(&layout, &doc)?;

    let svg_g2_a = render_svg(&g_v2, &opts_edit)?;
    let svg_g2_b = render_svg(&g_v2, &opts_edit)?;
    assert_eq!(svg_g2_a, svg_g2_b, "mutated editable SVG must be byte-identical across runs");
    validate_svg_structure(&svg_g2_a)?;
    assert_ne!(svg_g1, svg_g2_a, "editable SVG must refresh after upstream edits");

    let svg_g2_out_a = render_svg(&g_v2, &opts_out)?;
    let svg_g2_out_b = render_svg(&g_v2, &opts_out)?;
    assert_eq!(svg_g2_out_a, svg_g2_out_b, "mutated outlined SVG must be byte-identical across runs");
    validate_svg_structure(&svg_g2_out_a)?;
    assert_ne!(svg_g1_out, svg_g2_out_a, "outlined SVG must refresh after upstream edits");

    let svg_l2_a = render_svg(&l_v2, &opts_edit)?;
    let svg_l2_b = render_svg(&l_v2, &opts_edit)?;
    assert_eq!(svg_l2_a, svg_l2_b);
    validate_svg_structure(&svg_l2_a)?;
    assert_ne!(svg_l1, svg_l2_a, "layout SVG must refresh after upstream edits");

    // Verify stale numbers disappeared and updated values & expanded bounds appeared.
    for stale in ["98.2", "6.25", "2.5 "] {
        assert!(!svg_g2_a.contains(stale), "stale value `{stale}` remained in refreshed SVG");
    }
    for fresh in ["1840", "4.375", "1.625", "-3.75", "7.25", "77.5", "0.45"] {
        assert!(svg_g2_a.contains(fresh), "refreshed value `{fresh}` missing from SVG");
    }
    let hb_4pl_v2 = resolve_model_equation_hit_bounds(&g_v2, DocRectPt::new(68.0, 28.0, 0.0, 0.0), 10.0, true).ok_or("missing 4PL hit bounds v2")?;
    assert!(hb_4pl_v2.width > hb_4pl_v1.width);

    // Dirty status and Symbolic toggle refresh SVG immediately.
    doc.result_sheets[1].status = ResultStatus::Dirty;
    let g_dirty = compile_graph(&graph, &doc)?;
    let svg_dirty = render_svg(&g_dirty, &opts_edit)?;
    assert_eq!(svg_dirty, render_svg(&g_dirty, &opts_edit)?);
    assert!(svg_dirty.contains("(Fit pending)"));
    assert!(!svg_dirty.contains("-3.75"));

    graph.spec.annotations[0] = GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-4pl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(68.0, 28.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    };
    let g_sym = compile_graph(&graph, &doc)?;
    let svg_sym = render_svg(&g_sym, &opts_edit)?;
    assert_eq!(svg_sym, render_svg(&g_sym, &opts_edit)?);
    assert!(svg_sym.contains("HillSlope") && !svg_sym.contains("1840"));

    Ok(())
}

#[test]
fn r20_5_fit_summary_svg_export_is_byte_deterministic() -> TestResult {
    let (doc, mut graph, _) = build_svg_equation_project()?;
    graph.spec.annotations.clear();
    graph.spec.series[0].regression_overlay = Some(curvebook_doc::RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-4pl"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    let opts = curvebook_chart::CompileOptions {
        fit_summary: curvebook_chart::FitSummarySpec { enabled: true, ..curvebook_chart::FitSummarySpec::default() },
    };
    let s1 = curvebook_chart::compile_graph_with_options(&graph, &doc, &opts)?;
    let s2 = curvebook_chart::compile_graph_with_options(&graph, &doc, &opts)?;
    let svg_opts = SvgExportOptions::default();
    let svg_1 = render_svg(&s1, &svg_opts)?;
    let svg_2 = render_svg(&s2, &svg_opts)?;
    assert_eq!(sha256_bytes(svg_1.as_bytes()), sha256_bytes(svg_2.as_bytes()));
    validate_svg_structure(&svg_1)?;
    assert!(svg_1.contains("4PL"), "expected fit-summary equation label in SVG");
    assert!(svg_1.contains("LogEC50"), "expected fit-summary metric in SVG");
    Ok(())
}
