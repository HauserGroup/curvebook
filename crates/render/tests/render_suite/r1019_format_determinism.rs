//! Integration tests for `R10.19` bullet 2 in `curvebook-render`:
//! deterministic RGBA pixel buffers, PNG/TIFF export byte-identity for 2D Knuth-boxes-and-glue
//! `ModelEquation` / rich-text scenes, and live pixel refresh on upstream edits.

use std::io::Read;

use curvebook_chart::equation_box::{render_tex_equation_box, resolve_model_equation_hit_bounds};
use curvebook_chart::{ChartScene, PointPt, PositionedText, RectPt, ScenePrimitive, TextAnchor, compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, FittedCurveOverlay, GraphAnnotation, GraphSheet, LayoutItem, LayoutSheet,
    LayoutSpec, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument, RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_palette::Rgba8;
use curvebook_render::{RasterImage, RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};
use curvebook_testkit::sha256_bytes;
use curvebook_text::{BUNDLED_FONTS, has_bundled_fonts_feature, parse_markup};
use flate2::read::ZlibDecoder;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn assert_cleanroom_font_baseline() {
    if !has_bundled_fonts_feature() {
        assert!(BUNDLED_FONTS.is_empty(), "cleanroom build without bundled-fonts must not link bundled font blobs");
    }
}

fn read_u32_be(bytes: &[u8], off: usize) -> Option<u32> {
    let &[b0, b1, b2, b3] = bytes.get(off..off + 4)? else {
        return None;
    };
    Some(u32::from_be_bytes([b0, b1, b2, b3]))
}

fn decode_png_idat_len_and_signature(png: &[u8]) -> Result<usize, Box<dyn std::error::Error>> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("missing PNG signature").into());
    }
    let mut pos = 8usize;
    let mut idat = Vec::new();
    while pos + 12 <= png.len() {
        let len = read_u32_be(png, pos).ok_or("png chunk length")? as usize;
        let kind = png.get(pos + 4..pos + 8).ok_or("png chunk type")?;
        let data = png.get(pos + 8..pos + 8 + len).ok_or("png chunk body")?;
        if kind == b"IDAT" {
            idat.extend_from_slice(data);
        }
        pos = pos + 12 + len;
        if kind == b"IEND" {
            break;
        }
    }
    let mut dec = ZlibDecoder::new(idat.as_slice());
    let mut raw = Vec::new();
    dec.read_to_end(&mut raw)?;
    Ok(raw.len())
}

fn count_non_white_pixels_in_rect(img: &RasterImage, scene: &ChartScene, rect: RectPt) -> usize {
    let sx = f64::from(img.width_px) / scene.width_pt;
    let sy = f64::from(img.height_px) / scene.height_pt;
    let x0 = (rect.x * sx).floor().max(0.0) as u32;
    let y0 = (rect.y * sy).floor().max(0.0) as u32;
    let x1 = (rect.right() * sx).ceil().min(f64::from(img.width_px)) as u32;
    let y1 = (rect.bottom() * sy).ceil().min(f64::from(img.height_px)) as u32;

    let mut count = 0usize;
    for py in y0..y1 {
        for px in x0..x1 {
            let idx = ((py as usize) * (img.width_px as usize) + (px as usize)) * 4;
            if let Some(px_rgba) = img.rgba.get(idx..idx + 4)
                && px_rgba != [255, 255, 255, 255]
            {
                count += 1;
            }
        }
    }
    count
}

fn slice_pixels_in_rect(img: &RasterImage, scene: &ChartScene, rect: RectPt) -> Vec<u8> {
    let sx = f64::from(img.width_px) / scene.width_pt;
    let sy = f64::from(img.height_px) / scene.height_pt;
    let x0 = (rect.x * sx).floor().max(0.0) as u32;
    let y0 = (rect.y * sy).floor().max(0.0) as u32;
    let x1 = (rect.right() * sx).ceil().min(f64::from(img.width_px)) as u32;
    let y1 = (rect.bottom() * sy).ceil().min(f64::from(img.height_px)) as u32;

    let mut out = Vec::new();
    for py in y0..y1 {
        for px in x0..x1 {
            let idx = ((py as usize) * (img.width_px as usize) + (px as usize)) * 4;
            if let Some(px_rgba) = img.rgba.get(idx..idx + 4) {
                out.extend_from_slice(px_rgba);
            }
        }
    }
    out
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

fn build_equation_project() -> Result<(ProjectDocument, GraphSheet, LayoutSheet), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("RenderDeterminism")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6)?;
    let pts = [(0.5, 18.0), (1.0, 29.0), (2.0, 48.0), (4.0, 74.0), (8.0, 91.0), (16.0, 97.0)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        *xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()).ok_or("missing xy cell")? = Cell::number(y);
    }
    let ds = DataSheet::new("xy-1", "Enzyme Kinetics", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds.clone())?;

    let nl_4pl = build_nonlinear_result(
        "res-4pl",
        "xy-1",
        "log_agonist_vs_response_4pl",
        None,
        vec![
            ResultScalarEntry { key: "Bottom".to_owned(), data_set_index: Some(0), value: 12.5, units: None },
            ResultScalarEntry { key: "Top".to_owned(), data_set_index: Some(0), value: 98.2, units: None },
            ResultScalarEntry { key: "LogEC50".to_owned(), data_set_index: Some(0), value: -6.25, units: None },
            ResultScalarEntry { key: "HillSlope".to_owned(), data_set_index: Some(0), value: 1.45, units: None },
        ],
    )?;
    doc.add_result_sheet(nl_4pl)?;

    let lin_spec = AnalysisSpec::new("res-lin", "xy-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let mut lin_res = ResultSheet::new(lin_spec)?;
    lin_res.status = ResultStatus::UpToDate;
    lin_res.scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: 2.5, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -1.25, units: None },
    ];
    doc.add_result_sheet(lin_res)?;

    let rad_res = build_nonlinear_result(
        "res-rad",
        "xy-1",
        "one_site_specific_ligand_depletion",
        None,
        vec![
            ResultScalarEntry { key: "Rt".to_owned(), data_set_index: Some(0), value: 14.0, units: None },
            ResultScalarEntry { key: "Kd".to_owned(), data_set_index: Some(0), value: 3.5, units: None },
            ResultScalarEntry { key: "NS".to_owned(), data_set_index: Some(0), value: 0.12, units: None },
        ],
    )?;
    doc.add_result_sheet(rad_res)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-1", &ds)?;
    graph.spec.title = String::from("<b>Kinetics</b> (α = 0.05)");
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
        result_sheet_id: SheetId::new("res-rad"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(68.0, 128.0, 260.0, 44.0),
        font_size_pt: 9.0,
        bordered: true,
    });
    graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: DocRectPt::new(68.0, 180.0, 190.0, 22.0),
        markdown: String::from("<b>IC<sub>50</sub></b> = 2.4 ± 0.3 µM ∈ [10<sup>-6</sup>, 10<sup>-4</sup>]"),
        bordered: true,
    });
    doc.add_graph_sheet(graph.clone())?;

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-eq"),
        graph_sheet_id: SheetId::new("g-1"),
        bounds_pt: DocRectPt::new(16.0, 16.0, 320.0, 220.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("callout-eq"),
        markdown: String::from("<b>Fig. 1</b> α/β/Δ 10<sup>-6</sup>"),
        bounds_pt: DocRectPt::new(348.0, 24.0, 132.0, 40.0),
        font_size_pt: 10.0,
    });
    let layout = LayoutSheet::new(SheetId::new("layout-1"), "Equation Layout", layout_spec)?;
    doc.add_layout_sheet(layout.clone())?;

    Ok((doc, graph, layout))
}

#[test]
fn rgba_png_and_tiff_are_byte_identical_for_2d_math_model_equations_and_rich_text() -> TestResult {
    assert_cleanroom_font_baseline();

    // Direct 2D Knuth-boxes-and-glue equations covering nested fractions, stacked scripts,
    // roots, summations, integrals, scaled interval delimiters, Greek/math symbols, and negative coeffs.
    let mut math_scene = ChartScene::new("math-2d", "2D TeX Suite", 480.0, 320.0, Some(Rgba8::WHITE));
    let eq_specs = [
        (DocRectPt::new(20.0, 18.0, 0.0, 0.0), r"Y = \frac{1}{1 + \frac{\alpha}{\beta}} + \binom{n}{k}"),
        (DocRectPt::new(20.0, 74.0, 0.0, 0.0), r"S^n_j + \sqrt[3]{X + \Delta} - 2.5 \cdot 10^{-5}"),
        (
            DocRectPt::new(20.0, 130.0, 0.0, 0.0),
            r"\sum_{i=1}^{n} \frac{x_i}{\sigma_i^2} + \int_{0}^{\infty} \lambda \cdot e^{-\lambda \cdot t} \, dt",
        ),
        (DocRectPt::new(20.0, 196.0, 0.0, 0.0), r"\left[\frac{X - \mu}{\sigma}\right] \cdot \left(0 \le X \le t_e\right) \pm 0.05"),
    ];
    for (rect, tex) in eq_specs {
        render_tex_equation_box(&mut math_scene, rect, tex, 11.0, true);
    }
    math_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>IC<sub>50</sub></b> = 1.8 ± 0.2 µM (α, β, γ, Δ, Ω)"),
        anchor_pt: PointPt::new(24.0, 276.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let (doc, graph, layout) = build_equation_project()?;
    let g_scene = compile_graph(&graph, &doc)?;
    let l_scene = compile_layout(&layout, &doc)?;

    for (name, scene, target) in [
        ("math-72", &math_scene, RenderTargetSpec { width_px: 480, height_px: 320, dpi: 72 }),
        ("math-150", &math_scene, RenderTargetSpec { width_px: 1000, height_px: 667, dpi: 150 }),
        ("graph-150", &g_scene, RenderTargetSpec { width_px: 750, height_px: 540, dpi: 150 }),
        ("layout-150", &l_scene, RenderTargetSpec { width_px: 800, height_px: 560, dpi: 150 }),
    ] {
        let img_1 = render_scene(scene, target)?;
        let img_2 = render_scene(scene, target)?;
        assert_eq!(img_1.rgba, img_2.rgba, "RGBA buffer mismatch on repeat render for {name}");
        assert_eq!(sha256_bytes(&img_1.rgba), sha256_bytes(&img_2.rgba), "RGBA SHA-256 mismatch for {name}");

        let png_1 = encode_png(&img_1)?;
        let png_2 = encode_png(&img_2)?;
        assert_eq!(png_1, png_2, "PNG bytes not byte-identical for {name}");
        let raw_len = decode_png_idat_len_and_signature(&png_1)?;
        assert_eq!(raw_len, ((target.width_px as usize) * 4 + 1) * (target.height_px as usize));

        let tiff_def_1 = encode_tiff(&img_1, TiffCompression::Deflate)?;
        let tiff_def_2 = encode_tiff(&img_2, TiffCompression::Deflate)?;
        assert_eq!(tiff_def_1, tiff_def_2, "TIFF Deflate not byte-identical for {name}");

        let tiff_lzw_1 = encode_tiff(&img_1, TiffCompression::Lzw)?;
        let tiff_lzw_2 = encode_tiff(&img_2, TiffCompression::Lzw)?;
        assert_eq!(tiff_lzw_1, tiff_lzw_2, "TIFF LZW not byte-identical for {name}");
    }

    // Verify actual ink rasterized inside each auto-sized equation box in `math_scene`.
    let img_math = render_scene(&math_scene, RenderTargetSpec { width_px: 480, height_px: 320, dpi: 72 })?;
    for (rect, _) in eq_specs {
        let hit = resolve_model_equation_hit_bounds(&math_scene, rect, 11.0, true).ok_or("missing equation hit bounds")?;
        let ink = count_non_white_pixels_in_rect(&img_math, &math_scene, hit);
        assert!(ink >= 40, "expected rasterized equation ink inside {hit:?}, got {ink}");
    }

    Ok(())
}

#[test]
fn upstream_coefficient_and_model_edits_refresh_rgba_pixels_without_stale_ink() -> TestResult {
    assert_cleanroom_font_baseline();
    let (mut doc, mut graph, layout) = build_equation_project()?;
    let target = RenderTargetSpec { width_px: 720, height_px: 520, dpi: 150 };

    let g_v1 = compile_graph(&graph, &doc)?;
    let l_v1 = compile_layout(&layout, &doc)?;
    let img_g1_a = render_scene(&g_v1, target)?;
    let img_g1_b = render_scene(&g_v1, target)?;
    assert_eq!(img_g1_a.rgba, img_g1_b.rgba);
    let png_g1 = encode_png(&img_g1_a)?;

    let eq_bounds_4pl_v1 =
        resolve_model_equation_hit_bounds(&g_v1, DocRectPt::new(68.0, 28.0, 0.0, 0.0), 10.0, true).ok_or("missing 4PL hit bounds v1")?;
    let eq_bounds_lin_v1 =
        resolve_model_equation_hit_bounds(&g_v1, DocRectPt::new(68.0, 88.0, 0.0, 0.0), 10.0, true).ok_or("missing linear hit bounds v1")?;

    // Mutate upstream scalars: wider 4-digit Top + negative slope + model switch on third box.
    doc.result_sheets[0].scalars[1].value = 2480.0;
    doc.result_sheets[0].scalars[2].value = -4.375;
    doc.result_sheets[1].scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: -18.75, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -4.125, units: None },
    ];
    doc.result_sheets[2].analysis.kind = AnalysisKind::NonlinearRegression(NonlinearFitSpec {
        custom_dsl: Some(String::from("Y = Delta * exp(-theta * X) + sqrt(X + Km)")),
        ..NonlinearFitSpec::new("custom")
    });

    let g_v2 = compile_graph(&graph, &doc)?;
    let l_v2 = compile_layout(&layout, &doc)?;
    let img_g2_a = render_scene(&g_v2, target)?;
    let img_g2_b = render_scene(&g_v2, target)?;
    assert_eq!(img_g2_a.rgba, img_g2_b.rgba, "mutated graph must re-render deterministically");
    assert_ne!(img_g1_a.rgba, img_g2_a.rgba, "RGBA buffer must refresh on upstream coefficient edits");
    assert_ne!(encode_png(&img_g2_a)?, png_g1, "PNG output must refresh on upstream coefficient edits");

    let img_l1 = render_scene(&l_v1, target)?;
    let img_l2_a = render_scene(&l_v2, target)?;
    let img_l2_b = render_scene(&l_v2, target)?;
    assert_eq!(img_l2_a.rgba, img_l2_b.rgba, "mutated layout must re-render deterministically");
    assert_ne!(img_l1.rgba, img_l2_a.rgba, "layout RGBA buffer must refresh on upstream coefficient edits");

    let eq_bounds_4pl_v2 =
        resolve_model_equation_hit_bounds(&g_v2, DocRectPt::new(68.0, 28.0, 0.0, 0.0), 10.0, true).ok_or("missing 4PL hit bounds v2")?;
    let eq_bounds_lin_v2 =
        resolve_model_equation_hit_bounds(&g_v2, DocRectPt::new(68.0, 88.0, 0.0, 0.0), 10.0, true).ok_or("missing linear hit bounds v2")?;
    assert!(eq_bounds_4pl_v2.width > eq_bounds_4pl_v1.width, "4PL auto-box must widen when Top grows to 2480");
    assert!(eq_bounds_lin_v2.width > eq_bounds_lin_v1.width, "linear auto-box must widen when negative coeffs lengthen");

    assert_ne!(
        slice_pixels_in_rect(&img_g1_a, &g_v1, eq_bounds_4pl_v1),
        slice_pixels_in_rect(&img_g2_a, &g_v2, eq_bounds_4pl_v1),
        "4PL equation region pixels must change"
    );
    assert_ne!(
        slice_pixels_in_rect(&img_g1_a, &g_v1, eq_bounds_lin_v1),
        slice_pixels_in_rect(&img_g2_a, &g_v2, eq_bounds_lin_v1),
        "linear equation region pixels must change"
    );

    // Dirty status and Symbolic mode toggle must also refresh RGBA immediately and deterministically.
    doc.result_sheets[1].status = ResultStatus::Dirty;
    let g_dirty = compile_graph(&graph, &doc)?;
    let img_dirty_a = render_scene(&g_dirty, target)?;
    let img_dirty_b = render_scene(&g_dirty, target)?;
    assert_eq!(img_dirty_a.rgba, img_dirty_b.rgba);
    assert_ne!(img_dirty_a.rgba, img_g2_a.rgba, "Dirty status must replace stale equation pixels");

    graph.spec.annotations[0] = GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-4pl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(68.0, 28.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    };
    let g_sym = compile_graph(&graph, &doc)?;
    let img_sym_a = render_scene(&g_sym, target)?;
    let img_sym_b = render_scene(&g_sym, target)?;
    assert_eq!(img_sym_a.rgba, img_sym_b.rgba);
    assert_ne!(img_sym_a.rgba, img_dirty_a.rgba, "Symbolic toggle must refresh RGBA pixels");

    // Standalone auto-sized ModelEquation box proves newly widened tail region paints ink over clean white.
    let mut box_v1 = ChartScene::new("eq-tail", "Tail", 360.0, 100.0, Some(Rgba8::WHITE));
    let mut tail_doc = doc;
    tail_doc.result_sheets[1].status = ResultStatus::UpToDate;
    tail_doc.result_sheets[1].scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: 2.0, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: 1.0, units: None },
    ];
    let p_tail = curvebook_chart::equation_box::ModelEquationParams {
        result_sheet_id: &SheetId::new("res-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
        font_size_pt: 11.0,
        bordered: true,
    };
    curvebook_chart::equation_box::render_model_equation_annotation(&mut box_v1, &tail_doc, &p_tail);
    let hb_tail_1 = resolve_model_equation_hit_bounds(&box_v1, p_tail.bounds_pt, 11.0, true).ok_or("missing tail hb1")?;

    tail_doc.result_sheets[1].scalars[0].value = -1234.0;
    tail_doc.result_sheets[1].scalars[1].value = -0.000_045_6;
    let mut box_v2 = ChartScene::new("eq-tail", "Tail", 360.0, 100.0, Some(Rgba8::WHITE));
    curvebook_chart::equation_box::render_model_equation_annotation(&mut box_v2, &tail_doc, &p_tail);
    let hb_tail_2 = resolve_model_equation_hit_bounds(&box_v2, p_tail.bounds_pt, 11.0, true).ok_or("missing tail hb2")?;
    assert!(hb_tail_2.width > hb_tail_1.width + 20.0);

    let box_target = RenderTargetSpec { width_px: 360, height_px: 100, dpi: 72 };
    let img_box_1 = render_scene(&box_v1, box_target)?;
    let img_box_2 = render_scene(&box_v2, box_target)?;
    let tail_rect = RectPt::new(hb_tail_1.right() + 2.0, hb_tail_2.y, hb_tail_2.right() - hb_tail_1.right() - 2.0, hb_tail_2.height);
    assert_eq!(count_non_white_pixels_in_rect(&img_box_1, &box_v1, tail_rect), 0, "tail region must start clean white");
    assert!(count_non_white_pixels_in_rect(&img_box_2, &box_v2, tail_rect) > 20, "expanded equation tail must gain glyph/border ink");

    Ok(())
}

#[test]
fn r20_5_fit_summary_png_export_is_byte_deterministic() -> TestResult {
    let (doc, mut graph, _) = build_equation_project()?;
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
    let target = RenderTargetSpec { width_px: 360, height_px: 260, dpi: 72 };
    let img_1 = render_scene(&s1, target)?;
    let img_2 = render_scene(&s2, target)?;
    let png_1 = encode_png(&img_1)?;
    let png_2 = encode_png(&img_2)?;
    assert_eq!(sha256_bytes(&png_1), sha256_bytes(&png_2));
    assert!(decode_png_idat_len_and_signature(&png_1)? > 0);
    // Right gutter region outside plot_rect_pt must contain rendered fit-summary ink.
    let gutter = RectPt::new(s1.plot_rect_pt.right() + 4.0, s1.plot_rect_pt.y, s1.width_pt - s1.plot_rect_pt.right() - 4.0, s1.plot_rect_pt.height);
    assert!(count_non_white_pixels_in_rect(&img_1, &s1, gutter) > 40, "expected rendered fit-summary pixels in right gutter");
    Ok(())
}
