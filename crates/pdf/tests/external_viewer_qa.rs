//! External multi-engine `%PDF-1.7` + `/FontFile2` visual QA harness (`G5`/`G13`).
//!
//! Exports real `%PDF-1.7` documents via `curvebook_pdf`, rasterizes every page with BOTH Poppler
//! (`pdftoppm -r 150 -f 1 -l 1`) and Ghostscript (`gs -dSAFER -dBATCH -dNOPAUSE -sDEVICE=ppmraw -r150`),
//! parses binary `P6` PPM pixels, and asserts non-blank glyph ink, translucent `/CA`/`/ca` ribbon
//! blending, CJK/RTL coverage, crop marks, zero engine stderr warnings/errors, and cross-engine
//! pixel agreement.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_chart::{ChartScene, FillPaint, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_pdf::{PdfExportOptions, PdfVersion, render_pdf};
use curvebook_text::glyph_paths::{GlyphOutlineCmd, outline_ttf_glyph};
use curvebook_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, TextDirection, TrueTypeFont, parse_truetype_font_info, shape_bidi_runs};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(tag: &str) -> Result<Self, String> {
        let id = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-pdf-qa-{}-{}-{}", std::process::id(), tag, id));
        fs::create_dir_all(&path).map_err(|e| format!("create QA temp dir {}: {e}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct PpmImage {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl PpmImage {
    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let idx = (y * self.width + x) * 3;
        [self.rgb[idx], self.rgb[idx + 1], self.rgb[idx + 2]]
    }
}

fn find_binary(name: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for fb in fallbacks {
        let candidate = Path::new(fb);
        if candidate.is_file() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

fn find_existing_path(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().map(PathBuf::from).find(|p| p.is_file())
}

fn env_truthy(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

fn require_external_viewers() -> bool {
    env_truthy("CI") || env_truthy("CURVEBOOK_REQUIRE_EXTERNAL_VIEWERS")
}

fn require_cjk_rtl_fonts() -> bool {
    env_truthy("CURVEBOOK_REQUIRE_CJK_RTL_FONTS")
}

fn discover_engines_or_skip(test_name: &str) -> Option<(PathBuf, PathBuf)> {
    let pdftoppm = find_binary("pdftoppm", &["/opt/homebrew/bin/pdftoppm", "/usr/local/bin/pdftoppm", "/usr/bin/pdftoppm"]);
    let gs = find_binary("gs", &["/opt/homebrew/bin/gs", "/usr/local/bin/gs", "/usr/bin/gs"]);
    match (pdftoppm, gs) {
        (Some(p), Some(g)) => Some((p, g)),
        (p, g) => {
            assert!(
                !require_external_viewers(),
                "FAIL-CLOSED {test_name}: external viewer binary missing under CI/CURVEBOOK_REQUIRE_EXTERNAL_VIEWERS (pdftoppm={p:?}, gs={g:?})"
            );
            eprintln!("SKIP {test_name}: external viewer binary missing (pdftoppm={p:?}, gs={g:?})");
            None
        }
    }
}

fn parse_p6_ppm(data: &[u8]) -> Result<PpmImage, String> {
    if !data.starts_with(b"P6") {
        return Err(String::from("expected binary P6 PPM header"));
    }
    let mut pos = 2usize;

    let read_token = |buf: &[u8], cursor: &mut usize| -> Result<usize, String> {
        loop {
            while *cursor < buf.len() && buf[*cursor].is_ascii_whitespace() {
                *cursor += 1;
            }
            if *cursor < buf.len() && buf[*cursor] == b'#' {
                while *cursor < buf.len() && buf[*cursor] != b'\n' {
                    *cursor += 1;
                }
            } else {
                break;
            }
        }
        let start = *cursor;
        while *cursor < buf.len() && !buf[*cursor].is_ascii_whitespace() {
            *cursor += 1;
        }
        let token = std::str::from_utf8(&buf[start..*cursor]).map_err(|e| format!("non-UTF8 PPM header token: {e}"))?;
        token.parse::<usize>().map_err(|e| format!("invalid PPM integer token '{token}': {e}"))
    };

    let width = read_token(data, &mut pos)?;
    let height = read_token(data, &mut pos)?;
    let maxval = read_token(data, &mut pos)?;
    assert_eq!(maxval, 255, "expected 8-bit maxval 255 in PPM");
    assert!(pos < data.len() && data[pos].is_ascii_whitespace(), "missing whitespace byte after PPM maxval");
    pos += 1;

    let expected_bytes = width * height * 3;
    let payload = &data[pos..];
    assert_eq!(payload.len(), expected_bytes, "PPM byte payload mismatch");
    Ok(PpmImage { width, height, rgb: payload.to_vec() })
}

fn rasterize_both_engines(pdftoppm: &Path, gs: &Path, dir: &Path, label: &str, pdf_bytes: &[u8]) -> Result<(PpmImage, PpmImage), String> {
    let pdf_path = dir.join(format!("{label}.pdf"));
    fs::write(&pdf_path, pdf_bytes).map_err(|e| format!("write PDF fixture {}: {e}", pdf_path.display()))?;

    // 1. Poppler pdftoppm
    let poppler_prefix = dir.join(format!("{label}_poppler"));
    let poppler_out = Command::new(pdftoppm)
        .args(["-r", "150", "-f", "1", "-l", "1"])
        .arg(&pdf_path)
        .arg(&poppler_prefix)
        .output()
        .map_err(|e| format!("execute pdftoppm on {label}: {e}"))?;
    let poppler_stderr = String::from_utf8_lossy(&poppler_out.stderr);
    assert!(poppler_out.status.success(), "pdftoppm failed on {label} (status={}): {poppler_stderr}", poppler_out.status);
    assert!(poppler_stderr.trim().is_empty(), "pdftoppm emitted unexpected stderr on {label}: {poppler_stderr}");

    let mut poppler_ppm_path: Option<PathBuf> = None;
    let expected_prefix = format!("{label}_poppler");
    for entry in fs::read_dir(dir).map_err(|e| format!("read temp dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(&expected_prefix) && name_str.ends_with(".ppm") {
            poppler_ppm_path = Some(entry.path());
            break;
        }
    }
    let poppler_path = poppler_ppm_path.ok_or_else(|| format!("locate pdftoppm output .ppm in {}", dir.display()))?;
    let poppler_ppm = parse_p6_ppm(&fs::read(&poppler_path).map_err(|e| format!("read {}: {e}", poppler_path.display()))?)?;

    // 2. Ghostscript ppmraw
    let gs_ppm_path = dir.join(format!("{label}_gs.ppm"));
    let gs_out_arg = format!("-sOutputFile={}", gs_ppm_path.display());
    let gs_out = Command::new(gs)
        .args(["-dSAFER", "-dBATCH", "-dNOPAUSE", "-q", "-sDEVICE=ppmraw", "-r150"])
        .arg(&gs_out_arg)
        .arg(&pdf_path)
        .output()
        .map_err(|e| format!("execute gs on {label}: {e}"))?;
    let gs_stderr = String::from_utf8_lossy(&gs_out.stderr);
    assert!(gs_out.status.success(), "gs failed on {label} (status={}): {gs_stderr}", gs_out.status);
    assert!(gs_stderr.trim().is_empty(), "gs emitted unexpected stderr on {label}: {gs_stderr}");

    let gs_ppm = parse_p6_ppm(&fs::read(&gs_ppm_path).map_err(|e| format!("read {}: {e}", gs_ppm_path.display()))?)?;
    eprintln!("QA [{label}]: Poppler {}x{}, Ghostscript {}x{}", poppler_ppm.width, poppler_ppm.height, gs_ppm.width, gs_ppm.height);
    Ok((poppler_ppm, gs_ppm))
}

fn non_white_stats(img: &PpmImage, x0: usize, y0: usize, x1: usize, y1: usize) -> (usize, Option<[usize; 4]>) {
    let mut count = 0usize;
    let mut bbox: Option<[usize; 4]> = None;
    let xe = x1.min(img.width);
    let ye = y1.min(img.height);
    for y in y0.min(ye)..ye {
        for x in x0.min(xe)..xe {
            let [r, g, b] = img.pixel(x, y);
            if r < 248 || g < 248 || b < 248 {
                count += 1;
                bbox = Some(match bbox {
                    None => [x, y, x, y],
                    Some([bx0, by0, bx1, by1]) => [bx0.min(x), by0.min(y), bx1.max(x), by1.max(y)],
                });
            }
        }
    }
    (count, bbox)
}

fn assert_engines_consistent(label: &str, poppler: &PpmImage, gs: &PpmImage) {
    assert!((poppler.width as isize - gs.width as isize).abs() <= 1, "{label}: width mismatch {} vs {}", poppler.width, gs.width);
    assert!((poppler.height as isize - gs.height as isize).abs() <= 1, "{label}: height mismatch {} vs {}", poppler.height, gs.height);

    let (p_ink, p_box_opt) = non_white_stats(poppler, 0, 0, poppler.width, poppler.height);
    let (g_ink, g_box_opt) = non_white_stats(gs, 0, 0, gs.width, gs.height);
    assert!(p_box_opt.is_some(), "{label}: Poppler image must not be blank");
    assert!(g_box_opt.is_some(), "{label}: Ghostscript image must not be blank");
    let (Some(p_box), Some(g_box)) = (p_box_opt, g_box_opt) else {
        return;
    };
    assert!(p_ink > 100 && g_ink > 100, "{label}: insufficient rendered ink (poppler={p_ink}, gs={g_ink})");

    let ratio = (p_ink as f64) / (g_ink as f64);
    assert!((0.75..=1.35).contains(&ratio), "{label}: Poppler vs GS ink divergence (poppler={p_ink}, gs={g_ink}, ratio={ratio:.3})");

    let max_bbox_diff = (p_box[0] as isize - g_box[0] as isize)
        .abs()
        .max((p_box[1] as isize - g_box[1] as isize).abs())
        .max((p_box[2] as isize - g_box[2] as isize).abs())
        .max((p_box[3] as isize - g_box[3] as isize).abs());
    assert!(max_bbox_diff <= 10, "{label}: ink bbox divergence poppler={p_box:?} gs={g_box:?}");
}

fn add_crop_marks(scene: &mut ChartScene, trim: RectPt) {
    let len = 10.0;
    let gap = 3.0;
    let stroke = Some(StrokePaint::solid(Rgba8::rgba(20, 20, 20, 255), 0.75));
    let x0 = trim.x;
    let y0 = trim.y;
    let x1 = trim.x + trim.width;
    let y1 = trim.y + trim.height;
    let segs = [
        (PointPt::new(x0 - gap - len, y0), PointPt::new(x0 - gap, y0)),
        (PointPt::new(x0, y0 - gap - len), PointPt::new(x0, y0 - gap)),
        (PointPt::new(x1 + gap, y0), PointPt::new(x1 + gap + len, y0)),
        (PointPt::new(x1, y0 - gap - len), PointPt::new(x1, y0 - gap)),
        (PointPt::new(x0 - gap - len, y1), PointPt::new(x0 - gap, y1)),
        (PointPt::new(x0, y1 + gap), PointPt::new(x0, y1 + gap + len)),
        (PointPt::new(x1 + gap, y1), PointPt::new(x1 + gap + len, y1)),
        (PointPt::new(x1, y1 + gap), PointPt::new(x1, y1 + gap + len)),
    ];
    for (a, b) in segs {
        scene.primitives.push(ScenePrimitive::Path { path: Path2D::line(a, b), fill: None, stroke, clip: None });
    }
}

fn push_ttf_glyph_outlines(
    scene: &mut ChartScene,
    ttf: &TrueTypeFont<'_>,
    text: &str,
    start_x_pt: f64,
    baseline_y_pt: f64,
    size_pt: f64,
    color: Rgba8,
) -> (usize, usize) {
    let mut pen_x = start_x_pt;
    let mut mapped_chars = 0usize;
    let mut cmd_total = 0usize;
    for ch in text.chars() {
        let adv = ttf.advance_em(ch).unwrap_or(0.55) * size_pt;
        if let Some(cmds) = outline_ttf_glyph(ttf, ch, pen_x, baseline_y_pt, size_pt, false)
            && !cmds.is_empty()
        {
            mapped_chars += 1;
            cmd_total += cmds.len();
            let mut path = Path2D::new();
            for cmd in cmds {
                match cmd {
                    GlyphOutlineCmd::MoveTo { x_pt, y_pt } => path.commands.push(PathCmd::MoveTo(PointPt::new(x_pt, y_pt))),
                    GlyphOutlineCmd::LineTo { x_pt, y_pt } => path.commands.push(PathCmd::LineTo(PointPt::new(x_pt, y_pt))),
                    GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                        path.commands.push(PathCmd::QuadTo { control: PointPt::new(cx_pt, cy_pt), to: PointPt::new(x_pt, y_pt) });
                    }
                    GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                        path.commands.push(PathCmd::CubicTo {
                            c1: PointPt::new(c1x_pt, c1y_pt),
                            c2: PointPt::new(c2x_pt, c2y_pt),
                            to: PointPt::new(x_pt, y_pt),
                        });
                    }
                    GlyphOutlineCmd::Close => path.commands.push(PathCmd::Close),
                }
            }
            scene.primitives.push(ScenePrimitive::Path { path, fill: Some(FillPaint::solid(color)), stroke: None, clip: None });
        }
        pen_x += adv;
    }
    (mapped_chars, cmd_total)
}

fn pt_to_px_150dpi(pt: f64) -> usize {
    ((pt * 150.0 / 72.0).round() as isize).max(0) as usize
}

#[test]
fn external_qa_scientific_chart_pdf17_alpha_fontfile2_and_crop_marks() {
    let Some((pdftoppm, gs)) = discover_engines_or_skip("external_qa_scientific_chart") else {
        return;
    };
    let Some(arial_path) = find_existing_path(&["/System/Library/Fonts/Supplemental/Arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"])
    else {
        assert!(
            !require_external_viewers(),
            "FAIL-CLOSED external_qa_scientific_chart: host TrueType font (Arial.ttf/DejaVuSans.ttf) absent under CI/CURVEBOOK_REQUIRE_EXTERNAL_VIEWERS"
        );
        eprintln!("SKIP external_qa_scientific_chart: host TrueType font absent");
        return;
    };

    let ttf_bytes = fs::read(&arial_path).expect("read host Arial.ttf");
    // Multiples of 12 pt ensure exact integer dimensions at 150 DPI: 504x360 pt == 1050x750 px.
    let mut scene = ChartScene::new(String::from("qa_sci"), String::from("Dose Response"), 504.0, 360.0, Some(Rgba8::WHITE));
    let trim = RectPt::new(24.0, 24.0, 456.0, 312.0);
    let bleed = RectPt::new(12.0, 12.0, 480.0, 336.0);
    add_crop_marks(&mut scene, trim);

    // Reference axes and vertical dark guide line underneath translucent CI ribbon.
    let axes = Path2D::rect(RectPt::new(96.0, 72.0, 360.0, 216.0));
    scene.primitives.push(ScenePrimitive::Path {
        path: axes,
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::rgba(25, 25, 25, 255), 1.25)),
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(240.0, 84.0, 36.0, 192.0)),
        fill: Some(FillPaint::solid(Rgba8::rgba(20, 20, 20, 255))),
        stroke: None,
        clip: None,
    });

    // Translucent CI ribbon (/CA + /ca ExtGState alpha = 90 fill, 150 stroke).
    let mut ribbon = Path2D::new();
    ribbon.commands.push(PathCmd::MoveTo(PointPt::new(108.0, 132.0)));
    ribbon.commands.push(PathCmd::CubicTo { c1: PointPt::new(200.0, 118.0), c2: PointPt::new(320.0, 110.0), to: PointPt::new(444.0, 108.0) });
    ribbon.commands.push(PathCmd::LineTo(PointPt::new(444.0, 228.0)));
    ribbon.commands.push(PathCmd::CubicTo { c1: PointPt::new(320.0, 230.0), c2: PointPt::new(200.0, 242.0), to: PointPt::new(108.0, 252.0) });
    ribbon.commands.push(PathCmd::Close);
    scene.primitives.push(ScenePrimitive::Path {
        path: ribbon,
        fill: Some(FillPaint::solid(Rgba8::rgba(24, 108, 224, 90))),
        stroke: Some(StrokePaint::solid(Rgba8::rgba(24, 108, 224, 150), 2.0)),
        clip: None,
    });

    // Latin + Greek (μ, α, Δ) + math superscript/subscript text via /FontFile2 + /Symbol.
    let title_rich = RichText::new(vec![
        RichSpan::plain("Response Δ"),
        RichSpan::plain("μ"),
        RichSpan::plain(" (α = 0.05), 10"),
        RichSpan {
            text: String::from("-6"),
            script: SpanScript::Superscript,
            weight: SpanWeight::Regular,
            slope: SpanSlope::Upright,
            color: None,
            font_family: None,
            scale: 1.0,
        },
        RichSpan::plain(" M, K"),
        RichSpan {
            text: String::from("i"),
            script: SpanScript::Subscript,
            weight: SpanWeight::Regular,
            slope: SpanSlope::Italic,
            color: None,
            font_family: None,
            scale: 1.0,
        },
    ]);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: title_rich,
        anchor_pt: PointPt::new(276.0, 52.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 14.0,
        rotation_deg: 0.0,
        color: Rgba8::rgba(15, 15, 15, 255),
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Signal (μV ± Δ)"),
        anchor_pt: PointPt::new(56.0, 180.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: -90.0,
        color: Rgba8::rgba(15, 15, 15, 255),
    }));

    let pdf = render_pdf(
        &scene,
        &PdfExportOptions {
            version: PdfVersion::Pdf17,
            embedded_truetype_font: Some(ttf_bytes),
            outline_text: false,
            trim_pt: Some(trim),
            bleed_pt: Some(bleed),
            title: Some(String::from("Scientific QA")),
            ..PdfExportOptions::default()
        },
    )
    .expect("render %PDF-1.7 with FontFile2");

    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf.starts_with(b"%PDF-1.7"), "expected %PDF-1.7 header");
    assert!(pdf_str.contains("/FontFile2"), "expected embedded /FontFile2");
    assert!(pdf_str.contains("/TrimBox"), "expected /TrimBox");
    assert!(pdf_str.contains("/BleedBox"), "expected /BleedBox");
    assert!(pdf_str.contains("/CA ") && pdf_str.contains("/ca "), "expected /CA and /ca alpha ExtGState");

    let temp = TempDirGuard::new("sci").expect("create temp dir");
    let (poppler, gs) = rasterize_both_engines(&pdftoppm, &gs, temp.path(), "sci", &pdf).expect("rasterize sci PDF");
    assert_engines_consistent("sci", &poppler, &gs);

    for (name, img) in [("Poppler", &poppler), ("Ghostscript", &gs)] {
        // Verify crop marks extend near page outer corners.
        let (tl_marks, _) = non_white_stats(img, 0, 0, pt_to_px_150dpi(24.0), pt_to_px_150dpi(24.0));
        let (br_marks, _) = non_white_stats(img, pt_to_px_150dpi(480.0), pt_to_px_150dpi(336.0), img.width, img.height);
        assert!(tl_marks > 10 && br_marks > 10, "{name}: missing corner crop marks (tl={tl_marks}, br={br_marks})");

        // Verify title region (top strip inside TrimBox) has glyph ink.
        let (title_ink, _) = non_white_stats(img, pt_to_px_150dpi(120.0), pt_to_px_150dpi(30.0), pt_to_px_150dpi(430.0), pt_to_px_150dpi(66.0));
        assert!(title_ink > 120, "{name}: title Latin/Greek/superscript ink too small: {title_ink}");

        // Verify translucent CI ribbon over white background produces intermediate blended blue pixels
        // (R > 120 because alpha=90/255 over white, and B > R + 35 because blue tint is visible).
        let mut blended_white_hits = 0usize;
        let mut blended_dark_hits = 0usize;
        for y in pt_to_px_150dpi(150.0)..pt_to_px_150dpi(210.0) {
            for x in pt_to_px_150dpi(150.0)..pt_to_px_150dpi(220.0) {
                let [r, g, b] = img.pixel(x, y);
                if r > 120 && r < 230 && b > g && b > r + 35 {
                    blended_white_hits += 1;
                }
            }
            // Over the dark black vertical bar x=240..276: black blended with (24,108,224) at a=90/255
            // produces B strictly > R + 25 while still dark (R < 60, B < 140).
            for x in pt_to_px_150dpi(248.0)..pt_to_px_150dpi(268.0) {
                let [r, _g, b] = img.pixel(x, y);
                if r < 60 && b > r + 25 && b < 140 {
                    blended_dark_hits += 1;
                }
            }
        }
        assert!(blended_white_hits > 100, "{name}: translucent ribbon over white missing blended pixels ({blended_white_hits})");
        assert!(blended_dark_hits > 100, "{name}: translucent ribbon over dark bar missing blended pixels ({blended_dark_hits})");
    }
}

#[test]
fn external_qa_cjk_labels_embedded_single_face_ttf() {
    let Some((pdftoppm, gs)) = discover_engines_or_skip("external_qa_cjk") else {
        return;
    };
    let Some(cjk_path) =
        find_existing_path(&["/System/Library/Fonts/Supplemental/Arial Unicode.ttf", "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"])
    else {
        assert!(!require_cjk_rtl_fonts(), "FAIL-CLOSED external_qa_cjk: host single-face CJK .ttf absent under CURVEBOOK_REQUIRE_CJK_RTL_FONTS");
        eprintln!("SKIP external_qa_cjk: host single-face CJK .ttf absent");
        return;
    };

    let cjk_bytes = fs::read(&cjk_path).expect("read host Arial Unicode.ttf");
    let info = parse_truetype_font_info(&cjk_bytes).expect("parse CJK TrueTypeFontInfo");
    assert!(!info.postscript_name.is_empty());
    let cjk_ttf = TrueTypeFont::parse(&cjk_bytes).expect("parse TrueTypeFont for CJK");

    // Optionally also verify AppleGothic.ttf Korean face if present on host.
    let ko_bytes = find_existing_path(&["/System/Library/Fonts/Supplemental/AppleGothic.ttf"]).and_then(|p| fs::read(p).ok());
    let ko_ttf = ko_bytes.as_deref().and_then(|b| {
        parse_truetype_font_info(b).expect("parse AppleGothic.ttf info");
        TrueTypeFont::parse(b)
    });

    let mut scene = ChartScene::new(String::from("qa_cjk"), String::from("CJK Labels"), 504.0, 360.0, Some(Rgba8::WHITE));
    let trim = RectPt::new(24.0, 24.0, 456.0, 312.0);
    let bleed = RectPt::new(12.0, 12.0, 480.0, 336.0);
    add_crop_marks(&mut scene, trim);

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("CJK /FontFile2 QA: ja, zh-CN, ko"),
        anchor_pt: PointPt::new(252.0, 56.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 13.0,
        rotation_deg: 0.0,
        color: Rgba8::rgba(20, 20, 20, 255),
    }));

    let (ja_mapped, ja_cmds) = push_ttf_glyph_outlines(&mut scene, &cjk_ttf, "日本語 用量反応", 72.0, 120.0, 26.0, Rgba8::rgba(20, 40, 120, 255));
    let (zh_mapped, zh_cmds) = push_ttf_glyph_outlines(&mut scene, &cjk_ttf, "中文 剂量反应", 72.0, 192.0, 26.0, Rgba8::rgba(120, 30, 30, 255));
    let ko_font_ref = ko_ttf.as_ref().unwrap_or(&cjk_ttf);
    let (ko_mapped, ko_cmds) = push_ttf_glyph_outlines(&mut scene, ko_font_ref, "한국어 용량 반응", 72.0, 264.0, 26.0, Rgba8::rgba(20, 100, 50, 255));

    assert!(ja_mapped >= 6 && ja_cmds > 50, "ja outlines missing: mapped={ja_mapped}, cmds={ja_cmds}");
    assert!(zh_mapped >= 5 && zh_cmds > 50, "zh-CN outlines missing: mapped={zh_mapped}, cmds={zh_cmds}");
    assert!(ko_mapped >= 6 && ko_cmds > 50, "ko outlines missing: mapped={ko_mapped}, cmds={ko_cmds}");

    let pdf = render_pdf(
        &scene,
        &PdfExportOptions {
            version: PdfVersion::Pdf17,
            embedded_truetype_font: Some(cjk_bytes),
            outline_text: false,
            trim_pt: Some(trim),
            bleed_pt: Some(bleed),
            ..PdfExportOptions::default()
        },
    )
    .expect("render CJK %PDF-1.7 with FontFile2");

    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(String::from_utf8_lossy(&pdf).contains("/FontFile2"));

    let temp = TempDirGuard::new("cjk").expect("create temp dir");
    let (poppler, gs) = rasterize_both_engines(&pdftoppm, &gs, temp.path(), "cjk", &pdf).expect("rasterize cjk PDF");
    assert_engines_consistent("cjk", &poppler, &gs);

    for (name, img) in [("Poppler", &poppler), ("Ghostscript", &gs)] {
        let (ja_ink, ja_box) = non_white_stats(img, pt_to_px_150dpi(60.0), pt_to_px_150dpi(90.0), pt_to_px_150dpi(440.0), pt_to_px_150dpi(135.0));
        let (zh_ink, zh_box) = non_white_stats(img, pt_to_px_150dpi(60.0), pt_to_px_150dpi(160.0), pt_to_px_150dpi(440.0), pt_to_px_150dpi(210.0));
        let (ko_ink, ko_box) = non_white_stats(img, pt_to_px_150dpi(60.0), pt_to_px_150dpi(232.0), pt_to_px_150dpi(440.0), pt_to_px_150dpi(282.0));
        assert!(ja_ink > 300 && ja_box.is_some(), "{name}: missing ja glyph ink ({ja_ink})");
        assert!(zh_ink > 300 && zh_box.is_some(), "{name}: missing zh-CN glyph ink ({zh_ink})");
        assert!(ko_ink > 300 && ko_box.is_some(), "{name}: missing ko glyph ink ({ko_ink})");
    }
}

#[test]
fn external_qa_rtl_arabic_and_hebrew_bidi_visual_order() {
    let Some((pdftoppm, gs)) = discover_engines_or_skip("external_qa_rtl") else {
        return;
    };
    let Some(ttf_path) = find_existing_path(&[
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]) else {
        assert!(
            !require_cjk_rtl_fonts(),
            "FAIL-CLOSED external_qa_rtl: host TrueType font with Hebrew/Arabic absent under CURVEBOOK_REQUIRE_CJK_RTL_FONTS"
        );
        eprintln!("SKIP external_qa_rtl: host TrueType font with Hebrew/Arabic absent");
        return;
    };

    let ttf_bytes = fs::read(&ttf_path).expect("read host TrueType font");
    let ttf = TrueTypeFont::parse(&ttf_bytes).expect("parse TrueTypeFont for RTL");

    let mut scene = ChartScene::new(String::from("qa_rtl"), String::from("RTL BiDi"), 504.0, 360.0, Some(Rgba8::WHITE));
    let trim = RectPt::new(24.0, 24.0, 456.0, 312.0);
    let bleed = RectPt::new(12.0, 12.0, 480.0, 336.0);
    add_crop_marks(&mut scene, trim);

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("RTL BiDi Visual Shaping QA (ar / he)"),
        anchor_pt: PointPt::new(252.0, 56.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 13.0,
        rotation_deg: 0.0,
        color: Rgba8::rgba(20, 20, 20, 255),
    }));

    // Shape both Arabic and Hebrew mixed RTL/LTR scientific strings through BiDi ShapedRun path.
    let he_logical = "ריכוז [IC50] = 5.2";
    let ar_logical = "التركيز [IC50] = 5.2";
    let he_runs = shape_bidi_runs(he_logical, TextDirection::Rtl);
    let ar_runs = shape_bidi_runs(ar_logical, TextDirection::Rtl);
    assert!(he_runs.len() >= 2 && ar_runs.len() >= 2, "expected mixed LTR/RTL runs from shape_bidi_runs");

    let he_visual: String = he_runs.iter().map(|r| r.text.as_str()).collect();
    let ar_visual: String = ar_runs.iter().map(|r| r.text.as_str()).collect();
    assert!(he_visual.starts_with("5.2"), "RTL visual run must place trailing digits left first: {he_visual}");
    assert!(ar_visual.starts_with("5.2"), "RTL visual run must place trailing digits left first: {ar_visual}");

    let (he_mapped, he_cmds) = push_ttf_glyph_outlines(&mut scene, &ttf, &he_visual, 84.0, 150.0, 24.0, Rgba8::rgba(25, 65, 145, 255));
    let (ar_mapped, ar_cmds) = push_ttf_glyph_outlines(&mut scene, &ttf, &ar_visual, 84.0, 240.0, 24.0, Rgba8::rgba(135, 45, 25, 255));
    assert!(he_mapped >= 12 && he_cmds > 50, "Hebrew/Latin TTF outlines missing: mapped={he_mapped}, cmds={he_cmds}");
    assert!(ar_mapped >= 14 && ar_cmds > 50, "Arabic/Latin TTF outlines missing: mapped={ar_mapped}, cmds={ar_cmds}");

    let pdf = render_pdf(
        &scene,
        &PdfExportOptions {
            version: PdfVersion::Pdf17,
            embedded_truetype_font: Some(ttf_bytes),
            outline_text: false,
            trim_pt: Some(trim),
            bleed_pt: Some(bleed),
            ..PdfExportOptions::default()
        },
    )
    .expect("render RTL %PDF-1.7 with FontFile2");

    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(String::from_utf8_lossy(&pdf).contains("/FontFile2"));

    let temp = TempDirGuard::new("rtl").expect("create temp dir");
    let (poppler, gs) = rasterize_both_engines(&pdftoppm, &gs, temp.path(), "rtl", &pdf).expect("rasterize rtl PDF");
    assert_engines_consistent("rtl", &poppler, &gs);

    for (name, img) in [("Poppler", &poppler), ("Ghostscript", &gs)] {
        // Check Hebrew band (y = 115..165 pt) left LTR digits and right Hebrew letters both contain ink.
        let (he_left, _) = non_white_stats(img, pt_to_px_150dpi(75.0), pt_to_px_150dpi(115.0), pt_to_px_150dpi(170.0), pt_to_px_150dpi(165.0));
        let (he_right, _) = non_white_stats(img, pt_to_px_150dpi(210.0), pt_to_px_150dpi(115.0), pt_to_px_150dpi(340.0), pt_to_px_150dpi(165.0));
        assert!(he_left > 100 && he_right > 100, "{name}: Hebrew row ink missing (left={he_left}, right={he_right})");

        // Check Arabic band (y = 205..255 pt) left LTR digits and right Arabic letters both contain ink.
        let (ar_left, _) = non_white_stats(img, pt_to_px_150dpi(75.0), pt_to_px_150dpi(205.0), pt_to_px_150dpi(170.0), pt_to_px_150dpi(255.0));
        let (ar_right, _) = non_white_stats(img, pt_to_px_150dpi(210.0), pt_to_px_150dpi(205.0), pt_to_px_150dpi(370.0), pt_to_px_150dpi(255.0));
        assert!(ar_left > 100 && ar_right > 100, "{name}: Arabic row ink missing (left={ar_left}, right={ar_right})");
    }
}
