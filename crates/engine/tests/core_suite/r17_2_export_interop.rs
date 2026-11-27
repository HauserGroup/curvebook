#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use curvebook_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint,
    TextAnchor, compile_graph, compile_layout,
};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisThreshold, BracketLabelMode, BracketStyle, BreakOrnament, Cell, ConfidenceLevel,
    ConnectingLineStyle, CropInsetsPt, CurveSamplePoint, DataSheet, DiagnosticPlotKind, EquationDisplayMode, FittedCurveOverlay, FramePreset,
    GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, ImageBorderSpec, ImageMimeKind, JournalWidthPreset, LayoutItem, LayoutSheet,
    LayoutSpec, LineConnectionKind, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod,
    PValueFormat, PairwiseBracketSpec, ProjectDocument, ProjectImageAsset, RectPt as DocRectPt, RegressionOverlaySpec, ResultSheet, SheetId,
    SubcolumnMode, TableArchetype, XyTable, content_addressed_asset_id,
};
use curvebook_engine::Session;
use curvebook_palette::{LineDashStyle, MarkerStyle, OKABE_ITO_BLUE, OKABE_ITO_VERMILION, Rgba8};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions, PdfVersion, render_pdf, render_pdf_pages};
use curvebook_render::{RenderTargetSpec, TiffCompression, encode_emf, encode_png, encode_raster_emf, encode_tiff, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_text::parse_markup;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::json;

use super::common::{TempDir, col_sheet, ok_cmd};

type TestResult = Result<(), Box<dyn Error>>;

fn encode_alpha_quadrant_png(w: u32, h: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let img = RgbaImage::from_fn(w, h, |x, y| match (x >= w / 2, y >= h / 2) {
        (false, false) => Rgba([220, 40, 40, 140]),
        (true, false) => Rgba([25, 160, 70, 255]),
        (false, true) => Rgba([30, 80, 220, 255]),
        (true, true) => Rgba([235, 185, 25, 180]),
    });
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

fn encode_rgb_jpeg(w: u32, h: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    #[allow(clippy::cast_possible_truncation)]
    let img = RgbImage::from_fn(w, h, |x, y| Rgb([((x * 255) / w.max(1)) as u8, 128, ((y * 255) / h.max(1)) as u8]));
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)?;
    Ok(out)
}

struct InteropFixtures {
    doc: ProjectDocument,
    xy_broken_scene: ChartScene,
    brackets_scene: ChartScene,
    heatmap_scene: ChartScene,
    layout_scene: ChartScene,
    stress_scene: ChartScene,
}

fn build_stress_scene(png_bytes: &[u8]) -> Result<ChartScene, Box<dyn Error>> {
    let mut stress = ChartScene::new("scene-stress", "Vector & Typography Stress <Probe>", 420.0, 300.0, Some(Rgba8::WHITE));
    stress.primitives.push(ScenePrimitive::PushClip { rect: RectPt::new(16.0, 16.0, 180.0, 120.0) });
    stress.primitives.push(ScenePrimitive::PushClip { rect: RectPt::new(36.0, 30.0, 140.0, 90.0) });
    stress.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(10.0, 10.0, 200.0, 130.0)),
        fill: Some(FillPaint::solid(OKABE_ITO_BLUE.with_alpha(95))),
        stroke: Some(StrokePaint {
            color: OKABE_ITO_VERMILION.with_alpha(190),
            width_pt: 2.0,
            dash: LineDashStyle::Dashed,
            cap: LineCap::Round,
            join: LineJoin::Bevel,
        }),
        clip: None,
    });
    stress.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(20.0, 120.0)),
                PathCmd::CubicTo { c1: PointPt::new(70.0, 10.0), c2: PointPt::new(130.0, 140.0), to: PointPt::new(190.0, 40.0) },
            ],
        },
        fill: None,
        stroke: Some(StrokePaint {
            color: Rgba8::rgba(20, 20, 20, 220),
            width_pt: 2.5,
            dash: LineDashStyle::Solid,
            cap: LineCap::Square,
            join: LineJoin::Round,
        }),
        clip: None,
    });
    stress.primitives.push(ScenePrimitive::PopClip);
    stress.primitives.push(ScenePrimitive::PopClip);

    let dashes = [
        LineDashStyle::Solid,
        LineDashStyle::Dashed,
        LineDashStyle::Dotted,
        LineDashStyle::DashDot,
        LineDashStyle::DashDotDot,
        LineDashStyle::LongDash,
    ];
    for (idx, dash) in dashes.into_iter().enumerate() {
        let y = 152.0 + (idx as f64) * 14.0;
        for (x0, x1, color, width_pt) in
            [(16.0, 196.0, Rgba8::rgba(25, 95, 165, 220), 1.6), (206.0, 260.0, Rgba8::rgba(190, 50, 25, 220), if idx == 5 { 0.0 } else { 0.25 })]
        {
            stress.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x0, y), PointPt::new(x1, y)),
                fill: None,
                stroke: Some(StrokePaint { color, width_pt, dash, cap: LineCap::Butt, join: LineJoin::Miter }),
                clip: None,
            });
        }
    }

    let rgba_img = image::load_from_memory(png_bytes)?.to_rgba8();
    let (rw, rh) = rgba_img.dimensions();
    stress.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
        asset_id: String::from("stress-png-rot"),
        media_type: ImageMimeKind::Png,
        pixel_size: (rw, rh),
        rgba: rgba_img.into_raw(),
        bounds_pt: RectPt::new(280.0, 24.0, 96.0, 72.0),
        crop: CropInsetsPt { left_pt: 8.0, right_pt: 6.0, top_pt: 5.0, bottom_pt: 5.0 },
        rotation_deg: 22.5,
        clip: Some(RectPt::new(270.0, 14.0, 120.0, 100.0)),
    }));
    stress.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>K<sub>m</sub></b> = 10<sup>-3</sup> µM · αβγΔσχΩ · ±≤≥∞×÷°Å\nLine 2: <i>e\u{0301}</i> &amp; <i>x\u{0302}</i> < \"1\""),
        anchor_pt: PointPt::new(210.0, 260.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::rgba(20, 20, 20, 230),
    }));
    stress.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("Rotated 30°: μ<sub>eff</sub>"),
        anchor_pt: PointPt::new(330.0, 180.0),
        anchor: TextAnchor::End,
        font_size_pt: 9.5,
        rotation_deg: -30.0,
        color: OKABE_ITO_VERMILION,
    }));
    Ok(stress)
}

fn build_interop_fixtures() -> Result<InteropFixtures, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("R17_2_ExportInterop")?;

    // 1. XY Broken Axis + Semi-Transparent Band + Rich Typography + Dash Thresholds + 2D Equation
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 8)?;
    for (r, (x, y)) in
        [(1.0, 12.0), (3.0, 22.0), (6.0, 48.0), (9.0, 74.0), (110.0, 88.0), (140.0, 93.0), (170.0, 96.0), (195.0, 98.0)].into_iter().enumerate()
    {
        xy.set_x_cell(r, Cell::number(x))?;
        xy.data_sets[0].cells[r][0] = Cell::number(y - 3.5);
        xy.data_sets[0].cells[r][1] = Cell::number(y + 3.5);
    }
    doc.add_data_sheet(DataSheet::new("d-xy", "Dose Kinetics", TableArchetype::Xy(xy))?)?;

    let fit_spec = AnalysisSpec::new("fit-xy", "d-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten")))?;
    let mut fit_res = ResultSheet::new(fit_spec)?;
    let curve_pts = (1..=200)
        .map(|i| {
            let x = f64::from(i);
            let y = 100.0 * x / (18.0 + x);
            CurveSamplePoint { x, y, ci_lower: Some((y - 6.0).max(0.0)), ci_upper: Some((y + 6.0).min(108.0)), pred_lower: None, pred_upper: None }
        })
        .collect();
    fit_res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("michaelis_menten"),
        converged: true,
        ambiguous: false,
        points: curve_pts,
    });
    doc.add_result_sheet(fit_res)?;

    let mut g_xy = GraphSheet::new_uninitialized_for_data("g-xy-broken", doc.data_sheet(&SheetId::new("d-xy")).ok_or("missing d-xy")?)?;
    g_xy.spec.title = String::from("<b>IC<sub>50</sub></b> = 10<sup>-6</sup> µM (α, β, ΔG, χ², ±0.05, °C, Å)");
    g_xy.spec.x_axis.title = String::from("[Agonist] (10<sup>-6</sup> M)");
    g_xy.spec.y_axis.title = String::from("<i>Response</i><sub>norm</sub> (%)");
    g_xy.spec.x_axis.break_ornament = BreakOrnament::DoubleSlash;
    g_xy.spec.x_axis.gap_width_pt = 12.0;
    let seg = |min, max, major_tick_interval| AxisSegment {
        min: Some(min),
        max: Some(max),
        scale: AxisScale::Linear,
        length_share: 0.5,
        major_tick_interval: Some(major_tick_interval),
        minor_ticks_per_interval: 0,
        ..AxisSegment::default()
    };
    g_xy.spec.x_axis.segments = vec![seg(0.0, 10.0, 5.0), seg(100.0, 200.0, 50.0)];
    g_xy.spec.y_axis.thresholds = vec![
        AxisThreshold {
            value: 50.0,
            shade_to: None,
            color: OKABE_ITO_VERMILION.with_alpha(200),
            dash_style: LineDashStyle::DashDot,
            stroke_width_pt: 1.0,
            label: Some(String::from("EC<sub>50</sub>")),
        },
        AxisThreshold {
            value: 85.0,
            shade_to: None,
            color: OKABE_ITO_BLUE.with_alpha(170),
            dash_style: LineDashStyle::Dotted,
            stroke_width_pt: 1.0,
            label: None,
        },
    ];
    g_xy.spec.series[0].gdo = GraphicalDataObject::ScatterAndLine {
        marker: Some(MarkerStyle::default()),
        line: Some(ConnectingLineStyle {
            kind: LineConnectionKind::PointToPoint,
            dash_style: LineDashStyle::Solid,
            stroke_width_pt: 1.2,
            color: OKABE_ITO_BLUE,
        }),
        area_fill: None,
        replicate_lines: curvebook_doc::ReplicateLineMode::default(),
    };
    g_xy.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fit-xy"),
        sample_segments: 240,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    g_xy.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: DocRectPt::new(200.0, 145.0, 140.0, 32.0),
        markdown: String::from("<b>Fit:</b> Δ<i>G</i><sup>‡</sup> = −12.4 kJ·mol<sup>−1</sup>\np ≤ 0.001 (n = 8)"),
        bordered: true,
    });
    g_xy.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("fit-xy"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(190.0, 96.0, 150.0, 38.0),
        font_size_pt: 9.0,
        bordered: true,
    });
    doc.add_graph_sheet(g_xy.clone())?;

    // 2. Column + Pairwise Significance Brackets
    let col_ds = col_sheet(
        "d-col",
        "Treatment Arms",
        &[
            ("Arm α1", &[10.0, 11.0, 10.5, 9.8, 10.2]),
            ("Arm α2", &[10.4, 10.9, 10.3, 10.1, 10.6]),
            ("Arm α3", &[18.2, 19.4, 17.9, 18.7, 19.0]),
            ("Arm α4", &[27.2, 28.6, 26.9, 28.0, 28.2]),
        ],
    );
    doc.add_data_sheet(col_ds.clone())?;

    let mut anova_spec = AnalysisSpec::new(
        "res-anova",
        "d-col",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
            ..OneWayAnovaSpec::default()
        }),
    )?;
    anova_spec.p_value_format = PValueFormat::Apa;
    anova_spec.confidence_level = ConfidenceLevel::DEFAULT_95;
    doc.add_result_sheet(curvebook_stats::run_analysis(&col_ds, &anova_spec)?)?;

    let mut g_col = GraphSheet::new_uninitialized_for_data("g-col-brackets", &col_ds)?;
    g_col.spec.title = String::from("Tukey HSD (p ≤ 0.001, μg/mL)");
    g_col.spec.frame_preset = FramePreset::OpenL;
    let bracket = |group_a, group_b, style, label_mode, hide_if_non_significant, stroke_width_pt| PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-anova"),
        group_a,
        group_b,
        style,
        label_mode,
        hide_if_non_significant,
        stroke_width_pt,
        font_size_pt: 8.5,
        bracket_step_pt: 14.0,
    };
    g_col.spec.comparisons = vec![
        bracket(0, 2, BracketStyle::Square, BracketLabelMode::NumericPOrQ, true, 1.0),
        bracket(2, 3, BracketStyle::HalfTick, BracketLabelMode::SummaryStars, true, 1.0),
        bracket(0, 3, BracketStyle::CurlyBrace, BracketLabelMode::Custom(String::from("Δμ ≥ 17 ***")), false, 1.1),
    ];
    doc.add_graph_sheet(g_col.clone())?;

    // 3. Diagnostic Heatmap
    let mut hm_mv = MultipleVariablesTable::new(4, 5)?;
    for c in 0..4 {
        hm_mv.variables[c].title = format!("Gene β{}", c + 1);
        for r in 0..5 {
            hm_mv.variables[c].cells[r] = Cell::number(((r * 4 + c) as f64) * 0.25 - 1.5);
        }
    }
    let hm_ds = DataSheet::new("d-hm", "Expression Matrix", TableArchetype::MultipleVariables(hm_mv))?;
    doc.add_data_sheet(hm_ds.clone())?;
    let mut g_hm = GraphSheet::new_uninitialized_for_data("g-heatmap", &hm_ds)?;
    g_hm.spec.title = String::from("Normalized Expression Heatmap (z-score)");
    g_hm.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    doc.add_graph_sheet(g_hm.clone())?;

    // 4. Embedded PNG + JPEG Assets & Multi-panel DoubleColumn174Mm Page Layout
    let (png_bytes, jpg_bytes) = (encode_alpha_quadrant_png(96, 72)?, encode_rgb_jpeg(80, 60)?);
    let (png_id, jpg_id) = (content_addressed_asset_id(&png_bytes), content_addressed_asset_id(&jpg_bytes));
    for (asset_id, media_type, pixel_size, bytes) in
        [(png_id.clone(), ImageMimeKind::Png, (96, 72), png_bytes.clone()), (jpg_id.clone(), ImageMimeKind::Jpeg, (80, 60), jpg_bytes)]
    {
        doc.upsert_image_asset(ProjectImageAsset { asset_id, media_type, pixel_size, bytes })?;
    }

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    layout_spec.align_plot_origins = true;
    let live = |id: &str, graph: &GraphSheet, x, y| LayoutItem::LiveGraph {
        id: id.to_owned(),
        graph_sheet_id: graph.id.clone(),
        bounds_pt: DocRectPt::new(x, y, 226.0, 154.0),
        scale_percent: 100.0,
        badge_override: None,
    };
    layout_spec.items = vec![
        live("panel-a", &g_xy, 14.0, 12.0),
        live("panel-b", &g_col, 14.0, 174.0),
        live("panel-c", &g_hm, 252.0, 12.0),
        LayoutItem::EmbeddedImage {
            id: String::from("panel-d-png"),
            asset_id: png_id,
            bounds_pt: DocRectPt::new(252.0, 176.0, 96.0, 72.0),
            rotation_deg: 0.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt { left_pt: 4.0, right_pt: 4.0, top_pt: 3.0, bottom_pt: 3.0 },
            border: Some(ImageBorderSpec { width_pt: 1.2, color: OKABE_ITO_BLUE, dash: LineDashStyle::Dashed }),
            badge_override: Some(String::from("D")),
        },
        LayoutItem::EmbeddedImage {
            id: String::from("panel-e-jpg"),
            asset_id: jpg_id,
            bounds_pt: DocRectPt::new(364.0, 180.0, 88.0, 66.0),
            rotation_deg: 18.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt { left_pt: 5.0, right_pt: 5.0, top_pt: 4.0, bottom_pt: 4.0 },
            border: Some(ImageBorderSpec { width_pt: 1.0, color: OKABE_ITO_VERMILION, dash: LineDashStyle::Solid }),
            badge_override: Some(String::from("E")),
        },
        LayoutItem::TextCallout {
            id: String::from("panel-caption"),
            markdown: String::from("<b>Fig. 1</b> α<sub>0</sub> → β<sub>1</sub>; R<sup>2</sup> = 0.994 ± 0.002; p ≤ 0.001; ΔG° = −12.4"),
            bounds_pt: DocRectPt::new(252.0, 260.0, 226.0, 66.0),
            font_size_pt: 9.0,
        },
    ];
    let layout = LayoutSheet::new(SheetId::new("lay-multipanel"), "Multi-Panel Figure 1", layout_spec)?;
    doc.add_layout_sheet(layout.clone())?;

    Ok(InteropFixtures {
        xy_broken_scene: compile_graph(&g_xy, &doc)?,
        brackets_scene: compile_graph(&g_col, &doc)?,
        heatmap_scene: compile_graph(&g_hm, &doc)?,
        layout_scene: compile_layout(&layout, &doc)?,
        stress_scene: build_stress_scene(&png_bytes)?,
        doc,
    })
}

fn load_host_ttf() -> Option<Vec<u8>> {
    [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ]
    .iter()
    .find_map(|p| fs::read(p).ok())
}

fn write_if_dump(dir: Option<&Path>, name: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(root) = dir {
        fs::create_dir_all(root)?;
        fs::write(root.join(name), bytes)?;
    }
    Ok(())
}

fn u32_be(b: &[u8], off: usize, label: &str) -> Result<u32, String> {
    Ok(u32::from_be_bytes(b.get(off..off + 4).ok_or_else(|| format!("OOB {label}"))?.try_into().map_err(|_| label.to_owned())?))
}

fn u32_le(b: &[u8], off: usize, label: &str) -> Result<u32, String> {
    Ok(u32::from_le_bytes(b.get(off..off + 4).ok_or_else(|| format!("OOB {label}"))?.try_into().map_err(|_| label.to_owned())?))
}

fn u16_le(b: &[u8], off: usize, label: &str) -> Result<u16, String> {
    Ok(u16::from_le_bytes(b.get(off..off + 2).ok_or_else(|| format!("OOB {label}"))?.try_into().map_err(|_| label.to_owned())?))
}

fn parse_png_phys(bytes: &[u8]) -> Result<(u32, u32, u32, u32, u8), String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("invalid PNG signature"));
    }
    let (mut pos, mut width, mut height, mut phys) = (8usize, 0u32, 0u32, None);
    while pos + 12 <= bytes.len() {
        let len = u32_be(bytes, pos, "chunk len")? as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(len).ok_or_else(|| String::from("chunk overflow"))?;
        if data_end + 4 > bytes.len() {
            return Err(String::from("truncated PNG chunk"));
        }
        let data = &bytes[data_start..data_end];
        if kind == b"IHDR" && data.len() >= 8 {
            width = u32_be(data, 0, "IHDR w")?;
            height = u32_be(data, 4, "IHDR h")?;
        } else if kind == b"pHYs" && data.len() == 9 {
            phys = Some((u32_be(data, 0, "pHYs x")?, u32_be(data, 4, "pHYs y")?, data[8]));
        }
        pos = data_end + 4;
        if kind == b"IEND" {
            break;
        }
    }
    let (px, py, unit) = phys.ok_or_else(|| String::from("missing PNG pHYs chunk"))?;
    Ok((width, height, px, py, unit))
}

fn verify_tiff_ifd_metadata(bytes: &[u8], expected_dpi: u32, expected_comp: u16) -> Result<(), String> {
    if bytes.len() < 8 || &bytes[0..4] != b"II*\0" {
        return Err(String::from("invalid little-endian TIFF header"));
    }
    let ifd_off = u32_le(bytes, 4, "IFD offset")? as usize;
    let count = u16_le(bytes, ifd_off, "IFD count")? as usize;
    let (mut comp, mut res_unit, mut extra_samples, mut x_res, mut y_res) = (None, None, None, None, None);
    for i in 0..count {
        let e = ifd_off + 2 + i * 12;
        if e + 12 > bytes.len() {
            return Err(String::from("truncated TIFF IFD entry"));
        }
        let (tag, typ, val_or_off, inline_u16) =
            (u16_le(bytes, e, "tag")?, u16_le(bytes, e + 2, "type")?, u32_le(bytes, e + 8, "val")?, u16_le(bytes, e + 8, "u16")?);
        let rational = |off: usize| Ok::<_, String>((u32_le(bytes, off, "num")?, u32_le(bytes, off + 4, "den")?));
        match (tag, typ) {
            (259, 3) => comp = Some(inline_u16),
            (282, 5) => x_res = Some(rational(val_or_off as usize)?),
            (283, 5) => y_res = Some(rational(val_or_off as usize)?),
            (296, 3) => res_unit = Some(inline_u16),
            (338, 3) => extra_samples = Some(inline_u16),
            _ => {}
        }
    }
    if comp != Some(expected_comp) {
        return Err(format!("TIFF Compression={comp:?}, expected {expected_comp}"));
    }
    if res_unit != Some(2) {
        return Err(format!("TIFF ResolutionUnit={res_unit:?}, expected 2 (inches)"));
    }
    if extra_samples != Some(2) {
        return Err(format!("TIFF ExtraSamples={extra_samples:?}, expected 2 (unassociated alpha)"));
    }
    if x_res != Some((expected_dpi, 1)) || y_res != Some((expected_dpi, 1)) {
        return Err(format!("TIFF DPI=({x_res:?}, {y_res:?}), expected {expected_dpi}/1"));
    }
    Ok(())
}

fn verify_emf_interop_stream(bytes: &[u8], expect_extcreatepen: bool) -> Result<(), String> {
    if bytes.len() < 88 || !bytes.len().is_multiple_of(4) {
        return Err(format!("invalid EMF byte length {}", bytes.len()));
    }
    let sig = u32_le(bytes, 40, "EMF sig")?;
    if sig != 0x464D_4520 {
        return Err(format!("invalid EMF signature {sig:#x}"));
    }
    let (mut off, mut saw_ext_pen, mut saw_eof) = (0usize, false, false);
    while off + 8 <= bytes.len() {
        let i_type = u32_le(bytes, off, "EMF iType")?;
        let n_size = u32_le(bytes, off + 4, "EMF nSize")? as usize;
        if n_size < 8 || !n_size.is_multiple_of(4) || off + n_size > bytes.len() {
            return Err(format!("corrupt EMF record type={i_type} size={n_size}"));
        }
        saw_ext_pen |= i_type == 95;
        saw_eof |= i_type == 14;
        off += n_size;
    }
    if !saw_eof || off != bytes.len() {
        return Err(String::from("EMF missing terminating EMR_EOF or trailing bytes"));
    }
    if expect_extcreatepen && !saw_ext_pen {
        return Err(String::from("expected EMR_EXTCREATEPEN (type 95) in vector EMF"));
    }
    Ok(())
}

#[test]
fn export_comprehensive_interop_suite_and_verify_structure() -> TestResult {
    let fixtures = build_interop_fixtures()?;
    let dump_dir = std::env::var_os("CURVEBOOK_R17_2_DUMP_DIR").map(PathBuf::from);
    let dump_ref = dump_dir.as_deref();
    let host_ttf = load_host_ttf();

    let scenes: [(&str, &ChartScene); 5] = [
        ("01_xy_broken", &fixtures.xy_broken_scene),
        ("02_col_brackets", &fixtures.brackets_scene),
        ("03_heatmap", &fixtures.heatmap_scene),
        ("04_layout_multipanel", &fixtures.layout_scene),
        ("05_vector_stress", &fixtures.stress_scene),
    ];

    for (slug, scene) in scenes {
        // 1. SVG variants
        let svg_editable = render_svg(scene, &SvgExportOptions::default())?;
        validate_svg_structure(&svg_editable)?;
        assert!(!svg_editable.contains("stroke-width=\"0.000\""), "{slug}.editable.svg emitted 0-width stroke");
        assert!(!svg_editable.contains("stroke-dasharray=\"0.000"), "{slug}.editable.svg emitted degenerate dasharray");
        if svg_editable.contains("<image") {
            assert!(svg_editable.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""), "{slug}.editable.svg missing xmlns:xlink");
            assert!(svg_editable.contains("xlink:href=\"data:"), "{slug}.editable.svg missing xlink:href");
            assert!(svg_editable.contains(" href=\"data:"), "{slug}.editable.svg missing SVG2 href");
        }
        write_if_dump(dump_ref, &format!("{slug}.editable.svg"), svg_editable.as_bytes())?;

        for (suffix, opts, expect_text) in [
            ("outlined.svg", SvgExportOptions { outline_text: true, ..SvgExportOptions::default() }, false),
            ("embedfonts.svg", SvgExportOptions { embed_fonts: true, ..SvgExportOptions::default() }, true),
        ] {
            let svg = render_svg(scene, &opts)?;
            validate_svg_structure(&svg)?;
            assert_eq!(svg.contains("<text"), expect_text, "{slug}.{suffix} <text> check");
            assert!(!svg.contains("stroke-width=\"0.000\""), "{slug}.{suffix} emitted 0-width stroke");
            write_if_dump(dump_ref, &format!("{slug}.{suffix}"), svg.as_bytes())?;
        }

        // 2. PDF variants
        for (suffix, version, color_space, outline_text, prefix) in [
            ("pdf17_srgb.pdf", PdfVersion::Pdf17, PdfColorSpace::Srgb, false, Some(&b"%PDF-1.7"[..])),
            ("pdf14_srgb.pdf", PdfVersion::Pdf14, PdfColorSpace::Srgb, false, Some(&b"%PDF-1.4"[..])),
            ("pdf17_cmyk.pdf", PdfVersion::Pdf17, PdfColorSpace::Cmyk, false, None),
            ("pdf17_outlined.pdf", PdfVersion::Pdf17, PdfColorSpace::Srgb, true, None),
        ] {
            let pdf = render_pdf(
                scene,
                &PdfExportOptions {
                    title: Some(format!("CurveBook Interop {slug}")),
                    version,
                    color_space,
                    outline_text,
                    ..PdfExportOptions::default()
                },
            )?;
            if let Some(p) = prefix {
                assert!(pdf.starts_with(p));
            }
            if outline_text {
                assert!(!String::from_utf8_lossy(&pdf).contains("/Font <<"), "{slug}.{suffix} Page /Resources leaked /Font dictionary");
            }
            write_if_dump(dump_ref, &format!("{slug}.{suffix}"), &pdf)?;
        }
        if let Some(ref ttf) = host_ttf {
            let pdf_ttf = render_pdf(
                scene,
                &PdfExportOptions {
                    title: Some(format!("CurveBook Interop {slug} TTF")),
                    embedded_truetype_font: Some(ttf.clone()),
                    ..PdfExportOptions::default()
                },
            )?;
            write_if_dump(dump_ref, &format!("{slug}.pdf17_ttf.pdf"), &pdf_ttf)?;
        }

        // 3. EMF vector & raster
        let emf = encode_emf(scene)?;
        verify_emf_interop_stream(&emf, slug == "05_vector_stress")?;
        write_if_dump(dump_ref, &format!("{slug}.emf"), &emf)?;

        // 4. PNG & TIFF at 300 and 600 DPI
        for (dpi, expected_ppu) in [(300u16, 11_811u32), (600, 23_622)] {
            let raster = render_scene(scene, RenderTargetSpec::from_scene_dpi(scene, dpi)?)?;
            let png = encode_png(&raster)?;
            let (pw, ph, px, py, unit) = parse_png_phys(&png)?;
            assert!(pw > 0 && ph > 0 && unit == 1 && px == expected_ppu && py == expected_ppu, "{slug} PNG pHYs mismatch at {dpi} DPI");
            write_if_dump(dump_ref, &format!("{slug}_{dpi}dpi.png"), &png)?;

            if dpi == 300 {
                let emf_ras = encode_raster_emf(&raster)?;
                verify_emf_interop_stream(&emf_ras, false)?;
                write_if_dump(dump_ref, &format!("{slug}_{dpi}dpi_raster.emf"), &emf_ras)?;
                for (cname, comp, tag) in
                    [("deflate", TiffCompression::Deflate, 8u16), ("lzw", TiffCompression::Lzw, 5), ("none", TiffCompression::None, 1)]
                {
                    let tiff = encode_tiff(&raster, comp)?;
                    verify_tiff_ifd_metadata(&tiff, 300, tag)?;
                    write_if_dump(dump_ref, &format!("{slug}_{dpi}dpi_{cname}.tiff"), &tiff)?;
                }
            } else {
                let tiff = encode_tiff(&raster, TiffCompression::Deflate)?;
                verify_tiff_ifd_metadata(&tiff, 600, 8)?;
                write_if_dump(dump_ref, &format!("{slug}_{dpi}dpi_deflate.tiff"), &tiff)?;
            }
        }
    }

    // 5. Multi-page PDF export combining graphs & layout
    let multipage_pdf = render_pdf_pages(
        &[fixtures.xy_broken_scene.clone(), fixtures.brackets_scene.clone(), fixtures.heatmap_scene.clone(), fixtures.layout_scene.clone()],
        &PdfExportOptions {
            title: Some(String::from("CurveBook Multi-Page Suite")),
            embedded_truetype_font: host_ttf,
            ..PdfExportOptions::default()
        },
    )?;
    assert!(multipage_pdf.starts_with(b"%PDF-1.7"));
    write_if_dump(dump_ref, "06_multipage_all.pdf", &multipage_pdf)?;

    // 6. Verify Engine export.* command execution produces valid SVG/PDF/PNG/TIFF artifacts cleanly.
    let tmp = if dump_ref.is_none() { Some(TempDir::try_new("r17-2-engine")?) } else { None };
    let engine_dir = dump_ref.unwrap_or_else(|| tmp.as_ref().unwrap().path());
    let mut session = Session::from_document(fixtures.doc.clone());

    let engine_svg = engine_dir.join("engine_layout_cmd.svg");
    ok_cmd(&mut session, "export.svg", json!({ "sheet_id": "lay-multipanel", "path": engine_svg.to_string_lossy() }));
    let engine_svg_str = fs::read_to_string(&engine_svg)?;
    validate_svg_structure(&engine_svg_str)?;
    assert!(engine_svg_str.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\"") && engine_svg_str.contains("xlink:href=\"data:"));

    let engine_pdf = engine_dir.join("engine_layout_cmd.pdf");
    ok_cmd(&mut session, "export.pdf", json!({ "sheet_id": "lay-multipanel", "path": engine_pdf.to_string_lossy() }));
    assert!(fs::read(&engine_pdf)?.starts_with(b"%PDF-1.7"));

    let engine_png = engine_dir.join("engine_layout_cmd.png");
    ok_cmd(&mut session, "export.png", json!({ "sheet_id": "lay-multipanel", "dpi": 300, "path": engine_png.to_string_lossy() }));
    let (_, _, px, py, unit) = parse_png_phys(&fs::read(&engine_png)?)?;
    assert_eq!((px, py, unit), (11_811, 11_811, 1));

    let engine_tiff = engine_dir.join("engine_layout_cmd.tiff");
    ok_cmd(&mut session, "export.tiff", json!({ "sheet_id": "lay-multipanel", "dpi": 300, "path": engine_tiff.to_string_lossy() }));
    verify_tiff_ifd_metadata(&fs::read(&engine_tiff)?, 300, 8)?;
    Ok(())
}
