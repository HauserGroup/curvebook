//! Clean-room glyph vector path outline generator and TrueType `glyf` contour decoder for
//! SVG, PDF, and CPU raster export backends.
//!
//! All emitted contours in [`PositionedGlyphOutline::commands`] are **closed filled outlines**
//! in Y-down PostScript point coordinates (`ascenders < origin_y_pt`, `descenders > origin_y_pt`)
//! oriented for the nonzero winding fill rule (positive area for outer contours, negative area
//! for inner holes).

use std::borrow::Cow;
use std::cell::Cell;

use curvebook_palette::Rgba8;
use read_fonts::model::Error as FontModelError;
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cff::blend::BlendState;
use read_fonts::ps::cff::index::Index;
use read_fonts::ps::charstring::{self, CharstringContext, CharstringKind, CommandSink};
use read_fonts::tables::glyf::{Glyf, Glyph};
use read_fonts::tables::loca::Loca;
use read_fonts::types::{Fixed, GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};
use serde::{Deserialize, Serialize};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::pen::PathStyle;
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlineGlyphFormat, OutlinePen};

use crate::markup::{RichSpan, RichText, SpanSlope, SpanWeight};
use crate::metrics::{
    TrueTypeFont, char_advance_em_with_font, core14_advance_em, effective_span_scale, is_combining_mark, reorder_rich_text_bidi, rich_contains_rtl,
    script_baseline_offset_pt,
};
use crate::shaping::{FontFamilyKind, SystemFontSet};

const MAX_TTF_CONTOURS: usize = 256;
const MAX_TTF_POINTS: usize = 4096;
const MAX_TTF_TOTAL_POINTS: usize = 16_384;
const MAX_TTF_TOTAL_COMPONENTS: usize = 256;
const MAX_TTF_COMPONENT_DEPTH: usize = 8;
const MAX_CFF_SUBR_CALLS: usize = 1024;
const MAX_CFF_TOTAL_BYTECODE: usize = 65_536;

#[derive(Debug)]
struct GlyfBudgetState {
    remaining_components: usize,
    remaining_points: usize,
    active_gids: [u16; MAX_TTF_COMPONENT_DEPTH],
}

impl GlyfBudgetState {
    const fn new() -> Self {
        Self { remaining_components: MAX_TTF_TOTAL_COMPONENTS, remaining_points: MAX_TTF_TOTAL_POINTS, active_gids: [0; MAX_TTF_COMPONENT_DEPTH] }
    }

    fn consume_component(&mut self) -> Option<()> {
        self.remaining_components = self.remaining_components.checked_sub(1)?;
        Some(())
    }

    fn consume_points(&mut self, points: usize) -> Option<()> {
        self.remaining_points = self.remaining_points.checked_sub(points)?;
        Some(())
    }
}

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
/// Works identically with or without the `bundled-fonts` feature enabled; queries compiled
/// [`crate::BUNDLED_FONTS`], runtime `CURVEBOOK_FONTS_DIR`, cleanroom scientific vector strokes, and
/// host [`SystemFontSet`] TrueType faces before falling back to placeholder boxes.
#[must_use]
pub fn outline_rich_text(rich: &RichText, origin_x_pt: f64, origin_y_pt: f64, font_size_pt: f64) -> Vec<PositionedGlyphOutline> {
    outline_rich_text_rotated(rich, origin_x_pt, origin_y_pt, font_size_pt, 0.0)
}

/// Position [`RichText`] runs starting at `(origin_x_pt, origin_y_pt)` using caller-supplied
/// [`SystemFontSet`] faces for SFNT outlining and script fallback.
#[must_use]
pub fn outline_rich_text_with_fonts(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    fonts: &SystemFontSet,
) -> Vec<PositionedGlyphOutline> {
    outline_rich_text_rotated_with_fonts(rich, origin_x_pt, origin_y_pt, font_size_pt, 0.0, fonts)
}

/// Position [`RichText`] runs starting at `(origin_x_pt, origin_y_pt)` rotated by `angle_deg`
/// clockwise around `(origin_x_pt, origin_y_pt)` in Y-down PostScript points, applying BiDi
/// visual reordering automatically when RTL scripts are present.
#[must_use]
pub fn outline_rich_text_rotated(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    angle_deg: f64,
) -> Vec<PositionedGlyphOutline> {
    outline_rich_text_rotated_with_fonts(rich, origin_x_pt, origin_y_pt, font_size_pt, angle_deg, SystemFontSet::global())
}

/// Position [`RichText`] runs rotated by `angle_deg` using `fonts` for SFNT glyph outlining
/// and non-Latin script fallback, applying BiDi visual reordering automatically.
#[must_use]
pub fn outline_rich_text_rotated_with_fonts(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    angle_deg: f64,
    fonts: &SystemFontSet,
) -> Vec<PositionedGlyphOutline> {
    let visual: Cow<'_, RichText> = if rich_contains_rtl(rich) { Cow::Owned(reorder_rich_text_bidi(rich)) } else { Cow::Borrowed(rich) };
    outline_visual_rich_text_rotated_with_fonts(&visual, origin_x_pt, origin_y_pt, font_size_pt, angle_deg, fonts)
}

/// Position an already visually ordered [`RichText`] starting at `(origin_x_pt, origin_y_pt)`
/// rotated by `angle_deg` clockwise around `(origin_x_pt, origin_y_pt)` without re-running BiDi.
#[must_use]
pub fn outline_visual_rich_text_rotated(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    angle_deg: f64,
) -> Vec<PositionedGlyphOutline> {
    outline_visual_rich_text_rotated_with_fonts(rich, origin_x_pt, origin_y_pt, font_size_pt, angle_deg, SystemFontSet::global())
}

/// Position an already visually ordered [`RichText`] rotated by `angle_deg` using `fonts`
/// without re-running BiDi.
#[must_use]
pub fn outline_visual_rich_text_rotated_with_fonts(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    angle_deg: f64,
    fonts: &SystemFontSet,
) -> Vec<PositionedGlyphOutline> {
    outline_visual_rich_text_rotated_with_fonts_and_ttf(rich, origin_x_pt, origin_y_pt, font_size_pt, angle_deg, fonts, None)
}

/// Position an already visually ordered [`RichText`] rotated by `angle_deg`, preferring
/// `preferred_ttf` first (when supplied), then `fonts` primary tiers, cleanroom vector strokes,
/// and `fonts` host fallback faces without re-running BiDi.
#[must_use]
pub fn outline_visual_rich_text_rotated_with_fonts_and_ttf(
    rich: &RichText,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    angle_deg: f64,
    fonts: &SystemFontSet,
    preferred_ttf: Option<&TrueTypeFont<'_>>,
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
    let mut last_base: Option<BaseGlyphPlacement> = None;

    for span in &rich.spans {
        let span_scale = effective_span_scale(span);
        let glyph_size = base_size * span_scale;
        let y_offset = script_baseline_offset_pt(span.script, base_size);
        let family = font_family_for_span(span);

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
                last_base = None;
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

            let glyph_origin_y = line_y + y_offset;
            if is_combining_mark(ch) {
                let placement = GlyphPlacement { pen_x, origin_y: glyph_origin_y, size_pt: glyph_size, family };
                let (mut commands, anchor_x, anchor_y) =
                    resolve_combining_mark_outline(ch, placement, span, fonts, preferred_ttf, last_base.as_mut());
                if rotate_active && !commands.is_empty() {
                    rotate_commands(&mut commands, origin_x_pt, origin_y_pt, cos_a, sin_a);
                }
                let (rot_x, rot_y) = rotate_point(anchor_x, anchor_y, origin_x_pt, origin_y_pt, cos_a, sin_a, rotate_active);
                out.push(PositionedGlyphOutline {
                    codepoint: ch,
                    origin_x_pt: rot_x,
                    origin_y_pt: rot_y,
                    advance_x_pt: 0.0,
                    color: span.color,
                    commands,
                });
                continue;
            }

            let (mut commands, adv_em) = if ch.is_whitespace() {
                last_base = None;
                let adv_em = resolve_whitespace_advance_em(ch, span.weight, span.slope, family, fonts, preferred_ttf);
                (Vec::new(), adv_em)
            } else {
                let (cmds, adv_em) =
                    build_glyph_commands_with_fonts(ch, pen_x, glyph_origin_y, glyph_size, span.weight, span.slope, family, fonts, preferred_ttf);
                last_base = Some(BaseGlyphPlacement {
                    pen_x,
                    origin_y: glyph_origin_y,
                    glyph_size,
                    adv_em: adv_em.max(0.22),
                    tall_top: is_tall_base(ch),
                    above_stack: 0,
                    below_stack: 0,
                });
                (cmds, adv_em)
            };
            let advance = glyph_size * adv_em;

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

#[derive(Clone, Copy)]
struct GlyphPlacement {
    pen_x: f64,
    origin_y: f64,
    size_pt: f64,
    family: FontFamilyKind,
}

fn resolve_combining_mark_outline(
    ch: char,
    placement: GlyphPlacement,
    span: &RichSpan,
    fonts: &SystemFontSet,
    preferred_ttf: Option<&TrueTypeFont<'_>>,
    last_base: Option<&mut BaseGlyphPlacement>,
) -> (Vec<GlyphOutlineCmd>, f64, f64) {
    let (cr_x, cr_y, cr_size, cr_adv, y_shift) = if let Some(base) = last_base {
        let y_shift = if is_below_combining_mark(ch) {
            let k = base.below_stack;
            base.below_stack = base.below_stack.saturating_add(1);
            -f64::from(k) * 0.14
        } else {
            let k = base.above_stack;
            base.above_stack = base.above_stack.saturating_add(1);
            (if base.tall_top { 0.20 } else { 0.0 }) + f64::from(k) * 0.16
        };
        (base.pen_x, base.origin_y, base.glyph_size, base.adv_em, y_shift)
    } else {
        (placement.pen_x, placement.origin_y, placement.size_pt, 0.36, 0.0)
    };
    if let Some(ttf) = preferred_ttf
        && let Some(cmds) = outline_ttf_glyph(ttf, ch, placement.pen_x, placement.origin_y, placement.size_pt, span.slope == SpanSlope::Italic)
        && !cmds.is_empty()
    {
        return (cmds, placement.pen_x, placement.origin_y);
    }
    if let Some((cmds, _)) =
        fonts.outline_char(placement.family, span.weight, span.slope, ch, placement.pen_x, placement.origin_y, placement.size_pt, true)
    {
        return (cmds, placement.pen_x, placement.origin_y);
    }
    if !has_cleanroom_vector_outline(ch)
        && let Some((cmds, _)) =
            fonts.outline_char(placement.family, span.weight, span.slope, ch, placement.pen_x, placement.origin_y, placement.size_pt, false)
    {
        return (cmds, placement.pen_x, placement.origin_y);
    }
    let cmds = cleanroom_vector_outline_with_y_shift(ch, cr_x, cr_y, cr_size, cr_adv, span.weight, span.slope, y_shift);
    (cmds, cr_x, cr_y)
}

#[derive(Clone, Copy)]
struct BaseGlyphPlacement {
    pen_x: f64,
    origin_y: f64,
    glyph_size: f64,
    adv_em: f64,
    tall_top: bool,
    above_stack: u8,
    below_stack: u8,
}

const fn is_tall_base(ch: char) -> bool {
    matches!(ch, 'A'..='Z' | '0'..='9' | 'b' | 'd' | 'f' | 'h' | 'i' | 'j' | 'k' | 'l' | 't')
        || matches!(ch as u32, 0x00C0..=0x00D6 | 0x00D8..=0x00DF | 0x0130 | 0x0132 | 0x013F | 0x0141 | 0x0152 | 0x017F)
        || (matches!(ch as u32, 0x0100..=0x017E) && (ch as u32).is_multiple_of(2))
}

const fn is_below_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0316..=0x0333 | 0x0339..=0x033C | 0x0345 | 0x0347..=0x0349 | 0x034D | 0x034E | 0x0353..=0x0356 | 0x0359 | 0x035A | 0x035C | 0x0362
    )
}

/// Decode a single glyph's SFNT outline (`glyf`, `CFF`, or `CFF2`) from `ttf` into Y-down `GlyphOutlineCmd`s.
/// Returns `None` if `ch` is not mapped in `cmap`, `ttf` lacks outline tables, or decoding
/// exceeds depth, cycle, component, or point budgets.
#[must_use]
pub fn outline_ttf_glyph(
    ttf: &TrueTypeFont<'_>,
    ch: char,
    origin_x_pt: f64,
    origin_y_pt: f64,
    font_size_pt: f64,
    italic_slant: bool,
) -> Option<Vec<GlyphOutlineCmd>> {
    if !origin_x_pt.is_finite() || !origin_y_pt.is_finite() || !font_size_pt.is_finite() || font_size_pt <= 0.0 {
        return None;
    }
    let gid = ttf.glyph_id(ch)?;
    let upem = f64::from(ttf.units_per_em());
    if upem <= 0.0 {
        return None;
    }
    let font_ref = ttf.font_ref();
    let outlines = OutlineGlyphCollection::new(font_ref);
    let format = outlines.format()?;
    match format {
        OutlineGlyphFormat::Glyf => {
            validate_glyf_budgets(font_ref, gid, ttf.index_to_loc_format() == 1)?;
        }
        OutlineGlyphFormat::Cff => {
            validate_cff_budgets(font_ref, gid, ttf.units_per_em(), false)?;
        }
        OutlineGlyphFormat::Cff2 => {
            validate_cff_budgets(font_ref, gid, ttf.units_per_em(), true)?;
        }
        _ => {
            if ttf.has_glyf_outlines() {
                validate_glyf_budgets(font_ref, gid, ttf.index_to_loc_format() == 1)?;
            }
        }
    }
    let outline_glyph = outlines.get(GlyphId::from(gid))?;
    let mut cmds = Vec::new();
    let mut pen = GlyphOutlinePen {
        origin_x: origin_x_pt,
        origin_y: origin_y_pt,
        scale: font_size_pt / upem,
        slant: if italic_slant { 0.2125 } else { 0.0 },
        contour_start: None,
        last_was_line_to: false,
        contours: 0,
        segments: 0,
        failed: false,
        out: &mut cmds,
    };
    let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default()).with_path_style(PathStyle::HarfBuzz);
    outline_glyph.draw(settings, &mut pen).ok()?;
    if pen.failed {
        return None;
    }
    Some(cmds)
}

fn font_family_for_span(span: &RichSpan) -> FontFamilyKind {
    let Some(raw) = span.font_family.as_deref() else {
        return FontFamilyKind::UiSans;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.contains("mono") || lower.contains("courier") || lower.contains("consola") || lower.contains("menlo") || lower.contains("monaco") {
        FontFamilyKind::UiMono
    } else if lower.contains("serif") || lower.contains("times") || lower.contains("georgia") || lower.contains("cambria") {
        FontFamilyKind::ScientificSerif
    } else {
        FontFamilyKind::UiSans
    }
}

fn resolve_whitespace_advance_em(
    ch: char,
    weight: SpanWeight,
    slope: SpanSlope,
    family: FontFamilyKind,
    fonts: &SystemFontSet,
    preferred_ttf: Option<&TrueTypeFont<'_>>,
) -> f64 {
    if ch == '\t' {
        return 4.0 * resolve_whitespace_advance_em(' ', weight, slope, family, fonts, preferred_ttf);
    }
    if let Some(ttf) = preferred_ttf
        && let Some(adv) = ttf.advance_em(ch)
    {
        return adv;
    }
    if let Some(adv) = fonts.primary_tier_advance_em(family, weight, slope, ch) {
        return adv;
    }
    core14_advance_em(ch, weight, slope)
}

#[allow(clippy::too_many_arguments)]
fn build_glyph_commands_with_fonts(
    ch: char,
    pen_x: f64,
    origin_y: f64,
    glyph_size: f64,
    weight: SpanWeight,
    slope: SpanSlope,
    family: FontFamilyKind,
    fonts: &SystemFontSet,
    preferred_ttf: Option<&TrueTypeFont<'_>>,
) -> (Vec<GlyphOutlineCmd>, f64) {
    if let Some(ttf) = preferred_ttf
        && let Some(cmds) = outline_ttf_glyph(ttf, ch, pen_x, origin_y, glyph_size, slope == SpanSlope::Italic)
        && (!cmds.is_empty() || ch.is_control())
    {
        let adv_em = ttf.advance_em(ch).unwrap_or_else(|| core14_advance_em(ch, weight, slope));
        return (cmds, adv_em);
    }
    if let Some((cmds, adv_em)) = fonts.outline_char(family, weight, slope, ch, pen_x, origin_y, glyph_size, true) {
        return (cmds, adv_em);
    }
    if has_cleanroom_vector_outline(ch) {
        let adv_em = char_advance_em_with_font(ch, weight, slope, preferred_ttf);
        let cmds = cleanroom_vector_outline(ch, pen_x, origin_y, glyph_size, adv_em, weight, slope);
        return (cmds, adv_em);
    }
    if let Some((cmds, adv_em)) = fonts.outline_char(family, weight, slope, ch, pen_x, origin_y, glyph_size, false) {
        return (cmds, adv_em);
    }
    let adv_em = char_advance_em_with_font(ch, weight, slope, preferred_ttf);
    let cmds = cleanroom_vector_outline(ch, pen_x, origin_y, glyph_size, adv_em, weight, slope);
    (cmds, adv_em)
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

fn validate_glyf_budgets(font: &FontRef<'_>, gid: u16, is_long_loca: bool) -> Option<()> {
    let loca = font.loca(Some(is_long_loca)).ok()?;
    let glyf = font.glyf().ok()?;
    let mut state = GlyfBudgetState::new();
    validate_glyf_budgets_rec(&loca, &glyf, gid, 0, &mut state)
}

fn validate_glyf_budgets_rec(loca: &Loca<'_>, glyf: &Glyf<'_>, gid: u16, depth: usize, state: &mut GlyfBudgetState) -> Option<()> {
    if depth >= MAX_TTF_COMPONENT_DEPTH {
        return None;
    }
    if state.active_gids.get(..depth)?.contains(&gid) {
        return None;
    }
    state.consume_component()?;
    let Some(glyph) = loca.get_glyf(GlyphId::from(gid), glyf).ok()? else {
        return Some(());
    };
    match glyph {
        Glyph::Simple(simple) => {
            let num_contours = simple.number_of_contours();
            if num_contours == 0 {
                return Some(());
            }
            let count = usize::try_from(num_contours).ok()?;
            let end_pts = simple.end_pts_of_contours();
            if count > MAX_TTF_CONTOURS || end_pts.len() != count {
                return None;
            }
            let points = simple.num_points();
            if points > MAX_TTF_POINTS {
                return None;
            }
            state.consume_points(points)
        }
        Glyph::Composite(composite) => {
            *state.active_gids.get_mut(depth)? = gid;
            for (comp_gid, _) in composite.component_glyphs_and_flags() {
                let comp_gid_u16 = u16::try_from(comp_gid.to_u32()).ok()?;
                validate_glyf_budgets_rec(loca, glyf, comp_gid_u16, depth + 1, state)?;
            }
            Some(())
        }
    }
}

struct BoundedCharstringContext<'a> {
    inner: (&'a [u8], &'a Index<'a>, &'a Index<'a>, &'a Index<'a>),
    remaining_calls: Cell<usize>,
    remaining_bytes: Cell<usize>,
}

impl BoundedCharstringContext<'_> {
    fn consume_call(&self, bytes_len: usize) -> Result<(), FontModelError> {
        let calls = self.remaining_calls.get();
        if calls == 0 {
            return Err(FontModelError::LimitExceeded);
        }
        self.remaining_calls.set(calls - 1);
        let rem = self.remaining_bytes.get();
        let Some(next_rem) = rem.checked_sub(bytes_len) else {
            return Err(FontModelError::LimitExceeded);
        };
        self.remaining_bytes.set(next_rem);
        Ok(())
    }
}

impl CharstringContext for BoundedCharstringContext<'_> {
    fn kind(&self) -> CharstringKind {
        CharstringKind::Type2
    }

    fn seac_components(&self, base_code: i32, accent_code: i32) -> Result<[&[u8]; 2], FontModelError> {
        let [base, accent] = self.inner.seac_components(base_code, accent_code)?;
        self.consume_call(base.len().saturating_add(accent.len()))?;
        Ok([base, accent])
    }

    fn global_subr(&self, index: i32) -> Result<&[u8], FontModelError> {
        let subr = self.inner.global_subr(index)?;
        self.consume_call(subr.len())?;
        Ok(subr)
    }

    fn subr(&self, index: i32) -> Result<&[u8], FontModelError> {
        let subr = self.inner.subr(index)?;
        self.consume_call(subr.len())?;
        Ok(subr)
    }
}

struct CffBudgetSink {
    contours: usize,
    segments: usize,
    overflowed: bool,
}

impl CommandSink for CffBudgetSink {
    fn move_to(&mut self, _x: Fixed, _y: Fixed) {
        self.contours = self.contours.saturating_add(1);
        self.segments = self.segments.saturating_add(1);
        if self.contours > MAX_TTF_CONTOURS || self.segments > MAX_TTF_TOTAL_POINTS {
            self.overflowed = true;
        }
    }

    fn line_to(&mut self, _x: Fixed, _y: Fixed) {
        self.segments = self.segments.saturating_add(1);
        if self.segments > MAX_TTF_TOTAL_POINTS {
            self.overflowed = true;
        }
    }

    fn curve_to(&mut self, _cx0: Fixed, _cy0: Fixed, _cx1: Fixed, _cy1: Fixed, _x: Fixed, _y: Fixed) {
        self.segments = self.segments.saturating_add(1);
        if self.segments > MAX_TTF_TOTAL_POINTS {
            self.overflowed = true;
        }
    }

    fn close(&mut self) {}
}

fn validate_cff_budgets(font: &FontRef<'_>, gid: u16, upem: u16, is_cff2: bool) -> Option<()> {
    let table_bytes = if is_cff2 { font.table_data(Tag::new(b"CFF2"))?.as_bytes() } else { font.table_data(Tag::new(b"CFF "))?.as_bytes() };
    let cff = if is_cff2 {
        CffFontRef::new_cff2(table_bytes, Some(i32::from(upem))).ok()?
    } else {
        CffFontRef::new_cff(table_bytes, 0, Some(i32::from(upem))).ok()?
    };
    let gid_t = GlyphId::from(gid);
    let subfont_idx = cff.subfont_index(gid_t)?;
    let subfont = cff.subfont(subfont_idx, &[]).ok()?;
    let charstrings = cff.charstrings();
    let charstring_data = charstrings.get(usize::from(gid))?;
    let remaining_bytes = MAX_CFF_TOTAL_BYTECODE.checked_sub(charstring_data.len())?;
    let local_subrs = if subfont.subrs_offset() != 0 {
        let subr_slice = cff.data().get(usize::try_from(subfont.subrs_offset()).ok()?..)?;
        Index::new(subr_slice, is_cff2).ok()?
    } else {
        Index::Empty
    };
    let blend = cff.var_store().and_then(|store| BlendState::new(store.clone(), &[], subfont.vs_index()).ok());
    let ctx = BoundedCharstringContext {
        inner: (cff.data(), charstrings, cff.global_subrs(), &local_subrs),
        remaining_calls: Cell::new(MAX_CFF_SUBR_CALLS),
        remaining_bytes: Cell::new(remaining_bytes),
    };
    let mut sink = CffBudgetSink { contours: 0, segments: 0, overflowed: false };
    charstring::evaluate(&ctx, blend, charstring_data, &mut sink).ok()?;
    (!sink.overflowed).then_some(())
}

struct GlyphOutlinePen<'a> {
    origin_x: f64,
    origin_y: f64,
    scale: f64,
    slant: f64,
    contour_start: Option<(f64, f64)>,
    last_was_line_to: bool,
    contours: usize,
    segments: usize,
    failed: bool,
    out: &'a mut Vec<GlyphOutlineCmd>,
}

impl GlyphOutlinePen<'_> {
    fn map_pt(&self, fx: f32, fy: f32) -> Option<(f64, f64)> {
        let x = f64::from(fx);
        let y = f64::from(fy);
        let mx = self.origin_x + (x + y * self.slant) * self.scale;
        let my = self.origin_y - y * self.scale;
        (mx.is_finite() && my.is_finite()).then_some((mx, my))
    }

    fn consume_segment(&mut self, is_contour_start: bool) -> bool {
        if self.failed {
            return false;
        }
        if is_contour_start {
            self.contours = self.contours.saturating_add(1);
            if self.contours > MAX_TTF_CONTOURS {
                self.failed = true;
                return false;
            }
        }
        self.segments = self.segments.saturating_add(1);
        if self.segments > MAX_TTF_TOTAL_POINTS {
            self.failed = true;
            return false;
        }
        true
    }
}

impl OutlinePen for GlyphOutlinePen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        if !self.consume_segment(true) {
            return;
        }
        let Some((x_pt, y_pt)) = self.map_pt(x, y) else {
            self.failed = true;
            return;
        };
        self.contour_start = Some((x_pt, y_pt));
        self.last_was_line_to = false;
        self.out.push(GlyphOutlineCmd::MoveTo { x_pt, y_pt });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if !self.consume_segment(false) {
            return;
        }
        let Some((x_pt, y_pt)) = self.map_pt(x, y) else {
            self.failed = true;
            return;
        };
        self.last_was_line_to = true;
        self.out.push(GlyphOutlineCmd::LineTo { x_pt, y_pt });
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        if !self.consume_segment(false) {
            return;
        }
        let (Some((cx_pt, cy_pt)), Some((x_pt, y_pt))) = (self.map_pt(cx0, cy0), self.map_pt(x, y)) else {
            self.failed = true;
            return;
        };
        self.last_was_line_to = false;
        self.out.push(GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt });
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if !self.consume_segment(false) {
            return;
        }
        let (Some((c1x_pt, c1y_pt)), Some((c2x_pt, c2y_pt)), Some((x_pt, y_pt))) = (self.map_pt(cx0, cy0), self.map_pt(cx1, cy1), self.map_pt(x, y))
        else {
            self.failed = true;
            return;
        };
        self.last_was_line_to = false;
        self.out.push(GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt });
    }

    fn close(&mut self) {
        if self.failed {
            return;
        }
        if let Some((sx, sy)) = self.contour_start.take()
            && self.last_was_line_to
        {
            self.out.push(GlyphOutlineCmd::LineTo { x_pt: sx, y_pt: sy });
        }
        self.last_was_line_to = false;
        self.out.push(GlyphOutlineCmd::Close);
    }
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
    cleanroom_vector_outline_with_y_shift(ch, pen_x, origin_y, glyph_size, adv_em, weight, slope, 0.0)
}

#[allow(clippy::too_many_arguments)]
fn cleanroom_vector_outline_with_y_shift(
    ch: char,
    pen_x: f64,
    origin_y: f64,
    glyph_size: f64,
    adv_em: f64,
    weight: SpanWeight,
    slope: SpanSlope,
    y_shift: f64,
) -> Vec<GlyphOutlineCmd> {
    let half_stroke = match weight {
        SpanWeight::Regular => 0.043,
        SpanWeight::Bold => 0.062,
    };
    let slant = if slope == SpanSlope::Italic { 0.2125 } else { 0.0 };
    let items = build_glyph_strokes(ch);

    // Map normalized x in [0, 1] into padded glyph advance width [pad_l .. adv_em - pad_r].
    let cell_adv = if adv_em > 0.0 { adv_em } else { 0.36 };
    let pad = (cell_adv * 0.10).clamp(0.03, 0.09);
    let draw_w = (cell_adv - 2.0 * pad).max(0.08);
    let map_pt = |(nx, ny): (f64, f64)| -> (f64, f64) { (pad + nx * draw_w, ny + y_shift) };

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

/// Return `true` when the cleanroom vector outline generator has an explicit glyph recipe for
/// `ch` (rather than falling back to a generic missing-glyph rectangle).
#[must_use]
pub fn has_cleanroom_vector_outline(ch: char) -> bool {
    try_build_glyph_strokes(ch).is_some()
}

fn build_glyph_strokes(ch: char) -> Vec<StrokeItem> {
    if let Some(items) = try_build_glyph_strokes(ch) {
        return items;
    }
    // Fallback box for arbitrary unmapped glyphs
    vec![StrokeItem::Ring(vec![(0.12, 0.04), (0.12, 0.68), (0.88, 0.68), (0.88, 0.04)])]
}

fn try_build_glyph_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    if let Some(items) = ascii_upper_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = ascii_lower_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = ascii_digit_punct_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = greek_upper_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = greek_lower_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = math_symbol_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = sub_super_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = latin_extended_strokes(ch) {
        return Some(items);
    }
    if let Some(items) = cyrillic_strokes(ch) {
        return Some(items);
    }
    combining_diacritic_strokes(ch)
}

#[derive(Clone, Copy)]
enum DiacriticKind {
    Grave,
    Acute,
    Circumflex,
    Tilde,
    Macron,
    Breve,
    DotAbove,
    Diaeresis,
    RingAbove,
    DoubleAcute,
    Caron,
    Cedilla,
    Ogonek,
    DotBelow,
    RingBelow,
    MacronBelow,
}

fn diacritic_strokes(kind: DiacriticKind, y_shift: f64) -> Vec<StrokeItem> {
    use StrokeItem::{Dot, Open, Ring};
    let shift = |pts: Vec<(f64, f64)>| -> Vec<(f64, f64)> { pts.into_iter().map(|(x, y)| (x, y + y_shift)).collect() };
    match kind {
        DiacriticKind::Grave => vec![Open(shift(vec![(0.36, 0.78), (0.58, 0.62)]))],
        DiacriticKind::Acute => vec![Open(shift(vec![(0.64, 0.78), (0.42, 0.62)]))],
        DiacriticKind::Circumflex => vec![Open(shift(vec![(0.28, 0.62), (0.50, 0.78), (0.72, 0.62)]))],
        DiacriticKind::Tilde => vec![Open(shift(vec![(0.24, 0.66), (0.38, 0.74), (0.62, 0.62), (0.76, 0.70)]))],
        DiacriticKind::Macron => vec![Open(shift(vec![(0.24, 0.68), (0.76, 0.68)]))],
        DiacriticKind::Breve => vec![Open(shift(arc(0.50, 0.76, 0.24, 0.14, 195.0, 345.0, 6)))],
        DiacriticKind::DotAbove => vec![Dot(0.50, 0.68 + y_shift, 1.15)],
        DiacriticKind::Diaeresis => vec![Dot(0.32, 0.68 + y_shift, 1.05), Dot(0.68, 0.68 + y_shift, 1.05)],
        DiacriticKind::RingAbove => vec![Ring(shift(arc(0.50, 0.68, 0.14, 0.09, 0.0, 360.0, 8)))],
        DiacriticKind::DoubleAcute => {
            vec![Open(shift(vec![(0.46, 0.78), (0.30, 0.62)])), Open(shift(vec![(0.72, 0.78), (0.56, 0.62)]))]
        }
        DiacriticKind::Caron => vec![Open(shift(vec![(0.28, 0.78), (0.50, 0.62), (0.72, 0.78)]))],
        DiacriticKind::Cedilla => vec![Open(vec![(0.52, 0.0), (0.50, -0.10), (0.62, -0.14), (0.56, -0.22), (0.42, -0.22)])],
        DiacriticKind::Ogonek => vec![Open(vec![(0.70, 0.0), (0.60, -0.14), (0.74, -0.22), (0.86, -0.18)])],
        DiacriticKind::DotBelow => vec![Dot(0.50, -0.14, 1.10)],
        DiacriticKind::RingBelow => vec![Ring(arc(0.50, -0.14, 0.13, 0.08, 0.0, 360.0, 8))],
        DiacriticKind::MacronBelow => vec![Open(vec![(0.24, -0.14), (0.76, -0.14)])],
    }
}

fn combining_diacritic_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use DiacriticKind as D;
    use StrokeItem::{Open, Ring};
    let kind = match ch {
        '\u{0300}' => D::Grave,
        '\u{0301}' => D::Acute,
        '\u{0302}' => D::Circumflex,
        '\u{0303}' => D::Tilde,
        '\u{0304}' | '\u{0305}' => D::Macron,
        '\u{0306}' => D::Breve,
        '\u{0307}' => D::DotAbove,
        '\u{0308}' => D::Diaeresis,
        '\u{030A}' => D::RingAbove,
        '\u{030B}' => D::DoubleAcute,
        '\u{030C}' => D::Caron,
        '\u{0323}' => D::DotBelow,
        '\u{0325}' => D::RingBelow,
        '\u{0327}' => D::Cedilla,
        '\u{0328}' => D::Ogonek,
        '\u{0331}' | '\u{0332}' => D::MacronBelow,
        '\u{0309}' | '\u{0313}'..='\u{0315}' => return Some(vec![Open(arc(0.50, 0.70, 0.16, 0.10, 0.0, 220.0, 6))]),
        '\u{030D}' | '\u{030E}' => return Some(vec![Open(vec![(0.50, 0.62), (0.50, 0.78)])]),
        '\u{030F}' => return Some(vec![Open(vec![(0.30, 0.78), (0.48, 0.62)]), Open(vec![(0.54, 0.78), (0.72, 0.62)])]),
        '\u{0311}' => return Some(vec![Open(arc(0.50, 0.64, 0.24, 0.14, 15.0, 165.0, 6))]),
        '\u{0334}'..='\u{0338}' => return Some(vec![Open(vec![(0.20, 0.26), (0.80, 0.44)])]),
        '\u{0300}'..='\u{036F}' => {
            return Some(if is_below_combining_mark(ch) {
                vec![Open(vec![(0.28, -0.14), (0.72, -0.14)])]
            } else {
                vec![Ring(arc(0.50, 0.68, 0.14, 0.08, 0.0, 360.0, 6))]
            });
        }
        _ => return None,
    };
    Some(diacritic_strokes(kind, 0.0))
}

fn latin_decomposed(ch: char) -> Option<(char, DiacriticKind, bool)> {
    use DiacriticKind as D;
    let spec = match ch {
        'À' => ('A', D::Grave, true),
        'Á' => ('A', D::Acute, true),
        'Â' => ('A', D::Circumflex, true),
        'Ã' => ('A', D::Tilde, true),
        'Ä' => ('A', D::Diaeresis, true),
        'Ç' => ('C', D::Cedilla, false),
        'È' => ('E', D::Grave, true),
        'É' => ('E', D::Acute, true),
        'Ê' => ('E', D::Circumflex, true),
        'Ë' => ('E', D::Diaeresis, true),
        'Ì' => ('I', D::Grave, true),
        'Í' => ('I', D::Acute, true),
        'Î' => ('I', D::Circumflex, true),
        'Ï' => ('I', D::Diaeresis, true),
        'Ñ' => ('N', D::Tilde, true),
        'Ò' => ('O', D::Grave, true),
        'Ó' => ('O', D::Acute, true),
        'Ô' => ('O', D::Circumflex, true),
        'Õ' => ('O', D::Tilde, true),
        'Ö' => ('O', D::Diaeresis, true),
        'Ù' => ('U', D::Grave, true),
        'Ú' => ('U', D::Acute, true),
        'Û' => ('U', D::Circumflex, true),
        'Ü' => ('U', D::Diaeresis, true),
        'Ý' => ('Y', D::Acute, true),
        'à' => ('a', D::Grave, false),
        'á' => ('a', D::Acute, false),
        'â' => ('a', D::Circumflex, false),
        'ã' => ('a', D::Tilde, false),
        'ä' => ('a', D::Diaeresis, false),
        'å' => ('a', D::RingAbove, false),
        'ç' => ('c', D::Cedilla, false),
        'è' => ('e', D::Grave, false),
        'é' => ('e', D::Acute, false),
        'ê' => ('e', D::Circumflex, false),
        'ë' => ('e', D::Diaeresis, false),
        'ì' => ('\u{0131}', D::Grave, false),
        'í' => ('\u{0131}', D::Acute, false),
        'î' => ('\u{0131}', D::Circumflex, false),
        'ï' => ('\u{0131}', D::Diaeresis, false),
        'ñ' => ('n', D::Tilde, false),
        'ò' => ('o', D::Grave, false),
        'ó' => ('o', D::Acute, false),
        'ô' => ('o', D::Circumflex, false),
        'õ' => ('o', D::Tilde, false),
        'ö' => ('o', D::Diaeresis, false),
        'ù' => ('u', D::Grave, false),
        'ú' => ('u', D::Acute, false),
        'û' => ('u', D::Circumflex, false),
        'ü' => ('u', D::Diaeresis, false),
        'ý' => ('y', D::Acute, false),
        'ÿ' => ('y', D::Diaeresis, false),
        // Latin Extended-A (`0x0100..=0x017F`)
        'Ā' => ('A', D::Macron, true),
        'ā' => ('a', D::Macron, false),
        'Ă' => ('A', D::Breve, true),
        'ă' => ('a', D::Breve, false),
        'Ą' => ('A', D::Ogonek, false),
        'ą' => ('a', D::Ogonek, false),
        'Ć' => ('C', D::Acute, true),
        'ć' => ('c', D::Acute, false),
        'Ĉ' => ('C', D::Circumflex, true),
        'ĉ' => ('c', D::Circumflex, false),
        'Ċ' => ('C', D::DotAbove, true),
        'ċ' => ('c', D::DotAbove, false),
        'Č' => ('C', D::Caron, true),
        'č' => ('c', D::Caron, false),
        'Ď' => ('D', D::Caron, true),
        'ď' => ('d', D::Caron, true),
        'Ē' => ('E', D::Macron, true),
        'ē' => ('e', D::Macron, false),
        'Ĕ' => ('E', D::Breve, true),
        'ĕ' => ('e', D::Breve, false),
        'Ė' => ('E', D::DotAbove, true),
        'ė' => ('e', D::DotAbove, false),
        'Ę' => ('E', D::Ogonek, false),
        'ę' => ('e', D::Ogonek, false),
        'Ě' => ('E', D::Caron, true),
        'ě' => ('e', D::Caron, false),
        'Ĝ' => ('G', D::Circumflex, true),
        'ĝ' => ('g', D::Circumflex, false),
        'Ğ' => ('G', D::Breve, true),
        'ğ' => ('g', D::Breve, false),
        'Ġ' => ('G', D::DotAbove, true),
        'ġ' => ('g', D::DotAbove, false),
        'Ģ' => ('G', D::Cedilla, false),
        'ģ' => ('g', D::Cedilla, false),
        'Ĥ' => ('H', D::Circumflex, true),
        'ĥ' => ('h', D::Circumflex, true),
        'Ĩ' => ('I', D::Tilde, true),
        'ĩ' => ('\u{0131}', D::Tilde, false),
        'Ī' => ('I', D::Macron, true),
        'ī' => ('\u{0131}', D::Macron, false),
        'Ĭ' => ('I', D::Breve, true),
        'ĭ' => ('\u{0131}', D::Breve, false),
        'Į' => ('I', D::Ogonek, false),
        'į' => ('\u{0131}', D::Ogonek, false),
        'İ' => ('I', D::DotAbove, true),
        'Ĵ' => ('J', D::Circumflex, true),
        'ĵ' => ('\u{0237}', D::Circumflex, false),
        'Ķ' => ('K', D::Cedilla, false),
        'ķ' => ('k', D::Cedilla, false),
        'Ĺ' => ('L', D::Acute, true),
        'ĺ' => ('l', D::Acute, true),
        'Ļ' => ('L', D::Cedilla, false),
        'ļ' => ('l', D::Cedilla, false),
        'Ľ' => ('L', D::Caron, true),
        'ľ' => ('l', D::Caron, true),
        'Ń' => ('N', D::Acute, true),
        'ń' => ('n', D::Acute, false),
        'Ņ' => ('N', D::Cedilla, false),
        'ņ' => ('n', D::Cedilla, false),
        'Ň' => ('N', D::Caron, true),
        'ň' => ('n', D::Caron, false),
        'Ō' => ('O', D::Macron, true),
        'ō' => ('o', D::Macron, false),
        'Ŏ' => ('O', D::Breve, true),
        'ŏ' => ('o', D::Breve, false),
        'Ő' => ('O', D::DoubleAcute, true),
        'ő' => ('o', D::DoubleAcute, false),
        'Ŕ' => ('R', D::Acute, true),
        'ŕ' => ('r', D::Acute, false),
        'Ŗ' => ('R', D::Cedilla, false),
        'ŗ' => ('r', D::Cedilla, false),
        'Ř' => ('R', D::Caron, true),
        'ř' => ('r', D::Caron, false),
        'Ś' => ('S', D::Acute, true),
        'ś' => ('s', D::Acute, false),
        'Ŝ' => ('S', D::Circumflex, true),
        'ŝ' => ('s', D::Circumflex, false),
        'Ş' => ('S', D::Cedilla, false),
        'ş' => ('s', D::Cedilla, false),
        'Š' => ('S', D::Caron, true),
        'š' => ('s', D::Caron, false),
        'Ţ' => ('T', D::Cedilla, false),
        'ţ' => ('t', D::Cedilla, false),
        'Ť' => ('T', D::Caron, true),
        'ť' => ('t', D::Caron, true),
        'Ũ' => ('U', D::Tilde, true),
        'ũ' => ('u', D::Tilde, false),
        'Ū' => ('U', D::Macron, true),
        'ū' => ('u', D::Macron, false),
        'Ŭ' => ('U', D::Breve, true),
        'ŭ' => ('u', D::Breve, false),
        'Ů' => ('U', D::RingAbove, true),
        'ů' => ('u', D::RingAbove, false),
        'Ű' => ('U', D::DoubleAcute, true),
        'ű' => ('u', D::DoubleAcute, false),
        'Ų' => ('U', D::Ogonek, false),
        'ų' => ('u', D::Ogonek, false),
        'Ŵ' => ('W', D::Circumflex, true),
        'ŵ' => ('w', D::Circumflex, false),
        'Ŷ' => ('Y', D::Circumflex, true),
        'ŷ' => ('y', D::Circumflex, false),
        'Ÿ' => ('Y', D::Diaeresis, true),
        'Ź' => ('Z', D::Acute, true),
        'ź' => ('z', D::Acute, false),
        'Ż' => ('Z', D::DotAbove, true),
        'ż' => ('z', D::DotAbove, false),
        'Ž' => ('Z', D::Caron, true),
        'ž' => ('z', D::Caron, false),
        _ => return None,
    };
    Some(spec)
}

fn latin_extended_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Dot, Open, Ring};
    if let Some((base, kind, tall)) = latin_decomposed(ch) {
        let mut strokes = build_glyph_strokes(base);
        strokes.extend(diacritic_strokes(kind, if tall { 0.20 } else { 0.0 }));
        return Some(strokes);
    }
    let items = match ch {
        '\u{0131}' => vec![Open(vec![(0.50, 0.0), (0.50, 0.50)])],
        '\u{0237}' => vec![Open(chain(vec![(0.60, 0.50), (0.60, -0.04)], arc(0.32, -0.04, 0.28, 0.16, 0.0, -145.0, 6), vec![]))],
        '¡' => vec![Dot(0.50, 0.56, 1.2), Open(vec![(0.50, 0.40), (0.50, -0.10)])],
        '¢' => vec![Open(arc(0.54, 0.26, 0.40, 0.26, 40.0, 320.0, 12)), Open(vec![(0.54, -0.08), (0.54, 0.60)])],
        '£' => vec![
            Open(chain(arc(0.56, 0.56, 0.28, 0.16, 20.0, 200.0, 8), vec![(0.28, 0.0), (0.86, 0.0)], vec![])),
            Open(vec![(0.14, 0.34), (0.64, 0.34)]),
        ],
        '¤' => vec![
            Ring(arc(0.50, 0.36, 0.26, 0.24, 0.0, 360.0, 12)),
            Open(vec![(0.20, 0.10), (0.32, 0.20)]),
            Open(vec![(0.80, 0.10), (0.68, 0.20)]),
            Open(vec![(0.20, 0.62), (0.32, 0.52)]),
            Open(vec![(0.80, 0.62), (0.68, 0.52)]),
        ],
        '¥' => vec![
            Open(vec![(0.10, 0.72), (0.50, 0.36), (0.90, 0.72)]),
            Open(vec![(0.50, 0.36), (0.50, 0.0)]),
            Open(vec![(0.16, 0.32), (0.84, 0.32)]),
            Open(vec![(0.16, 0.18), (0.84, 0.18)]),
        ],
        '¦' => vec![Open(vec![(0.50, 0.72), (0.50, 0.42)]), Open(vec![(0.50, 0.30), (0.50, 0.0)])],
        '¨' => diacritic_strokes(DiacriticKind::Diaeresis, 0.0),
        'ª' => {
            vec![Ring(arc(0.50, 0.56, 0.20, 0.14, 0.0, 360.0, 10)), Open(vec![(0.70, 0.68), (0.70, 0.42)]), Open(vec![(0.22, 0.32), (0.78, 0.32)])]
        }
        '«' => vec![Open(vec![(0.48, 0.46), (0.14, 0.26), (0.48, 0.06)]), Open(vec![(0.86, 0.46), (0.52, 0.26), (0.86, 0.06)])],
        '¬' => vec![Open(vec![(0.12, 0.44), (0.88, 0.44), (0.88, 0.18)])],
        '\u{00AD}' => vec![Open(vec![(0.20, 0.30), (0.80, 0.30)])],
        '¯' => diacritic_strokes(DiacriticKind::Macron, 0.06),
        '´' => diacritic_strokes(DiacriticKind::Acute, 0.0),
        '¶' => vec![
            Open(vec![(0.60, 0.0), (0.60, 0.72), (0.88, 0.72)]),
            Open(vec![(0.80, 0.0), (0.80, 0.72)]),
            Open(arc(0.54, 0.54, 0.32, 0.18, 90.0, 270.0, 8)),
        ],
        '¸' => vec![Open(vec![(0.52, 0.08), (0.50, -0.02), (0.62, -0.06), (0.56, -0.14), (0.42, -0.14)])],
        'º' => vec![Ring(arc(0.50, 0.56, 0.22, 0.14, 0.0, 360.0, 10)), Open(vec![(0.22, 0.32), (0.78, 0.32)])],
        '»' => vec![Open(vec![(0.14, 0.46), (0.48, 0.26), (0.14, 0.06)]), Open(vec![(0.52, 0.46), (0.86, 0.26), (0.52, 0.06)])],
        '¼' | '½' | '¾' => vec![Open(vec![(0.16, 0.04), (0.84, 0.68)]), Dot(0.26, 0.56, 1.25), Dot(0.74, 0.16, 1.25)],
        '¿' => vec![Dot(0.50, 0.56, 1.2), Open(chain(vec![(0.50, 0.38)], arc(0.52, 0.08, 0.36, 0.18, 135.0, -45.0, 8), vec![]))],
        'Æ' => vec![
            Open(vec![(0.06, 0.0), (0.44, 0.72), (0.44, 0.0)]),
            Open(vec![(0.18, 0.26), (0.44, 0.26)]),
            Open(vec![(0.92, 0.72), (0.44, 0.72)]),
            Open(vec![(0.44, 0.38), (0.84, 0.38)]),
            Open(vec![(0.44, 0.0), (0.92, 0.0)]),
        ],
        'æ' => vec![
            Ring(arc(0.28, 0.22, 0.22, 0.18, 0.0, 360.0, 10)),
            Open(vec![(0.50, 0.0), (0.50, 0.50)]),
            Open(vec![(0.50, 0.26), (0.94, 0.26)]),
            Open(arc(0.72, 0.26, 0.22, 0.24, 0.0, 320.0, 10)),
        ],
        'Ð' | 'Đ' => {
            let mut d = build_glyph_strokes('D');
            d.push(Open(vec![(0.02, 0.36), (0.38, 0.36)]));
            d
        }
        'ð' => vec![
            Ring(arc(0.50, 0.24, 0.36, 0.24, 0.0, 360.0, 12)),
            Open(arc(0.50, 0.44, 0.36, 0.28, -10.0, 135.0, 8)),
            Open(vec![(0.32, 0.56), (0.84, 0.72)]),
        ],
        'đ' => {
            let mut d = build_glyph_strokes('d');
            d.push(Open(vec![(0.60, 0.58), (0.96, 0.58)]));
            d
        }
        'Ø' => {
            let mut o = build_glyph_strokes('O');
            o.push(Open(vec![(0.12, -0.04), (0.88, 0.76)]));
            o
        }
        'ø' => {
            let mut o = build_glyph_strokes('o');
            o.push(Open(vec![(0.12, -0.04), (0.88, 0.56)]));
            o
        }
        'Þ' => vec![
            Open(vec![(0.16, 0.0), (0.16, 0.72)]),
            Open(chain(vec![(0.16, 0.56)], arc(0.56, 0.38, 0.36, 0.18, 90.0, -90.0, 8), vec![(0.16, 0.20)])),
        ],
        'þ' => vec![Open(vec![(0.16, -0.20), (0.16, 0.72)]), Ring(arc(0.54, 0.26, 0.36, 0.26, 0.0, 360.0, 12))],
        'ß' => vec![
            Open(chain(vec![(0.20, 0.0), (0.20, 0.56)], arc(0.48, 0.56, 0.28, 0.16, 180.0, 0.0, 6), arc(0.56, 0.22, 0.34, 0.22, 90.0, -110.0, 8))),
            Open(arc(0.54, 0.52, 0.24, 0.16, 0.0, -110.0, 6)),
        ],
        'Ħ' => {
            let mut h = build_glyph_strokes('H');
            h.push(Open(vec![(0.02, 0.54), (0.98, 0.54)]));
            h
        }
        'ħ' => {
            let mut h = build_glyph_strokes('h');
            h.push(Open(vec![(0.02, 0.60), (0.58, 0.60)]));
            h
        }
        'Ĳ' => vec![Open(vec![(0.24, 0.0), (0.24, 0.72)]), Open(chain(vec![(0.82, 0.72)], arc(0.62, 0.22, 0.20, 0.22, 0.0, -170.0, 6), vec![]))],
        'ĳ' => vec![
            Dot(0.24, 0.68, 1.1),
            Open(vec![(0.24, 0.0), (0.24, 0.50)]),
            Dot(0.74, 0.68, 1.1),
            Open(chain(vec![(0.74, 0.50), (0.74, -0.04)], arc(0.54, -0.04, 0.20, 0.16, 0.0, -145.0, 5), vec![])),
        ],
        'ĸ' => vec![Open(vec![(0.14, 0.52), (0.14, 0.0)]), Open(vec![(0.86, 0.52), (0.18, 0.24), (0.88, 0.0)])],
        'Ŀ' => {
            let mut l = build_glyph_strokes('L');
            l.push(Dot(0.68, 0.36, 1.15));
            l
        }
        'ŀ' => {
            let mut l = build_glyph_strokes('l');
            l.push(Dot(0.78, 0.36, 1.15));
            l
        }
        'Ł' => {
            let mut l = build_glyph_strokes('L');
            l.push(Open(vec![(0.02, 0.26), (0.46, 0.48)]));
            l
        }
        'ł' => {
            let mut l = build_glyph_strokes('l');
            l.push(Open(vec![(0.22, 0.28), (0.78, 0.48)]));
            l
        }
        'ŉ' => {
            let mut n = build_glyph_strokes('n');
            n.push(Open(vec![(0.18, 0.74), (0.18, 0.56)]));
            n
        }
        'Ŋ' => vec![Open(vec![(0.12, 0.0), (0.12, 0.72), (0.86, 0.0), (0.86, 0.72)]), Open(arc(0.68, 0.0, 0.18, 0.20, 0.0, -150.0, 6))],
        'ŋ' => vec![
            Open(vec![(0.12, 0.0), (0.12, 0.52)]),
            Open(chain(arc(0.48, 0.36, 0.36, 0.16, 175.0, 0.0, 8), vec![(0.84, -0.06)], arc(0.66, -0.06, 0.18, 0.14, 0.0, -150.0, 6))),
        ],
        'Œ' => vec![
            Ring(arc(0.32, 0.36, 0.26, 0.36, 0.0, 360.0, 12)),
            Open(vec![(0.94, 0.72), (0.58, 0.72), (0.58, 0.0), (0.94, 0.0)]),
            Open(vec![(0.58, 0.36), (0.88, 0.36)]),
        ],
        'œ' => vec![
            Ring(arc(0.30, 0.26, 0.22, 0.24, 0.0, 360.0, 10)),
            Open(vec![(0.52, 0.26), (0.94, 0.26)]),
            Open(arc(0.72, 0.26, 0.22, 0.24, 0.0, 320.0, 10)),
        ],
        'Ŧ' => {
            let mut t = build_glyph_strokes('T');
            t.push(Open(vec![(0.22, 0.38), (0.78, 0.38)]));
            t
        }
        'ŧ' => {
            let mut t = build_glyph_strokes('t');
            t.push(Open(vec![(0.20, 0.30), (0.74, 0.30)]));
            t
        }
        'ſ' => vec![Open(chain(arc(0.76, 0.60, 0.26, 0.12, 30.0, 180.0, 6), vec![(0.50, 0.0)], vec![]))],
        'ƒ' => vec![
            Open(chain(arc(0.74, 0.60, 0.22, 0.12, 20.0, 180.0, 6), vec![(0.46, -0.10)], arc(0.24, -0.10, 0.22, 0.10, 0.0, -160.0, 5))),
            Open(vec![(0.18, 0.44), (0.80, 0.44)]),
        ],
        '€' => {
            vec![Open(arc(0.58, 0.36, 0.38, 0.36, 40.0, 320.0, 14)), Open(vec![(0.08, 0.42), (0.72, 0.42)]), Open(vec![(0.08, 0.28), (0.68, 0.28)])]
        }
        _ => return None,
    };
    Some(items)
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
        '\u{2018}' => vec![Open(vec![(0.56, 0.72), (0.42, 0.56)]), Dot(0.54, 0.54, 0.9)],
        '\u{2019}' => vec![Open(vec![(0.54, 0.72), (0.40, 0.54)]), Dot(0.52, 0.68, 0.9)],
        '\u{201C}' => vec![Open(vec![(0.34, 0.72), (0.24, 0.54)]), Open(vec![(0.66, 0.72), (0.56, 0.54)])],
        '\u{201D}' => vec![Open(vec![(0.38, 0.72), (0.28, 0.54)]), Open(vec![(0.70, 0.72), (0.60, 0.54)])],
        '\u{2039}' => vec![Open(vec![(0.68, 0.46), (0.30, 0.26), (0.68, 0.06)])],
        '\u{203A}' => vec![Open(vec![(0.32, 0.46), (0.70, 0.26), (0.32, 0.06)])],
        '\u{2500}' => vec![Open(vec![(0.0, 0.36), (1.0, 0.36)])],
        '\u{FF08}' => ascii_digit_punct_strokes('(')?,
        '\u{FF09}' => ascii_digit_punct_strokes(')')?,
        '\u{FF0B}' => ascii_digit_punct_strokes('+')?,
        '\u{FF0C}' => ascii_digit_punct_strokes(',')?,
        '\u{FF0E}' => ascii_digit_punct_strokes('.')?,
        '\u{FF0F}' => ascii_digit_punct_strokes('/')?,
        '\u{25B6}' => vec![Solid(vec![(0.16, 0.66), (0.86, 0.36), (0.16, 0.06)])],
        '\u{25B8}' => vec![Solid(vec![(0.24, 0.58), (0.76, 0.36), (0.24, 0.14)])],
        '\u{25BC}' => vec![Solid(vec![(0.12, 0.62), (0.88, 0.62), (0.50, 0.08)])],
        '\u{25BE}' => vec![Solid(vec![(0.18, 0.54), (0.82, 0.54), (0.50, 0.16)])],
        '\u{FF1A}' => ascii_digit_punct_strokes(':')?,
        _ => return None,
    };
    Some(items)
}

fn transform_stroke_items(items: Vec<StrokeItem>, y_scale: f64, y_shift: f64) -> Vec<StrokeItem> {
    let tx = |pts: Vec<(f64, f64)>| -> Vec<(f64, f64)> { pts.into_iter().map(|(x, y)| (x, y * y_scale + y_shift)).collect() };
    items
        .into_iter()
        .map(|item| match item {
            StrokeItem::Open(pts) => StrokeItem::Open(tx(pts)),
            StrokeItem::Ring(pts) => StrokeItem::Ring(tx(pts)),
            StrokeItem::Dot(x, y, r) => StrokeItem::Dot(x, y * y_scale + y_shift, r * 0.75),
            StrokeItem::Solid(pts) => StrokeItem::Solid(tx(pts)),
        })
        .collect()
}

fn sub_super_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    let (base, is_sup) = match ch {
        '\u{1D62}' => ('i', false),
        '\u{1D63}' => ('r', false),
        '\u{1D64}' => ('u', false),
        '\u{1D65}' => ('v', false),
        '\u{2071}' => ('i', true),
        '\u{2074}' => ('4', true),
        '\u{2075}' => ('5', true),
        '\u{2076}' => ('6', true),
        '\u{2077}' => ('7', true),
        '\u{2078}' => ('8', true),
        '\u{2079}' => ('9', true),
        '\u{207A}' => ('+', true),
        '\u{207B}' => ('-', true),
        '\u{207C}' => ('=', true),
        '\u{207D}' => ('(', true),
        '\u{207E}' => (')', true),
        '\u{207F}' => ('n', true),
        '\u{2080}' => ('0', false),
        '\u{2081}' => ('1', false),
        '\u{2082}' => ('2', false),
        '\u{2083}' => ('3', false),
        '\u{2084}' => ('4', false),
        '\u{2085}' => ('5', false),
        '\u{2086}' => ('6', false),
        '\u{2087}' => ('7', false),
        '\u{2088}' => ('8', false),
        '\u{2089}' => ('9', false),
        '\u{208A}' => ('+', false),
        '\u{208B}' => ('-', false),
        '\u{208C}' => ('=', false),
        '\u{208D}' => ('(', false),
        '\u{208E}' => (')', false),
        '\u{2090}' => ('a', false),
        '\u{2091}' => ('e', false),
        '\u{2092}' => ('o', false),
        '\u{2093}' => ('x', false),
        '\u{2095}' => ('h', false),
        '\u{2096}' => ('k', false),
        '\u{2097}' => ('l', false),
        '\u{2098}' => ('m', false),
        '\u{2099}' => ('n', false),
        '\u{209A}' => ('p', false),
        '\u{209B}' => ('s', false),
        '\u{209C}' => ('t', false),
        _ => return None,
    };
    let raw = ascii_digit_punct_strokes(base).or_else(|| ascii_lower_strokes(base))?;
    let (y_scale, y_shift) = if is_sup { (0.56, 0.34) } else { (0.56, -0.18) };
    Some(transform_stroke_items(raw, y_scale, y_shift))
}

fn cyrillic_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    if let Some(upper) = cyrillic_upper_strokes(ch) {
        return Some(upper);
    }
    cyrillic_lower_strokes(ch)
}

fn cyrillic_upper_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::{Dot, Open, Ring};
    let items = match ch {
        'А' => ascii_upper_strokes('A')?,
        'Б' => vec![
            Open(vec![(0.86, 0.72), (0.14, 0.72), (0.14, 0.0)]),
            Open(chain(vec![(0.14, 0.38)], arc(0.52, 0.19, 0.38, 0.19, 90.0, -90.0, 8), vec![(0.14, 0.0)])),
        ],
        'В' => ascii_upper_strokes('B')?,
        'Г' => greek_upper_strokes('\u{0393}')?,
        'Д' => vec![
            Open(vec![(0.14, 0.06), (0.28, 0.72), (0.82, 0.72), (0.82, 0.06)]),
            Open(vec![(0.06, 0.06), (0.94, 0.06)]),
            Open(vec![(0.08, 0.06), (0.08, -0.10)]),
            Open(vec![(0.92, 0.06), (0.92, -0.10)]),
        ],
        'Е' => ascii_upper_strokes('E')?,
        'Ё' => {
            let mut e = ascii_upper_strokes('E')?;
            e.push(Dot(0.32, 0.86, 1.0));
            e.push(Dot(0.68, 0.86, 1.0));
            e
        }
        'Ж' => vec![
            Open(vec![(0.50, 0.0), (0.50, 0.72)]),
            Open(vec![(0.08, 0.72), (0.50, 0.36), (0.92, 0.72)]),
            Open(vec![(0.08, 0.0), (0.50, 0.36), (0.92, 0.0)]),
        ],
        'З' => vec![Open(chain(arc(0.48, 0.54, 0.38, 0.18, 155.0, -90.0, 8), arc(0.50, 0.18, 0.40, 0.18, 90.0, -155.0, 8), vec![]))],
        'И' => vec![Open(vec![(0.12, 0.72), (0.12, 0.0), (0.88, 0.72), (0.88, 0.0)])],
        'Й' => {
            let mut i = vec![Open(vec![(0.12, 0.72), (0.12, 0.0), (0.88, 0.72), (0.88, 0.0)])];
            i.extend(diacritic_strokes(DiacriticKind::Breve, 0.12));
            i
        }
        'К' => ascii_upper_strokes('K')?,
        'Л' => vec![Open(vec![(0.08, 0.0), (0.28, 0.72), (0.86, 0.72), (0.86, 0.0)])],
        'М' => ascii_upper_strokes('M')?,
        'Н' => ascii_upper_strokes('H')?,
        'О' => ascii_upper_strokes('O')?,
        'П' => greek_upper_strokes('\u{03A0}')?,
        'Р' => ascii_upper_strokes('P')?,
        'С' => ascii_upper_strokes('C')?,
        'Т' => ascii_upper_strokes('T')?,
        'У' => vec![Open(vec![(0.12, 0.72), (0.50, 0.24)]), Open(vec![(0.88, 0.72), (0.36, 0.0), (0.16, 0.0)])],
        'Ф' => greek_upper_strokes('\u{03A6}')?,
        'Х' => ascii_upper_strokes('X')?,
        'Ц' => vec![Open(vec![(0.12, 0.72), (0.12, 0.04), (0.82, 0.04), (0.82, 0.72)]), Open(vec![(0.82, 0.04), (0.94, 0.04), (0.94, -0.10)])],
        'Ч' => vec![Open(chain(vec![(0.14, 0.72)], arc(0.50, 0.42, 0.36, 0.14, 180.0, 360.0, 6), vec![(0.86, 0.72), (0.86, 0.0)]))],
        'Ш' => vec![Open(vec![(0.08, 0.72), (0.08, 0.0), (0.92, 0.0), (0.92, 0.72)]), Open(vec![(0.50, 0.72), (0.50, 0.0)])],
        'Щ' => vec![
            Open(vec![(0.08, 0.72), (0.08, 0.04), (0.84, 0.04), (0.84, 0.72)]),
            Open(vec![(0.46, 0.72), (0.46, 0.04)]),
            Open(vec![(0.84, 0.04), (0.96, 0.04), (0.96, -0.10)]),
        ],
        'Ъ' => vec![
            Open(vec![(0.08, 0.72), (0.34, 0.72), (0.34, 0.0)]),
            Open(chain(vec![(0.34, 0.38)], arc(0.64, 0.19, 0.28, 0.19, 90.0, -90.0, 8), vec![(0.34, 0.0)])),
        ],
        'Ы' => vec![
            Open(vec![(0.10, 0.72), (0.10, 0.0)]),
            Open(chain(vec![(0.10, 0.36)], arc(0.38, 0.18, 0.24, 0.18, 90.0, -90.0, 8), vec![(0.10, 0.0)])),
            Open(vec![(0.86, 0.72), (0.86, 0.0)]),
        ],
        'Ь' => vec![
            Open(vec![(0.16, 0.72), (0.16, 0.0)]),
            Open(chain(vec![(0.16, 0.38)], arc(0.54, 0.19, 0.36, 0.19, 90.0, -90.0, 8), vec![(0.16, 0.0)])),
        ],
        'Э' => vec![Open(arc(0.44, 0.36, 0.44, 0.36, 140.0, -140.0, 14)), Open(vec![(0.42, 0.36), (0.88, 0.36)])],
        'Ю' => {
            vec![Open(vec![(0.10, 0.72), (0.10, 0.0)]), Open(vec![(0.10, 0.36), (0.36, 0.36)]), Ring(arc(0.64, 0.36, 0.28, 0.36, 0.0, 360.0, 14))]
        }
        'Я' => vec![
            Open(vec![(0.88, 0.0), (0.88, 0.72)]),
            Open(chain(vec![(0.88, 0.72)], arc(0.44, 0.54, 0.36, 0.18, 90.0, 270.0, 8), vec![(0.88, 0.36)])),
            Open(vec![(0.48, 0.36), (0.10, 0.0)]),
        ],
        _ => return None,
    };
    Some(items)
}

fn cyrillic_lower_strokes(ch: char) -> Option<Vec<StrokeItem>> {
    use StrokeItem::Dot;
    let items = match ch {
        'а' => ascii_lower_strokes('a')?,
        'б' => vec![StrokeItem::Ring(arc(0.50, 0.24, 0.36, 0.24, 0.0, 360.0, 12)), StrokeItem::Open(vec![(0.52, 0.48), (0.24, 0.66), (0.78, 0.72)])],
        'е' => ascii_lower_strokes('e')?,
        'ё' => {
            let mut e = ascii_lower_strokes('e')?;
            e.push(Dot(0.32, 0.68, 1.0));
            e.push(Dot(0.68, 0.68, 1.0));
            e
        }
        'о' => ascii_lower_strokes('o')?,
        'р' => ascii_lower_strokes('p')?,
        'с' => ascii_lower_strokes('c')?,
        'у' => ascii_lower_strokes('y')?,
        'х' => ascii_lower_strokes('x')?,
        'в' | 'г' | 'д' | 'ж' | 'з' | 'и' | 'й' | 'к' | 'л' | 'м' | 'н' | 'п' | 'т' | 'ф' | 'ц' | 'ч' | 'ш' | 'щ' | 'ъ' | 'ы' | 'ь' | 'э' | 'ю'
        | 'я' => {
            let upper = cyrillic_upper_for_lower(ch)?;
            transform_stroke_items(cyrillic_upper_strokes(upper)?, 0.72, 0.0)
        }
        _ => return None,
    };
    Some(items)
}

const fn cyrillic_upper_for_lower(ch: char) -> Option<char> {
    match ch {
        'в' => Some('В'),
        'г' => Some('Г'),
        'д' => Some('Д'),
        'ж' => Some('Ж'),
        'з' => Some('З'),
        'и' => Some('И'),
        'й' => Some('Й'),
        'к' => Some('К'),
        'л' => Some('Л'),
        'м' => Some('М'),
        'н' => Some('Н'),
        'п' => Some('П'),
        'т' => Some('Т'),
        'ф' => Some('Ф'),
        'ц' => Some('Ц'),
        'ч' => Some('Ч'),
        'ш' => Some('Ш'),
        'щ' => Some('Щ'),
        'ъ' => Some('Ъ'),
        'ы' => Some('Ы'),
        'ь' => Some('Ь'),
        'э' => Some('Э'),
        'ю' => Some('Ю'),
        'я' => Some('Я'),
        _ => None,
    }
}

fn chain(mut a: Vec<(f64, f64)>, b: Vec<(f64, f64)>, c: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    a.extend(b);
    a.extend(c);
    a
}
