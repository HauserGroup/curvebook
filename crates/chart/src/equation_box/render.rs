//! Model equation box rendering, line wrapping, scene primitive emission, and hit-bounds resolution.
//!
//! Renders 2D math [`super::layout::LayoutBox`] trees (built per Knuth, D. E. (1984),
//! *The TeXbook*, Addison-Wesley, Chapter 17 & Appendix G) into [`crate::scene::ChartScene`]
//! primitives with span coalescing and auto-sized bordered/borderless annotation frames.

use curvebook_doc::ProjectDocument;
use curvebook_palette::Rgba8;
use curvebook_text::{RichSpan, RichText, SpanSlope, SpanWeight, measure_text_pt, parse_markup};

use super::layout::{BoxOp, LayoutBox, LayoutCtx, MathStyle, layout_nodes, layout_single_node, translate_path};
use super::parser::{DelimKind, MathNode, parse_math_nodes};
use super::{DEFAULT_FONT_SIZE_PT, ModelEquationParams, ResolvedEquation, resolve_model_equation};
use crate::scene::{ChartScene, FillPaint, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};

/// Compile and append primitives for a `GraphAnnotation::ModelEquation` into `scene`.
pub fn render_model_equation_annotation(scene: &mut ChartScene, doc: &ProjectDocument, params: &ModelEquationParams<'_>) {
    let safe_font = sanitize_font_size(params.font_size_pt);
    let bounds = RectPt::from(params.bounds_pt);

    match resolve_model_equation(doc, params.result_sheet_id, params.data_set_index, params.mode) {
        ResolvedEquation::Empty => {}
        ResolvedEquation::Placeholder(label) => {
            let _ = render_placeholder_box(scene, bounds, &label, safe_font, params.bordered);
        }
        ResolvedEquation::Tex(tex) => {
            if tex.trim().is_empty() {
                return;
            }
            let _ = render_tex_box(scene, bounds, &tex, safe_font, params.bordered);
        }
    }
}

/// Directly compile and append 2D TeX math primitives for `tex` into `scene` inside `bounds_pt`.
///
/// Uses the same Knuth Appendix G two-dimensional mathematical layout engine as
/// [`render_model_equation_annotation`].
pub fn render_tex_equation_box(scene: &mut ChartScene, bounds_pt: curvebook_doc::RectPt, tex: &str, font_size_pt: f64, bordered: bool) {
    if tex.trim().is_empty() {
        return;
    }
    let _ = render_tex_box(scene, RectPt::from(bounds_pt), tex, sanitize_font_size(font_size_pt), bordered);
}

fn path_bbox_pt(path: &Path2D) -> Option<RectPt> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut include = |p: PointPt| {
        if p.x.is_finite() && p.y.is_finite() {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    };
    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => include(p),
            PathCmd::QuadTo { control, to } => {
                include(control);
                include(to);
            }
            PathCmd::CubicTo { c1, c2, to } => {
                include(c1);
                include(c2);
                include(to);
            }
            PathCmd::Close => {}
        }
    }
    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some(RectPt::new(min_x, min_y, (max_x - min_x).max(0.0), (max_y - min_y).max(0.0)))
    } else {
        None
    }
}

/// Resolve the effective hit-test bounding box for a [`curvebook_doc::GraphAnnotation::ModelEquation`].
///
/// When either `bounds_pt.width` or `bounds_pt.height` is non-positive (auto-sized equation box),
/// recovers the compiled box dimensions from `scene.primitives` (background box, overflow clip rect,
/// or laid-out equation runs anchored at `(bounds_pt.x, bounds_pt.y)`).
#[must_use]
pub fn resolve_model_equation_hit_bounds(scene: &ChartScene, bounds_pt: curvebook_doc::RectPt, font_size_pt: f64, bordered: bool) -> Option<RectPt> {
    let b = RectPt::from(bounds_pt);
    if b.width > 0.0 && b.height > 0.0 {
        return Some(b);
    }
    let safe_font = sanitize_font_size(font_size_pt);
    let (pad_x, pad_y) = sanitize_padding(safe_font);
    let matches_fixed = |r: RectPt| -> bool {
        (r.x - b.x).abs() <= 0.5
            && (r.y - b.y).abs() <= 0.5
            && r.width > 0.0
            && r.height > 0.0
            && (b.width <= 0.0 || (r.width - b.width).abs() <= 0.5)
            && (b.height <= 0.0 || (r.height - b.height).abs() <= 0.5)
    };

    for prim in scene.primitives.iter().rev() {
        match prim {
            ScenePrimitive::PushClip { rect } if matches_fixed(*rect) => return Some(*rect),
            ScenePrimitive::Path { path, fill: Some(fill), stroke: Some(stroke), clip: None }
                if bordered && fill.color == Rgba8::WHITE && stroke.color == Rgba8::BLACK && (stroke.width_pt - 0.8).abs() <= 1e-2 =>
            {
                if path.commands.len() == 5
                    && let Some(r) = path_bbox_pt(path)
                    && matches_fixed(r)
                {
                    return Some(r);
                }
            }
            _ => {}
        }
    }

    let min_font = 0.45 * safe_font;
    let max_font = safe_font + 0.05;
    let max_start_y = if b.height > 0.0 { b.y + b.height + pad_y } else { b.y + pad_y + 3.0 * safe_font };
    let is_eq_text = |t: &PositionedText| -> bool {
        t.anchor == TextAnchor::Start
            && t.rotation_deg.abs() <= 1e-6
            && (min_font..=max_font).contains(&t.font_size_pt)
            && t.anchor_pt.x >= b.x + pad_x - 0.5
            && t.anchor_pt.y >= b.y + pad_y * 0.4
    };
    let start_i = scene.primitives.iter().position(|prim| match prim {
        ScenePrimitive::Text(t) => is_eq_text(t) && t.anchor_pt.x <= b.x + pad_x + 4.0 * safe_font && t.anchor_pt.y <= max_start_y,
        _ => false,
    })?;

    let mut max_x = b.x + pad_x;
    let mut max_y = b.y + pad_y;
    let scan_from = if start_i > 0
        && matches!(
            &scene.primitives[start_i - 1],
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None }
                if path_bbox_pt(path).is_some_and(|r| r.x >= b.x + pad_x - 0.5 && r.y >= b.y)
        ) {
        start_i - 1
    } else {
        start_i
    };

    for prim in &scene.primitives[scan_from..] {
        match prim {
            ScenePrimitive::Text(t) if is_eq_text(t) => {
                if let Some(tb) = crate::scene::text_primitive_bounds_pt(t) {
                    max_x = max_x.max(tb.right());
                    max_y = max_y.max(tb.bottom());
                } else {
                    max_x = max_x.max(t.anchor_pt.x);
                    max_y = max_y.max(t.anchor_pt.y);
                }
            }
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } => {
                let Some(pb) = path_bbox_pt(path) else { break };
                if pb.x < b.x + pad_x - 0.5 || pb.y < b.y {
                    break;
                }
                max_x = max_x.max(pb.right());
                max_y = max_y.max(pb.bottom());
            }
            _ => break,
        }
    }

    let box_w = if b.width > 0.0 { b.width } else { (max_x + pad_x - b.x).max(2.0 * pad_x) };
    let box_h = if b.height > 0.0 { b.height } else { (max_y + pad_y - b.y).max(2.0 * pad_y) };
    Some(RectPt::new(b.x, b.y, box_w, box_h))
}

fn sanitize_font_size(font_size_pt: f64) -> f64 {
    if font_size_pt.is_finite() && font_size_pt > 0.0 { font_size_pt.clamp(4.0, 96.0) } else { DEFAULT_FONT_SIZE_PT }
}

fn sanitize_padding(font_size: f64) -> (f64, f64) {
    let pad_x = (font_size * 0.6).clamp(4.0, 10.0);
    let pad_y = (font_size * 0.45).clamp(3.0, 8.0);
    (pad_x, pad_y)
}

fn emit_box_background(scene: &mut ChartScene, box_rect: RectPt) {
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(box_rect),
        fill: Some(FillPaint::solid(Rgba8::WHITE)),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.8)),
        clip: None,
    });
}

fn render_placeholder_box(scene: &mut ChartScene, bounds: RectPt, label: &str, font_size: f64, bordered: bool) -> RectPt {
    let (pad_x, pad_y) = sanitize_padding(font_size);
    let rich = parse_markup(label);
    let m = measure_text_pt(&rich, font_size);
    let box_w = if bounds.width > 0.0 { bounds.width } else { m.width_pt + 2.0 * pad_x };
    let box_h = if bounds.height > 0.0 { bounds.height } else { (m.ascent_pt + m.descent_pt).max(font_size) + 2.0 * pad_y };
    let box_rect = RectPt::new(bounds.x, bounds.y, box_w, box_h);

    if bordered {
        emit_box_background(scene, box_rect);
    }

    let overflow =
        (bounds.width > 0.0 && m.width_pt + 2.0 * pad_x > box_w + 0.5) || (bounds.height > 0.0 && m.line_height_pt + 2.0 * pad_y > box_h + 0.5);
    if overflow {
        scene.primitives.push(ScenePrimitive::PushClip { rect: box_rect });
    }

    let content_h = (m.ascent_pt + m.descent_pt).max(font_size * 0.8);
    let top_y = box_rect.y + ((box_rect.height - content_h) * 0.5).max(pad_y);
    let baseline_y = top_y + m.ascent_pt;
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich,
        anchor_pt: PointPt::new(box_rect.x + pad_x, baseline_y),
        anchor: TextAnchor::Start,
        font_size_pt: font_size,
        rotation_deg: 0.0,
        color: Rgba8::rgb(120, 120, 120),
    }));

    if overflow {
        scene.primitives.push(ScenePrimitive::PopClip);
    }
    box_rect
}

fn render_tex_box(scene: &mut ChartScene, bounds: RectPt, tex: &str, font_size: f64, bordered: bool) -> Option<RectPt> {
    let nodes = parse_math_nodes(tex);
    if nodes.is_empty() {
        return None;
    }

    let root_ctx = LayoutCtx { style: MathStyle::Display, base_font: font_size, color: Rgba8::BLACK, depth: 0 };
    let (pad_x, pad_y) = sanitize_padding(font_size);
    let lines = if bounds.width > 0.0 {
        let max_inner_w = (bounds.width - 2.0 * pad_x).max(1.0);
        wrap_math_nodes(&nodes, root_ctx, max_inner_w)
    } else {
        vec![nodes]
    };

    let measured_lines: Vec<LayoutBox> = lines.iter().map(|line| layout_nodes(line, root_ctx)).collect();
    let line_gap = (font_size * 0.35).clamp(2.0, 6.0);
    let max_row_w = measured_lines.iter().map(|r| r.width).fold(0.0_f64, f64::max);
    let total_h: f64 = measured_lines.iter().map(|r| r.height + r.depth).sum::<f64>() + (measured_lines.len().saturating_sub(1) as f64) * line_gap;

    let box_w = if bounds.width > 0.0 { bounds.width } else { max_row_w + 2.0 * pad_x };
    let box_h = if bounds.height > 0.0 { bounds.height } else { total_h + 2.0 * pad_y };
    let box_rect = RectPt::new(bounds.x, bounds.y, box_w, box_h);

    if bordered {
        emit_box_background(scene, box_rect);
    }

    let overflow = (bounds.width > 0.0 && max_row_w + 2.0 * pad_x > box_w + 0.5) || (bounds.height > 0.0 && total_h + 2.0 * pad_y > box_h + 0.5);
    if overflow {
        scene.primitives.push(ScenePrimitive::PushClip { rect: box_rect });
    }

    let mut top_y = box_rect.y + ((box_rect.height - total_h) * 0.5).max(pad_y);
    let start_x = box_rect.x + pad_x;
    for row in &measured_lines {
        let baseline_y = top_y + row.height;
        emit_layout_box(scene, row, start_x, baseline_y);
        top_y += row.height + row.depth + line_gap;
    }

    if overflow {
        scene.primitives.push(ScenePrimitive::PopClip);
    }
    Some(box_rect)
}

// ---------------------------------------------------------------------------
// Line Wrapping & ScenePrimitive Emission
// ---------------------------------------------------------------------------

fn wrap_math_nodes(nodes: &[MathNode], ctx: LayoutCtx, max_width: f64) -> Vec<Vec<MathNode>> {
    let whole = layout_nodes(nodes, ctx);
    if whole.width <= max_width {
        return vec![nodes.to_vec()];
    }

    let flat = flatten_for_wrap(nodes, ctx);
    let mut lines: Vec<Vec<MathNode>> = Vec::new();
    let mut current: Vec<MathNode> = Vec::new();
    let mut current_w = 0.0_f64;
    let mut pending_spaces: Vec<MathNode> = Vec::new();
    let mut pending_w = 0.0_f64;

    for token in flat {
        if let MathNode::Space { .. } = token {
            let sw = layout_single_node(&token, ctx).width;
            pending_w += sw;
            pending_spaces.push(token);
            continue;
        }
        let tw = layout_single_node(&token, ctx).width;
        let stick_prev = matches!(token, MathNode::ThinSpace);
        if !stick_prev && !current.is_empty() && current_w + pending_w + tw > max_width {
            lines.push(current);
            current = Vec::new();
            current_w = 0.0;
            pending_spaces.clear();
            pending_w = 0.0;
        } else if !pending_spaces.is_empty() {
            current.append(&mut pending_spaces);
            current_w += pending_w;
            pending_w = 0.0;
        }
        current.push(token);
        current_w += tw;
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() { vec![nodes.to_vec()] } else { lines }
}

fn flatten_for_wrap(nodes: &[MathNode], ctx: LayoutCtx) -> Vec<MathNode> {
    enum FlattenItem<'a> {
        Node(&'a MathNode, usize),
        Literal(&'static str),
    }

    let mut out = Vec::new();
    let mut stack: Vec<FlattenItem<'_>> = nodes.iter().rev().map(|n| FlattenItem::Node(n, ctx.depth)).collect();
    while let Some(item) = stack.pop() {
        match item {
            FlattenItem::Literal(text) => {
                if !text.is_empty() {
                    out.push(MathNode::Text { text: text.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true });
                }
            }
            FlattenItem::Node(node, depth) => match node {
                MathNode::Group(inner) if depth < super::MAX_TEX_DEPTH => {
                    stack.extend(inner.iter().rev().map(|n| FlattenItem::Node(n, depth + 1)));
                }
                MathNode::Delimited { left, inner, right, explicit: false } if depth < super::MAX_TEX_DEPTH => {
                    let inner_ctx = LayoutCtx { depth: depth + 1, ..ctx };
                    let d = ctx.dimens();
                    let inner_box = layout_nodes(inner, inner_ctx);
                    let delta = (inner_box.height - d.axis).max(0.0).max((inner_box.depth + d.axis).max(0.0));
                    if 2.0 * delta <= 1.25 * d.em {
                        let l_text = match left {
                            DelimKind::Paren => "(",
                            DelimKind::Bracket => "[",
                            DelimKind::Brace => "{",
                            DelimKind::Pipe => "|",
                            DelimKind::DoublePipe => "\u{2016}",
                            DelimKind::Null => "",
                        };
                        let r_text = match right {
                            DelimKind::Paren => ")",
                            DelimKind::Bracket => "]",
                            DelimKind::Brace => "}",
                            DelimKind::Pipe => "|",
                            DelimKind::DoublePipe => "\u{2016}",
                            DelimKind::Null => "",
                        };
                        stack.push(FlattenItem::Literal(r_text));
                        stack.extend(inner.iter().rev().map(|n| FlattenItem::Node(n, depth + 1)));
                        stack.push(FlattenItem::Literal(l_text));
                    } else {
                        out.push(node.clone());
                    }
                }
                _ => out.push(node.clone()),
            },
        }
    }
    out
}

struct PendingText {
    start_x: f64,
    end_x: f64,
    baseline_y: f64,
    font_size: f64,
    color: Rgba8,
    spans: Vec<RichSpan>,
}

fn emit_layout_box(scene: &mut ChartScene, layout: &LayoutBox, origin_x: f64, baseline_y: f64) {
    let mut pending: Option<PendingText> = None;

    for op in &layout.ops {
        match op {
            BoxOp::Text { x, baseline_y: rel_y, width, span, font_size, color } => {
                let ax = origin_x + x;
                let ay = baseline_y + rel_y;
                if let Some(p) = &mut pending
                    && (ay - p.baseline_y).abs() < 1e-4
                    && (*font_size - p.font_size).abs() < 1e-4
                    && *color == p.color
                    && (ax - p.end_x).abs() < 1e-3
                {
                    push_coalesced_span(&mut p.spans, span.clone());
                    p.end_x = ax + width;
                    continue;
                }
                flush_pending_text(scene, &mut pending);
                pending = Some(PendingText {
                    start_x: ax,
                    end_x: ax + width,
                    baseline_y: ay,
                    font_size: *font_size,
                    color: *color,
                    spans: vec![span.clone()],
                });
            }
            BoxOp::Path { path, fill, stroke } => {
                flush_pending_text(scene, &mut pending);
                scene.primitives.push(ScenePrimitive::Path {
                    path: translate_path(path, origin_x, baseline_y),
                    fill: *fill,
                    stroke: *stroke,
                    clip: None,
                });
            }
        }
    }
    flush_pending_text(scene, &mut pending);
}

fn push_coalesced_span(spans: &mut Vec<RichSpan>, span: RichSpan) {
    if span.text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.weight == span.weight
        && last.slope == span.slope
        && last.script == span.script
        && last.color == span.color
        && last.font_family == span.font_family
        && (last.scale - span.scale).abs() < 1e-9
    {
        last.text.push_str(&span.text);
        return;
    }
    spans.push(span);
}

fn flush_pending_text(scene: &mut ChartScene, pending: &mut Option<PendingText>) {
    if let Some(p) = pending.take()
        && !p.spans.is_empty()
        && p.spans.iter().any(|s| !s.text.trim().is_empty())
    {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText { spans: p.spans },
            anchor_pt: PointPt::new(p.start_x, p.baseline_y),
            anchor: TextAnchor::Start,
            font_size_pt: p.font_size,
            rotation_deg: 0.0,
            color: p.color,
        }));
    }
}
