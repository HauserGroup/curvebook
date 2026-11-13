//! Clean-room glyph vector path outline generator and TrueType `glyf` contour decoder for
//! SVG, PDF, and CPU raster export backends.
//!
//! All emitted contours in [`PositionedGlyphOutline::commands`] are **closed filled outlines**
//! in Y-down PostScript point coordinates (`ascenders < origin_y_pt`, `descenders > origin_y_pt`)
//! oriented for the nonzero winding fill rule (positive area for outer contours, negative area
//! for inner holes).

use lumencraft_palette::Rgba8;
use serde::{Deserialize, Serialize};

use crate::markup::{RichText, SpanSlope, SpanWeight};
use crate::metrics::{
    TrueTypeFont, char_advance_em_with_font, effective_span_scale, read_i16_be, read_u16_be, script_baseline_offset_pt, select_bundled_font,
};

const MAX_TTF_CONTOURS: usize = 256;
const MAX_TTF_POINTS: usize = 4096;
const MAX_TTF_COMPONENT_DEPTH: usize = 8;

/// Vector path command representing a glyph contour in PostScript points (`pt`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GlyphOutlineCmd {
    MoveTo { x_pt: f64, y_pt: f64 },
    LineTo { x_pt: f64, y_pt: f64 },
    QuadTo { cx_pt: f64, cy_pt: f64, x_pt: f64, y_pt: f64 },
    CubicTo { c1x_pt: f64, c1y_pt: f64, c2x_pt: f64, c2y_pt: f64, x_pt: f64, y_pt: f64 },
    Close,
}

/// Positioned glyph outline emitted for downstream SVG, PDF, and CPU raster backends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionedGlyphOutline {
    pub codepoint: char,
    pub origin_x_pt: f64,
    pub origin_y_pt: f64,
    pub advance_x_pt: f64,
    pub color: Option<Rgba8>,
    pub commands: Vec<GlyphOutlineCmd>,
}

/// Position [`RichText`] runs starting at `(origin_x_pt, origin_y_pt)` and emit closed filled glyph outlines.
///
/// Works identically with or without the `bundled-fonts` feature enabled; when bundled TrueType
/// font tables are linked in [`crate::BUNDLED_FONTS`], native `glyf` contours are decoded first
/// with seamless clean-room fallback for unmapped scientific or Greek glyphs.
#[must_use]
pub fn outline_rich_text(rich: &RichText, origin_x_pt: f64, origin_y_pt: f64, font_size_pt: f64) -> Vec<PositionedGlyphOutline> {
    outline_rich_text_rotated(rich, origin_x_pt, origin_y_pt, font_size_pt, 0.0)
}

/// Position [`RichText`] runs starting at `(origin_x_pt, origin_y_pt)` rotated by `angle_deg`
/// clockwise around `(origin_x_pt, origin_y_pt)` in Y-down PostScript points.
#[must_use]
pub fn outline_rich_text_rotated(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    angle_deg: f64,
) -> Vec<PositionedGlyphOutline> {
    let base_size = if font_size_pt.is_finite() && font_size_pt > 0.0 { font_size_pt } else { 0.0 };
    if base_size == 0.0 || !origin_x_pt.is_finite() || !origin_y_pt.is_finite() {
        return Vec::new();
    }
    let angle = if angle_deg.is_finite() { angle_deg } else { 0.0 };
    let rad = angle.to_radians();
    let (sin_a, cos_a) = rad.sin_cos();
    let rotate_active = angle.abs() > 1e-12;

    let mut out = Vec::new();
    let mut pen_x = origin_x_pt;
    let mut line_y = origin_y_pt;
    let line_step = base_size * 1.20;

    for span in &rich.spans {
        let span_scale = effective_span_scale(span);
        let glyph_size = base_size * span_scale;
        let y_offset = script_baseline_offset_pt(span.script, base_size);
        let bundled = select_bundled_font(span.weight, span.slope);

        for ch in span.text.chars() {
            if ch == '\n' {
                let (gx, gy) = rotate_point(pen_x, line_y + y_offset, origin_x_pt, origin_y_pt, cos_a, sin_a, rotate_active);
                out.push(PositionedGlyphOutline {
                    codepoint: ch,
                    origin_x_pt: gx,
                    origin_y_pt: gy,
                    advance_x_pt: 0.0,
                    color: span.color,
                    commands: Vec::new(),
                });
                pen_x = origin_x_pt;
                line_y += line_step;
                continue;
            }
            if ch == '\r' {
                let (gx, gy) = rotate_point(pen_x, line_y + y_offset, origin_x_pt, origin_y_pt, cos_a, sin_a, rotate_active);
                out.push(PositionedGlyphOutline {
                    codepoint: ch,
                    origin_x_pt: gx,
                    origin_y_pt: gy,
                    advance_x_pt: 0.0,
                    color: span.color,
                    commands: Vec::new(),
                });
                continue;
            }

            let adv_em = char_advance_em_with_font(ch, span.weight, span.slope, bundled);
            let advance = glyph_size * adv_em;
            let glyph_origin_y = line_y + y_offset;

            let mut commands = if ch.is_whitespace() {
                Vec::new()
            } else {
                build_glyph_commands(ch, pen_x, glyph_origin_y, glyph_size, adv_em, span.weight, span.slope, bundled)
            };

            if rotate_active && !commands.is_empty() {
                rotate_commands(&mut commands, origin_x_pt, origin_y_pt, cos_a, sin_a);
            }
            let (rot_x, rot_y) = rotate_point(pen_x, glyph_origin_y, origin_x_pt, origin_y_pt, cos_a, sin_a, rotate_active);

            out.push(PositionedGlyphOutline {
                codepoint: ch,
                origin_x_pt: rot_x,
                origin_y_pt: rot_y,
                advance_x_pt: advance,
                color: span.color,
                commands,
            });
            pen_x += advance;
        }
    }
    out
}

/// Decode a single glyph's TrueType `glyf` contour from `ttf` into Y-down `GlyphOutlineCmd`s.
/// Returns `None` if `ch` is not mapped in `cmap` or `ttf` lacks valid `loca`/`glyf` tables.
#[must_use]
pub fn outline_ttf_glyph(
    ttf: &TrueTypeFont<'_>,
    ch: char,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    italic_slant: bool,
) -> Option<Vec<GlyphOutlineCmd>> {
    if !font_size_pt.is_finite() || font_size_pt <= 0.0 || !ttf.has_glyf_outlines() {
        return None;
    }
    let gid = ttf.glyph_id(ch)?;
    let upem = f64::from(ttf.units_per_em());
    if upem <= 0.0 {
        return None;
    }
    let scale = font_size_pt / upem;
    let mut cmds = Vec::new();
    let slant = if italic_slant { 0.2125 } else { 0.0 };
    decode_ttf_glyph_recursive(ttf, gid, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], scale, origin_x_pt, origin_y_pt, slant, 0, &mut cmds)?;
    Some(cmds)
}

#[allow(clippy::too_many_arguments)]
fn build_glyph_commands(
    ch: char,
    pen_x: f64,
    origin_y: f64,
    glyph_size: f64,
    adv_em: f64,
    weight: SpanWeight,
    slope: SpanSlope,
    bundled: Option<&TrueTypeFont<'_>>,
) -> Vec<GlyphOutlineCmd> {
    if let Some(ttf) = bundled
        && let Some(cmds) = outline_ttf_glyph(ttf, ch, pen_x, origin_y, glyph_size, slope == SpanSlope::Italic)
    {
        return cmds;
    }
    cleanroom_vector_outline(ch, pen_x, origin_y, glyph_size, adv_em, weight, slope)
}

fn rotate_point(x: f64, y: f64, cx: f64, cy: f64, cos_a: f64, sin_a: f64, active: bool) -> (f64, f64) {
    if !active {
        return (x, y);
    }
    let dx = x - cx;
    let dy = y - cy;
    (cx + dx * cos_a - dy * sin_a, cy + dx * sin_a + dy * cos_a)
}

fn rotate_commands(cmds: &mut [GlyphOutlineCmd], cx: f64, cy: f64, cos_a: f64, sin_a: f64) {
    for cmd in cmds {
        match cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } | GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                (*x_pt, *y_pt) = rotate_point(*x_pt, *y_pt, cx, cy, cos_a, sin_a, true);
            }
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                (*cx_pt, *cy_pt) = rotate_point(*cx_pt, *cy_pt, cx, cy, cos_a, sin_a, true);
                (*x_pt, *y_pt) = rotate_point(*x_pt, *y_pt, cx, cy, cos_a, sin_a, true);
            }
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                (*c1x_pt, *c1y_pt) = rotate_point(*c1x_pt, *c1y_pt, cx, cy, cos_a, sin_a, true);
                (*c2x_pt, *c2y_pt) = rotate_point(*c2x_pt, *c2y_pt, cx, cy, cos_a, sin_a, true);
                (*x_pt, *y_pt) = rotate_point(*x_pt, *y_pt, cx, cy, cos_a, sin_a, true);
            }
            GlyphOutlineCmd::Close => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_ttf_glyph_recursive(
    ttf: &TrueTypeFont<'_>,
    gid: u16,
    affine: [f64; 6],
    scale: f64,
    origin_x: f64,
    origin_y: f64,
    slant: f64,
    depth: usize,
    out: &mut Vec<GlyphOutlineCmd>,
) -> Option<()> {
    if depth >= MAX_TTF_COMPONENT_DEPTH {
        return Some(());
    }
    let slice = ttf.glyf_slice(gid)?;
    if slice.is_empty() {
        return Some(());
    }
    if slice.len() < 10 {
        return None;
    }
    let num_contours = read_i16_be(slice, 0)?;
    if num_contours == 0 {
        return Some(());
    }
    if num_contours > 0 {
        let count = usize::try_from(num_contours).ok()?.min(MAX_TTF_CONTOURS);
        return decode_simple_ttf_glyph(slice, count, affine, scale, origin_x, origin_y, slant, out);
    }
    decode_compound_ttf_glyph(ttf, slice, affine, scale, origin_x, origin_y, slant, depth, out)
}

#[allow(clippy::too_many_arguments)]
fn decode_simple_ttf_glyph(
    slice: &[u8],
    num_contours: usize,
    affine: [f64; 6],
    scale: f64,
    origin_x: f64,
    origin_y: f64,
    slant: f64,
    out: &mut Vec<GlyphOutlineCmd>,
) -> Option<()> {
    let mut end_pts = Vec::with_capacity(num_contours);
    for i in 0..num_contours {
        end_pts.push(usize::from(read_u16_be(slice, 10 + i * 2)?));
    }
    let total_points = end_pts.last().copied()?.checked_add(1)?;
    if total_points > MAX_TTF_POINTS {
        return None;
    }

    let inst_len_off = 10 + num_contours * 2;
    let inst_len = usize::from(read_u16_be(slice, inst_len_off)?);
    let mut cursor = inst_len_off.checked_add(2)?.checked_add(inst_len)?;
    if cursor > slice.len() {
        return None;
    }

    let flags = read_ttf_flags(slice, &mut cursor, total_points)?;
    let xs = read_ttf_coords(slice, &mut cursor, &flags, 0x02, 0x10)?;
    let ys = read_ttf_coords(slice, &mut cursor, &flags, 0x04, 0x20)?;

    let mut pts = Vec::with_capacity(total_points);
    for i in 0..total_points {
        let raw_x = f64::from(*xs.get(i)?);
        let raw_y = f64::from(*ys.get(i)?);
        let tx = affine[0] * raw_x + affine[2] * raw_y + affine[4];
        let ty = affine[1] * raw_x + affine[3] * raw_y + affine[5];
        let x_pt = origin_x + (tx + ty * slant) * scale;
        let y_pt = origin_y - ty * scale;
        let on_curve = (*flags.get(i)? & 0x01) != 0;
        pts.push((x_pt, y_pt, on_curve));
    }

    let mut start = 0usize;
    for end in end_pts {
        if end < start || end >= pts.len() {
            return None;
        }
        emit_ttf_contour(&pts[start..=end], out);
        start = end + 1;
    }
    Some(())
}

fn read_ttf_flags(slice: &[u8], cursor: &mut usize, total_points: usize) -> Option<Vec<u8>> {
    let mut flags = Vec::with_capacity(total_points);
    while flags.len() < total_points {
        let flag = *slice.get(*cursor)?;
        *cursor += 1;
        flags.push(flag);
        if (flag & 0x08) != 0 {
            let repeat = usize::from(*slice.get(*cursor)?);
            *cursor += 1;
            let count = repeat.min(total_points.saturating_sub(flags.len()));
            for _ in 0..count {
                flags.push(flag);
            }
        }
    }
    Some(flags)
}

fn read_ttf_coords(slice: &[u8], cursor: &mut usize, flags: &[u8], short_bit: u8, same_or_pos_bit: u8) -> Option<Vec<i32>> {
    let mut coords = Vec::with_capacity(flags.len());
    let mut acc = 0i32;
    for &flag in flags {
        let delta = if (flag & short_bit) != 0 {
            let mag = i32::from(*slice.get(*cursor)?);
            *cursor += 1;
            if (flag & same_or_pos_bit) != 0 { mag } else { -mag }
        } else if (flag & same_or_pos_bit) != 0 {
            0
        } else {
            let val = i32::from(read_i16_be(slice, *cursor)?);
            *cursor += 2;
            val
        };
        acc = acc.saturating_add(delta);
        coords.push(acc);
    }
    Some(coords)
}

fn emit_ttf_contour(contour: &[(f64, f64, bool)], out: &mut Vec<GlyphOutlineCmd>) {
    let n = contour.len();
    if n < 2 {
        return;
    }
    let Some((first_on_idx, start_pt)) = contour.iter().enumerate().find(|(_, p)| p.2).map(|(i, p)| (i, (p.0, p.1))) else {
        // All off-curve points: start at midpoint between last and first.
        let p0 = contour[0];
        let pn = contour[n - 1];
        let start = (0.5 * (pn.0 + p0.0), 0.5 * (pn.1 + p0.1));
        out.push(GlyphOutlineCmd::MoveTo { x_pt: start.0, y_pt: start.1 });
        for i in 0..n {
            let cur = contour[i];
            let nxt = contour[(i + 1) % n];
            let mid = (0.5 * (cur.0 + nxt.0), 0.5 * (cur.1 + nxt.1));
            out.push(GlyphOutlineCmd::QuadTo { cx_pt: cur.0, cy_pt: cur.1, x_pt: mid.0, y_pt: mid.1 });
        }
        out.push(GlyphOutlineCmd::Close);
        return;
    };

    out.push(GlyphOutlineCmd::MoveTo { x_pt: start_pt.0, y_pt: start_pt.1 });
    let mut step = 1usize;
    while step <= n {
        let cur = contour[(first_on_idx + step) % n];
        if cur.2 {
            out.push(GlyphOutlineCmd::LineTo { x_pt: cur.0, y_pt: cur.1 });
            step += 1;
        } else {
            let nxt = contour[(first_on_idx + step + 1) % n];
            if nxt.2 {
                out.push(GlyphOutlineCmd::QuadTo { cx_pt: cur.0, cy_pt: cur.1, x_pt: nxt.0, y_pt: nxt.1 });
                step += 2;
            } else {
                let mid = (0.5 * (cur.0 + nxt.0), 0.5 * (cur.1 + nxt.1));
                out.push(GlyphOutlineCmd::QuadTo { cx_pt: cur.0, cy_pt: cur.1, x_pt: mid.0, y_pt: mid.1 });
                step += 1;
            }
        }
    }
    out.push(GlyphOutlineCmd::Close);
}

#[allow(clippy::too_many_arguments)]
fn decode_compound_ttf_glyph(
    ttf: &TrueTypeFont<'_>,
    slice: &[u8],
    parent: [f64; 6],
    scale: f64,
    origin_x: f64,
    origin_y: f64,
    slant: f64,
    depth: usize,
    out: &mut Vec<GlyphOutlineCmd>,
) -> Option<()> {
    let mut cursor = 10usize;
    for _ in 0..64 {
        let flags = read_u16_be(slice, cursor)?;
        let comp_gid = read_u16_be(slice, cursor + 2)?;
        cursor += 4;

        let (arg1, arg2) = if (flags & 0x0001) != 0 {
            let a1 = read_i16_be(slice, cursor)?;
            let a2 = read_i16_be(slice, cursor + 2)?;
            cursor += 4;
            (f64::from(a1), f64::from(a2))
        } else {
            let a1 = *slice.get(cursor)? as i8;
            let a2 = *slice.get(cursor + 1)? as i8;
            cursor += 2;
            (f64::from(a1), f64::from(a2))
        };

        let (dx, dy) = if (flags & 0x0002) != 0 { (arg1, arg2) } else { (0.0, 0.0) };

        let (a, b, c, d) = if (flags & 0x0008) != 0 {
            let s = f64::from(read_i16_be(slice, cursor)?) / 16384.0;
            cursor += 2;
            (s, 0.0, 0.0, s)
        } else if (flags & 0x0040) != 0 {
            let sx = f64::from(read_i16_be(slice, cursor)?) / 16384.0;
            let sy = f64::from(read_i16_be(slice, cursor + 2)?) / 16384.0;
            cursor += 4;
            (sx, 0.0, 0.0, sy)
        } else if (flags & 0x0080) != 0 {
            let m0 = f64::from(read_i16_be(slice, cursor)?) / 16384.0;
            let m1 = f64::from(read_i16_be(slice, cursor + 2)?) / 16384.0;
            let m2 = f64::from(read_i16_be(slice, cursor + 4)?) / 16384.0;
            let m3 = f64::from(read_i16_be(slice, cursor + 6)?) / 16384.0;
            cursor += 8;
            (m0, m1, m2, m3)
        } else {
            (1.0, 0.0, 0.0, 1.0)
        };

        let combined = [
            parent[0] * a + parent[2] * b,
            parent[1] * a + parent[3] * b,
            parent[0] * c + parent[2] * d,
            parent[1] * c + parent[3] * d,
            parent[0] * dx + parent[2] * dy + parent[4],
            parent[1] * dx + parent[3] * dy + parent[5],
        ];
        decode_ttf_glyph_recursive(ttf, comp_gid, combined, scale, origin_x, origin_y, slant, depth + 1, out)?;
        if (flags & 0x0020) == 0 {
            break;
        }
    }
    Some(())
}

// ---------------------------------------------------------------------------
// Clean-room Vector Glyph Outline Generator (normalized box -> closed contours)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum StrokeItem {
    Open(Vec<(f64, f64)>),
    Ring(Vec<(f64, f64)>),
    Dot(f64, f64, f64),
    Solid(Vec<(f64, f64)>),
}

fn cleanroom_vector_outline(
    ch: char,
    pen_x: f64,
    origin_y: f64,
    glyph_size: f64,
    adv_em: f64,
    weight: SpanWeight,
    slope: SpanSlope,
) -> Vec<GlyphOutlineCmd> {
    let half_stroke = match weight {
        SpanWeight::Regular => 0.043,
        SpanWeight::Bold => 0.062,
    };
    let slant = if slope == SpanSlope::Italic { 0.2125 } else { 0.0 };
    let items = build_glyph_strokes(ch);

    // Map normalized x in [0, 1] into padded glyph advance width [pad_l .. adv_em - pad_r].
    let pad = (adv_em * 0.10).clamp(0.03, 0.09);
    let draw_w = (adv_em - 2.0 * pad).max(0.08);
    let map_pt = |(nx, ny): (f64, f64)| -> (f64, f64) { (pad + nx * draw_w, ny) };

    let mut contours: Vec<Vec<(f64, f64)>> = Vec::new();
    for item in items {
        match item {
            StrokeItem::Open(pts) => {
                let mapped: Vec<(f64, f64)> = pts.into_iter().map(map_pt).collect();
                if let Some(poly) = thicken_open_polyline(&mapped, half_stroke) {
                    contours.push(poly);
                }
            }
            StrokeItem::Ring(pts) => {
                let mapped: Vec<(f64, f64)> = pts.into_iter().map(map_pt).collect();
                thicken_closed_ring(&mapped, half_stroke, &mut contours);
            }
            StrokeItem::Dot(nx, ny, radius_mult) => {
                let (cx, cy) = map_pt((nx, ny));
                contours.push(make_circle_cw_yup(cx, cy, half_stroke * radius_mult, 12));
            }
            StrokeItem::Solid(pts) => {
                let mut mapped: Vec<(f64, f64)> = pts.into_iter().map(map_pt).collect();
                ensure_cw_yup(&mut mapped);
                if mapped.len() >= 3 {
                    contours.push(mapped);
                }
            }
        }
    }

    let mut commands = Vec::with_capacity(contours.len() * 12);
    for contour in contours {
        if contour.len() < 3 {
            continue;
        }
        for (idx, &(ux, uy)) in contour.iter().enumerate() {
            let x_slanted = ux + uy * slant;
            let x_pt = pen_x + x_slanted * glyph_size;
            let y_pt = origin_y - uy * glyph_size;
            if idx == 0 {
                commands.push(GlyphOutlineCmd::MoveTo { x_pt, y_pt });
            } else {
                commands.push(GlyphOutlineCmd::LineTo { x_pt, y_pt });
            }
        }
        commands.push(GlyphOutlineCmd::Close);
    }
    commands
}

fn thicken_open_polyline(pts: &[(f64, f64)], half_w: f64) -> Option<Vec<(f64, f64)>> {
    let clean = dedup_points(pts);
    if clean.is_empty() {
        return None;
    }
    if clean.len() == 1 {
        return Some(make_circle_cw_yup(clean[0].0, clean[0].1, half_w * 1.25, 12));
    }

    let seg_count = clean.len() - 1;
    let mut tangents = Vec::with_capacity(seg_count);
    let mut normals = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let dx = clean[i + 1].0 - clean[i].0;
        let dy = clean[i + 1].1 - clean[i].1;
        let len = dx.hypot(dy);
        let (tx, ty) = if len > 1e-9 { (dx / len, dy / len) } else { (1.0, 0.0) };
        tangents.push((tx, ty));
        normals.push((-ty, tx));
    }

    let mut poly = Vec::with_capacity(clean.len() * 2 + 6);
    // Start at first point left edge
    poly.push((clean[0].0 + normals[0].0 * half_w, clean[0].1 + normals[0].1 * half_w));
    for i in 1..seg_count {
        push_join_offset(&mut poly, clean[i], normals[i - 1], normals[i], half_w, true);
    }
    // End cap at last point
    let last = clean[seg_count];
    let n_last = normals[seg_count - 1];
    let t_last = tangents[seg_count - 1];
    poly.push((last.0 + n_last.0 * half_w, last.1 + n_last.1 * half_w));
    poly.push((last.0 + t_last.0 * half_w * 0.72, last.1 + t_last.1 * half_w * 0.72));
    poly.push((last.0 - n_last.0 * half_w, last.1 - n_last.1 * half_w));

    for i in (1..seg_count).rev() {
        let nr0 = (-normals[i].0, -normals[i].1);
        let nr1 = (-normals[i - 1].0, -normals[i - 1].1);
        push_join_offset(&mut poly, clean[i], nr0, nr1, half_w, true);
    }

    let first = clean[0];
    let n0 = normals[0];
    let t0 = tangents[0];
    poly.push((first.0 - n0.0 * half_w, first.1 - n0.1 * half_w));
    poly.push((first.0 - t0.0 * half_w * 0.72, first.1 - t0.1 * half_w * 0.72));
    Some(poly)
}

fn thicken_closed_ring(pts: &[(f64, f64)], half_w: f64, out: &mut Vec<Vec<(f64, f64)>>) {
    let mut ring = dedup_points(pts);
    if ring.len() >= 2
        && let (Some(&f), Some(&l)) = (ring.first(), ring.last())
        && (f.0 - l.0).hypot(f.1 - l.1) < 1e-6
    {
        ring.pop();
    }
    if ring.len() < 3 {
        return;
    }
    ensure_cw_yup(&mut ring);

    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in &ring {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let min_span = (max_x - min_x).min(max_y - min_y);
    let inner_w = half_w.min((min_span * 0.34).max(0.008));

    let n = ring.len();
    let mut normals = Vec::with_capacity(n);
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len = dx.hypot(dy);
        let (tx, ty) = if len > 1e-9 { (dx / len, dy / len) } else { (1.0, 0.0) };
        normals.push((-ty, tx));
    }

    let mut outer = Vec::with_capacity(n * 2);
    for i in 0..n {
        let prev_n = normals[(i + n - 1) % n];
        let next_n = normals[i];
        push_join_offset(&mut outer, ring[i], prev_n, next_n, half_w, false);
    }

    let mut inner = Vec::with_capacity(n * 2);
    for i in (0..n).rev() {
        let prev_n = normals[(i + n - 1) % n];
        let next_n = normals[i];
        let in_prev = (-prev_n.0, -prev_n.1);
        let in_next = (-next_n.0, -next_n.1);
        push_join_offset(&mut inner, ring[i], in_next, in_prev, inner_w, false);
    }

    out.push(outer);
    if signed_area_yup(&inner).abs() > 1e-6 {
        out.push(inner);
    }
}

fn push_join_offset(poly: &mut Vec<(f64, f64)>, pt: (f64, f64), n_in: (f64, f64), n_out: (f64, f64), half_w: f64, allow_bevel: bool) {
    let dot = n_in.0 * n_out.0 + n_in.1 * n_out.1;
    let denom = 1.0 + dot;
    if denom > 0.35 {
        let mx = n_in.0 + n_out.0;
        let my = n_in.1 + n_out.1;
        let scale = (half_w / denom).min(half_w * 2.0);
        poly.push((pt.0 + mx * scale, pt.1 + my * scale));
    } else if allow_bevel {
        poly.push((pt.0 + n_in.0 * half_w, pt.1 + n_in.1 * half_w));
        poly.push((pt.0 + n_out.0 * half_w, pt.1 + n_out.1 * half_w));
    } else {
        let avg_x = n_in.0 + n_out.0;
        let avg_y = n_in.1 + n_out.1;
        let len = avg_x.hypot(avg_y);
        if len > 1e-9 {
            poly.push((pt.0 + (avg_x / len) * half_w, pt.1 + (avg_y / len) * half_w));
        } else {
            poly.push((pt.0 + n_in.0 * half_w, pt.1 + n_in.1 * half_w));
        }
    }
}

fn dedup_points(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last().is_none_or(|&prev| (prev.0 - p.0).hypot(prev.1 - p.1) > 1e-6) {
            out.push(p);
        }
    }
    out
}

fn signed_area_yup(pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        sum += a.0 * b.1 - b.0 * a.1;
    }
    0.5 * sum
}

fn ensure_cw_yup(pts: &mut [(f64, f64)]) {
    if signed_area_yup(pts) > 0.0 {
        pts.reverse();
    }
}

fn make_circle_cw_yup(cx: f64, cy: f64, r: f64, steps: usize) -> Vec<(f64, f64)> {
    let mut pts = Vec::with_capacity(steps);
    for i in 0..steps {
        let theta = -2.0 * std::f64::consts::PI * (i as f64) / (steps as f64);
        let (s, c) = theta.sin_cos();
        pts.push((cx + r * c, cy + r * s));
    }
    pts
}

fn arc(cx: f64, cy: f64, rx: f64, ry: f64, start_deg: f64, end_deg: f64, steps: usize) -> Vec<(f64, f64)> {
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = (i as f64) / (steps as f64);
        let ang = (start_deg + (end_deg - start_deg) * t).to_radians();
        let (s, c) = ang.sin_cos();
        pts.push((cx + rx * c, cy + ry * s));
    }
    pts
}

fn build_glyph_strokes(ch: char) -> Vec<StrokeItem> {
    if let Some(items) = ascii_upper_strokes(ch) {
        return items;
    }
    if let Some(items) = ascii_lower_strokes(ch) {
        return items;
    }
    if let Some(items) = ascii_digit_punct_strokes(ch) {
        return items;
    }
    if let Some(items) = greek_upper_strokes(ch) {
        return items;
    }
    if let Some(items) = greek_lower_strokes(ch) {
        return items;
    }
    if let Some(items) = math_symbol_strokes(ch) {
        return items;
    }
    // Fallback box for arbitrary unmapped glyphs
    vec![StrokeItem::Ring(vec![(0.12, 0.04), (0.12, 0.68), (0.88, 0.68), (0.88, 0.04)])]
}

fn ascii_upper_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Open, Ring};
    let items = match ch {
        'A' => vec![Open(vec![(0.05, 0.0), (0.50, 0.72), (0.95, 0.0)]), Open(vec![(0.20, 0.24), (0.80, 0.24)])],
        'B' => vec![
            Open(vec![(0.10, 0.0), (0.10, 0.72)]),
            Open(chain(vec![(0.10, 0.72)], arc(0.55, 0.55, 0.35, 0.17, 90.0, -90.0, 8), vec![(0.10, 0.38)])),
            Open(chain(vec![(0.10, 0.38)], arc(0.58, 0.19, 0.36, 0.19, 90.0, -90.0, 8), vec![(0.10, 0.0)])),
        ],
        'C' => vec![Open(arc(0.54, 0.36, 0.44, 0.36, 40.0, 320.0, 14))],
        'D' => vec![Ring(chain(vec![(0.10, 0.0), (0.10, 0.72)], arc(0.48, 0.36, 0.44, 0.36, 90.0, -90.0, 10), vec![]))],
        'E' => vec![Open(vec![(0.90, 0.72), (0.12, 0.72), (0.12, 0.0), (0.90, 0.0)]), Open(vec![(0.12, 0.38), (0.78, 0.38)])],
        'F' => vec![Open(vec![(0.90, 0.72), (0.12, 0.72), (0.12, 0.0)]), Open(vec![(0.12, 0.38), (0.76, 0.38)])],
        'G' => vec![Open(arc(0.54, 0.36, 0.44, 0.36, 40.0, 355.0, 14)), Open(vec![(0.56, 0.36), (0.96, 0.36), (0.96, 0.0)])],
        'H' => vec![Open(vec![(0.12, 0.0), (0.12, 0.72)]), Open(vec![(0.88, 0.0), (0.88, 0.72)]), Open(vec![(0.12, 0.38), (0.88, 0.38)])],
        'I' => vec![Open(vec![(0.50, 0.0), (0.50, 0.72)])],
        'J' => vec![Open(chain(vec![(0.82, 0.72)], arc(0.46, 0.22, 0.36, 0.22, 0.0, -170.0, 8), vec![]))],
        'K' => vec![Open(vec![(0.12, 0.0), (0.12, 0.72)]), Open(vec![(0.88, 0.72), (0.14, 0.34), (0.90, 0.0)])],
        'L' => vec![Open(vec![(0.12, 0.72), (0.12, 0.0), (0.90, 0.0)])],
        'M' => vec![Open(vec![(0.08, 0.0), (0.08, 0.72), (0.50, 0.14), (0.92, 0.72), (0.92, 0.0)])],
        'N' => vec![Open(vec![(0.12, 0.0), (0.12, 0.72), (0.88, 0.0), (0.88, 0.72)])],
        'O' => vec![Ring(arc(0.50, 0.36, 0.44, 0.36, 0.0, 360.0, 16))],
        'P' => vec![
            Open(vec![(0.12, 0.0), (0.12, 0.72)]),
            Open(chain(vec![(0.12, 0.72)], arc(0.56, 0.54, 0.36, 0.18, 90.0, -90.0, 8), vec![(0.12, 0.36)])),
        ],
        'Q' => vec![Ring(arc(0.50, 0.37, 0.44, 0.35, 0.0, 360.0, 16)), Open(vec![(0.58, 0.20), (0.94, -0.06)])],
        'R' => vec![
            Open(vec![(0.12, 0.0), (0.12, 0.72)]),
            Open(chain(vec![(0.12, 0.72)], arc(0.56, 0.54, 0.36, 0.18, 90.0, -90.0, 8), vec![(0.12, 0.36)])),
            Open(vec![(0.52, 0.36), (0.90, 0.0)]),
        ],
        'S' => vec![Open(chain(arc(0.50, 0.54, 0.38, 0.18, 25.0, 265.0, 8), arc(0.50, 0.18, 0.40, 0.18, 90.0, -155.0, 8), vec![]))],
        'T' => vec![Open(vec![(0.05, 0.72), (0.95, 0.72)]), Open(vec![(0.50, 0.72), (0.50, 0.0)])],
        'U' => vec![Open(chain(vec![(0.12, 0.72)], arc(0.50, 0.24, 0.38, 0.24, 180.0, 360.0, 8), vec![(0.88, 0.72)]))],
        'V' => vec![Open(vec![(0.06, 0.72), (0.50, 0.0), (0.94, 0.72)])],
        'W' => vec![Open(vec![(0.04, 0.72), (0.27, 0.0), (0.50, 0.50), (0.73, 0.0), (0.96, 0.72)])],
        'X' => vec![Open(vec![(0.08, 0.72), (0.92, 0.0)]), Open(vec![(0.92, 0.72), (0.08, 0.0)])],
        'Y' => vec![Open(vec![(0.08, 0.72), (0.50, 0.36), (0.92, 0.72)]), Open(vec![(0.50, 0.36), (0.50, 0.0)])],
        'Z' => vec![Open(vec![(0.08, 0.72), (0.92, 0.72), (0.08, 0.0), (0.92, 0.0)])],
        _ => return None,
    };
    Some(items)
}

fn ascii_lower_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Dot, Open, Ring};
    let items = match ch {
        'a' => vec![
            Open(chain(arc(0.50, 0.44, 0.36, 0.10, 150.0, 0.0, 6), vec![(0.86, 0.0)], vec![])),
            Ring(arc(0.48, 0.16, 0.38, 0.16, 0.0, 360.0, 12)),
        ],
        'b' => vec![Open(vec![(0.12, 0.72), (0.12, 0.0)]), Ring(arc(0.52, 0.26, 0.38, 0.26, 0.0, 360.0, 14))],
        'c' => vec![Open(arc(0.54, 0.26, 0.40, 0.26, 40.0, 320.0, 12))],
        'd' => vec![Open(vec![(0.88, 0.72), (0.88, 0.0)]), Ring(arc(0.48, 0.26, 0.38, 0.26, 0.0, 360.0, 14))],
        'e' => vec![Open(vec![(0.12, 0.27), (0.90, 0.27)]), Open(arc(0.51, 0.26, 0.40, 0.26, 0.0, 320.0, 14))],
        'f' => vec![Open(chain(arc(0.76, 0.60, 0.26, 0.12, 30.0, 180.0, 6), vec![(0.50, 0.0)], vec![])), Open(vec![(0.14, 0.48), (0.84, 0.48)])],
        'g' => vec![
            Ring(arc(0.48, 0.28, 0.38, 0.24, 0.0, 360.0, 12)),
            Open(chain(vec![(0.86, 0.52), (0.86, -0.04)], arc(0.48, -0.04, 0.38, 0.16, 0.0, -155.0, 8), vec![])),
        ],
        'h' => vec![Open(vec![(0.12, 0.72), (0.12, 0.0)]), Open(chain(arc(0.50, 0.36, 0.38, 0.16, 170.0, 0.0, 8), vec![(0.88, 0.0)], vec![]))],
        'i' => vec![Dot(0.50, 0.68, 1.15), Open(vec![(0.50, 0.0), (0.50, 0.50)])],
        'j' => vec![Dot(0.60, 0.68, 1.15), Open(chain(vec![(0.60, 0.50), (0.60, -0.04)], arc(0.32, -0.04, 0.28, 0.16, 0.0, -145.0, 6), vec![]))],
        'k' => vec![Open(vec![(0.12, 0.72), (0.12, 0.0)]), Open(vec![(0.88, 0.50), (0.16, 0.24), (0.90, 0.0)])],
        'l' => vec![Open(vec![(0.50, 0.0), (0.50, 0.72)])],
        'm' => vec![
            Open(vec![(0.08, 0.0), (0.08, 0.52)]),
            Open(chain(arc(0.29, 0.38, 0.21, 0.14, 175.0, 0.0, 6), vec![(0.50, 0.0)], vec![])),
            Open(chain(arc(0.71, 0.38, 0.21, 0.14, 175.0, 0.0, 6), vec![(0.92, 0.0)], vec![])),
        ],
        'n' => vec![Open(vec![(0.12, 0.0), (0.12, 0.52)]), Open(chain(arc(0.50, 0.36, 0.38, 0.16, 175.0, 0.0, 8), vec![(0.88, 0.0)], vec![]))],
        'o' => vec![Ring(arc(0.50, 0.26, 0.42, 0.26, 0.0, 360.0, 14))],
        'p' => vec![Open(vec![(0.12, -0.20), (0.12, 0.52)]), Ring(arc(0.52, 0.26, 0.38, 0.26, 0.0, 360.0, 14))],
        'q' => vec![Open(vec![(0.88, -0.20), (0.88, 0.52)]), Ring(arc(0.48, 0.26, 0.38, 0.26, 0.0, 360.0, 14))],
        'r' => vec![Open(vec![(0.20, 0.0), (0.20, 0.52)]), Open(arc(0.62, 0.36, 0.42, 0.16, 175.0, 30.0, 6))],
        's' => vec![Open(chain(arc(0.50, 0.39, 0.36, 0.13, 25.0, 265.0, 7), arc(0.50, 0.13, 0.38, 0.13, 90.0, -155.0, 7), vec![]))],
        't' => vec![
            Open(chain(vec![(0.44, 0.68), (0.44, 0.12)], arc(0.66, 0.12, 0.22, 0.12, 180.0, 300.0, 5), vec![])),
            Open(vec![(0.14, 0.48), (0.86, 0.48)]),
        ],
        'u' => vec![Open(chain(vec![(0.12, 0.52)], arc(0.50, 0.18, 0.38, 0.18, 180.0, 360.0, 8), vec![(0.88, 0.52), (0.88, 0.0)]))],
        'v' => vec![Open(vec![(0.08, 0.52), (0.50, 0.0), (0.92, 0.52)])],
        'w' => vec![Open(vec![(0.06, 0.52), (0.28, 0.0), (0.50, 0.42), (0.72, 0.0), (0.94, 0.52)])],
        'x' => vec![Open(vec![(0.10, 0.52), (0.90, 0.0)]), Open(vec![(0.90, 0.52), (0.10, 0.0)])],
        'y' => vec![Open(vec![(0.10, 0.52), (0.50, 0.0)]), Open(vec![(0.90, 0.52), (0.34, -0.20)])],
        'z' => vec![Open(vec![(0.10, 0.52), (0.90, 0.52), (0.10, 0.0), (0.90, 0.0)])],
        _ => return None,
    };
    Some(items)
}

fn ascii_digit_punct_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Dot, Open, Ring};
    let items = match ch {
        '0' => vec![Ring(arc(0.50, 0.36, 0.40, 0.36, 0.0, 360.0, 16))],
        '1' => vec![Open(vec![(0.24, 0.56), (0.56, 0.72), (0.56, 0.0)]), Open(vec![(0.22, 0.0), (0.84, 0.0)])],
        '2' => vec![Open(chain(arc(0.48, 0.52, 0.38, 0.20, 165.0, -35.0, 9), vec![(0.10, 0.0), (0.90, 0.0)], vec![]))],
        '3' => vec![Open(chain(arc(0.48, 0.54, 0.38, 0.18, 155.0, -90.0, 8), arc(0.50, 0.18, 0.40, 0.18, 90.0, -155.0, 8), vec![]))],
        '4' => vec![Open(vec![(0.72, 0.0), (0.72, 0.72), (0.10, 0.22), (0.92, 0.22)])],
        '5' => vec![Open(chain(vec![(0.86, 0.72), (0.18, 0.72), (0.14, 0.38)], arc(0.50, 0.22, 0.40, 0.22, 130.0, -145.0, 9), vec![]))],
        '6' => vec![Open(arc(0.52, 0.36, 0.40, 0.36, 40.0, 220.0, 10)), Ring(arc(0.50, 0.22, 0.38, 0.22, 0.0, 360.0, 12))],
        '7' => vec![Open(vec![(0.08, 0.72), (0.90, 0.72), (0.34, 0.0)])],
        '8' => vec![Ring(arc(0.50, 0.54, 0.34, 0.18, 0.0, 360.0, 12)), Ring(arc(0.50, 0.18, 0.38, 0.18, 0.0, 360.0, 12))],
        '9' => vec![Ring(arc(0.50, 0.50, 0.38, 0.22, 0.0, 360.0, 12)), Open(arc(0.48, 0.36, 0.40, 0.36, 30.0, -140.0, 10))],
        '!' => vec![Open(vec![(0.50, 0.72), (0.50, 0.20)]), Dot(0.50, 0.04, 1.2)],
        '"' => vec![Open(vec![(0.30, 0.72), (0.30, 0.52)]), Open(vec![(0.70, 0.72), (0.70, 0.52)])],
        '#' => vec![
            Open(vec![(0.32, 0.04), (0.44, 0.68)]),
            Open(vec![(0.62, 0.04), (0.74, 0.68)]),
            Open(vec![(0.12, 0.46), (0.92, 0.46)]),
            Open(vec![(0.08, 0.24), (0.88, 0.24)]),
        ],
        '$' => vec![
            Open(chain(arc(0.50, 0.52, 0.36, 0.16, 25.0, 265.0, 7), arc(0.50, 0.20, 0.38, 0.16, 90.0, -155.0, 7), vec![])),
            Open(vec![(0.50, -0.06), (0.50, 0.78)]),
        ],
        '%' => vec![
            Ring(arc(0.24, 0.56, 0.14, 0.14, 0.0, 360.0, 10)),
            Open(vec![(0.18, 0.0), (0.82, 0.72)]),
            Ring(arc(0.76, 0.16, 0.14, 0.14, 0.0, 360.0, 10)),
        ],
        '&' => vec![
            Ring(arc(0.44, 0.56, 0.20, 0.14, 0.0, 360.0, 10)),
            Open(chain(arc(0.44, 0.20, 0.32, 0.20, 120.0, 330.0, 8), vec![(0.88, 0.34)], vec![])),
            Open(vec![(0.42, 0.42), (0.90, 0.0)]),
        ],
        '\'' => vec![Open(vec![(0.50, 0.72), (0.50, 0.50)])],
        '(' => vec![Open(arc(0.82, 0.30, 0.52, 0.48, 125.0, 235.0, 10))],
        ')' => vec![Open(arc(0.18, 0.30, 0.52, 0.48, -55.0, 55.0, 10))],
        '*' => vec![Open(vec![(0.50, 0.70), (0.50, 0.36)]), Open(vec![(0.22, 0.62), (0.78, 0.44)]), Open(vec![(0.78, 0.62), (0.22, 0.44)])],
        '+' => vec![Open(vec![(0.10, 0.34), (0.90, 0.34)]), Open(vec![(0.50, 0.04), (0.50, 0.64)])],
        ',' => vec![Dot(0.52, 0.06, 1.15), Open(vec![(0.54, 0.04), (0.36, -0.14)])],
        '-' => vec![Open(vec![(0.14, 0.30), (0.86, 0.30)])],
        '.' => vec![Dot(0.50, 0.04, 1.25)],
        '/' => vec![Open(vec![(0.12, -0.06), (0.88, 0.74)])],
        ':' => vec![Dot(0.50, 0.44, 1.2), Dot(0.50, 0.04, 1.2)],
        ';' => vec![Dot(0.50, 0.44, 1.2), Dot(0.50, 0.04, 1.15), Open(vec![(0.52, 0.02), (0.36, -0.14)])],
        '<' => vec![Open(vec![(0.88, 0.56), (0.12, 0.34), (0.88, 0.12)])],
        '=' => vec![Open(vec![(0.10, 0.44), (0.90, 0.44)]), Open(vec![(0.10, 0.24), (0.90, 0.24)])],
        '>' => vec![Open(vec![(0.12, 0.56), (0.88, 0.34), (0.12, 0.12)])],
        '?' => vec![Open(chain(arc(0.48, 0.54, 0.38, 0.18, 165.0, -45.0, 8), vec![(0.50, 0.22)], vec![])), Dot(0.50, 0.04, 1.2)],
        '@' => vec![Ring(arc(0.50, 0.32, 0.22, 0.20, 0.0, 360.0, 10)), Open(arc(0.50, 0.32, 0.42, 0.38, -25.0, 315.0, 14))],
        '[' => vec![Open(vec![(0.80, 0.74), (0.28, 0.74), (0.28, -0.14), (0.80, -0.14)])],
        '\\' => vec![Open(vec![(0.12, 0.74), (0.88, -0.06)])],
        ']' => vec![Open(vec![(0.20, 0.74), (0.72, 0.74), (0.72, -0.14), (0.20, -0.14)])],
        '^' => vec![Open(vec![(0.12, 0.44), (0.50, 0.72), (0.88, 0.44)])],
        '_' => vec![Open(vec![(0.06, -0.12), (0.94, -0.12)])],
        '`' => vec![Open(vec![(0.32, 0.72), (0.64, 0.52)])],
        '{' => vec![Open(vec![(0.80, 0.74), (0.52, 0.70), (0.52, 0.38), (0.24, 0.30), (0.52, 0.22), (0.52, -0.10), (0.80, -0.14)])],
        '|' => vec![Open(vec![(0.50, -0.14), (0.50, 0.74)])],
        '}' => vec![Open(vec![(0.20, 0.74), (0.48, 0.70), (0.48, 0.38), (0.76, 0.30), (0.48, 0.22), (0.48, -0.10), (0.20, -0.14)])],
        '~' => vec![Open(vec![(0.08, 0.32), (0.30, 0.44), (0.70, 0.24), (0.92, 0.36)])],
        _ => return None,
    };
    Some(items)
}

fn greek_upper_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Open, Ring};
    let items = match ch {
        '\u{0391}' => ascii_upper_strokes('A')?,
        '\u{0392}' => ascii_upper_strokes('B')?,
        '\u{0393}' => vec![Open(vec![(0.14, 0.0), (0.14, 0.72), (0.92, 0.72)])],
        '\u{0394}' => vec![Ring(vec![(0.06, 0.0), (0.50, 0.72), (0.94, 0.0)])],
        '\u{0395}' => ascii_upper_strokes('E')?,
        '\u{0396}' => ascii_upper_strokes('Z')?,
        '\u{0397}' => ascii_upper_strokes('H')?,
        '\u{0398}' => vec![Ring(arc(0.50, 0.36, 0.44, 0.36, 0.0, 360.0, 16)), Open(vec![(0.24, 0.36), (0.76, 0.36)])],
        '\u{0399}' => ascii_upper_strokes('I')?,
        '\u{039A}' => ascii_upper_strokes('K')?,
        '\u{039B}' => vec![Open(vec![(0.06, 0.0), (0.50, 0.72), (0.94, 0.0)])],
        '\u{039C}' => ascii_upper_strokes('M')?,
        '\u{039D}' => ascii_upper_strokes('N')?,
        '\u{039E}' => vec![Open(vec![(0.10, 0.72), (0.90, 0.72)]), Open(vec![(0.22, 0.38), (0.78, 0.38)]), Open(vec![(0.10, 0.0), (0.90, 0.0)])],
        '\u{039F}' => ascii_upper_strokes('O')?,
        '\u{03A0}' => vec![Open(vec![(0.12, 0.0), (0.12, 0.72), (0.88, 0.72), (0.88, 0.0)])],
        '\u{03A1}' => ascii_upper_strokes('P')?,
        '\u{03A3}' => vec![Open(vec![(0.90, 0.72), (0.10, 0.72), (0.54, 0.36), (0.10, 0.0), (0.90, 0.0)])],
        '\u{03A4}' => ascii_upper_strokes('T')?,
        '\u{03A5}' => ascii_upper_strokes('Y')?,
        '\u{03A6}' => vec![Ring(arc(0.50, 0.36, 0.42, 0.24, 0.0, 360.0, 14)), Open(vec![(0.50, 0.0), (0.50, 0.72)])],
        '\u{03A7}' => ascii_upper_strokes('X')?,
        '\u{03A8}' => vec![
            Open(vec![(0.50, 0.0), (0.50, 0.72)]),
            Open(chain(vec![(0.12, 0.64)], arc(0.50, 0.36, 0.38, 0.18, 180.0, 360.0, 8), vec![(0.88, 0.64)])),
        ],
        '\u{03A9}' => {
            vec![Open(chain(vec![(0.08, 0.0), (0.34, 0.0)], arc(0.50, 0.40, 0.40, 0.32, 235.0, -55.0, 12), vec![(0.66, 0.0), (0.92, 0.0)]))]
        }
        _ => return None,
    };
    Some(items)
}

fn greek_lower_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Open, Ring};
    let items = match ch {
        '\u{03B1}' => vec![Open(chain(vec![(0.90, 0.50)], arc(0.42, 0.26, 0.34, 0.24, -40.0, -320.0, 12), vec![(0.92, 0.02)]))],
        '\u{03B2}' => vec![
            Open(chain(vec![(0.14, -0.20), (0.14, 0.56)], arc(0.48, 0.56, 0.34, 0.16, 175.0, -90.0, 8), vec![(0.28, 0.40)])),
            Open(chain(vec![(0.28, 0.40)], arc(0.52, 0.20, 0.38, 0.20, 90.0, -140.0, 8), vec![(0.16, 0.04)])),
        ],
        '\u{03B3}' => vec![Open(vec![(0.10, 0.50), (0.52, -0.18), (0.90, 0.50)])],
        '\u{03B4}' => vec![Ring(arc(0.50, 0.24, 0.38, 0.24, 0.0, 360.0, 12)), Open(vec![(0.52, 0.48), (0.24, 0.66), (0.76, 0.72)])],
        '\u{03B5}' | '\u{03F5}' => vec![Open(arc(0.52, 0.39, 0.36, 0.13, 30.0, 265.0, 8)), Open(arc(0.52, 0.13, 0.38, 0.13, 95.0, 330.0, 8))],
        '\u{03B6}' => vec![Open(vec![(0.20, 0.72), (0.82, 0.72), (0.16, 0.12), (0.80, 0.04), (0.58, -0.20)])],
        '\u{03B7}' => {
            vec![Open(vec![(0.14, 0.0), (0.14, 0.52)]), Open(chain(arc(0.50, 0.36, 0.36, 0.16, 175.0, 0.0, 8), vec![(0.86, -0.20)], vec![]))]
        }
        '\u{03B8}' | '\u{03D1}' => vec![Ring(arc(0.50, 0.36, 0.38, 0.36, 0.0, 360.0, 14)), Open(vec![(0.14, 0.36), (0.86, 0.36)])],
        '\u{03B9}' => vec![Open(vec![(0.50, 0.52), (0.50, 0.06), (0.74, 0.0)])],
        '\u{03BA}' | '\u{03F0}' => ascii_lower_strokes('k')?,
        '\u{03BB}' => vec![Open(vec![(0.12, 0.72), (0.34, 0.72), (0.88, 0.0)]), Open(vec![(0.50, 0.44), (0.12, 0.0)])],
        '\u{03BC}' | '\u{00B5}' => vec![
            Open(vec![(0.14, -0.20), (0.14, 0.52)]),
            Open(chain(vec![(0.14, 0.18)], arc(0.48, 0.18, 0.34, 0.18, 180.0, 360.0, 8), vec![(0.82, 0.52), (0.82, 0.0)])),
        ],
        '\u{03BD}' => ascii_lower_strokes('v')?,
        '\u{03BE}' => vec![
            Open(vec![(0.24, 0.72), (0.78, 0.72)]),
            Open(arc(0.50, 0.52, 0.32, 0.16, 110.0, 265.0, 6)),
            Open(chain(arc(0.50, 0.22, 0.34, 0.18, 110.0, 270.0, 6), vec![(0.76, -0.06), (0.56, -0.20)], vec![])),
        ],
        '\u{03BF}' => ascii_lower_strokes('o')?,
        '\u{03C0}' | '\u{03D6}' => {
            vec![Open(vec![(0.06, 0.50), (0.94, 0.50)]), Open(vec![(0.30, 0.50), (0.30, 0.0)]), Open(vec![(0.70, 0.50), (0.70, 0.0)])]
        }
        '\u{03C1}' | '\u{03F1}' => vec![Open(vec![(0.16, -0.20), (0.16, 0.28)]), Ring(arc(0.52, 0.28, 0.36, 0.24, 0.0, 360.0, 12))],
        '\u{03C2}' => vec![Open(chain(arc(0.50, 0.28, 0.36, 0.24, 30.0, 270.0, 10), vec![(0.76, -0.08), (0.52, -0.20)], vec![]))],
        '\u{03C3}' => vec![Ring(arc(0.44, 0.26, 0.36, 0.26, 0.0, 360.0, 12)), Open(vec![(0.44, 0.52), (0.94, 0.52)])],
        '\u{03C4}' => vec![Open(vec![(0.08, 0.50), (0.92, 0.50)]), Open(vec![(0.50, 0.50), (0.50, 0.04), (0.74, 0.0)])],
        '\u{03C5}' => vec![Open(chain(vec![(0.14, 0.52)], arc(0.50, 0.20, 0.36, 0.20, 180.0, 360.0, 8), vec![(0.86, 0.52)]))],
        '\u{03C6}' | '\u{03D5}' => vec![Ring(arc(0.50, 0.28, 0.40, 0.22, 0.0, 360.0, 12)), Open(vec![(0.50, -0.20), (0.50, 0.60)])],
        '\u{03C7}' => vec![Open(vec![(0.12, 0.52), (0.88, -0.20)]), Open(vec![(0.88, 0.52), (0.12, -0.20)])],
        '\u{03C8}' => vec![
            Open(vec![(0.50, -0.20), (0.50, 0.56)]),
            Open(chain(vec![(0.12, 0.52)], arc(0.50, 0.24, 0.38, 0.20, 180.0, 360.0, 8), vec![(0.88, 0.52)])),
        ],
        '\u{03C9}' => vec![Open(chain(arc(0.30, 0.24, 0.22, 0.24, 130.0, 360.0, 7), arc(0.70, 0.24, 0.22, 0.24, 180.0, 410.0, 7), vec![]))],
        _ => return None,
    };
    Some(items)
}

fn math_symbol_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Dot, Open, Ring, Solid};
    let items = match ch {
        '\u{2212}' | '\u{2013}' => vec![Open(vec![(0.10, 0.34), (0.90, 0.34)])],
        '\u{2014}' => vec![Open(vec![(0.02, 0.34), (0.98, 0.34)])],
        '\u{00B1}' => vec![Open(vec![(0.12, 0.44), (0.88, 0.44)]), Open(vec![(0.50, 0.18), (0.50, 0.70)]), Open(vec![(0.12, 0.06), (0.88, 0.06)])],
        '\u{2213}' => vec![Open(vec![(0.12, 0.64), (0.88, 0.64)]), Open(vec![(0.12, 0.28), (0.88, 0.28)]), Open(vec![(0.50, 0.02), (0.50, 0.54)])],
        '\u{00D7}' => vec![Open(vec![(0.14, 0.58), (0.86, 0.10)]), Open(vec![(0.86, 0.58), (0.14, 0.10)])],
        '\u{00F7}' => vec![Open(vec![(0.10, 0.34), (0.90, 0.34)]), Dot(0.50, 0.58, 1.25), Dot(0.50, 0.10, 1.25)],
        '\u{00B7}' => vec![Dot(0.50, 0.34, 1.3)],
        '\u{2022}' => vec![Dot(0.50, 0.34, 2.2)],
        '\u{2264}' => vec![Open(vec![(0.86, 0.66), (0.14, 0.42), (0.86, 0.18)]), Open(vec![(0.14, 0.06), (0.86, 0.06)])],
        '\u{2265}' => vec![Open(vec![(0.14, 0.66), (0.86, 0.42), (0.14, 0.18)]), Open(vec![(0.14, 0.06), (0.86, 0.06)])],
        '\u{2260}' => vec![Open(vec![(0.10, 0.44), (0.90, 0.44)]), Open(vec![(0.10, 0.24), (0.90, 0.24)]), Open(vec![(0.24, 0.02), (0.76, 0.68)])],
        '\u{2248}' => vec![
            Open(vec![(0.08, 0.44), (0.30, 0.54), (0.70, 0.34), (0.92, 0.44)]),
            Open(vec![(0.08, 0.20), (0.30, 0.30), (0.70, 0.10), (0.92, 0.20)]),
        ],
        '\u{2261}' => vec![Open(vec![(0.10, 0.54), (0.90, 0.54)]), Open(vec![(0.10, 0.34), (0.90, 0.34)]), Open(vec![(0.10, 0.14), (0.90, 0.14)])],
        '\u{223C}' => vec![Open(vec![(0.08, 0.32), (0.30, 0.44), (0.70, 0.22), (0.92, 0.34)])],
        '\u{221D}' => {
            vec![Open(chain(arc(0.32, 0.34, 0.24, 0.18, 45.0, 315.0, 10), vec![(0.92, 0.50)], vec![])), Open(vec![(0.46, 0.22), (0.92, 0.18)])]
        }
        '\u{226A}' => vec![Open(vec![(0.54, 0.56), (0.08, 0.34), (0.54, 0.12)]), Open(vec![(0.92, 0.56), (0.46, 0.34), (0.92, 0.12)])],
        '\u{226B}' => vec![Open(vec![(0.08, 0.56), (0.54, 0.34), (0.08, 0.12)]), Open(vec![(0.46, 0.56), (0.92, 0.34), (0.46, 0.12)])],
        '\u{221E}' => vec![Ring(arc(0.27, 0.34, 0.22, 0.18, 0.0, 360.0, 10)), Ring(arc(0.73, 0.34, 0.22, 0.18, 0.0, 360.0, 10))],
        '\u{00B0}' | '\u{2218}' => vec![Ring(arc(0.50, 0.58, 0.28, 0.12, 0.0, 360.0, 10))],
        '\u{00C5}' => vec![
            Open(vec![(0.06, 0.0), (0.50, 0.68), (0.94, 0.0)]),
            Open(vec![(0.22, 0.22), (0.78, 0.22)]),
            Ring(arc(0.50, 0.78, 0.14, 0.07, 0.0, 360.0, 8)),
        ],
        '\u{2032}' => vec![Open(vec![(0.38, 0.42), (0.64, 0.72)])],
        '\u{2033}' => vec![Open(vec![(0.22, 0.42), (0.44, 0.72)]), Open(vec![(0.58, 0.42), (0.80, 0.72)])],
        '\u{2030}' => vec![
            Ring(arc(0.20, 0.56, 0.12, 0.12, 0.0, 360.0, 8)),
            Open(vec![(0.14, 0.0), (0.66, 0.72)]),
            Ring(arc(0.54, 0.16, 0.11, 0.12, 0.0, 360.0, 8)),
            Ring(arc(0.84, 0.16, 0.11, 0.12, 0.0, 360.0, 8)),
        ],
        '\u{2026}' => vec![Dot(0.18, 0.04, 1.15), Dot(0.50, 0.04, 1.15), Dot(0.82, 0.04, 1.15)],
        '\u{2202}' => vec![Ring(arc(0.50, 0.24, 0.36, 0.24, 0.0, 360.0, 12)), Open(arc(0.50, 0.44, 0.36, 0.28, -10.0, 135.0, 8))],
        '\u{2207}' => vec![Ring(vec![(0.06, 0.70), (0.94, 0.70), (0.50, 0.0)])],
        '\u{2211}' => vec![Open(vec![(0.90, 0.72), (0.10, 0.72), (0.54, 0.36), (0.10, 0.0), (0.90, 0.0)])],
        '\u{220F}' => vec![Open(vec![(0.12, 0.0), (0.12, 0.72), (0.88, 0.72), (0.88, 0.0)])],
        '\u{222B}' => {
            vec![Open(chain(arc(0.72, 0.64, 0.24, 0.12, 30.0, 180.0, 5), vec![(0.48, -0.06)], arc(0.28, -0.06, 0.20, 0.12, 0.0, -150.0, 5)))]
        }
        '\u{221A}' => vec![Open(vec![(0.06, 0.34), (0.28, 0.0), (0.58, 0.72), (0.94, 0.72)])],
        '\u{2192}' => vec![Open(vec![(0.06, 0.34), (0.92, 0.34)]), Open(vec![(0.66, 0.54), (0.92, 0.34), (0.66, 0.14)])],
        '\u{2190}' => vec![Open(vec![(0.08, 0.34), (0.94, 0.34)]), Open(vec![(0.34, 0.54), (0.08, 0.34), (0.34, 0.14)])],
        '\u{2194}' => vec![
            Open(vec![(0.08, 0.34), (0.92, 0.34)]),
            Open(vec![(0.30, 0.52), (0.08, 0.34), (0.30, 0.16)]),
            Open(vec![(0.70, 0.52), (0.92, 0.34), (0.70, 0.16)]),
        ],
        '\u{2191}' => vec![Open(vec![(0.50, 0.0), (0.50, 0.72)]), Open(vec![(0.22, 0.48), (0.50, 0.72), (0.78, 0.48)])],
        '\u{2193}' => vec![Open(vec![(0.50, 0.72), (0.50, 0.0)]), Open(vec![(0.22, 0.24), (0.50, 0.0), (0.78, 0.24)])],
        '\u{2200}' => vec![Open(vec![(0.08, 0.72), (0.50, 0.0), (0.92, 0.72)]), Open(vec![(0.26, 0.42), (0.74, 0.42)])],
        '\u{2203}' => vec![Open(vec![(0.14, 0.72), (0.86, 0.72), (0.86, 0.0), (0.14, 0.0)]), Open(vec![(0.24, 0.36), (0.86, 0.36)])],
        '\u{2208}' => vec![Open(arc(0.54, 0.36, 0.40, 0.30, 60.0, 300.0, 10)), Open(vec![(0.14, 0.36), (0.84, 0.36)])],
        '\u{2209}' => {
            vec![Open(arc(0.54, 0.36, 0.40, 0.30, 60.0, 300.0, 10)), Open(vec![(0.14, 0.36), (0.84, 0.36)]), Open(vec![(0.24, 0.0), (0.78, 0.72)])]
        }
        '\u{2282}' => vec![Open(arc(0.54, 0.36, 0.40, 0.30, 60.0, 300.0, 10))],
        '\u{2286}' => vec![Open(arc(0.54, 0.42, 0.40, 0.26, 60.0, 300.0, 10)), Open(vec![(0.16, 0.04), (0.88, 0.04)])],
        '\u{222A}' => vec![Open(chain(vec![(0.14, 0.68)], arc(0.50, 0.26, 0.36, 0.24, 180.0, 360.0, 8), vec![(0.86, 0.68)]))],
        '\u{2229}' => vec![Open(chain(vec![(0.14, 0.0)], arc(0.50, 0.44, 0.36, 0.24, 180.0, 0.0, 8), vec![(0.86, 0.0)]))],
        '\u{2205}' => vec![Ring(arc(0.50, 0.36, 0.38, 0.32, 0.0, 360.0, 14)), Open(vec![(0.14, 0.0), (0.86, 0.72)])],
        '\u{2234}' => vec![Dot(0.50, 0.56, 1.25), Dot(0.20, 0.12, 1.25), Dot(0.80, 0.12, 1.25)],
        '\u{2020}' => vec![Open(vec![(0.50, -0.08), (0.50, 0.72)]), Open(vec![(0.14, 0.48), (0.86, 0.48)])],
        '\u{2021}' => vec![Open(vec![(0.50, -0.08), (0.50, 0.72)]), Open(vec![(0.14, 0.52), (0.86, 0.52)]), Open(vec![(0.14, 0.20), (0.86, 0.20)])],
        '\u{00A7}' => vec![Open(chain(arc(0.50, 0.56, 0.32, 0.16, 30.0, 250.0, 8), arc(0.50, 0.16, 0.32, 0.16, 90.0, -150.0, 8), vec![]))],
        '\u{00A9}' => vec![Ring(arc(0.50, 0.36, 0.42, 0.36, 0.0, 360.0, 14)), Open(arc(0.54, 0.36, 0.20, 0.18, 45.0, 315.0, 8))],
        '\u{00AE}' => vec![
            Ring(arc(0.50, 0.36, 0.42, 0.36, 0.0, 360.0, 14)),
            Open(vec![(0.36, 0.18), (0.36, 0.54), (0.62, 0.54), (0.62, 0.36), (0.36, 0.36), (0.64, 0.18)]),
        ],
        '\u{2122}' => vec![
            Open(vec![(0.06, 0.72), (0.44, 0.72)]),
            Open(vec![(0.25, 0.72), (0.25, 0.40)]),
            Open(vec![(0.54, 0.40), (0.54, 0.72), (0.74, 0.50), (0.94, 0.72), (0.94, 0.40)]),
        ],
        '\u{00B9}' => vec![Open(vec![(0.32, 0.58), (0.54, 0.72), (0.54, 0.32)])],
        '\u{00B2}' => vec![Open(chain(arc(0.50, 0.60, 0.28, 0.12, 165.0, -30.0, 7), vec![(0.22, 0.32), (0.80, 0.32)], vec![]))],
        '\u{00B3}' => vec![Open(chain(arc(0.48, 0.60, 0.26, 0.11, 150.0, -90.0, 6), arc(0.50, 0.42, 0.28, 0.11, 90.0, -150.0, 6), vec![]))],
        '\u{2070}' => vec![Ring(arc(0.50, 0.52, 0.28, 0.20, 0.0, 360.0, 10))],
        '\u{25CF}' => vec![Solid(make_circle_cw_yup(0.50, 0.36, 0.38, 14))],
        _ => return None,
    };
    Some(items)
}

fn chain(mut a: Vec<(f64, f64)>, b: Vec<(f64, f64)>, c: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    a.extend(b);
    a.extend(c);
    a
}
