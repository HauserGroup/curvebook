//! Integration tests for `R10.19` bullet 2 in `curvebook-pdf`:
//! byte-identical `%PDF-1.7` exports for 2D Knuth-boxes-and-glue `ModelEquation` / rich-text scenes
//! (Core-14 fallback, cleanroom embedded TrueType `/F6`, and outlined glyph modes)
//! and live `%PDF-1.7` binary & content-stream refresh on upstream edits.

use std::collections::BTreeMap;
use std::io::Read;

use curvebook_chart::equation_box::render_tex_equation_box;
use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor, compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, FittedCurveOverlay, GraphAnnotation, GraphSheet, LayoutItem, LayoutSheet,
    LayoutSpec, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument, RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_palette::Rgba8;
use curvebook_pdf::{PdfExportOptions, PdfVersion, render_pdf, render_pdf_pages};
use curvebook_testkit::sha256_bytes;
use curvebook_text::{BUNDLED_FONTS, build_minimal_truetype_font, has_bundled_fonts_feature, parse_markup};
use flate2::read::ZlibDecoder;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct ParsedPdf {
    objects_text: String,
    page_streams: Vec<String>,
    fontfile2_streams: Vec<Vec<u8>>,
}

fn assert_cleanroom_font_baseline() {
    if !has_bundled_fonts_feature() {
        assert!(BUNDLED_FONTS.is_empty(), "cleanroom build without bundled-fonts must not link bundled font blobs");
    }
}

fn extract_pdf_tj_text(stream: &str) -> String {
    let mut out = String::new();
    for line in stream.lines() {
        let Some(inner) = line.strip_prefix('(').and_then(|s| s.strip_suffix(") Tj")) else {
            continue;
        };
        let bytes = inner.as_bytes();
        let mut i = 0usize;
        while let Some(&b) = bytes.get(i) {
            if b == b'\\'
                && let Some(&next) = bytes.get(i + 1)
            {
                if (b'0'..=b'7').contains(&next) {
                    let end = (i + 4).min(bytes.len());
                    if let Some(oct_bytes) = bytes.get(i + 1..end)
                        && let Ok(oct_str) = std::str::from_utf8(oct_bytes)
                        && let Ok(code) = u8::from_str_radix(oct_str, 8)
                    {
                        out.push(char::from(code));
                        i = end;
                        continue;
                    }
                }
                out.push(char::from(next));
                i += 2;
            } else {
                out.push(char::from(b));
                i += 1;
            }
        }
        out.push('|');
    }
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn extract_pdf_int(dict: &str, key: &str) -> Option<usize> {
    let idx = dict.find(key)?;
    let rest = dict.get(idx + key.len()..)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn parse_pdf17_strict(bytes: &[u8]) -> Result<ParsedPdf, Box<dyn std::error::Error>> {
    if !bytes.starts_with(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n") {
        return Err(String::from("missing %PDF-1.7 binary header").into());
    }
    if !bytes.ends_with(b"%%EOF\n") {
        return Err(String::from("missing %%EOF trailer").into());
    }

    let startxref_pos = rfind_subslice(bytes, b"startxref\n").ok_or("missing startxref")?;
    let after = std::str::from_utf8(bytes.get(startxref_pos + b"startxref\n".len()..).ok_or("startxref tail")?)?;
    let xref_offset: usize = after.lines().next().ok_or("missing xref offset line")?.trim().parse()?;
    let trailer_rel = find_subslice(bytes.get(xref_offset..startxref_pos).ok_or("xref slice")?, b"trailer\n").ok_or("missing trailer")?;
    let xref_str = std::str::from_utf8(bytes.get(xref_offset..xref_offset + trailer_rel).ok_or("xref utf8")?)?;

    let mut lines = xref_str.lines();
    if lines.next() != Some("xref") {
        return Err(String::from("missing xref header").into());
    }
    let count_line = lines.next().ok_or("missing xref range")?;
    let total_entries: usize = count_line.split_whitespace().nth(1).ok_or("missing xref total")?.parse()?;

    let mut offsets = Vec::with_capacity(total_entries);
    for (idx, line) in lines.enumerate() {
        if line.len() != 19 {
            return Err(format!("xref entry must be 19 chars: `{line}`").into());
        }
        let off: usize = line.get(0..10).ok_or("xref off")?.parse()?;
        let kind = line.chars().nth(17).ok_or("xref kind")?;
        if idx > 0 && (kind != 'n' || off >= xref_offset) {
            return Err(format!("invalid xref entry {idx}").into());
        }
        offsets.push((off, kind));
    }
    if offsets.len() != total_entries {
        return Err(format!("xref count mismatch: {} vs {total_entries}", offsets.len()).into());
    }

    let mut objects = BTreeMap::new();
    let mut page_streams = Vec::new();
    let mut fontfile2_streams = Vec::new();
    for (obj_id, &(off, kind)) in offsets.iter().enumerate() {
        if kind != 'n' {
            continue;
        }
        let prefix = format!("{obj_id} 0 obj\n");
        let tail = bytes.get(off..xref_offset).ok_or("object range")?;
        if !tail.starts_with(prefix.as_bytes()) {
            return Err(format!("missing object header {prefix}").into());
        }
        let body_start = off + prefix.len();
        let endobj_rel = find_subslice(bytes.get(body_start..xref_offset).ok_or("obj body")?, b"endobj\n").ok_or("missing endobj")?;
        let body = bytes.get(body_start..body_start + endobj_rel).ok_or("body slice")?;
        objects.insert(obj_id, String::from_utf8_lossy(body).into_owned());

        if let Some(stream_rel) = find_subslice(body, b"\nstream\n") {
            let dict_str = String::from_utf8_lossy(body.get(..stream_rel).ok_or("dict slice")?).into_owned();
            let data_start = stream_rel + b"\nstream\n".len();
            let endstream_rel = find_subslice(body.get(data_start..).ok_or("stream tail")?, b"\nendstream").ok_or("missing endstream")?;
            let raw_stream = body.get(data_start..data_start + endstream_rel).ok_or("raw stream")?;
            let declared_len = extract_pdf_int(&dict_str, "/Length ").ok_or("missing /Length")?;
            if raw_stream.len() != declared_len {
                return Err(format!("stream length mismatch: {} vs {declared_len}", raw_stream.len()).into());
            }
            let decoded = if dict_str.contains("/Filter /FlateDecode") {
                let mut dec = ZlibDecoder::new(raw_stream);
                let mut out = Vec::new();
                dec.read_to_end(&mut out)?;
                out
            } else {
                raw_stream.to_vec()
            };
            if dict_str.contains("/Length1 ") {
                fontfile2_streams.push(decoded);
            } else {
                page_streams.push(String::from_utf8(decoded)?);
            }
        }
    }

    let objects_text = objects.into_values().collect::<Vec<_>>().join("\n");
    Ok(ParsedPdf { objects_text, page_streams, fontfile2_streams })
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

fn build_pdf_equation_project() -> Result<(ProjectDocument, GraphSheet, LayoutSheet), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("PdfDeterminism")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6)?;
    let pts = [(0.5, 16.0), (1.0, 28.0), (2.0, 49.0), (4.0, 76.0), (8.0, 92.0), (16.0, 98.0)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        *xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()).ok_or("missing xy cell")? = Cell::number(y);
    }
    let ds = DataSheet::new("xy-pdf", "Dose-Response", TableArchetype::Xy(xy))?;
    doc.add_data_sheet(ds.clone())?;

    let res_4pl = build_nonlinear_result(
        "res-4pl",
        "xy-pdf",
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

    let lin_spec = AnalysisSpec::new("res-lin", "xy-pdf", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?;
    let mut lin_res = ResultSheet::new(lin_spec)?;
    lin_res.status = ResultStatus::UpToDate;
    lin_res.scalars = vec![
        ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: 2.5, units: None },
        ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -1.25, units: None },
    ];
    doc.add_result_sheet(lin_res)?;

    let res_interval = build_nonlinear_result(
        "res-interval",
        "xy-pdf",
        "beta_growth_then_decay",
        None,
        vec![
            ResultScalarEntry { key: "Ym".to_owned(), data_set_index: Some(0), value: 42.5, units: None },
            ResultScalarEntry { key: "te".to_owned(), data_set_index: Some(0), value: 12.0, units: None },
            ResultScalarEntry { key: "tm".to_owned(), data_set_index: Some(0), value: 5.5, units: None },
        ],
    )?;
    doc.add_result_sheet(res_interval)?;

    let mut graph = GraphSheet::new_uninitialized_for_data("g-pdf", &ds)?;
    graph.spec.title = String::from("<b>EC<sub>50</sub></b> &amp; Decay (α = 0.05)");
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
        id: String::from("panel-pdf"),
        graph_sheet_id: SheetId::new("g-pdf"),
        bounds_pt: DocRectPt::new(16.0, 16.0, 320.0, 220.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("note-pdf"),
        markdown: String::from("<i>ΔG</i> = -5.4 kJ/mol"),
        bounds_pt: DocRectPt::new(348.0, 24.0, 132.0, 40.0),
        font_size_pt: 10.0,
    });
    let layout = LayoutSheet::new(SheetId::new("layout-pdf"), "PDF Equation Layout", layout_spec)?;
    doc.add_layout_sheet(layout.clone())?;

    Ok((doc, graph, layout))
}

#[test]
fn pdf17_exports_are_byte_identical_across_core14_embedded_ttf_and_outlined_modes() -> TestResult {
    assert_cleanroom_font_baseline();
    let minimal_ttf = build_minimal_truetype_font();

    let mut math_scene = ChartScene::new("pdf-math", "2D TeX PDF", 500.0, 340.0, Some(Rgba8::WHITE));
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
        rich: parse_markup("<b>IC<sub>50</sub></b> = 1.8 ± 0.2 µM (α, β, γ, Δ, Ω)"),
        anchor_pt: PointPt::new(24.0, 286.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let (doc, graph, layout) = build_pdf_equation_project()?;
    let g_scene = compile_graph(&graph, &doc)?;
    let l_scene = compile_layout(&layout, &doc)?;

    let configs = [
        ("core14", PdfExportOptions { version: PdfVersion::Pdf17, embedded_truetype_font: None, outline_text: false, ..PdfExportOptions::default() }),
        (
            "embedded-ttf",
            PdfExportOptions {
                version: PdfVersion::Pdf17,
                embedded_truetype_font: Some(minimal_ttf.clone()),
                outline_text: false,
                ..PdfExportOptions::default()
            },
        ),
        (
            "outlined",
            PdfExportOptions { version: PdfVersion::Pdf17, embedded_truetype_font: None, outline_text: true, ..PdfExportOptions::default() },
        ),
    ];

    for (cfg_name, opts) in &configs {
        for (scene_name, scene) in [("math-2d", &math_scene), ("graph", &g_scene), ("layout", &l_scene)] {
            let pdf_1 = render_pdf(scene, opts)?;
            let pdf_2 = render_pdf(scene, opts)?;
            assert_eq!(pdf_1, pdf_2, "PDF binary differed ({cfg_name}, {scene_name})");
            assert_eq!(sha256_bytes(&pdf_1), sha256_bytes(&pdf_2));

            let parsed = parse_pdf17_strict(&pdf_1)?;
            assert_eq!(parsed.page_streams.len(), 1);
            let stream = &parsed.page_streams[0];

            match *cfg_name {
                "embedded-ttf" => {
                    assert!(parsed.objects_text.contains("/Subtype /TrueType"));
                    assert!(parsed.objects_text.contains("/FontDescriptor"));
                    assert_eq!(parsed.fontfile2_streams.as_slice(), std::slice::from_ref(&minimal_ttf));
                    assert!(stream.contains("/F6 ") && stream.contains("/F4 "), "expected /F6 TTF + /F4 Symbol in `{cfg_name}`");
                }
                "core14" => {
                    if BUNDLED_FONTS.is_empty() {
                        assert!(!parsed.objects_text.contains("/FontFile2"));
                    }
                    assert!(stream.contains("/F4 ") && stream.contains(" Tj\n"));
                    assert!(!stream.contains("(??) Tj"), "never substitute `?` placeholders");
                }
                _ => {
                    assert!(!stream.contains(" Tj\n"), "outlined PDF must not emit Tj");
                    assert!(stream.contains(" m\n") && stream.lines().any(|l| l == "f" || l == "S"));
                }
            }
        }

        // Multi-page PDF export combining graph + layout scenes must also be byte-identical.
        let multi_1 = render_pdf_pages(&[g_scene.clone(), l_scene.clone()], opts)?;
        let multi_2 = render_pdf_pages(&[g_scene.clone(), l_scene.clone()], opts)?;
        assert_eq!(multi_1, multi_2, "multi-page PDF binary differed ({cfg_name})");
        let parsed_multi = parse_pdf17_strict(&multi_1)?;
        assert_eq!(parsed_multi.page_streams.len(), 2);

        // Uncompressed page stream export must also be byte-identical.
        let raw_opts = PdfExportOptions { compress_streams: false, ..opts.clone() };
        assert_eq!(render_pdf(&g_scene, &raw_opts)?, render_pdf(&g_scene, &raw_opts)?, "raw stream PDF differed ({cfg_name})");
    }

    Ok(())
}

#[test]
fn upstream_coefficient_and_model_edits_refresh_pdf17_binary_and_streams() -> TestResult {
    assert_cleanroom_font_baseline();
    let (mut doc, mut graph, layout) = build_pdf_equation_project()?;
    let minimal_ttf = build_minimal_truetype_font();

    let configs = [
        ("core14", PdfExportOptions { version: PdfVersion::Pdf17, embedded_truetype_font: None, outline_text: false, ..PdfExportOptions::default() }),
        (
            "embedded-ttf",
            PdfExportOptions {
                version: PdfVersion::Pdf17,
                embedded_truetype_font: Some(minimal_ttf),
                outline_text: false,
                ..PdfExportOptions::default()
            },
        ),
        (
            "outlined",
            PdfExportOptions { version: PdfVersion::Pdf17, embedded_truetype_font: None, outline_text: true, ..PdfExportOptions::default() },
        ),
    ];

    let g_v1 = compile_graph(&graph, &doc)?;
    let l_v1 = compile_layout(&layout, &doc)?;
    let mut v1_binaries = Vec::new();
    let mut l1_binaries = Vec::new();
    for (cfg_name, opts) in &configs {
        let p_g = render_pdf(&g_v1, opts)?;
        let p_l = render_pdf(&l_v1, opts)?;
        assert_eq!(p_g, render_pdf(&g_v1, opts)?, "v1 graph repeat mismatch ({cfg_name})");
        assert_eq!(p_l, render_pdf(&l_v1, opts)?, "v1 layout repeat mismatch ({cfg_name})");
        if *cfg_name != "outlined" {
            let parsed = parse_pdf17_strict(&p_g)?;
            let text = extract_pdf_tj_text(&parsed.page_streams[0]);
            assert!(text.contains("98.2") && text.contains("6.25") && text.contains("2.5"), "missing v1 coeffs in {cfg_name}: {text}");
        }
        v1_binaries.push(p_g);
        l1_binaries.push(p_l);
    }

    // Mutate upstream scalars + switch third model to custom DSL with sqrt & Greek.
    doc.result_sheets[0].scalars[1].value = 1840.0;
    doc.result_sheets[0].scalars[2].value = -4.375;
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
    for (idx, (cfg_name, opts)) in configs.iter().enumerate() {
        let p_g2_a = render_pdf(&g_v2, opts)?;
        let p_g2_b = render_pdf(&g_v2, opts)?;
        assert_eq!(p_g2_a, p_g2_b, "mutated PDF must be byte-identical across runs ({cfg_name})");
        assert_ne!(v1_binaries[idx], p_g2_a, "graph PDF binary must refresh ({cfg_name})");

        let p_l2_a = render_pdf(&l_v2, opts)?;
        let p_l2_b = render_pdf(&l_v2, opts)?;
        assert_eq!(p_l2_a, p_l2_b, "mutated layout PDF must be byte-identical ({cfg_name})");
        assert_ne!(l1_binaries[idx], p_l2_a, "layout PDF binary must refresh ({cfg_name})");

        if *cfg_name != "outlined" {
            let parsed = parse_pdf17_strict(&p_g2_a)?;
            let text = extract_pdf_tj_text(&parsed.page_streams[0]);
            for stale in ["98.2", "6.25", "2.5 "] {
                assert!(!text.contains(stale), "stale literal `{stale}` remained in `{cfg_name}` PDF text: {text}");
            }
            for fresh in ["1840", "4.375", "-3.75", "7.25", "77.5", "0.45"] {
                assert!(text.contains(fresh), "refreshed literal `{fresh}` missing from `{cfg_name}` PDF text: {text}");
            }
        }
    }

    // Dirty status & Symbolic toggle refresh PDF deterministically.
    doc.result_sheets[1].status = ResultStatus::Dirty;
    let g_dirty = compile_graph(&graph, &doc)?;
    let p_dirty_a = render_pdf(&g_dirty, &configs[1].1)?;
    let p_dirty_b = render_pdf(&g_dirty, &configs[1].1)?;
    assert_eq!(p_dirty_a, p_dirty_b);
    let dirty_text = extract_pdf_tj_text(&parse_pdf17_strict(&p_dirty_a)?.page_streams[0]);
    assert!(dirty_text.contains("(Fit pending)") && !dirty_text.contains("-3.75"), "dirty text={dirty_text}");

    graph.spec.annotations[0] = GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-4pl"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(68.0, 28.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    };
    let g_sym = compile_graph(&graph, &doc)?;
    let p_sym_a = render_pdf(&g_sym, &configs[1].1)?;
    let p_sym_b = render_pdf(&g_sym, &configs[1].1)?;
    assert_eq!(p_sym_a, p_sym_b);
    let sym_text = extract_pdf_tj_text(&parse_pdf17_strict(&p_sym_a)?.page_streams[0]);
    assert!(sym_text.contains("HillSlope") && !sym_text.contains("1840"), "sym text={sym_text}");

    Ok(())
}
