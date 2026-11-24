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
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisThreshold, BracketLabelMode, BracketStyle, BreakOrnament, Cell, ColumnDesignHint,
    ColumnTable, ConfidenceLevel, ConnectingLineStyle, CropInsetsPt, CurveSamplePoint, DataSheet, DiagnosticPlotKind, EquationDisplayMode,
    FittedCurveOverlay, FramePreset, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, ImageBorderSpec, ImageMimeKind, JournalWidthPreset,
    LayoutItem, LayoutSheet, LayoutSpec, LineConnectionKind, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NonlinearFitSpec,
    OneWayAnovaSpec, OneWayMethod, PValueFormat, PairwiseBracketSpec, ProjectDocument, ProjectImageAsset, RectPt as DocRectPt, RegressionOverlaySpec,
    ResultSheet, SheetId, SubcolumnMode, TableArchetype, XyTable, content_addressed_asset_id,
};
use curvebook_engine::Session;
use curvebook_palette::{LineDashStyle, MarkerStyle, OKABE_ITO_BLUE, OKABE_ITO_VERMILION, Rgba8};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions, PdfVersion, render_pdf, render_pdf_pages};
use curvebook_render::{RenderTargetSpec, TiffCompression, encode_emf, encode_png, encode_raster_emf, encode_tiff, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_text::parse_markup;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::json;

type TestResult = Result<(), Box<dyn Error>>;

fn encode_alpha_quadrant_png(w: u32, h: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let img = RgbaImage::from_fn(w, h, |x, y| {
        let right = x >= w / 2;
        let bottom = y >= h / 2;
        match (right, bottom) {
            (false, false) => Rgba([220, 40, 40, 140]),
            (true, false) => Rgba([25, 160, 70, 255]),
            (false, true) => Rgba([30, 80, 220, 255]),
            (true, true) => Rgba([235, 185, 25, 180]),
        }
    });
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

fn encode_rgb_jpeg(w: u32, h: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let img = RgbImage::from_fn(w, h, |x, y| {
        #[allow(clippy::cast_possible_truncation)]
        let r = ((x * 255) / w.max(1)) as u8;
        #[allow(clippy::cast_possible_truncation)]
        let b = ((y * 255) / h.max(1)) as u8;
        Rgb([r, 128, b])
    });
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

fn build_interop_fixtures() -> Result<InteropFixtures, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("R17_2_ExportInterop")?;

    // 1. XY Broken Axis + Semi-Transparent Band + Rich Typography + Dash Thresholds + 2D Equation
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 8)?;
    let pts = [(1.0, 12.0), (3.0, 22.0), (6.0, 48.0), (9.0, 74.0), (110.0, 88.0), (140.0, 93.0), (170.0, 96.0), (195.0, 98.0)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
        xy.data_sets[0].cells[r][0] = Cell::number(y - 3.5);
        xy.data_sets[0].cells[r][1] = Cell::number(y + 3.5);
    }
    doc.add_data_sheet(DataSheet::new("d-xy", "Dose Kinetics", TableArchetype::Xy(xy))?)?;

    let fit_spec = AnalysisSpec::new("fit-xy", "d-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("michaelis_menten")))?;
    let mut fit_res = ResultSheet::new(fit_spec)?;
    let mut curve_pts = Vec::new();
    for i in 1..=200 {
        let x = f64::from(i);
        let y = 100.0 * x / (18.0 + x);
        curve_pts.push(CurveSamplePoint {
            x,
            y,
            ci_lower: Some((y - 6.0).max(0.0)),
            ci_upper: Some((y + 6.0).min(108.0)),
            pred_lower: None,
            pred_upper: None,
        });
    }
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
    g_xy.spec.x_axis.segments = vec![
        AxisSegment {
            min: Some(0.0),
            max: Some(10.0),
            scale: AxisScale::Linear,
            length_share: 0.5,
            major_tick_interval: Some(5.0),
            minor_ticks_per_interval: 0,
        },
        AxisSegment {
            min: Some(100.0),
            max: Some(200.0),
            scale: AxisScale::Linear,
            length_share: 0.5,
            major_tick_interval: Some(50.0),
            minor_ticks_per_interval: 0,
        },
    ];
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
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 5)?;
    let groups = [[10.0, 11.0, 10.5, 9.8, 10.2], [10.4, 10.9, 10.3, 10.1, 10.6], [18.2, 19.4, 17.9, 18.7, 19.0], [27.2, 28.6, 26.9, 28.0, 28.2]];
    for (c, vals) in groups.iter().enumerate() {
        col.data_sets[c].title = format!("Arm α{}", c + 1);
        for (r, &v) in vals.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let col_ds = DataSheet::new("d-col", "Treatment Arms", TableArchetype::Column(col))?;
    doc.add_data_sheet(col_ds.clone())?;

    let mut anova_spec = AnalysisSpec::new(
        "res-anova",
        "d-col",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
            ..OneWayAnovaSpec::default()
        }),
    )?;
    anova_spec.p_value_format = PValueFormat::Apa;
    anova_spec.confidence_level = ConfidenceLevel::DEFAULT_95;
    let anova_res = curvebook_stats::run_analysis(&col_ds, &anova_spec)?;
    doc.add_result_sheet(anova_res)?;

    let mut g_col = GraphSheet::new_uninitialized_for_data("g-col-brackets", doc.data_sheet(&SheetId::new("d-col")).ok_or("missing d-col")?)?;
    g_col.spec.title = String::from("Tukey HSD (p ≤ 0.001, μg/mL)");
    g_col.spec.frame_preset = FramePreset::OpenL;
    g_col.spec.comparisons = vec![
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 0,
            group_b: 2,
            style: BracketStyle::Square,
            label_mode: BracketLabelMode::NumericPOrQ,
            hide_if_non_significant: true,
            stroke_width_pt: 1.0,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 2,
            group_b: 3,
            style: BracketStyle::HalfTick,
            label_mode: BracketLabelMode::SummaryStars,
            hide_if_non_significant: true,
            stroke_width_pt: 1.0,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
        PairwiseBracketSpec {
            result_sheet_id: SheetId::new("res-anova"),
            group_a: 0,
            group_b: 3,
            style: BracketStyle::CurlyBrace,
            label_mode: BracketLabelMode::Custom(String::from("Δμ ≥ 17 ***")),
            hide_if_non_significant: false,
            stroke_width_pt: 1.1,
            font_size_pt: 8.5,
            bracket_step_pt: 14.0,
        },
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
    let png_bytes = encode_alpha_quadrant_png(96, 72)?;
    let png_id = content_addressed_asset_id(&png_bytes);
    doc.upsert_image_asset(ProjectImageAsset {
        asset_id: png_id.clone(),
        media_type: ImageMimeKind::Png,
        pixel_size: (96, 72),
        bytes: png_bytes.clone(),
    })?;
    let jpg_bytes = encode_rgb_jpeg(80, 60)?;
    let jpg_id = content_addressed_asset_id(&jpg_bytes);
    doc.upsert_image_asset(ProjectImageAsset {
        asset_id: jpg_id.clone(),
        media_type: ImageMimeKind::Jpeg,
        pixel_size: (80, 60),
        bytes: jpg_bytes.clone(),
    })?;

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    layout_spec.align_plot_origins = true;
    layout_spec.items = vec![
        LayoutItem::LiveGraph {
            id: String::from("panel-a"),
            graph_sheet_id: g_xy.id.clone(),
            bounds_pt: DocRectPt::new(14.0, 12.0, 226.0, 154.0),
            scale_percent: 100.0,
            badge_override: None,
        },
        LayoutItem::LiveGraph {
            id: String::from("panel-b"),
            graph_sheet_id: g_col.id.clone(),
            bounds_pt: DocRectPt::new(14.0, 174.0, 226.0, 154.0),
            scale_percent: 100.0,
            badge_override: None,
        },
        LayoutItem::LiveGraph {
            id: String::from("panel-c"),
            graph_sheet_id: g_hm.id.clone(),
            bounds_pt: DocRectPt::new(252.0, 12.0, 226.0, 154.0),
            scale_percent: 100.0,
            badge_override: None,
        },
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

    let xy_broken_scene = compile_graph(&g_xy, &doc)?;
    let brackets_scene = compile_graph(&g_col, &doc)?;
    let heatmap_scene = compile_graph(&g_hm, &doc)?;
    let layout_scene = compile_layout(&layout, &doc)?;

    // 5. Targeted Vector & Raster Stress Scene
    let mut stress = ChartScene::new("scene-stress", "Vector & Typography Stress <Probe>", 420.0, 300.0, Some(Rgba8::WHITE));
    let outer_clip = RectPt::new(16.0, 16.0, 180.0, 120.0);
    let inner_clip = RectPt::new(36.0, 30.0, 140.0, 90.0);
    stress.primitives.push(ScenePrimitive::PushClip { rect: outer_clip });
    stress.primitives.push(ScenePrimitive::PushClip { rect: inner_clip });
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

    // Every dash variant across regular, hairline, and zero-width edge cases
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
        stress.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(16.0, y), PointPt::new(196.0, y)),
            fill: None,
            stroke: Some(StrokePaint { color: Rgba8::rgba(25, 95, 165, 220), width_pt: 1.6, dash, cap: LineCap::Butt, join: LineJoin::Miter }),
            clip: None,
        });
        stress.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(206.0, y), PointPt::new(260.0, y)),
            fill: None,
            stroke: Some(StrokePaint {
                color: Rgba8::rgba(190, 50, 25, 220),
                width_pt: if idx == 5 { 0.0 } else { 0.25 },
                dash,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
            }),
            clip: None,
        });
    }

    // Embedded RGBA PNG decoded into SceneRasterImage + rotated cropped instance
    let rgba_img = image::load_from_memory(&png_bytes)?.to_rgba8();
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

    // Rich typography stress: superscript/subscript/Greek/Unicode/combining marks/multiline/rotated
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

    Ok(InteropFixtures { doc, xy_broken_scene, brackets_scene, heatmap_scene, layout_scene, stress_scene: stress })
}

fn load_host_ttf() -> Option<Vec<u8>> {
    let candidates = [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ];
    candidates.iter().find_map(|p| fs::read(p).ok())
}

fn write_if_dump(dir: Option<&Path>, name: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(root) = dir {
        fs::create_dir_all(root)?;
        fs::write(root.join(name), bytes)?;
    }
    Ok(())
}

fn parse_png_phys(bytes: &[u8]) -> Result<(u32, u32, u32, u32, u8), String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("invalid PNG signature"));
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut phys = None;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().map_err(|_| String::from("chunk len"))?) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(len).ok_or_else(|| String::from("chunk overflow"))?;
        if data_end + 4 > bytes.len() {
            return Err(String::from("truncated PNG chunk"));
        }
        let data = &bytes[data_start..data_end];
        if kind == b"IHDR" && data.len() >= 8 {
            width = u32::from_be_bytes(data[0..4].try_into().map_err(|_| String::from("IHDR w"))?);
            height = u32::from_be_bytes(data[4..8].try_into().map_err(|_| String::from("IHDR h"))?);
        } else if kind == b"pHYs" && data.len() == 9 {
            let ppu_x = u32::from_be_bytes(data[0..4].try_into().map_err(|_| String::from("pHYs x"))?);
            let ppu_y = u32::from_be_bytes(data[4..8].try_into().map_err(|_| String::from("pHYs y"))?);
            let unit = data[8];
            phys = Some((ppu_x, ppu_y, unit));
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
    let ifd_off = u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| String::from("IFD offset"))?) as usize;
    if ifd_off + 2 > bytes.len() {
        return Err(String::from("truncated TIFF IFD"));
    }
    let count = u16::from_le_bytes(bytes[ifd_off..ifd_off + 2].try_into().map_err(|_| String::from("IFD count"))?) as usize;
    let mut comp = None;
    let mut res_unit = None;
    let mut extra_samples = None;
    let mut x_res = None;
    let mut y_res = None;
    for i in 0..count {
        let e_off = ifd_off + 2 + i * 12;
        if e_off + 12 > bytes.len() {
            return Err(String::from("truncated TIFF IFD entry"));
        }
        let tag = u16::from_le_bytes(bytes[e_off..e_off + 2].try_into().map_err(|_| String::from("tag"))?);
        let typ = u16::from_le_bytes(bytes[e_off + 2..e_off + 4].try_into().map_err(|_| String::from("type"))?);
        let val_or_off = u32::from_le_bytes(bytes[e_off + 8..e_off + 12].try_into().map_err(|_| String::from("val"))?);
        let inline_u16 = u16::from_le_bytes(bytes[e_off + 8..e_off + 10].try_into().map_err(|_| String::from("u16"))?);
        let read_rational = |off: usize| -> Result<(u32, u32), String> {
            if off + 8 > bytes.len() {
                return Err(String::from("rational OOB"));
            }
            let num = u32::from_le_bytes(bytes[off..off + 4].try_into().map_err(|_| String::from("num"))?);
            let den = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().map_err(|_| String::from("den"))?);
            Ok((num, den))
        };
        match (tag, typ) {
            (259, 3) => comp = Some(inline_u16),
            (282, 5) => x_res = Some(read_rational(val_or_off as usize)?),
            (283, 5) => y_res = Some(read_rational(val_or_off as usize)?),
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
    let sig = u32::from_le_bytes(bytes[40..44].try_into().map_err(|_| String::from("EMF sig"))?);
    if sig != 0x464D_4520 {
        return Err(format!("invalid EMF signature {sig:#x}"));
    }
    let mut off = 0usize;
    let mut saw_ext_pen = false;
    let mut saw_eof = false;
    while off + 8 <= bytes.len() {
        let i_type = u32::from_le_bytes(bytes[off..off + 4].try_into().map_err(|_| String::from("EMF iType"))?);
        let n_size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().map_err(|_| String::from("EMF nSize"))?) as usize;
        if n_size < 8 || !n_size.is_multiple_of(4) || off + n_size > bytes.len() {
            return Err(format!("corrupt EMF record type={i_type} size={n_size}"));
        }
        if i_type == 95 {
            saw_ext_pen = true;
        } else if i_type == 14 {
            saw_eof = true;
        }
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

        let svg_outlined = render_svg(scene, &SvgExportOptions { outline_text: true, ..SvgExportOptions::default() })?;
        validate_svg_structure(&svg_outlined)?;
        assert!(!svg_outlined.contains("<text"), "{slug}.outlined.svg leaked <text>");
        assert!(!svg_outlined.contains("stroke-width=\"0.000\""), "{slug}.outlined.svg emitted 0-width stroke");
        write_if_dump(dump_ref, &format!("{slug}.outlined.svg"), svg_outlined.as_bytes())?;

        let svg_embed = render_svg(scene, &SvgExportOptions { embed_fonts: true, ..SvgExportOptions::default() })?;
        validate_svg_structure(&svg_embed)?;
        assert!(svg_embed.contains("<text"), "{slug}.embedfonts.svg missing editable <text>");
        assert!(!svg_embed.contains("stroke-width=\"0.000\""), "{slug}.embedfonts.svg emitted 0-width stroke");
        write_if_dump(dump_ref, &format!("{slug}.embedfonts.svg"), svg_embed.as_bytes())?;

        // 2. PDF variants
        let pdf17 = render_pdf(
            scene,
            &PdfExportOptions {
                title: Some(format!("CurveBook Interop {slug}")),
                version: PdfVersion::Pdf17,
                color_space: PdfColorSpace::Srgb,
                ..PdfExportOptions::default()
            },
        )?;
        assert!(pdf17.starts_with(b"%PDF-1.7"));
        write_if_dump(dump_ref, &format!("{slug}.pdf17_srgb.pdf"), &pdf17)?;

        let pdf14 = render_pdf(
            scene,
            &PdfExportOptions {
                title: Some(format!("CurveBook Interop {slug}")),
                version: PdfVersion::Pdf14,
                color_space: PdfColorSpace::Srgb,
                ..PdfExportOptions::default()
            },
        )?;
        assert!(pdf14.starts_with(b"%PDF-1.4"));
        write_if_dump(dump_ref, &format!("{slug}.pdf14_srgb.pdf"), &pdf14)?;

        let pdf_cmyk = render_pdf(
            scene,
            &PdfExportOptions {
                title: Some(format!("CurveBook Interop {slug}")),
                version: PdfVersion::Pdf17,
                color_space: PdfColorSpace::Cmyk,
                ..PdfExportOptions::default()
            },
        )?;
        write_if_dump(dump_ref, &format!("{slug}.pdf17_cmyk.pdf"), &pdf_cmyk)?;

        let pdf_outlined = render_pdf(
            scene,
            &PdfExportOptions { title: Some(format!("CurveBook Interop {slug}")), outline_text: true, ..PdfExportOptions::default() },
        )?;
        let out_txt = String::from_utf8_lossy(&pdf_outlined);
        assert!(!out_txt.contains("/Font <<"), "{slug}.pdf17_outlined.pdf Page /Resources leaked /Font dictionary");
        write_if_dump(dump_ref, &format!("{slug}.pdf17_outlined.pdf"), &pdf_outlined)?;

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
        for dpi in [300u16, 600u16] {
            let spec = RenderTargetSpec::from_scene_dpi(scene, dpi)?;
            let raster = render_scene(scene, spec)?;
            let png = encode_png(&raster)?;
            let (pw, ph, px, py, unit) = parse_png_phys(&png)?;
            let expected_ppu = if dpi == 300 { 11_811 } else { 23_622 };
            assert!(pw > 0 && ph > 0 && unit == 1 && px == expected_ppu && py == expected_ppu, "{slug} PNG pHYs mismatch at {dpi} DPI");
            write_if_dump(dump_ref, &format!("{slug}_{dpi}dpi.png"), &png)?;

            if dpi == 300 {
                let emf_ras = encode_raster_emf(&raster)?;
                verify_emf_interop_stream(&emf_ras, false)?;
                write_if_dump(dump_ref, &format!("{slug}_{dpi}dpi_raster.emf"), &emf_ras)?;
                for (cname, comp, tag) in
                    [("deflate", TiffCompression::Deflate, 8u16), ("lzw", TiffCompression::Lzw, 5u16), ("none", TiffCompression::None, 1u16)]
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
    struct TempDirCleanup(PathBuf);
    impl Drop for TempDirCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _guard;
    let engine_dir = if let Some(root) = dump_ref {
        _guard = None;
        root.to_path_buf()
    } else {
        let dir = std::env::temp_dir().join(format!("cb-r17-2-engine-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.clone();
        _guard = Some(TempDirCleanup(dir));
        path
    };
    let mut session = Session::from_document(fixtures.doc.clone());
    let engine_svg = engine_dir.join("engine_layout_cmd.svg");
    let _ = session.execute("export.svg", json!({ "sheet_id": "lay-multipanel", "path": engine_svg.to_string_lossy() }))?;
    let engine_svg_str = fs::read_to_string(&engine_svg)?;
    validate_svg_structure(&engine_svg_str)?;
    assert!(engine_svg_str.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""));
    assert!(engine_svg_str.contains("xlink:href=\"data:"));

    let engine_pdf = engine_dir.join("engine_layout_cmd.pdf");
    let _ = session.execute("export.pdf", json!({ "sheet_id": "lay-multipanel", "path": engine_pdf.to_string_lossy() }))?;
    assert!(fs::read(&engine_pdf)?.starts_with(b"%PDF-1.7"));

    let engine_png = engine_dir.join("engine_layout_cmd.png");
    let _ = session.execute("export.png", json!({ "sheet_id": "lay-multipanel", "dpi": 300, "path": engine_png.to_string_lossy() }))?;
    let (_, _, px, py, unit) = parse_png_phys(&fs::read(&engine_png)?)?;
    assert_eq!((px, py, unit), (11_811, 11_811, 1));

    let engine_tiff = engine_dir.join("engine_layout_cmd.tiff");
    let _ = session.execute("export.tiff", json!({ "sheet_id": "lay-multipanel", "dpi": 300, "path": engine_tiff.to_string_lossy() }))?;
    verify_tiff_ifd_metadata(&fs::read(&engine_tiff)?, 300, 8)?;

    Ok(())
}
