//! Two-dimensional math box layout engine following Knuth, D. E. (1984), *The TeXbook*,
//! Addison-Wesley, Chapter 17 (pp. 140–145) and Appendix G ("Generating Boxes from Formulas",
//! pp. 441–447): eight cramped/uncramped math styles, fontdimens, Rule 11 radicals,
//! Rules 15a–15e fractions/binomials, Rules 18a–18f scripts, and Rule 19 scaled delimiters.

use curvebook_palette::Rgba8;
use curvebook_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, measure_text_pt};

use super::parser::{DelimKind, FractionKind, MathNode};
use super::{MAX_TEX_DEPTH, MIN_RULE_THICKNESS_PT};
use crate::scene::{FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, StrokePaint};

// ---------------------------------------------------------------------------
// Knuth Appendix G 2D Math Styles, Fontdimens & Box Layout Engine
// ---------------------------------------------------------------------------

/// TeXbook Chapter 17 (p. 140) & Appendix G (p. 441) math layout styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MathStyle {
    Display,
    DisplayCramped,
    Text,
    TextCramped,
    Script,
    ScriptCramped,
    ScriptScript,
    ScriptScriptCramped,
}

impl MathStyle {
    const fn is_display(self) -> bool {
        matches!(self, Self::Display | Self::DisplayCramped)
    }

    const fn is_cramped(self) -> bool {
        matches!(self, Self::DisplayCramped | Self::TextCramped | Self::ScriptCramped | Self::ScriptScriptCramped)
    }

    const fn scale(self) -> f64 {
        match self {
            Self::Display | Self::DisplayCramped | Self::Text | Self::TextCramped => 1.00,
            Self::Script | Self::ScriptCramped => 0.70,
            Self::ScriptScript | Self::ScriptScriptCramped => 0.50,
        }
    }

    /// Cramped variant $C'$ of the current style (Rule 11 radicands).
    const fn cramp(self) -> Self {
        match self {
            Self::Display | Self::DisplayCramped => Self::DisplayCramped,
            Self::Text | Self::TextCramped => Self::TextCramped,
            Self::Script | Self::ScriptCramped => Self::ScriptCramped,
            Self::ScriptScript | Self::ScriptScriptCramped => Self::ScriptScriptCramped,
        }
    }

    /// Appendix G (p. 441) superscript style transition:
    /// `D, T -> S`; `D', T' -> S'`; `S, SS -> SS`; `S', SS' -> SS'`.
    const fn superscript_style(self) -> Self {
        match self {
            Self::Display | Self::Text => Self::Script,
            Self::DisplayCramped | Self::TextCramped => Self::ScriptCramped,
            Self::Script | Self::ScriptScript => Self::ScriptScript,
            Self::ScriptCramped | Self::ScriptScriptCramped => Self::ScriptScriptCramped,
        }
    }

    /// Appendix G (p. 441) subscript style transition:
    /// `D, D', T, T' -> S'`; `S, S', SS, SS' -> SS'`.
    const fn subscript_style(self) -> Self {
        match self {
            Self::Display | Self::DisplayCramped | Self::Text | Self::TextCramped => Self::ScriptCramped,
            Self::Script | Self::ScriptCramped | Self::ScriptScript | Self::ScriptScriptCramped => Self::ScriptScriptCramped,
        }
    }

    /// Appendix G (p. 441) fraction numerator style transition:
    /// `D -> T`, `D' -> T'`, `T -> S`, `T' -> S'`, `S/SS -> SS`, `S'/SS' -> SS'`.
    const fn frac_numerator_style(self) -> Self {
        match self {
            Self::Display => Self::Text,
            Self::DisplayCramped => Self::TextCramped,
            Self::Text => Self::Script,
            Self::TextCramped => Self::ScriptCramped,
            Self::Script | Self::ScriptScript => Self::ScriptScript,
            Self::ScriptCramped | Self::ScriptScriptCramped => Self::ScriptScriptCramped,
        }
    }

    /// Appendix G (p. 441) fraction denominator style transition:
    /// `D, D' -> T'`, `T, T' -> S'`, `S, S', SS, SS' -> SS'`.
    const fn frac_denominator_style(self) -> Self {
        match self {
            Self::Display | Self::DisplayCramped => Self::TextCramped,
            Self::Text | Self::TextCramped => Self::ScriptCramped,
            Self::Script | Self::ScriptCramped | Self::ScriptScript | Self::ScriptScriptCramped => Self::ScriptScriptCramped,
        }
    }
}

/// Knuth, *The TeXbook* Appendix G (p. 447) fontdimen parameters in PostScript points (`pt`).
#[derive(Debug, Clone, Copy)]
pub(super) struct TeXMathDimens {
    pub(super) em: f64,
    pub(super) theta: f64,    // xi_8: default rule thickness
    pub(super) axis: f64,     // sigma_22: math axis height above baseline
    pub(super) x_h: f64,      // sigma_5: x-height
    pub(super) num1: f64,     // sigma_8: display fraction numerator shift
    pub(super) num2: f64,     // sigma_9: non-display fraction numerator shift (with rule)
    pub(super) num3: f64,     // sigma_10: non-display rule-less fraction numerator shift
    pub(super) denom1: f64,   // sigma_11: display denominator shift
    pub(super) denom2: f64,   // sigma_12: non-display denominator shift
    pub(super) sup1: f64,     // sigma_13: display superscript shift
    pub(super) sup2: f64,     // sigma_14: non-display uncramped superscript shift
    pub(super) sup3: f64,     // sigma_15: cramped superscript shift
    pub(super) sub1: f64,     // sigma_16: subscript-only shift
    pub(super) sub2: f64,     // sigma_17: subscript shift with superscript
    pub(super) sup_drop: f64, // sigma_18
    pub(super) sub_drop: f64, // sigma_19
    pub(super) script_space: f64,
    pub(super) null_delim_space: f64,
}

impl TeXMathDimens {
    fn for_style(style: MathStyle, base_font: f64) -> Self {
        let em = (base_font * style.scale()).max(3.5);
        Self {
            em,
            theta: (0.050 * em).max(MIN_RULE_THICKNESS_PT),
            axis: 0.340 * em,
            x_h: 0.520 * em,
            num1: 0.677 * em,
            num2: 0.394 * em,
            num3: 0.444 * em,
            denom1: 0.686 * em,
            denom2: 0.345 * em,
            sup1: 0.413 * em,
            sup2: 0.363 * em,
            sup3: 0.289 * em,
            sub1: 0.170 * em,
            sub2: 0.247 * em,
            sup_drop: 0.386 * em,
            sub_drop: 0.050 * em,
            script_space: 0.050 * em,
            null_delim_space: (0.120 * em).max(0.6),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LayoutCtx {
    pub(super) style: MathStyle,
    pub(super) base_font: f64,
    pub(super) color: Rgba8,
    pub(super) depth: usize,
}

impl LayoutCtx {
    fn with_style(self, style: MathStyle) -> Self {
        Self { style, depth: self.depth.saturating_add(1), ..self }
    }

    pub(super) fn dimens(self) -> TeXMathDimens {
        TeXMathDimens::for_style(self.style, self.base_font)
    }
}

#[derive(Debug, Clone)]
pub(super) enum BoxOp {
    Text { x: f64, baseline_y: f64, width: f64, span: RichSpan, font_size: f64, color: Rgba8 },
    Path { path: Path2D, fill: Option<FillPaint>, stroke: Option<StrokePaint> },
}

#[derive(Debug, Clone)]
pub(super) struct LayoutBox {
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) depth: f64,
    pub(super) is_single_char: bool,
    pub(super) italic_end: bool,
    pub(super) ops: Vec<BoxOp>,
}

impl LayoutBox {
    fn empty() -> Self {
        Self { width: 0.0, height: 0.0, depth: 0.0, is_single_char: false, italic_end: false, ops: Vec::new() }
    }

    fn push_shifted(&mut self, other: &Self, dx: f64, dy: f64) {
        for op in &other.ops {
            match op {
                BoxOp::Text { x, baseline_y, width, span, font_size, color } => {
                    self.ops.push(BoxOp::Text {
                        x: x + dx,
                        baseline_y: baseline_y + dy,
                        width: *width,
                        span: span.clone(),
                        font_size: *font_size,
                        color: *color,
                    });
                }
                BoxOp::Path { path, fill, stroke } => {
                    self.ops.push(BoxOp::Path { path: translate_path(path, dx, dy), fill: *fill, stroke: *stroke });
                }
            }
        }
    }
}

pub(super) fn translate_path(path: &Path2D, dx: f64, dy: f64) -> Path2D {
    let shift = |p: PointPt| PointPt::new(p.x + dx, p.y + dy);
    let commands = path
        .commands
        .iter()
        .map(|cmd| match *cmd {
            PathCmd::MoveTo(p) => PathCmd::MoveTo(shift(p)),
            PathCmd::LineTo(p) => PathCmd::LineTo(shift(p)),
            PathCmd::QuadTo { control, to } => PathCmd::QuadTo { control: shift(control), to: shift(to) },
            PathCmd::CubicTo { c1, c2, to } => PathCmd::CubicTo { c1: shift(c1), c2: shift(c2), to: shift(to) },
            PathCmd::Close => PathCmd::Close,
        })
        .collect();
    Path2D { commands }
}

pub(super) fn layout_nodes(nodes: &[MathNode], ctx: LayoutCtx) -> LayoutBox {
    if nodes.is_empty() || ctx.depth >= MAX_TEX_DEPTH {
        return LayoutBox::empty();
    }
    if nodes.len() == 1
        && let Some(single) = nodes.first()
    {
        return layout_single_node(single, ctx);
    }

    let d = ctx.dimens();
    let mut out = LayoutBox { width: 0.0, height: 0.15 * d.em, depth: 0.15 * d.em, is_single_char: false, italic_end: false, ops: Vec::new() };
    for node in nodes {
        let child = layout_single_node(node, ctx);
        let child_x = out.width;
        out.width += child.width;
        out.height = out.height.max(child.height);
        out.depth = out.depth.max(child.depth);
        out.italic_end = child.italic_end;
        out.push_shifted(&child, child_x, 0.0);
    }
    out
}

pub(super) fn layout_single_node(node: &MathNode, ctx: LayoutCtx) -> LayoutBox {
    if ctx.depth >= MAX_TEX_DEPTH {
        return LayoutBox::empty();
    }
    let d = ctx.dimens();
    match node {
        MathNode::Text { text, weight, slope, is_single_char } => layout_text_leaf(text, *weight, *slope, *is_single_char, ctx),
        MathNode::Space { width_em } => layout_space_node(*width_em, ctx),
        MathNode::ThinSpace => LayoutBox { width: 0.150 * d.em, ..LayoutBox::empty() },
        MathNode::Kern { width_em } => LayoutBox { width: (width_em * d.em).max(0.0), ..LayoutBox::empty() },
        MathNode::Group(inner) => layout_nodes(inner, ctx.with_style(ctx.style)),
        MathNode::Fraction { numerator, denominator, kind } => layout_fraction(numerator, denominator, *kind, ctx),
        MathNode::Radical { degree, radicand } => layout_radical(degree.as_deref(), radicand, ctx),
        MathNode::Delimited { left, inner, right, explicit } => layout_delimited(*left, inner, *right, *explicit, ctx),
        MathNode::Scripted { nucleus, sup, sub } => layout_scripted(nucleus, sup.as_deref(), sub.as_deref(), ctx),
    }
}

fn layout_text_leaf(text: &str, weight: SpanWeight, slope: SpanSlope, is_single_char: bool, ctx: LayoutCtx) -> LayoutBox {
    if text.is_empty() {
        return LayoutBox::empty();
    }
    let d = ctx.dimens();
    let span = RichSpan { text: text.to_owned(), script: SpanScript::Baseline, weight, slope, color: None, font_family: None, scale: 1.0 };
    let rich = RichText { spans: vec![span.clone()] };
    let m = measure_text_pt(&rich, d.em);
    let width = if m.width_pt.is_finite() { m.width_pt.max(0.0) } else { 0.0 };
    let height = if m.ascent_pt.is_finite() && m.ascent_pt > 0.0 { m.ascent_pt } else { 0.80 * d.em };
    let depth = if m.descent_pt.is_finite() && m.descent_pt >= 0.0 { m.descent_pt } else { 0.20 * d.em };
    LayoutBox {
        width,
        height,
        depth,
        is_single_char,
        italic_end: slope == SpanSlope::Italic,
        ops: vec![BoxOp::Text { x: 0.0, baseline_y: 0.0, width, span, font_size: d.em, color: ctx.color }],
    }
}

fn layout_space_node(width_em: f64, ctx: LayoutCtx) -> LayoutBox {
    let d = ctx.dimens();
    let target_w = (width_em * d.em).max(0.0);
    if target_w <= 0.0 {
        return LayoutBox::empty();
    }
    // Emit an ASCII space span so adjacent baseline tokens coalesce into one
    // `PositionedText` with real spaces (preserving SVG/PDF text copy/search).
    let space_span = RichSpan {
        text: " ".to_owned(),
        script: SpanScript::Baseline,
        weight: SpanWeight::Regular,
        slope: SpanSlope::Upright,
        color: None,
        font_family: None,
        scale: 1.0,
    };
    let space_w = measure_text_pt(&RichText { spans: vec![space_span.clone()] }, d.em).width_pt;
    if space_w.is_finite() && (target_w - space_w).abs() <= 0.05 * d.em {
        LayoutBox {
            width: space_w,
            height: 0.0,
            depth: 0.0,
            is_single_char: false,
            italic_end: false,
            ops: vec![BoxOp::Text { x: 0.0, baseline_y: 0.0, width: space_w, span: space_span, font_size: d.em, color: ctx.color }],
        }
    } else {
        LayoutBox { width: target_w, ..LayoutBox::empty() }
    }
}

/// Lay out a stacked fraction or binomial according to Knuth Appendix G Rules 15a–15e.
fn layout_fraction(num_nodes: &[MathNode], den_nodes: &[MathNode], kind: FractionKind, ctx: LayoutCtx) -> LayoutBox {
    let eff_style = match kind {
        FractionKind::Display => MathStyle::Display,
        FractionKind::Text => MathStyle::Text,
        FractionKind::Auto | FractionKind::Binomial => ctx.style,
    };
    let eff_ctx = LayoutCtx { style: eff_style, ..ctx };
    let d = eff_ctx.dimens();

    let num_box = layout_nodes(num_nodes, eff_ctx.with_style(eff_style.frac_numerator_style()));
    let den_box = layout_nodes(den_nodes, eff_ctx.with_style(eff_style.frac_denominator_style()));

    let has_rule = kind != FractionKind::Binomial;
    let theta = if has_rule { d.theta } else { 0.0 };
    let mut u = if eff_style.is_display() {
        d.num1
    } else if has_rule {
        d.num2
    } else {
        d.num3
    };
    let mut v = if eff_style.is_display() { d.denom1 } else { d.denom2 };

    if has_rule {
        // Appendix G Rule 15d: ensure clearance >= 3*theta (display) or >= theta (non-display)
        // between numerator/denominator boxes and the vinculum centered on math axis `sigma_22`.
        let clr = if eff_style.is_display() { 3.0 * theta } else { theta };
        u = u.max(d.axis + 0.5 * theta + clr + num_box.depth);
        v = v.max(clr + den_box.height - d.axis + 0.5 * theta);
    } else {
        // Appendix G Rule 15c: rule-less fraction (\binom) clearance >= 7*theta (display) or 3*theta.
        let clr = if eff_style.is_display() { 7.0 * d.theta } else { 3.0 * d.theta };
        let gap = (u - num_box.depth) - (den_box.height - v);
        if gap < clr {
            let half = 0.5 * (clr - gap);
            u += half;
            v += half;
        }
    }

    // Appendix G Rule 15e: center numerator and denominator over bar of width `max(w_num, w_den) + pad`,
    // flanked by `\nulldelimiterspace`.
    let overhang = (1.5 * d.theta).max(0.8);
    let col_w = num_box.width.max(den_box.width) + 2.0 * overhang;
    let total_w = d.null_delim_space + col_w + d.null_delim_space;
    let num_x = d.null_delim_space + 0.5 * (col_w - num_box.width);
    let den_x = d.null_delim_space + 0.5 * (col_w - den_box.width);

    let mut inner_box = LayoutBox {
        width: total_w,
        height: (u + num_box.height).max(d.axis + 0.5 * theta),
        depth: (v + den_box.depth).max((0.5 * theta - d.axis).max(0.0)),
        is_single_char: false,
        italic_end: false,
        ops: Vec::new(),
    };
    inner_box.push_shifted(&num_box, num_x, -u);
    if has_rule {
        let rule_y = -d.axis;
        inner_box.ops.push(BoxOp::Path {
            path: Path2D::line(PointPt::new(d.null_delim_space, rule_y), PointPt::new(d.null_delim_space + col_w, rule_y)),
            fill: None,
            stroke: Some(StrokePaint::solid(ctx.color, theta)),
        });
    }
    inner_box.push_shifted(&den_box, den_x, v);

    if kind == FractionKind::Binomial { wrap_box_with_delimiters(inner_box, DelimKind::Paren, DelimKind::Paren, true, eff_ctx) } else { inner_box }
}

/// Lay out a radical `\sqrt{...}` (or `\sqrt[n]{...}`) per Knuth Appendix G Rule 11 (p. 443).
fn layout_radical(degree: Option<&[MathNode]>, radicand: &[MathNode], ctx: LayoutCtx) -> LayoutBox {
    let d = ctx.dimens();
    let rad_raw = layout_nodes(radicand, ctx.with_style(ctx.style.cramp()));
    let rad_box = if rad_raw.width <= 0.0 && rad_raw.ops.is_empty() {
        LayoutBox { width: 0.40 * d.em, height: 0.72 * d.em, depth: 0.10 * d.em, ..LayoutBox::empty() }
    } else {
        rad_raw
    };

    // Appendix G Rule 11: phi = theta + 1/4 * psi (psi = x_height in display, else theta).
    let psi = if ctx.style.is_display() { d.x_h } else { d.theta };
    let phi = d.theta + 0.25 * psi;
    let y_bar_bottom = -rad_box.height - phi;
    let y_bar_center = y_bar_bottom - 0.5 * d.theta;
    let y_bar_top = y_bar_center - 0.5 * d.theta;
    let y_surd_bottom = (rad_box.depth + 0.5 * phi).max(0.20 * d.em);
    let rad_h = y_surd_bottom - y_bar_center;

    let w_surd = (0.55 * d.em).max((0.28 * rad_h).min(1.10 * d.em));
    let deg_box = degree.map(|nodes| {
        let deg_style = ctx.style.superscript_style().superscript_style();
        layout_nodes(nodes, LayoutCtx { style: deg_style, depth: ctx.depth + 1, ..ctx })
    });

    let base_left_pad = 0.06 * d.em;
    let left_pad = deg_box.as_ref().map_or(base_left_pad, |b| base_left_pad.max(b.width - 0.22 * w_surd + 0.06 * d.em));
    let overhang = (0.12 * d.em).max(1.5 * d.theta);
    let x_join = left_pad + w_surd;
    let x_rad = x_join + 0.06 * d.em;
    let x_end = x_rad + rad_box.width + overhang;

    // Radical surd polyline starting at left hook and ascending to the overbar join point `(x_join, y_bar_center)`.
    let y_hook = y_bar_center + 0.56 * rad_h;
    let surd_path = Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(left_pad + 0.04 * w_surd, y_hook)),
            PathCmd::LineTo(PointPt::new(left_pad + 0.24 * w_surd, y_hook - 0.08 * rad_h)),
            PathCmd::LineTo(PointPt::new(left_pad + 0.50 * w_surd, y_surd_bottom)),
            PathCmd::LineTo(PointPt::new(x_join, y_bar_center)),
        ],
    };
    let surd_stroke = StrokePaint { cap: LineCap::Round, join: LineJoin::Round, ..StrokePaint::solid(ctx.color, d.theta) };
    let overbar_path = Path2D::line(PointPt::new(x_join, y_bar_center), PointPt::new(x_end, y_bar_center));

    let mut out = LayoutBox {
        width: x_end + 0.06 * d.em,
        height: (-y_bar_top).max(rad_box.height + phi + d.theta),
        depth: (y_surd_bottom + 0.5 * d.theta).max(rad_box.depth),
        is_single_char: false,
        italic_end: false,
        ops: vec![
            BoxOp::Path { path: surd_path, fill: None, stroke: Some(surd_stroke) },
            BoxOp::Path { path: overbar_path, fill: None, stroke: Some(StrokePaint::solid(ctx.color, d.theta)) },
        ],
    };
    if let Some(db) = deg_box {
        let deg_x = (left_pad + 0.26 * w_surd - db.width).max(0.02 * d.em);
        let deg_baseline_y = y_hook - 0.14 * rad_h - db.depth;
        out.height = out.height.max(-(deg_baseline_y - db.height));
        out.push_shifted(&db, deg_x, deg_baseline_y);
    }
    out.push_shifted(&rad_box, x_rad, 0.0);
    out
}

/// Lay out a delimited group `( ... )`, `[ ... ]`, `\left...\right` per Knuth Appendix G Rule 19 (p. 446).
fn layout_delimited(left: DelimKind, inner: &[MathNode], right: DelimKind, explicit: bool, ctx: LayoutCtx) -> LayoutBox {
    let inner_box = layout_nodes(inner, ctx.with_style(ctx.style));
    wrap_box_with_delimiters(inner_box, left, right, explicit, ctx)
}

fn wrap_box_with_delimiters(inner_box: LayoutBox, left: DelimKind, right: DelimKind, _explicit: bool, ctx: LayoutCtx) -> LayoutBox {
    let d = ctx.dimens();
    let y_axis = -d.axis;
    let delta_above = (inner_box.height - d.axis).max(0.0);
    let delta_below = (inner_box.depth + d.axis).max(0.0);
    let delta = delta_above.max(delta_below);
    let need_scaled = 2.0 * delta > 1.25 * d.em;

    let (l_box, r_box) = if need_scaled {
        let half_span = (delta + 0.06 * d.em).max(0.65 * d.em);
        (build_scaled_delimiter(left, true, y_axis, half_span, d, ctx.color), build_scaled_delimiter(right, false, y_axis, half_span, d, ctx.color))
    } else {
        (delim_inline_pair_box(left, true, ctx), delim_inline_pair_box(right, false, ctx))
    };

    let mut out = LayoutBox {
        width: l_box.width + inner_box.width + r_box.width,
        height: l_box.height.max(inner_box.height).max(r_box.height),
        depth: l_box.depth.max(inner_box.depth).max(r_box.depth),
        is_single_char: false,
        italic_end: false,
        ops: Vec::new(),
    };
    out.push_shifted(&l_box, 0.0, 0.0);
    out.push_shifted(&inner_box, l_box.width, 0.0);
    out.push_shifted(&r_box, l_box.width + inner_box.width, 0.0);
    out
}

fn delim_inline_pair_box(kind: DelimKind, is_open: bool, ctx: LayoutCtx) -> LayoutBox {
    let d = ctx.dimens();
    let s = match (kind, is_open) {
        (DelimKind::Paren, true) => "(",
        (DelimKind::Paren, false) => ")",
        (DelimKind::Bracket, true) => "[",
        (DelimKind::Bracket, false) => "]",
        (DelimKind::Brace, true) => "{",
        (DelimKind::Brace, false) => "}",
        (DelimKind::Pipe, _) => "|",
        (DelimKind::DoublePipe, _) => "\u{2016}",
        (DelimKind::Null, _) => return LayoutBox { width: d.null_delim_space, ..LayoutBox::empty() },
    };
    layout_text_leaf(s, SpanWeight::Regular, SpanSlope::Upright, true, ctx)
}

fn build_scaled_delimiter(kind: DelimKind, is_open: bool, y_axis: f64, half_span: f64, d: TeXMathDimens, color: Rgba8) -> LayoutBox {
    if kind == DelimKind::Null {
        return LayoutBox { width: d.null_delim_space, ..LayoutBox::empty() };
    }
    let y_top = y_axis - half_span;
    let y_bot = y_axis + half_span;
    let h_total = 2.0 * half_span;
    let stroke_w = (0.052 * d.em + 0.008 * h_total).clamp(0.45, 1.35);
    let pad = 0.08 * d.em;

    let (w, path, cap, join) = match kind {
        DelimKind::Paren => {
            let w = (0.28 * d.em + 0.06 * h_total).clamp(0.32 * d.em, 0.72 * d.em);
            let (x_tip, x_belly) = if is_open { (w - pad, pad) } else { (pad, w - pad) };
            let x_ctrl = x_belly + 0.06 * (x_tip - x_belly);
            let p = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(x_tip, y_top)),
                    PathCmd::CubicTo {
                        c1: PointPt::new(x_tip - 0.55 * (x_tip - x_belly), y_top + 0.14 * h_total),
                        c2: PointPt::new(x_ctrl, y_axis - 0.24 * h_total),
                        to: PointPt::new(x_belly, y_axis),
                    },
                    PathCmd::CubicTo {
                        c1: PointPt::new(x_ctrl, y_axis + 0.24 * h_total),
                        c2: PointPt::new(x_tip - 0.55 * (x_tip - x_belly), y_bot - 0.14 * h_total),
                        to: PointPt::new(x_tip, y_bot),
                    },
                ],
            };
            (w, p, LineCap::Round, LineJoin::Round)
        }
        DelimKind::Bracket => {
            let w = (0.28 * d.em + 0.03 * h_total).clamp(0.30 * d.em, 0.55 * d.em);
            let (x_spine, x_arm) = if is_open { (pad, w - 0.5 * pad) } else { (w - pad, 0.5 * pad) };
            let p = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(x_arm, y_top)),
                    PathCmd::LineTo(PointPt::new(x_spine, y_top)),
                    PathCmd::LineTo(PointPt::new(x_spine, y_bot)),
                    PathCmd::LineTo(PointPt::new(x_arm, y_bot)),
                ],
            };
            (w, p, LineCap::Butt, LineJoin::Miter)
        }
        DelimKind::Brace => {
            let w = (0.34 * d.em + 0.05 * h_total).clamp(0.36 * d.em, 0.72 * d.em);
            let (x_end, x_stem, x_cusp) = if is_open { (w - pad, 0.52 * w, pad) } else { (pad, 0.48 * w, w - pad) };
            let p = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(x_end, y_top)),
                    PathCmd::QuadTo { control: PointPt::new(x_stem, y_top), to: PointPt::new(x_stem, y_top + 0.16 * h_total) },
                    PathCmd::LineTo(PointPt::new(x_stem, y_axis - 0.12 * h_total)),
                    PathCmd::QuadTo { control: PointPt::new(x_stem, y_axis), to: PointPt::new(x_cusp, y_axis) },
                    PathCmd::QuadTo { control: PointPt::new(x_stem, y_axis), to: PointPt::new(x_stem, y_axis + 0.12 * h_total) },
                    PathCmd::LineTo(PointPt::new(x_stem, y_bot - 0.16 * h_total)),
                    PathCmd::QuadTo { control: PointPt::new(x_stem, y_bot), to: PointPt::new(x_end, y_bot) },
                ],
            };
            (w, p, LineCap::Round, LineJoin::Round)
        }
        DelimKind::Pipe => {
            let w = (0.28 * d.em).max(2.2);
            let x_mid = 0.5 * w;
            let p = Path2D::line(PointPt::new(x_mid, y_top), PointPt::new(x_mid, y_bot));
            (w, p, LineCap::Butt, LineJoin::Miter)
        }
        DelimKind::DoublePipe => {
            let w = (0.42 * d.em).max(3.6);
            let gap = (0.14 * d.em).max(1.2);
            let x1 = 0.5 * (w - gap);
            let x2 = 0.5 * (w + gap);
            let p = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(x1, y_top)),
                    PathCmd::LineTo(PointPt::new(x1, y_bot)),
                    PathCmd::MoveTo(PointPt::new(x2, y_top)),
                    PathCmd::LineTo(PointPt::new(x2, y_bot)),
                ],
            };
            (w, p, LineCap::Butt, LineJoin::Miter)
        }
        DelimKind::Null => unreachable_zero_box(),
    };

    let height = (-y_top + 0.5 * stroke_w).max(0.0);
    let depth = (y_bot + 0.5 * stroke_w).max(0.0);
    LayoutBox {
        width: w,
        height,
        depth,
        is_single_char: false,
        italic_end: false,
        ops: vec![BoxOp::Path { path, fill: None, stroke: Some(StrokePaint { cap, join, ..StrokePaint::solid(color, stroke_w) }) }],
    }
}

fn unreachable_zero_box() -> (f64, Path2D, LineCap, LineJoin) {
    (0.0, Path2D::new(), LineCap::Butt, LineJoin::Miter)
}

/// Lay out superscript and/or subscript attached to `nucleus` per Knuth Appendix G Rules 18a–18f (p. 445).
fn layout_scripted(nucleus: &MathNode, sup: Option<&[MathNode]>, sub: Option<&[MathNode]>, ctx: LayoutCtx) -> LayoutBox {
    let nuc_ctx = if matches!(nucleus, MathNode::Scripted { .. }) { ctx.with_style(ctx.style) } else { ctx };
    let nuc_box = layout_single_node(nucleus, nuc_ctx);
    if sup.is_none() && sub.is_none() {
        return nuc_box;
    }

    let d = ctx.dimens();
    let sup_box = sup.map(|nodes| layout_nodes(nodes, ctx.with_style(ctx.style.superscript_style())));
    let sub_box = sub.map(|nodes| layout_nodes(nodes, ctx.with_style(ctx.style.subscript_style())));

    // Rule 18a: zero initial shift for simple single-char nuclei; box-drop for composite nuclei.
    let (mut u, mut v) =
        if nuc_box.is_single_char { (0.0, 0.0) } else { ((nuc_box.height - d.sup_drop).max(0.0), (nuc_box.depth + d.sub_drop).max(0.0)) };

    // Rule 18b: subscript only.
    if sup_box.is_none()
        && let Some(sb) = &sub_box
    {
        v = v.max(d.sub1).max(sb.height - 0.8 * d.x_h);
    }

    // Rule 18c: superscript present.
    if let Some(sp) = &sup_box {
        let p = if ctx.style.is_cramped() {
            d.sup3
        } else if ctx.style.is_display() {
            d.sup1
        } else {
            d.sup2
        };
        u = u.max(p).max(sp.depth + 0.25 * d.x_h);
    }

    // Rules 18d & 18e: both superscript and subscript present — enforce >= 4*theta separation.
    if let (Some(sp), Some(sb)) = (&sup_box, &sub_box) {
        v = v.max(d.sub2);
        let gap = (u - sp.depth) - (sb.height - v);
        let min_gap = 4.0 * d.theta;
        if gap < min_gap {
            v += min_gap - gap;
            let psi = 0.8 * d.x_h - (u - sp.depth);
            if psi > 0.0 {
                u += psi;
                v -= psi;
            }
        }
    }

    // Rule 18f: assemble nucleus, superscript (with italic correction), subscript, and scriptspace.
    let ic = if nuc_box.italic_end { 0.045 * d.em } else { 0.0 };
    let sup_w = sup_box.as_ref().map_or(0.0, |b| ic + b.width);
    let sub_w = sub_box.as_ref().map_or(0.0, |b| b.width);
    let script_w = sup_w.max(sub_w) + d.script_space;

    let mut out = LayoutBox {
        width: nuc_box.width + script_w,
        height: nuc_box.height.max(sup_box.as_ref().map_or(0.0, |b| u + b.height)),
        depth: nuc_box.depth.max(sub_box.as_ref().map_or(0.0, |b| v + b.depth)),
        is_single_char: false,
        italic_end: false,
        ops: Vec::new(),
    };
    out.push_shifted(&nuc_box, 0.0, 0.0);
    if let Some(sp) = &sup_box {
        out.push_shifted(sp, nuc_box.width + ic, -u);
    }
    if let Some(sb) = &sub_box {
        out.push_shifted(sb, nuc_box.width, v);
    }
    out
}
