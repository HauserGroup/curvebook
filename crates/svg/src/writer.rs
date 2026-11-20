//! Publication-quality SVG 1.1 serializer and structural validator.

use std::fmt::Write;

use curvebook_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
};
use curvebook_palette::Rgba8;
use curvebook_text::metrics::split_rich_text_lines;
use curvebook_text::{BUNDLED_FONTS, GlyphOutlineCmd, RichText, SpanScript, SpanSlope, SpanWeight, outline_rich_text};
use quick_xml::encoding::Decoder;
use quick_xml::events::{BytesEnd, BytesRef, BytesStart, Event};
use quick_xml::{Reader as XmlReader, XmlVersion};
use serde::{Deserialize, Serialize};

use crate::SVG_XMLNS;

const MAX_PRECISION: u8 = 6;
const DEFAULT_FONT_FAMILY: &str = "Helvetica, Arial, sans-serif";

/// Configuration controlling SVG figure serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvgExportOptions {
    pub pretty: bool,
    pub include_xml_declaration: bool,
    pub include_metadata: bool,
    pub embed_fonts: bool,
    #[serde(default)]
    pub outline_text: bool,
    pub precision: u8,
}

impl Default for SvgExportOptions {
    fn default() -> Self {
        Self { pretty: true, include_xml_declaration: false, include_metadata: true, embed_fonts: false, outline_text: false, precision: 3 }
    }
}

/// Errors produced during SVG export or structural validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SvgError {
    InvalidScene { message: String },
    ValidationFailed { message: String },
}

impl std::fmt::Display for SvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScene { message } => write!(f, "invalid scene for SVG export: {message}"),
            Self::ValidationFailed { message } => write!(f, "SVG validation failed: {message}"),
        }
    }
}

impl std::error::Error for SvgError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipEntry {
    id: String,
    x: String,
    y: String,
    width: String,
    height: String,
}

#[derive(Default)]
struct ClipRegistry {
    entries: Vec<ClipEntry>,
}

impl ClipRegistry {
    fn register(&mut self, rect: RectPt, prec: usize) -> Result<String, SvgError> {
        if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() {
            return Err(SvgError::InvalidScene { message: String::from("clip rectangle must have finite coordinates and extents") });
        }
        if rect.width < 0.0 || rect.height < 0.0 {
            return Err(SvgError::InvalidScene { message: String::from("clip rectangle dimensions must be non-negative") });
        }
        let x = format_scalar(rect.x, prec)?;
        let y = format_scalar(rect.y, prec)?;
        let width = format_scalar(rect.width, prec)?;
        let height = format_scalar(rect.height, prec)?;

        if let Some(existing) = self.entries.iter().find(|e| e.x == x && e.y == y && e.width == width && e.height == height) {
            return Ok(existing.id.clone());
        }
        let id = format!("clip-{}", self.entries.len());
        self.entries.push(ClipEntry { id: id.clone(), x, y, width, height });
        Ok(id)
    }
}

struct SvgRenderCtx<'a> {
    options: &'a SvgExportOptions,
    prec: usize,
    opacity_prec: usize,
    clips: ClipRegistry,
    body: String,
    clip_depth: usize,
}

impl<'a> SvgRenderCtx<'a> {
    fn new(options: &'a SvgExportOptions, prec: usize) -> Self {
        Self { options, prec, opacity_prec: prec.max(3), clips: ClipRegistry::default(), body: String::with_capacity(2048), clip_depth: 0 }
    }

    fn push_indent(&mut self, extra: usize) {
        if self.options.pretty {
            let depth = 1 + self.clip_depth + extra;
            for _ in 0..depth {
                self.body.push_str("  ");
            }
        }
    }

    fn push_newline(&mut self) {
        if self.options.pretty {
            self.body.push('\n');
        }
    }
}

/// Render a compiled [`ChartScene`] to a deterministic standalone SVG 1.1 XML string.
///
/// # Errors
/// Returns [`SvgError::InvalidScene`] when scene dimensions, coordinates, or stroke widths are
/// non-finite or invalid.
pub fn render_svg(scene: &ChartScene, options: &SvgExportOptions) -> Result<String, SvgError> {
    if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
        return Err(SvgError::InvalidScene { message: String::from("scene dimensions must be finite and positive") });
    }
    let prec = usize::from(options.precision.min(MAX_PRECISION));
    let w = format_scalar(scene.width_pt, prec)?;
    let h = format_scalar(scene.height_pt, prec)?;

    let mut ctx = SvgRenderCtx::new(options, prec);
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, fill, stroke, clip } => {
                if !path.is_empty() {
                    write_path_element(&mut ctx, path, *fill, stroke.as_ref(), *clip)?;
                }
            }
            ScenePrimitive::Text(text) => write_text_element(&mut ctx, text)?,
            ScenePrimitive::PushClip { rect } => {
                let clip_id = ctx.clips.register(*rect, ctx.prec)?;
                ctx.push_indent(0);
                let _ = write!(ctx.body, "<g clip-path=\"url(#{clip_id})\">");
                ctx.push_newline();
                ctx.clip_depth += 1;
            }
            ScenePrimitive::PopClip => {
                if ctx.clip_depth > 0 {
                    ctx.clip_depth -= 1;
                    ctx.push_indent(0);
                    ctx.body.push_str("</g>");
                    ctx.push_newline();
                }
            }
        }
    }
    while ctx.clip_depth > 0 {
        ctx.clip_depth -= 1;
        ctx.push_indent(0);
        ctx.body.push_str("</g>");
        ctx.push_newline();
    }

    let mut out = String::with_capacity(ctx.body.len() + 512);
    if options.include_xml_declaration {
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    }
    let _ = write!(out, "<svg xmlns=\"{SVG_XMLNS}\" version=\"1.1\" width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\">");
    if options.pretty {
        out.push('\n');
    }

    if options.include_metadata && !scene.title.is_empty() {
        let escaped = escape_xml(&scene.title);
        if options.pretty {
            out.push_str("  ");
        }
        let _ = write!(out, "<title>{escaped}</title>");
        if options.pretty {
            out.push('\n');
        }
    }

    write_defs(&mut out, &ctx.clips.entries, options);
    out.push_str(&ctx.body);
    out.push_str("</svg>\n");
    Ok(out)
}

fn write_defs(out: &mut String, clips: &[ClipEntry], options: &SvgExportOptions) {
    let embed_faces = options.embed_fonts && !options.outline_text && !BUNDLED_FONTS.is_empty();
    if clips.is_empty() && !embed_faces {
        return;
    }

    let indent1 = if options.pretty { "  " } else { "" };
    let indent2 = if options.pretty { "    " } else { "" };
    let indent3 = if options.pretty { "      " } else { "" };
    let nl = if options.pretty { "\n" } else { "" };

    let _ = write!(out, "{indent1}<defs>{nl}");
    for clip in clips {
        let _ = write!(out, "{indent2}<clipPath id=\"{}\">{nl}", clip.id);
        let _ = write!(out, "{indent3}<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>{nl}", clip.x, clip.y, clip.width, clip.height);
        let _ = write!(out, "{indent2}</clipPath>{nl}");
    }
    if embed_faces {
        let _ = write!(out, "{indent2}<style type=\"text/css\"><![CDATA[{nl}");
        for (name, bytes) in BUNDLED_FONTS {
            let b64 = encode_base64(bytes);
            let family = escape_css_string(name);
            let _ = write!(out, "@font-face {{ font-family: \"{family}\"; src: url(\"data:font/ttf;base64,{b64}\"); }}{nl}");
        }
        let _ = write!(out, "]]></style>{nl}");
    }
    let _ = write!(out, "{indent1}</defs>{nl}");
}

fn write_path_element(
    ctx: &mut SvgRenderCtx<'_>,
    path: &Path2D,
    fill: Option<FillPaint>,
    stroke: Option<&StrokePaint>,
    clip: Option<RectPt>,
) -> Result<(), SvgError> {
    let d = build_path_d(path, ctx.prec)?;
    let clip_id = match clip {
        Some(rect) => Some(ctx.clips.register(rect, ctx.prec)?),
        None => None,
    };

    ctx.push_indent(0);
    let _ = write!(ctx.body, "<path d=\"{d}\"");

    if let Some(f) = fill {
        let fill_hex = f.color.to_hex_rgb();
        let _ = write!(ctx.body, " fill=\"{fill_hex}\"");
        if f.color.a < u8::MAX {
            let opacity = format_opacity(f.color.a, ctx.opacity_prec)?;
            let _ = write!(ctx.body, " fill-opacity=\"{opacity}\"");
        }
    } else {
        ctx.body.push_str(" fill=\"none\"");
    }

    if let Some(st) = stroke {
        write_stroke_attrs(ctx, st)?;
    } else {
        ctx.body.push_str(" stroke=\"none\"");
    }

    if let Some(id) = clip_id {
        let _ = write!(ctx.body, " clip-path=\"url(#{id})\"");
    }
    ctx.body.push_str("/>");
    ctx.push_newline();
    Ok(())
}

fn write_stroke_attrs(ctx: &mut SvgRenderCtx<'_>, st: &StrokePaint) -> Result<(), SvgError> {
    if !st.width_pt.is_finite() || st.width_pt < 0.0 {
        return Err(SvgError::InvalidScene { message: String::from("stroke width must be finite and non-negative") });
    }
    let stroke_hex = st.color.to_hex_rgb();
    let _ = write!(ctx.body, " stroke=\"{stroke_hex}\"");
    if st.color.a < u8::MAX {
        let opacity = format_opacity(st.color.a, ctx.opacity_prec)?;
        let _ = write!(ctx.body, " stroke-opacity=\"{opacity}\"");
    }
    let width_str = format_scalar(st.width_pt, ctx.prec)?;
    let _ = write!(ctx.body, " stroke-width=\"{width_str}\"");

    let dash_mults = st.dash.dash_multipliers();
    if !dash_mults.is_empty() {
        ctx.body.push_str(" stroke-dasharray=\"");
        for (idx, &mult) in dash_mults.iter().enumerate() {
            if idx > 0 {
                ctx.body.push(',');
            }
            let len_str = format_scalar(mult * st.width_pt, ctx.prec)?;
            ctx.body.push_str(&len_str);
        }
        ctx.body.push('"');
    }

    let cap = match st.cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    };
    let join = match st.join {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    };
    let _ = write!(ctx.body, " stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"");
    Ok(())
}

fn build_path_d(path: &Path2D, prec: usize) -> Result<String, SvgError> {
    let mut d = String::with_capacity(path.commands.len() * 16);
    for (idx, cmd) in path.commands.iter().enumerate() {
        if idx > 0 {
            d.push(' ');
        }
        match *cmd {
            PathCmd::MoveTo(pt) => {
                let (x, y) = format_pt(pt, prec)?;
                let _ = write!(d, "M{x} {y}");
            }
            PathCmd::LineTo(pt) => {
                let (x, y) = format_pt(pt, prec)?;
                let _ = write!(d, "L{x} {y}");
            }
            PathCmd::QuadTo { control, to } => {
                let (cx, cy) = format_pt(control, prec)?;
                let (x, y) = format_pt(to, prec)?;
                let _ = write!(d, "Q{cx} {cy} {x} {y}");
            }
            PathCmd::CubicTo { c1, c2, to } => {
                let (c1x, c1y) = format_pt(c1, prec)?;
                let (c2x, c2y) = format_pt(c2, prec)?;
                let (x, y) = format_pt(to, prec)?;
                let _ = write!(d, "C{c1x} {c1y} {c2x} {c2y} {x} {y}");
            }
            PathCmd::Close => d.push('Z'),
        }
    }
    Ok(d)
}

fn write_text_element(ctx: &mut SvgRenderCtx<'_>, text: &PositionedText) -> Result<(), SvgError> {
    if !text.anchor_pt.x.is_finite() || !text.anchor_pt.y.is_finite() || !text.font_size_pt.is_finite() || !text.rotation_deg.is_finite() {
        return Err(SvgError::InvalidScene { message: String::from("text position, size, and rotation must be finite") });
    }
    if text.font_size_pt < 0.0 {
        return Err(SvgError::InvalidScene { message: String::from("text font size must be non-negative") });
    }
    if text.rich.is_empty() || text.font_size_pt == 0.0 {
        return Ok(());
    }

    if ctx.options.outline_text { write_outlined_text(ctx, text) } else { write_editable_text(ctx, text) }
}

fn write_editable_text(ctx: &mut SvgRenderCtx<'_>, text: &PositionedText) -> Result<(), SvgError> {
    let (x, y) = format_pt(text.anchor_pt, ctx.prec)?;
    let font_size = format_scalar(text.font_size_pt, ctx.prec)?;
    let anchor = match text.anchor {
        TextAnchor::Start => "start",
        TextAnchor::Middle => "middle",
        TextAnchor::End => "end",
    };
    let color = text.color.to_hex_rgb();
    let lines = split_rich_text_lines(&text.rich);
    let preserve_space = needs_xml_space_preserve_lines(&lines);

    ctx.push_indent(0);
    let _ = write!(
        ctx.body,
        "<text x=\"{x}\" y=\"{y}\" font-family=\"{DEFAULT_FONT_FAMILY}\" font-size=\"{font_size}\" text-anchor=\"{anchor}\" fill=\"{color}\""
    );
    if text.color.a < u8::MAX {
        let opacity = format_opacity(text.color.a, ctx.opacity_prec)?;
        let _ = write!(ctx.body, " fill-opacity=\"{opacity}\"");
    }
    let deg_str = format_scalar(text.rotation_deg, ctx.prec)?;
    if !is_formatted_zero(&deg_str) {
        let _ = write!(ctx.body, " transform=\"rotate({deg_str}, {x}, {y})\"");
    }
    if preserve_space {
        ctx.body.push_str(" xml:space=\"preserve\"");
    }
    ctx.body.push('>');

    let line_step = text.font_size_pt * 1.20;
    let mut current_y_offset = 0.0_f64;
    for (line_idx, line) in lines.iter().enumerate() {
        let line_y = (line_idx as f64) * line_step;
        let mut need_line_x = line_idx > 0;
        for span in &line.spans {
            if span.text.is_empty() {
                continue;
            }
            let script_y_offset = match span.script {
                SpanScript::Baseline => 0.0,
                SpanScript::Superscript => -0.35 * text.font_size_pt,
                SpanScript::Subscript => 0.20 * text.font_size_pt,
            };
            let target_y_offset = line_y + script_y_offset;
            let dy = target_y_offset - current_y_offset;
            current_y_offset = target_y_offset;

            let effective_scale = resolve_span_scale(span.scale, span.script);
            ctx.body.push_str("<tspan");
            if need_line_x {
                let _ = write!(ctx.body, " x=\"{x}\"");
                need_line_x = false;
            }

            let dy_str = format_scalar(dy, ctx.prec)?;
            if !is_formatted_zero(&dy_str) {
                let _ = write!(ctx.body, " dy=\"{dy_str}\"");
            }
            if (effective_scale - 1.0).abs() > 1e-6 {
                let span_size = format_scalar(text.font_size_pt * effective_scale, ctx.prec)?;
                let _ = write!(ctx.body, " font-size=\"{span_size}\"");
            }
            if span.weight == SpanWeight::Bold {
                ctx.body.push_str(" font-weight=\"bold\"");
            }
            if span.slope == SpanSlope::Italic {
                ctx.body.push_str(" font-style=\"italic\"");
            }
            if let Some(fam) = &span.font_family
                && !fam.is_empty()
            {
                let escaped_fam = escape_xml(fam);
                let _ = write!(ctx.body, " font-family=\"{escaped_fam}\"");
            }
            if let Some(c) = span.color {
                let span_hex = c.to_hex_rgb();
                let _ = write!(ctx.body, " fill=\"{span_hex}\"");
                if c.a < u8::MAX {
                    let span_op = format_opacity(c.a, ctx.opacity_prec)?;
                    let _ = write!(ctx.body, " fill-opacity=\"{span_op}\"");
                }
            }
            let escaped_text = escape_xml(&span.text);
            let _ = write!(ctx.body, ">{escaped_text}</tspan>");
        }
    }

    ctx.body.push_str("</text>");
    ctx.push_newline();
    Ok(())
}

fn write_outlined_text(ctx: &mut SvgRenderCtx<'_>, text: &PositionedText) -> Result<(), SvgError> {
    let lines = split_rich_text_lines(&text.rich);
    let line_step = text.font_size_pt * 1.20;
    let cos_sin = if text.rotation_deg != 0.0 {
        let rad = text.rotation_deg.to_radians();
        Some((rad.cos(), rad.sin()))
    } else {
        None
    };

    let mut path_entries: Vec<(String, Rgba8)> = Vec::new();
    for (line_idx, line) in lines.iter().enumerate() {
        let y_line = (line_idx as f64) * line_step;
        let glyphs = outline_rich_text(line, 0.0, y_line, text.font_size_pt);
        let total_width: f64 = glyphs.iter().map(|g| if g.advance_x_pt.is_finite() { g.advance_x_pt } else { 0.0 }).sum();
        let dx_anchor = match text.anchor {
            TextAnchor::Start => 0.0,
            TextAnchor::Middle => -0.5 * total_width,
            TextAnchor::End => -total_width,
        };
        for glyph in &glyphs {
            if glyph.commands.is_empty() {
                continue;
            }
            let d = build_glyph_path_d(&glyph.commands, dx_anchor, text.anchor_pt, cos_sin, ctx.prec)?;
            if !d.is_empty() {
                path_entries.push((d, glyph.color.unwrap_or(text.color)));
            }
        }
    }
    if path_entries.is_empty() {
        return Ok(());
    }

    ctx.push_indent(0);
    ctx.body.push_str("<g>");
    ctx.push_newline();
    for (d, color) in &path_entries {
        ctx.push_indent(1);
        let fill_hex = color.to_hex_rgb();
        let _ = write!(ctx.body, "<path d=\"{d}\" fill=\"{fill_hex}\"");
        if color.a < u8::MAX {
            let opacity = format_opacity(color.a, ctx.opacity_prec)?;
            let _ = write!(ctx.body, " fill-opacity=\"{opacity}\"");
        }
        ctx.body.push_str(" fill-rule=\"nonzero\" stroke=\"none\"/>");
        ctx.push_newline();
    }
    ctx.push_indent(0);
    ctx.body.push_str("</g>");
    ctx.push_newline();
    Ok(())
}

fn build_glyph_path_d(
    commands: &[GlyphOutlineCmd],
    dx_anchor: f64,
    anchor: PointPt,
    cos_sin: Option<(f64, f64)>,
    prec: usize,
) -> Result<String, SvgError> {
    let mut d = String::with_capacity(commands.len() * 16);
    for (idx, cmd) in commands.iter().enumerate() {
        if idx > 0 {
            d.push(' ');
        }
        match *cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                let (x, y) = format_transformed_pt(x_pt, y_pt, dx_anchor, anchor, cos_sin, prec)?;
                let _ = write!(d, "M{x} {y}");
            }
            GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                let (x, y) = format_transformed_pt(x_pt, y_pt, dx_anchor, anchor, cos_sin, prec)?;
                let _ = write!(d, "L{x} {y}");
            }
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                let (cx, cy) = format_transformed_pt(cx_pt, cy_pt, dx_anchor, anchor, cos_sin, prec)?;
                let (x, y) = format_transformed_pt(x_pt, y_pt, dx_anchor, anchor, cos_sin, prec)?;
                let _ = write!(d, "Q{cx} {cy} {x} {y}");
            }
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                let (c1x, c1y) = format_transformed_pt(c1x_pt, c1y_pt, dx_anchor, anchor, cos_sin, prec)?;
                let (c2x, c2y) = format_transformed_pt(c2x_pt, c2y_pt, dx_anchor, anchor, cos_sin, prec)?;
                let (x, y) = format_transformed_pt(x_pt, y_pt, dx_anchor, anchor, cos_sin, prec)?;
                let _ = write!(d, "C{c1x} {c1y} {c2x} {c2y} {x} {y}");
            }
            GlyphOutlineCmd::Close => d.push('Z'),
        }
    }
    Ok(d)
}

fn format_transformed_pt(
    lx: f64,
    ly: f64,
    dx_anchor: f64,
    anchor: PointPt,
    cos_sin: Option<(f64, f64)>,
    prec: usize,
) -> Result<(String, String), SvgError> {
    if !lx.is_finite() || !ly.is_finite() {
        return Err(SvgError::InvalidScene { message: String::from("glyph outline point must be finite") });
    }
    let sx = lx + dx_anchor;
    let sy = ly;
    let (rx, ry) = match cos_sin {
        Some((cos_a, sin_a)) => (sx * cos_a - sy * sin_a, sx * sin_a + sy * cos_a),
        None => (sx, sy),
    };
    Ok((format_scalar(anchor.x + rx, prec)?, format_scalar(anchor.y + ry, prec)?))
}

fn resolve_span_scale(scale: f64, script: SpanScript) -> f64 {
    if !scale.is_finite() || scale <= 0.0 {
        match script {
            SpanScript::Baseline => 1.0,
            SpanScript::Superscript | SpanScript::Subscript => 0.7,
        }
    } else if script != SpanScript::Baseline && (scale - 1.0).abs() < 1e-6 {
        0.7
    } else {
        scale
    }
}

fn needs_xml_space_preserve_lines(lines: &[RichText]) -> bool {
    for line in lines {
        let mut prev_space = false;
        let mut seen_non_empty = false;
        for span in &line.spans {
            if span.text.is_empty() {
                continue;
            }
            if !seen_non_empty && span.text.starts_with(char::is_whitespace) {
                return true;
            }
            seen_non_empty = true;
            for ch in span.text.chars() {
                if ch.is_whitespace() {
                    if prev_space || ch != ' ' {
                        return true;
                    }
                    prev_space = true;
                } else {
                    prev_space = false;
                }
            }
        }
        if prev_space {
            return true;
        }
    }
    false
}

fn format_pt(pt: PointPt, prec: usize) -> Result<(String, String), SvgError> {
    Ok((format_scalar(pt.x, prec)?, format_scalar(pt.y, prec)?))
}

fn format_opacity(alpha: u8, prec: usize) -> Result<String, SvgError> {
    format_scalar(f64::from(alpha) / 255.0, prec)
}

fn format_scalar(value: f64, prec: usize) -> Result<String, SvgError> {
    if !value.is_finite() {
        return Err(SvgError::InvalidScene { message: String::from("non-finite floating-point value in scene") });
    }
    let clean = if value == 0.0 { 0.0 } else { value };
    let mut formatted = format!("{clean:.prec$}");
    if let Some(rest) = formatted.strip_prefix('-')
        && rest.bytes().all(|b| b == b'0' || b == b'.')
    {
        formatted.remove(0);
    }
    Ok(formatted)
}

fn is_formatted_zero(s: &str) -> bool {
    let rest = s.strip_prefix('-').unwrap_or(s);
    !rest.is_empty() && rest.bytes().all(|b| b == b'0' || b == b'.')
}

const fn is_valid_xml_char(ch: char) -> bool {
    matches!(ch as u32, 0x09 | 0x0A | 0x0D | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10_FFFF)
}

fn escape_xml(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if !is_valid_xml_char(ch) {
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_css_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if !is_valid_xml_char(ch) {
            continue;
        }
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let i0 = ((triple >> 18) & 0x3F) as usize;
        let i1 = ((triple >> 12) & 0x3F) as usize;
        let i2 = ((triple >> 6) & 0x3F) as usize;
        let i3 = (triple & 0x3F) as usize;
        out.push(char::from(TABLE.get(i0).copied().unwrap_or(b'A')));
        out.push(char::from(TABLE.get(i1).copied().unwrap_or(b'A')));
        if chunk.len() > 1 {
            out.push(char::from(TABLE.get(i2).copied().unwrap_or(b'A')));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE.get(i3).copied().unwrap_or(b'A')));
        } else {
            out.push('=');
        }
    }
    out
}

#[derive(Default)]
struct TagValidationState {
    root_seen: bool,
    root_closed: bool,
    has_xmlns: bool,
    has_viewbox: bool,
    depth: usize,
}

/// Verify well-formed SVG XML structure: balanced tags, `<svg>` root with standard `xmlns` and
/// `viewBox`, valid XML entities, and finite numeric SVG attributes.
///
/// # Errors
/// Returns [`SvgError::ValidationFailed`] on structural, namespace, balance, or non-finite
/// attribute errors.
pub fn validate_svg_structure(svg: &str) -> Result<(), SvgError> {
    if svg.trim().is_empty() {
        return Err(SvgError::ValidationFailed { message: String::from("SVG content is empty") });
    }
    if let Some(bad_ch) = svg.chars().find(|&c| !is_valid_xml_char(c)) {
        return Err(SvgError::ValidationFailed { message: format!("forbidden XML 1.0 character U+{:04X} in SVG content", bad_ch as u32) });
    }

    let mut reader = XmlReader::from_str(svg);
    reader.config_mut().enable_all_checks(true);

    let mut state = TagValidationState::default();
    loop {
        let event = reader.read_event().map_err(|err| SvgError::ValidationFailed { message: format!("malformed SVG XML: {err}") })?;
        match event {
            Event::Start(start) => validate_start_tag(&start, false, reader.decoder(), &mut state)?,
            Event::Empty(empty) => validate_start_tag(&empty, true, reader.decoder(), &mut state)?,
            Event::End(end) => validate_end_tag(&end, &mut state)?,
            Event::Text(text) => {
                let decoded = text.decode().map_err(|err| SvgError::ValidationFailed { message: format!("invalid SVG text encoding: {err}") })?;
                if (!state.root_seen || state.root_closed) && !decoded.trim().is_empty() {
                    return Err(SvgError::ValidationFailed { message: String::from("unexpected character data outside <svg> root") });
                }
            }
            Event::GeneralRef(entity) => validate_general_ref(&entity, &state)?,
            Event::CData(_) => {
                if !state.root_seen || state.root_closed {
                    return Err(SvgError::ValidationFailed { message: String::from("CDATA outside <svg> root") });
                }
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }

    if !state.root_seen || !state.root_closed || state.depth != 0 {
        return Err(SvgError::ValidationFailed { message: String::from("unclosed or missing <svg> root element") });
    }
    if !state.has_xmlns {
        return Err(SvgError::ValidationFailed { message: String::from("missing standard SVG XML namespace") });
    }
    if !state.has_viewbox {
        return Err(SvgError::ValidationFailed { message: String::from("missing viewBox attribute") });
    }
    Ok(())
}

fn validate_start_tag(start: &BytesStart<'_>, self_closing: bool, decoder: Decoder, state: &mut TagValidationState) -> Result<(), SvgError> {
    if state.root_closed {
        return Err(SvgError::ValidationFailed { message: String::from("unexpected element after closing </svg>") });
    }
    let qname = start.name();
    let tag_name = std::str::from_utf8(qname.as_ref()).map_err(|_| SvgError::ValidationFailed { message: String::from("invalid UTF-8 tag name") })?;
    if !is_valid_xml_name(tag_name) {
        return Err(SvgError::ValidationFailed { message: format!("invalid XML tag name `{tag_name}`") });
    }
    let is_root = if !state.root_seen {
        if tag_name != "svg" {
            return Err(SvgError::ValidationFailed { message: format!("missing <svg> root element (found <{tag_name}>)") });
        }
        state.root_seen = true;
        true
    } else {
        false
    };

    validate_tag_attributes(start, is_root, decoder, state)?;

    if !self_closing {
        state.depth = state.depth.saturating_add(1);
    } else if is_root {
        state.root_closed = true;
    }
    Ok(())
}

fn validate_end_tag(end: &BytesEnd<'_>, state: &mut TagValidationState) -> Result<(), SvgError> {
    let qname = end.name();
    let name = std::str::from_utf8(qname.as_ref()).map_err(|_| SvgError::ValidationFailed { message: String::from("invalid UTF-8 closing tag") })?;
    if !is_valid_xml_name(name) {
        return Err(SvgError::ValidationFailed { message: format!("invalid closing tag name `{name}`") });
    }
    let Some(next_depth) = state.depth.checked_sub(1) else {
        return Err(SvgError::ValidationFailed { message: format!("mismatched closing tag </{name}>") });
    };
    state.depth = next_depth;
    if state.depth == 0 {
        state.root_closed = true;
    }
    Ok(())
}

fn validate_general_ref(entity: &BytesRef<'_>, state: &TagValidationState) -> Result<(), SvgError> {
    if !state.root_seen || state.root_closed {
        return Err(SvgError::ValidationFailed { message: String::from("unexpected character data outside <svg> root") });
    }
    match entity.resolve_char_ref() {
        Ok(Some(ch)) => {
            if !is_valid_xml_char(ch) {
                return Err(SvgError::ValidationFailed { message: format!("invalid XML numeric character reference U+{:04X}", ch as u32) });
            }
            Ok(())
        }
        Ok(None) => {
            let name = entity.decode().map_err(|err| SvgError::ValidationFailed { message: format!("invalid XML entity reference: {err}") })?;
            if name.is_empty() || name.chars().any(|c| c.is_ascii_whitespace() || c == '&' || c == '<') {
                return Err(SvgError::ValidationFailed { message: String::from("malformed XML entity reference") });
            }
            Ok(())
        }
        Err(err) => Err(SvgError::ValidationFailed { message: format!("invalid XML numeric character reference: {err}") }),
    }
}

fn validate_tag_attributes(start: &BytesStart<'_>, is_root: bool, decoder: Decoder, state: &mut TagValidationState) -> Result<(), SvgError> {
    for raw_attr in start.attributes().with_checks(true) {
        let attr = raw_attr.map_err(|err| SvgError::ValidationFailed { message: format!("malformed XML attribute: {err}") })?;
        let name = std::str::from_utf8(attr.key.as_ref())
            .map_err(|_| SvgError::ValidationFailed { message: String::from("invalid UTF-8 attribute name") })?;
        if !is_valid_xml_name(name) {
            return Err(SvgError::ValidationFailed { message: format!("invalid XML attribute name `{name}`") });
        }
        if attr.value.contains(&b'<') {
            return Err(SvgError::ValidationFailed { message: String::from("unescaped '<' inside attribute value") });
        }
        let val = attr
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
            .map_err(|err| SvgError::ValidationFailed { message: format!("invalid XML attribute `{name}`: {err}") })?;
        if let Some(bad_ch) = val.chars().find(|&c| !is_valid_xml_char(c)) {
            return Err(SvgError::ValidationFailed {
                message: format!("invalid XML numeric character reference U+{:04X} in attribute `{name}`", bad_ch as u32),
            });
        }
        if is_root {
            if name == "xmlns" {
                if val.as_ref() != SVG_XMLNS {
                    return Err(SvgError::ValidationFailed { message: String::from("missing standard SVG XML namespace") });
                }
                state.has_xmlns = true;
            } else if name == "viewBox" {
                state.has_viewbox = true;
            }
        }
        validate_attribute_value(name, val.as_ref())?;
    }
    Ok(())
}

fn validate_attribute_value(name: &str, val: &str) -> Result<(), SvgError> {
    match name {
        "viewBox" => validate_viewbox_attr(val),
        "width" | "height" | "font-size" | "stroke-width" | "r" | "rx" | "ry" => validate_length_list(val, false, name),
        "x" | "y" | "dx" | "dy" | "x1" | "y1" | "x2" | "y2" | "cx" | "cy" => validate_length_list(val, true, name),
        "fill-opacity" | "stroke-opacity" | "opacity" => validate_finite_number_list(val, false, name),
        "stroke-dasharray" => {
            if val.trim() == "none" {
                Ok(())
            } else {
                validate_finite_number_list(val, false, name)
            }
        }
        "d" => validate_path_data(val),
        "transform" => validate_transform_attr(val),
        _ => Ok(()),
    }
}

fn validate_viewbox_attr(val: &str) -> Result<(), SvgError> {
    let nums = parse_number_tokens(val, "viewBox")?;
    if nums.len() != 4 {
        return Err(SvgError::ValidationFailed { message: String::from("viewBox must contain 4 finite numbers") });
    }
    let w = nums.get(2).copied().unwrap_or(-1.0);
    let h = nums.get(3).copied().unwrap_or(-1.0);
    if w < 0.0 || h < 0.0 {
        return Err(SvgError::ValidationFailed { message: String::from("viewBox width and height must be non-negative") });
    }
    Ok(())
}

fn validate_length_list(val: &str, allow_negative: bool, attr: &str) -> Result<(), SvgError> {
    let mut found = false;
    for raw in val.split(|c: char| c.is_ascii_whitespace() || c == ',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        found = true;
        let num_part = strip_svg_unit(token);
        let parsed: f64 =
            num_part.parse().map_err(|_| SvgError::ValidationFailed { message: format!("invalid numeric `{attr}` attribute `{val}`") })?;
        if !parsed.is_finite() || (!allow_negative && parsed < 0.0) {
            return Err(SvgError::ValidationFailed { message: format!("non-finite or negative `{attr}` attribute `{val}`") });
        }
    }
    if !found {
        return Err(SvgError::ValidationFailed { message: format!("empty `{attr}` attribute") });
    }
    Ok(())
}

fn validate_finite_number_list(val: &str, allow_negative: bool, attr: &str) -> Result<(), SvgError> {
    let nums = parse_number_tokens(val, attr)?;
    if nums.is_empty() {
        return Err(SvgError::ValidationFailed { message: format!("empty numeric `{attr}` attribute") });
    }
    if !allow_negative && nums.iter().any(|&v| v < 0.0) {
        return Err(SvgError::ValidationFailed { message: format!("negative `{attr}` attribute `{val}`") });
    }
    Ok(())
}

fn parse_number_tokens(val: &str, attr: &str) -> Result<Vec<f64>, SvgError> {
    let mut nums = Vec::new();
    for raw in val.split(|c: char| c.is_ascii_whitespace() || c == ',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let parsed: f64 = token.parse().map_err(|_| SvgError::ValidationFailed { message: format!("invalid numeric `{attr}` value `{token}`") })?;
        if !parsed.is_finite() {
            return Err(SvgError::ValidationFailed { message: format!("non-finite `{attr}` value `{token}`") });
        }
        nums.push(parsed);
    }
    Ok(nums)
}

fn validate_path_data(d: &str) -> Result<(), SvgError> {
    let is_cmd =
        |c: char| matches!(c, 'M' | 'm' | 'L' | 'l' | 'H' | 'h' | 'V' | 'v' | 'C' | 'c' | 'S' | 's' | 'Q' | 'q' | 'T' | 't' | 'A' | 'a' | 'Z' | 'z');
    for raw in d.split(|c: char| c.is_ascii_whitespace() || c == ',' || is_cmd(c)) {
        if raw.is_empty() {
            continue;
        }
        validate_compact_number_run(raw, "d")?;
    }
    Ok(())
}

fn validate_compact_number_run(run: &str, attr: &str) -> Result<(), SvgError> {
    let bytes = run.as_bytes();
    let mut start = 0usize;
    for i in 1..bytes.len() {
        let b = bytes.get(i).copied().unwrap_or_default();
        let prev = bytes.get(i - 1).copied().unwrap_or_default();
        if (b == b'+' || b == b'-') && prev != b'e' && prev != b'E' {
            let part = run.get(start..i).ok_or_else(|| SvgError::ValidationFailed { message: format!("invalid `{attr}` slice") })?;
            parse_finite_f64(part, attr)?;
            start = i;
        }
    }
    let tail = run.get(start..).ok_or_else(|| SvgError::ValidationFailed { message: format!("invalid `{attr}` tail") })?;
    parse_finite_f64(tail, attr)?;
    Ok(())
}

fn parse_finite_f64(token: &str, attr: &str) -> Result<f64, SvgError> {
    let parsed: f64 = token.parse().map_err(|_| SvgError::ValidationFailed { message: format!("invalid `{attr}` token `{token}`") })?;
    if !parsed.is_finite() {
        return Err(SvgError::ValidationFailed { message: format!("non-finite `{attr}` value `{token}`") });
    }
    Ok(parsed)
}

fn validate_transform_attr(t: &str) -> Result<(), SvgError> {
    let mut rest = t.trim();
    while !rest.is_empty() {
        let open = rest.find('(').ok_or_else(|| SvgError::ValidationFailed { message: format!("malformed transform `{t}`") })?;
        let fname = rest.get(..open).map_or("", str::trim);
        if fname.is_empty() || !fname.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(SvgError::ValidationFailed { message: format!("invalid transform function `{fname}`") });
        }
        let after_open = rest.get(open + 1..).ok_or_else(|| SvgError::ValidationFailed { message: format!("malformed transform `{t}`") })?;
        let close = after_open.find(')').ok_or_else(|| SvgError::ValidationFailed { message: format!("unclosed transform `{t}`") })?;
        let args = after_open.get(..close).unwrap_or_default();
        let nums = parse_number_tokens(args, "transform")?;
        if nums.is_empty() {
            return Err(SvgError::ValidationFailed { message: format!("empty transform arguments in `{t}`") });
        }
        rest = after_open.get(close + 1..).unwrap_or_default().trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
    }
    Ok(())
}

fn strip_svg_unit(token: &str) -> &str {
    for unit in ["pt", "px", "mm", "cm", "in", "pc", "em", "ex", "%"] {
        if let Some(prefix) = token.strip_suffix(unit) {
            return prefix;
        }
    }
    token
}

fn is_valid_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

#[cfg(test)]
mod tests {
    use super::validate_svg_structure;
    use crate::SVG_XMLNS;

    #[test]
    fn validator_accepts_tolerated_xml_constructs_and_entities() {
        let fixtures = [
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!-- header comment -->\
                 <svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 120 80\">\
                 <style><![CDATA[.axis {{ stroke: #000; }}]]></style>\
                 <text x=\"10\" y=\"20\" data-label=\"A &amp; B &#x03B1;\">1 &lt; 2 &amp; &#160; &#x03B2;</text>\
                 </svg><!-- trailing comment -->"
            ),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/>"),
            format!("<?xml-stylesheet type=\"text/css\" href=\"fig.css\"?><svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"></svg>"),
        ];
        for fixture in fixtures {
            validate_svg_structure(&fixture).unwrap_or_else(|err| panic!("expected valid fixture `{fixture}`, got {err}"));
        }
    }

    #[test]
    fn validator_rejects_malformed_xml_comments_cdata_attrs_and_entities() {
        let invalid = [
            format!("<!-- unclosed comment <svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/>"),
            format!("<!-- bad -- comment --><svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/>"),
            format!("<![CDATA[outside]]><svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/>"),
            format!("stray text<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/><![CDATA[after]]>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"/>stray"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\" id=\"a<b\"/>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\" id=\"a\" id=\"b\"/>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\" id=\"&#x0B;\"/>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\" id=\"A & B\"/>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"><text>&foo bar;</text></svg>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"><g>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\"><1bad/></svg>"),
            format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 10 10\" 1bad=\"x\"/>"),
        ];
        for bad in invalid {
            assert!(validate_svg_structure(&bad).is_err(), "expected validation failure for: {bad}");
        }
    }
}
