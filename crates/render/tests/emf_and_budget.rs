//! Clean-room verification of `MAX_RASTER_PIXELS` memory budgeting (`G11`),
//! `apply_canvas_preview_filter` (`QOL-9`), and strict binary ECMA-234 `.emf` output (`G11`).

use std::collections::{HashMap, HashSet};

use lumencraft_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
};
use lumencraft_palette::{CanvasPreviewFilter, LineDashStyle, Rgba8, simulate_preview_filter};
use lumencraft_render::{MAX_RASTER_PIXELS, RasterImage, RenderError, RgbaPixmap, apply_canvas_preview_filter, encode_emf, encode_raster_emf};
use lumencraft_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight};

const EMR_HEADER: u32 = 1;
const EMR_POLYBEZIER: u32 = 2;
const EMR_POLYBEZIERTO: u32 = 5;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_EOF: u32 = 14;
const EMR_SETMAPMODE: u32 = 17;
const EMR_MOVETOEX: u32 = 27;
const EMR_INTERSECTCLIPRECT: u32 = 30;
const EMR_SAVEDC: u32 = 33;
const EMR_RESTOREDC: u32 = 34;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_DELETEOBJECT: u32 = 40;
const EMR_LINETO: u32 = 54;
const EMR_BEGINPATH: u32 = 59;
const EMR_ENDPATH: u32 = 60;
const EMR_CLOSEFIGURE: u32 = 61;
const EMR_FILLPATH: u32 = 62;
const EMR_STROKEANDFILLPATH: u32 = 63;
const EMR_STROKEPATH: u32 = 64;
const EMR_STRETCHDIBITS: u32 = 81;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTW: u32 = 84;
const EMR_EXTCREATEPEN: u32 = 95;

const ENHMETA_SIGNATURE: u32 = 0x464D_4520;
const ENHMETA_VERSION: u32 = 0x0001_0000;
const MM_ANISOTROPIC: u32 = 8;

fn read_u16_le(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}

fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn read_i32_le(bytes: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

struct ParsedEmfSummary {
    record_counts: HashMap<u32, usize>,
    max_handle_allocated: u32,
    n_handles_header: u16,
    frame_hmm: [i32; 4],
    map_modes: Vec<u32>,
    decoded_texts: Vec<String>,
    polybezier_point_counts: Vec<u32>,
    dib_records: Vec<ParsedDib>,
}

struct ParsedDib {
    width: i32,
    height: i32,
    bit_count: u16,
    compression: u32,
    pixels_bgra: Vec<u8>,
}

struct EmfParserState {
    n_handles_header: u16,
    record_counts: HashMap<u32, usize>,
    live_handles: HashSet<u32>,
    max_handle_allocated: u32,
    clip_depth: usize,
    in_path: bool,
    saw_eof: bool,
    map_modes: Vec<u32>,
    decoded_texts: Vec<String>,
    polybezier_point_counts: Vec<u32>,
    dib_records: Vec<ParsedDib>,
}

impl EmfParserState {
    fn register_handle(&mut self, rec: &[u8]) {
        let h = read_u32_le(rec, 8);
        assert!(h >= 1 && h < u32::from(self.n_handles_header), "handle {h} out of nHandles={}", self.n_handles_header);
        assert!(self.live_handles.insert(h), "handle {h} already live");
        self.max_handle_allocated = self.max_handle_allocated.max(h);
    }

    fn decode_exttextoutw(&mut self, rec: &[u8]) {
        assert!(rec.len() >= 76);
        let n_chars = read_u32_le(rec, 44) as usize;
        let off_str = read_u32_le(rec, 48) as usize;
        let off_dx = read_u32_le(rec, 72) as usize;
        assert!(off_str >= 76);
        assert!(off_str + n_chars * 2 <= off_dx);
        assert!(off_dx + n_chars * 4 <= rec.len());
        let units: Vec<u16> = rec[off_str..off_str + n_chars * 2].as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        let decoded = String::from_utf16(&units);
        assert!(decoded.is_ok(), "valid UTF-16LE in EMR_EXTTEXTOUTW");
        if let Ok(text) = decoded {
            self.decoded_texts.push(text);
        }
    }

    fn decode_stretchdibits(&mut self, rec: &[u8]) {
        assert!(rec.len() >= 120);
        let off_bmi = read_u32_le(rec, 48) as usize;
        let cb_bmi = read_u32_le(rec, 52) as usize;
        let off_bits = read_u32_le(rec, 56) as usize;
        let cb_bits = read_u32_le(rec, 60) as usize;
        assert_eq!(off_bmi, 80);
        assert_eq!(cb_bmi, 40);
        assert_eq!(off_bits, 120);
        assert!(off_bits + cb_bits <= rec.len());
        let bi_size = read_u32_le(rec, off_bmi);
        let width = read_i32_le(rec, off_bmi + 4);
        let height = read_i32_le(rec, off_bmi + 8);
        let bit_count = read_u16_le(rec, off_bmi + 14);
        let compression = read_u32_le(rec, off_bmi + 16);
        assert_eq!(bi_size, 40);
        assert_eq!(cb_bits, (width as usize) * (height as usize) * 4);
        self.dib_records.push(ParsedDib { width, height, bit_count, compression, pixels_bgra: rec[off_bits..off_bits + cb_bits].to_vec() });
    }

    fn validate_path_or_clip_record(&mut self, i_type: u32, n_size: u32, rec: &[u8]) {
        match i_type {
            EMR_SAVEDC => {
                assert_eq!(n_size, 8);
                self.clip_depth += 1;
            }
            EMR_INTERSECTCLIPRECT => {
                assert_eq!(n_size, 24);
                assert!(read_i32_le(rec, 8) <= read_i32_le(rec, 16) && read_i32_le(rec, 12) <= read_i32_le(rec, 20));
            }
            EMR_RESTOREDC => {
                assert_eq!(n_size, 12);
                assert!(self.clip_depth > 0, "EMR_RESTOREDC without matching EMR_SAVEDC");
                self.clip_depth -= 1;
            }
            EMR_BEGINPATH => {
                assert_eq!(n_size, 8);
                assert!(!self.in_path, "nested EMR_BEGINPATH");
                self.in_path = true;
            }
            EMR_ENDPATH => {
                assert_eq!(n_size, 8);
                assert!(self.in_path, "EMR_ENDPATH without EMR_BEGINPATH");
                self.in_path = false;
            }
            EMR_CLOSEFIGURE => {
                assert_eq!(n_size, 8);
                assert!(self.in_path, "EMR_CLOSEFIGURE outside path");
            }
            EMR_POLYBEZIER => {
                assert!(!self.in_path, "EMR_POLYBEZIER should not appear inside BeginPath/EndPath; use EMR_POLYBEZIERTO");
                let cptl = read_u32_le(rec, 24);
                assert!(cptl >= 4 && (cptl - 1).is_multiple_of(3), "invalid EMR_POLYBEZIER cptl={cptl}");
                assert_eq!(n_size as usize, 28 + (cptl as usize) * 8);
                self.polybezier_point_counts.push(cptl);
            }
            EMR_POLYBEZIERTO => {
                assert!(self.in_path, "EMR_POLYBEZIERTO emitted outside path");
                let cptl = read_u32_le(rec, 24);
                assert!(cptl >= 3 && cptl.is_multiple_of(3), "invalid EMR_POLYBEZIERTO cptl={cptl}");
                assert_eq!(n_size as usize, 28 + (cptl as usize) * 8);
                self.polybezier_point_counts.push(cptl);
            }
            EMR_FILLPATH | EMR_STROKEANDFILLPATH | EMR_STROKEPATH => {
                assert_eq!(n_size, 24);
                assert!(!self.in_path, "path paint before EMR_ENDPATH");
            }
            _ => {}
        }
    }

    fn validate_record(&mut self, i_type: u32, n_size: u32, offset: usize, rec: &[u8]) {
        match i_type {
            EMR_HEADER => assert_eq!(offset, 0, "EMR_HEADER encountered after offset 0"),
            EMR_SETMAPMODE => {
                assert_eq!(n_size, 12);
                self.map_modes.push(read_u32_le(rec, 8));
            }
            EMR_SETWINDOWEXTEX | EMR_SETVIEWPORTEXTEX | EMR_MOVETOEX | EMR_LINETO => assert_eq!(n_size, 16),
            EMR_CREATEPEN => {
                assert_eq!(n_size, 28);
                self.register_handle(rec);
            }
            EMR_EXTCREATEPEN => {
                assert!(n_size >= 56);
                self.register_handle(rec);
            }
            EMR_CREATEBRUSHINDIRECT => {
                assert_eq!(n_size, 24);
                self.register_handle(rec);
            }
            EMR_EXTCREATEFONTINDIRECTW => {
                assert!(n_size == 332 || n_size == 104, "unexpected font record size {n_size}");
                self.register_handle(rec);
            }
            EMR_SELECTOBJECT => {
                assert_eq!(n_size, 12);
                let h = read_u32_le(rec, 8);
                assert!((h & 0x8000_0000) != 0 || self.live_handles.contains(&h), "selecting non-live handle {h:#x}");
            }
            EMR_DELETEOBJECT => {
                assert_eq!(n_size, 12);
                let h = read_u32_le(rec, 8);
                assert!(self.live_handles.remove(&h), "deleting non-live handle {h}");
            }
            EMR_SAVEDC
            | EMR_INTERSECTCLIPRECT
            | EMR_RESTOREDC
            | EMR_BEGINPATH
            | EMR_ENDPATH
            | EMR_CLOSEFIGURE
            | EMR_POLYBEZIER
            | EMR_POLYBEZIERTO
            | EMR_FILLPATH
            | EMR_STROKEANDFILLPATH
            | EMR_STROKEPATH => self.validate_path_or_clip_record(i_type, n_size, rec),
            EMR_EXTTEXTOUTW => self.decode_exttextoutw(rec),
            EMR_STRETCHDIBITS => self.decode_stretchdibits(rec),
            EMR_EOF => {
                assert!(n_size >= 20);
                assert_eq!(read_u32_le(rec, rec.len() - 4), n_size, "EMR_EOF nSizeLast mismatch");
                self.saw_eof = true;
            }
            _ => {}
        }
    }
}

fn parse_and_validate_emf(bytes: &[u8]) -> ParsedEmfSummary {
    assert!(bytes.len() >= 88, "EMF too short for EMR_HEADER: {}", bytes.len());
    assert!(bytes.len().is_multiple_of(4), "EMF total length {} is not 4-byte aligned", bytes.len());

    let header_type = read_u32_le(bytes, 0);
    let header_size = read_u32_le(bytes, 4);
    assert_eq!(header_type, EMR_HEADER, "first record must be EMR_HEADER");
    assert!(header_size >= 88 && header_size.is_multiple_of(4));

    let frame_hmm = [read_i32_le(bytes, 24), read_i32_le(bytes, 28), read_i32_le(bytes, 32), read_i32_le(bytes, 36)];
    assert_eq!(read_u32_le(bytes, 40), ENHMETA_SIGNATURE, "missing EMF signature 0x464D4520");
    assert_eq!(read_u32_le(bytes, 44), ENHMETA_VERSION, "unexpected EMF version");
    let n_bytes_hdr = read_u32_le(bytes, 48);
    let n_records_hdr = read_u32_le(bytes, 52);
    let n_handles_header = read_u16_le(bytes, 56);
    assert_eq!(n_bytes_hdr as usize, bytes.len(), "header nBytes mismatch");
    assert!(n_handles_header >= 1, "nHandles must reserve slot 0");

    let mut state = EmfParserState {
        n_handles_header,
        record_counts: HashMap::new(),
        live_handles: HashSet::new(),
        max_handle_allocated: 0,
        clip_depth: 0,
        in_path: false,
        saw_eof: false,
        map_modes: Vec::new(),
        decoded_texts: Vec::new(),
        polybezier_point_counts: Vec::new(),
        dib_records: Vec::new(),
    };

    let mut offset = 0usize;
    let mut walked_records = 0u32;
    while offset < bytes.len() {
        assert!(!state.saw_eof, "records exist after EMR_EOF at offset {offset}");
        assert!(offset + 8 <= bytes.len(), "truncated record header at offset {offset}");
        let i_type = read_u32_le(bytes, offset);
        let n_size = read_u32_le(bytes, offset + 4);
        assert!(n_size >= 8 && n_size.is_multiple_of(4), "invalid record size {n_size} for iType={i_type}");
        let rec_end = offset + n_size as usize;
        assert!(rec_end <= bytes.len(), "record iType={i_type} overruns EMF stream");
        *state.record_counts.entry(i_type).or_insert(0) += 1;
        walked_records += 1;
        state.validate_record(i_type, n_size, offset, &bytes[offset..rec_end]);
        offset = rec_end;
    }

    assert!(state.saw_eof, "missing terminating EMR_EOF record");
    assert_eq!(walked_records, n_records_hdr, "header nRecords mismatch");
    assert_eq!(n_handles_header as u32, state.max_handle_allocated + 1, "header nHandles mismatch");
    assert!(state.live_handles.is_empty(), "leaked EMF GDI handles: {:?}", state.live_handles);
    assert_eq!(state.clip_depth, 0, "unbalanced EMR_SAVEDC / EMR_RESTOREDC");
    assert!(!state.in_path, "unclosed EMR_BEGINPATH");

    ParsedEmfSummary {
        record_counts: state.record_counts,
        max_handle_allocated: state.max_handle_allocated,
        n_handles_header,
        frame_hmm,
        map_modes: state.map_modes,
        decoded_texts: state.decoded_texts,
        polybezier_point_counts: state.polybezier_point_counts,
        dib_records: state.dib_records,
    }
}

#[test]
fn max_raster_pixels_rejects_oversized_targets_without_oom() {
    assert_eq!(MAX_RASTER_PIXELS, 67_108_864);

    let err_max_x_1 = RasterImage::new(u32::MAX, 1, 96, Rgba8::WHITE).expect_err("u32::MAX x 1 must fail safely");
    assert!(matches!(err_max_x_1, RenderError::InvalidTarget { .. }));

    let err_1_x_max = RasterImage::new(1, u32::MAX, 96, Rgba8::WHITE).expect_err("1 x u32::MAX must fail safely");
    assert!(matches!(err_1_x_max, RenderError::InvalidTarget { .. }));

    // Within MAX_RASTER_DIM_PX (16384) per side, yet exceeds MAX_RASTER_PIXELS (67,108,864).
    let err_16384_sq = RasterImage::new(16_384, 16_384, 300, Rgba8::WHITE).expect_err("16384x16384 exceeds MAX_RASTER_PIXELS");
    assert!(matches!(err_16384_sq, RenderError::InvalidTarget { .. }));

    let err_just_over_budget = RasterImage::new(16_384, 4_097, 300, Rgba8::WHITE).expect_err("16384x4097 = 67,125,248 > MAX_RASTER_PIXELS");
    assert!(matches!(err_just_over_budget, RenderError::InvalidTarget { .. }));

    let err_8193_8192 = RasterImage::new(8_193, 8_192, 300, Rgba8::WHITE).expect_err("8193x8192 > 64M pixel budget");
    assert!(matches!(err_8193_8192, RenderError::InvalidTarget { .. }));

    let ok_small = RasterImage::new(64, 48, 96, Rgba8::WHITE).expect("small raster target should succeed");
    assert_eq!(ok_small.rgba.len(), 64 * 48 * 4);
}

#[test]
fn apply_canvas_preview_filter_transforms_pixels_and_preserves_alpha() {
    let px0 = Rgba8::rgba(220, 40, 35, 255);
    let px1 = Rgba8::rgba(35, 175, 65, 180);
    let px2 = Rgba8::rgba(40, 95, 210, 90);
    let mut pixmap: RgbaPixmap = RasterImage::new(3, 1, 96, Rgba8::TRANSPARENT).expect("3x1 pixmap");
    pixmap.rgba.clear();
    for px in [px0, px1, px2] {
        pixmap.rgba.extend_from_slice(&[px.r, px.g, px.b, px.a]);
    }

    let original = pixmap.clone();
    apply_canvas_preview_filter(&mut pixmap, CanvasPreviewFilter::Normal);
    assert_eq!(pixmap, original, "Normal filter must leave pixels unchanged");

    let mut deut = pixmap.clone();
    apply_canvas_preview_filter(&mut deut, CanvasPreviewFilter::Deuteranopia);
    for (idx, src) in [px0, px1, px2].into_iter().enumerate() {
        let exp = simulate_preview_filter(src, CanvasPreviewFilter::Deuteranopia);
        let got = Rgba8::rgba(deut.rgba[idx * 4], deut.rgba[idx * 4 + 1], deut.rgba[idx * 4 + 2], deut.rgba[idx * 4 + 3]);
        assert_eq!(got, exp);
        assert_eq!(got.a, src.a, "alpha must be preserved under Deuteranopia");
    }
    assert_ne!(deut.rgba, original.rgba, "Deuteranopia must transform saturated red/green channels");

    let mut gray = pixmap;
    apply_canvas_preview_filter(&mut gray, CanvasPreviewFilter::GrayscalePrint);
    for (idx, src) in [px0, px1, px2].into_iter().enumerate() {
        let exp = simulate_preview_filter(src, CanvasPreviewFilter::GrayscalePrint);
        let r = gray.rgba[idx * 4];
        let g = gray.rgba[idx * 4 + 1];
        let b = gray.rgba[idx * 4 + 2];
        let a = gray.rgba[idx * 4 + 3];
        assert_eq!((r, g, b, a), (exp.r, exp.g, exp.b, exp.a));
        assert!(r == g && g == b, "GrayscalePrint must emit neutral luminance RGB");
        assert_eq!(a, src.a);
    }
}

#[test]
fn encode_emf_emits_valid_ecma234_records_paths_clips_and_unicode_text() {
    let mut scene = ChartScene::new(String::from("sheet_emf"), String::from("Dose-Response"), 420.0, 300.0, Some(Rgba8::rgba(255, 255, 255, 255)));

    // Push clip rect around plot area, draw cubic + quadratic Bezier curve and polygon fill, then pop clip.
    scene.primitives.push(ScenePrimitive::PushClip { rect: RectPt::new(40.0, 40.0, 340.0, 220.0) });

    let mut ribbon = Path2D::new();
    ribbon.commands.push(PathCmd::MoveTo(PointPt::new(60.0, 180.0)));
    ribbon.commands.push(PathCmd::CubicTo { c1: PointPt::new(130.0, 150.0), c2: PointPt::new(220.0, 90.0), to: PointPt::new(340.0, 75.0) });
    ribbon.commands.push(PathCmd::LineTo(PointPt::new(340.0, 125.0)));
    ribbon.commands.push(PathCmd::QuadTo { control: PointPt::new(210.0, 160.0), to: PointPt::new(60.0, 215.0) });
    ribbon.commands.push(PathCmd::Close);

    scene.primitives.push(ScenePrimitive::Path {
        path: ribbon,
        fill: Some(FillPaint::solid(Rgba8::rgba(45, 120, 210, 70))),
        stroke: Some(StrokePaint {
            color: Rgba8::rgba(20, 80, 160, 255),
            width_pt: 1.75,
            dash: LineDashStyle::Dashed,
            cap: LineCap::Round,
            join: LineJoin::Bevel,
        }),
        clip: None,
    });

    // Simple solid Butt/Miter path (exercises EMR_CREATEPEN alongside EMR_EXTCREATEPEN above).
    let axis = Path2D::line(PointPt::new(50.0, 240.0), PointPt::new(360.0, 240.0));
    scene.primitives.push(ScenePrimitive::Path {
        path: axis,
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::rgba(30, 30, 30, 255), 1.0)),
        clip: Some(RectPt::new(45.0, 230.0, 320.0, 20.0)),
    });

    scene.primitives.push(ScenePrimitive::PopClip);

    // Rich Unicode text with Greek letters and superscripts + rotated axis title.
    let title_rich = RichText::new(vec![
        RichSpan::plain("Response Δ"),
        RichSpan {
            text: String::from("μ"),
            script: SpanScript::Baseline,
            weight: SpanWeight::Bold,
            slope: SpanSlope::Upright,
            color: None,
            font_family: None,
            scale: 1.0,
        },
        RichSpan::plain(" / cm"),
        RichSpan {
            text: String::from("-2"),
            script: SpanScript::Superscript,
            weight: SpanWeight::Regular,
            slope: SpanSlope::Italic,
            color: Some(Rgba8::rgba(180, 30, 30, 255)),
            font_family: Some(String::from("Arial")),
            scale: 1.0,
        },
    ]);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: title_rich,
        anchor_pt: PointPt::new(210.0, 24.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::rgba(15, 15, 15, 255),
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Concentration (αM)"),
        anchor_pt: PointPt::new(18.0, 150.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 10.5,
        rotation_deg: -90.0,
        color: Rgba8::rgba(15, 15, 15, 255),
    }));

    let emf = encode_emf(&scene).expect("EMF encoding must succeed");
    let parsed = parse_and_validate_emf(&emf);

    // Frame bounds in 0.01 mm: 420 pt * 2540 / 72 = 14817, 300 pt * 2540 / 72 = 10583.
    let expected_w_hmm = (420.0_f64 * 2540.0 / 72.0).round() as i32;
    let expected_h_hmm = (300.0_f64 * 2540.0 / 72.0).round() as i32;
    assert_eq!(parsed.frame_hmm[0], 0);
    assert_eq!(parsed.frame_hmm[1], 0);
    assert!((parsed.frame_hmm[2] - expected_w_hmm).abs() <= 1);
    assert!((parsed.frame_hmm[3] - expected_h_hmm).abs() <= 1);

    assert_eq!(parsed.map_modes, vec![MM_ANISOTROPIC]);
    assert!(parsed.record_counts.contains_key(&EMR_SETWINDOWEXTEX));
    assert!(parsed.record_counts.contains_key(&EMR_SETVIEWPORTEXTEX));
    assert!(parsed.record_counts.contains_key(&EMR_CREATEPEN));
    assert!(parsed.record_counts.contains_key(&EMR_EXTCREATEPEN));
    assert!(parsed.record_counts.contains_key(&EMR_CREATEBRUSHINDIRECT));
    assert!(parsed.record_counts.contains_key(&EMR_STROKEANDFILLPATH));
    assert!(parsed.record_counts.contains_key(&EMR_FILLPATH));
    assert!(parsed.record_counts.contains_key(&EMR_STROKEPATH));
    assert!(parsed.record_counts.contains_key(&EMR_POLYBEZIERTO));
    assert_eq!(parsed.polybezier_point_counts, vec![3, 3]);
    assert_eq!(parsed.record_counts.get(&EMR_SAVEDC).copied(), Some(2));
    assert_eq!(parsed.record_counts.get(&EMR_INTERSECTCLIPRECT).copied(), Some(2));
    assert_eq!(parsed.record_counts.get(&EMR_RESTOREDC).copied(), Some(2));
    assert!(parsed.record_counts.contains_key(&EMR_EXTCREATEFONTINDIRECTW));
    assert!(parsed.record_counts.contains_key(&EMR_DELETEOBJECT));
    assert_eq!(parsed.record_counts.get(&EMR_EOF).copied(), Some(1));
    assert!(parsed.max_handle_allocated >= 1);
    assert_eq!(parsed.n_handles_header, (parsed.max_handle_allocated + 1) as u16);

    let joined_text = parsed.decoded_texts.join("|");
    assert!(joined_text.contains("Response Δ"));
    assert!(joined_text.contains('μ'));
    assert!(joined_text.contains("-2"));
    assert!(joined_text.contains("αM"));
}

#[test]
fn encode_raster_emf_emits_bottom_up_bgra_stretchdibits() {
    // 2x2 distinct RGBA pixels:
    // top row (y=0):    Red   [255, 0, 0, 255], Green [0, 255, 0, 255]
    // bottom row (y=1): Blue  [0, 0, 255, 255], White [255, 255, 255, 200]
    let mut img = RasterImage::new(2, 2, 150, Rgba8::TRANSPARENT).expect("2x2 raster");
    img.rgba = vec![
        255, 0, 0, 255, // (0, 0) Red
        0, 255, 0, 255, // (1, 0) Green
        0, 0, 255, 255, // (0, 1) Blue
        255, 255, 255, 200, // (1, 1) White-ish
    ];

    let emf = encode_raster_emf(&img).expect("raster EMF must succeed");
    let parsed = parse_and_validate_emf(&emf);
    assert_eq!(parsed.dib_records.len(), 1);
    let dib = &parsed.dib_records[0];
    assert_eq!(dib.width, 2);
    assert_eq!(dib.height, 2, "positive biHeight required for bottom-up DIB");
    assert_eq!(dib.bit_count, 32);
    assert_eq!(dib.compression, 0);
    // DIB scanline 0 is bottom image row (y=1) in BGRA; scanline 1 is top image row (y=0) in BGRA.
    assert_eq!(
        dib.pixels_bgra,
        vec![
            255, 0, 0, 255, // bottom-left Blue -> BGRA
            255, 255, 255, 200, // bottom-right -> BGRA
            0, 0, 255, 255, // top-left Red -> BGRA
            0, 255, 0, 255, // top-right Green -> BGRA
        ]
    );
}
