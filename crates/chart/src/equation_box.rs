//! Live TeX model equation box annotation compiler (`R9.16` / `QOL-16`).
//!
//! Resolves regression equations dynamically from the target [`ResultSheet`] in
//! [`ProjectDocument`], parses `\frac{...}{...}` and inline rich-text markup
//! (`_sub`, `^sup`, Greek letters, symbols), lays out stacked 2D fractions with
//! horizontal vinculum rules inside `bounds_pt`, and emits device-independent
//! [`ScenePrimitive`] geometry.

use lumencraft_doc::{AnalysisKind, EquationDisplayMode, ProjectDocument, ResultSheet, ResultStatus, SheetId};
use lumencraft_fit::tex_format::{format_model_equation_tex, format_polynomial_equation_tex};
use lumencraft_palette::Rgba8;
use lumencraft_text::{EquationSegment, RichSpan, RichText, SpanScript, measure_text_pt, parse_equation_segments, parse_markup};

use crate::scene::{ChartScene, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};

/// Fractional height of the typographic math axis above the baseline, matched to
/// the vector glyph metrics (`=` bars around 0.24/0.44 em, `+` crossbar at 0.34 em).
const MATH_AXIS_FRAC: f64 = 0.34;
const DEFAULT_FONT_SIZE_PT: f64 = 10.0;
const FRACTION_SCALE: f64 = 0.82;
const FRACTION_COL_PAD_PT: f64 = 4.0;
const FRACTION_GAP_PT: f64 = 2.0;
const EQUATION_SIG_FIGS: usize = 4;

/// Inputs required to compile a [`GraphAnnotation::ModelEquation`] annotation.
#[derive(Debug, Clone, Copy)]
pub struct ModelEquationParams<'a> {
    /// Target regression [`ResultSheet`] ID in the [`ProjectDocument`].
    pub result_sheet_id: &'a SheetId,
    /// Zero-based dataset index within the result sheet.
    pub data_set_index: usize,
    /// Symbolic parameter names vs live substituted coefficient values.
    pub mode: EquationDisplayMode,
    /// Annotation bounding box in points (zero width/height triggers auto-sizing).
    pub bounds_pt: lumencraft_doc::RectPt,
    /// Base font size in points for top-level equation runs.
    pub font_size_pt: f64,
    /// Whether to draw a white filled background and 0.8 pt black border.
    pub bordered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedEquation {
    Tex(String),
    Placeholder(String),
    Empty,
}

#[derive(Debug, Clone)]
enum MeasuredNode {
    Inline { rich: RichText, width: f64, font_size: f64 },
    Fraction { num: MeasuredRow, den: MeasuredRow, width: f64, above: f64, below: f64, bar_gap: f64, stroke_w: f64 },
}

#[derive(Debug, Clone)]
struct MeasuredRow {
    nodes: Vec<MeasuredNode>,
    width: f64,
    above: f64,
    below: f64,
}

#[derive(Debug, Clone)]
enum EqToken {
    Word(RichText),
    Space(RichSpan),
    Fraction(Vec<EquationSegment>),
}

/// Collect scalar parameters for `data_set_index` from `res`, preferring
/// dataset-specific entries and falling back to shared (`data_set_index: None`)
/// parameters for keys not already bound to the dataset.
#[must_use]
pub(crate) fn collect_dataset_scalars(res: &ResultSheet, data_set_index: usize) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();

    for scalar in &res.scalars {
        if scalar.data_set_index == Some(data_set_index) && scalar.value.is_finite() {
            let lower = scalar.key.to_ascii_lowercase();
            if !seen_keys.iter().any(|k| k == &lower) {
                seen_keys.push(lower);
                out.push((scalar.key.clone(), scalar.value));
            }
        }
    }

    for scalar in &res.scalars {
        if scalar.data_set_index.is_none() && scalar.value.is_finite() {
            let lower = scalar.key.to_ascii_lowercase();
            if !seen_keys.iter().any(|k| k == &lower) {
                seen_keys.push(lower);
                out.push((scalar.key.clone(), scalar.value));
            }
        }
    }

    out
}

/// Resolve the live TeX equation string (or diagnostic placeholder) from `doc`.
fn resolve_model_equation(doc: &ProjectDocument, result_sheet_id: &SheetId, data_set_index: usize, mode: EquationDisplayMode) -> ResolvedEquation {
    let Some(res) = doc.result_sheet(result_sheet_id) else {
        return ResolvedEquation::Placeholder("(Equation unavailable)".to_owned());
    };

    if let ResultStatus::Failed { message, .. } = &res.status {
        let label = if message.trim().is_empty() { "(Fit failed)".to_owned() } else { "(Fit failed — check analysis)".to_owned() };
        return ResolvedEquation::Placeholder(label);
    }

    let substituted = matches!(mode, EquationDisplayMode::Substituted);
    let params = collect_dataset_scalars(res, data_set_index);

    match &res.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) => {
            let overlay = res
                .fitted_curves
                .iter()
                .find(|c| c.data_set_index == data_set_index && !c.equation_id.trim().is_empty())
                .or_else(|| res.fitted_curves.iter().find(|c| !c.equation_id.trim().is_empty()));
            let equation_id = overlay.map(|c| c.equation_id.as_str()).unwrap_or(nl.equation_id.as_str());
            if equation_id.trim().is_empty() {
                return ResolvedEquation::Placeholder("(No nonlinear model selected)".to_owned());
            }
            let custom_dsl = if overlay.is_none() || equation_id == nl.equation_id { nl.custom_dsl.as_deref() } else { None };
            let tex = format_model_equation_tex(equation_id, custom_dsl, &params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::LinearRegression(lin) => {
            let through_origin = lin.force_through_origin && !lin.deming_model_ii;
            let lin_params = collect_linear_params(&params, through_origin);
            let tex = format_polynomial_equation_tex(1, through_origin, &lin_params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::PassingBablok(_) => {
            let pb_params = collect_linear_params(&params, false);
            if pb_params.is_empty() && substituted {
                return ResolvedEquation::Empty;
            }
            let tex = format_polynomial_equation_tex(1, false, &pb_params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        _ => {
            if let Some(curve) = res
                .fitted_curves
                .iter()
                .find(|c| c.data_set_index == data_set_index && !c.equation_id.trim().is_empty())
                .or_else(|| res.fitted_curves.iter().find(|c| !c.equation_id.trim().is_empty()))
            {
                let tex = format_model_equation_tex(&curve.equation_id, None, &params, substituted, EQUATION_SIG_FIGS);
                ResolvedEquation::Tex(tex)
            } else {
                ResolvedEquation::Empty
            }
        }
    }
}

/// Extract canonical `("Slope", ..)` and `("YIntercept", ..)` entries so
/// unrelated scalars like `"R2"` are never consumed positionally.
fn collect_linear_params(params: &[(String, f64)], force_through_origin: bool) -> Vec<(String, f64)> {
    let mut out = Vec::with_capacity(2);
    if let Some((_, slope)) = params.iter().find(|(k, _)| k.eq_ignore_ascii_case("Slope")) {
        out.push(("Slope".to_owned(), *slope));
    }
    if !force_through_origin
        && let Some((_, intercept)) = params.iter().find(|(k, _)| k.eq_ignore_ascii_case("YIntercept") || k.eq_ignore_ascii_case("Intercept"))
    {
        out.push(("YIntercept".to_owned(), *intercept));
    }
    out
}

/// Compile and append primitives for a `GraphAnnotation::ModelEquation` into `scene`.
pub fn render_model_equation_annotation(scene: &mut ChartScene, doc: &ProjectDocument, params: &ModelEquationParams<'_>) {
    let safe_font =
        if params.font_size_pt.is_finite() && params.font_size_pt > 0.0 { params.font_size_pt.clamp(4.0, 96.0) } else { DEFAULT_FONT_SIZE_PT };
    let bounds = RectPt::from(params.bounds_pt);

    match resolve_model_equation(doc, params.result_sheet_id, params.data_set_index, params.mode) {
        ResolvedEquation::Empty => {}
        ResolvedEquation::Placeholder(label) => {
            render_placeholder_box(scene, bounds, &label, safe_font, params.bordered);
        }
        ResolvedEquation::Tex(tex) => {
            if tex.trim().is_empty() {
                return;
            }
            render_tex_box(scene, bounds, &tex, safe_font, params.bordered);
        }
    }
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

fn render_placeholder_box(scene: &mut ChartScene, bounds: RectPt, label: &str, font_size: f64, bordered: bool) {
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
        bounds.width > 0.0 && bounds.height > 0.0 && (m.width_pt + 2.0 * pad_x > box_w + 0.5 || m.line_height_pt + 2.0 * pad_y > box_h + 0.5);
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
}

fn render_tex_box(scene: &mut ChartScene, bounds: RectPt, tex: &str, font_size: f64, bordered: bool) {
    let segments = parse_equation_segments(tex);
    if segments.is_empty() {
        return;
    }

    let (pad_x, pad_y) = sanitize_padding(font_size);
    let lines = if bounds.width > 0.0 {
        let max_inner_w = (bounds.width - 2.0 * pad_x).max(1.0);
        wrap_segments(&segments, font_size, max_inner_w)
    } else {
        vec![segments]
    };

    let measured_lines: Vec<MeasuredRow> = lines.iter().map(|line| measure_row(line, font_size, 0)).collect();

    let line_gap = (font_size * 0.35).clamp(2.0, 6.0);
    let max_row_w = measured_lines.iter().map(|r| r.width).fold(0.0_f64, f64::max);
    let total_h: f64 = measured_lines.iter().map(|r| r.above + r.below).sum::<f64>() + (measured_lines.len().saturating_sub(1) as f64) * line_gap;

    let box_w = if bounds.width > 0.0 { bounds.width } else { max_row_w + 2.0 * pad_x };
    let box_h = if bounds.height > 0.0 { bounds.height } else { total_h + 2.0 * pad_y };
    let box_rect = RectPt::new(bounds.x, bounds.y, box_w, box_h);

    if bordered {
        emit_box_background(scene, box_rect);
    }

    let overflow = bounds.width > 0.0 && bounds.height > 0.0 && (max_row_w + 2.0 * pad_x > box_w + 0.5 || total_h + 2.0 * pad_y > box_h + 0.5);
    if overflow {
        scene.primitives.push(ScenePrimitive::PushClip { rect: box_rect });
    }

    let mut top_y = box_rect.y + ((box_rect.height - total_h) * 0.5).max(pad_y);
    let start_x = box_rect.x + pad_x;
    for row in &measured_lines {
        let mid_y = top_y + row.above;
        emit_row(scene, row, start_x, mid_y, Rgba8::BLACK);
        top_y += row.above + row.below + line_gap;
    }

    if overflow {
        scene.primitives.push(ScenePrimitive::PopClip);
    }
}

fn measure_row(segments: &[EquationSegment], font_size: f64, depth: usize) -> MeasuredRow {
    let mut nodes = Vec::with_capacity(segments.len());
    let mut total_w = 0.0_f64;
    let mut max_above = 0.2 * font_size;
    let mut max_below = 0.2 * font_size;
    let mut prev_was_frac = false;

    for seg in segments {
        match seg {
            EquationSegment::Inline(rich) => {
                let m = measure_text_pt(rich, font_size);
                let width = if m.width_pt.is_finite() { m.width_pt.max(0.0) } else { 0.0 };
                let ascent = if m.ascent_pt.is_finite() { m.ascent_pt.max(0.0) } else { 0.75 * font_size };
                let descent = if m.descent_pt.is_finite() { m.descent_pt.max(0.0) } else { 0.22 * font_size };
                let axis = MATH_AXIS_FRAC * font_size;
                max_above = max_above.max((ascent - axis).max(0.15 * font_size));
                max_below = max_below.max((descent + axis).max(0.15 * font_size));
                total_w += width;
                nodes.push(MeasuredNode::Inline { rich: rich.clone(), width, font_size });
                prev_was_frac = false;
            }
            EquationSegment::Fraction { numerator, denominator } => {
                if prev_was_frac {
                    total_w += FRACTION_GAP_PT;
                }
                let frac = measure_fraction(numerator, denominator, font_size, depth);
                if let MeasuredNode::Fraction { width, above, below, .. } = &frac {
                    total_w += *width;
                    max_above = max_above.max(*above);
                    max_below = max_below.max(*below);
                }
                nodes.push(frac);
                prev_was_frac = true;
            }
        }
    }

    MeasuredRow { nodes, width: total_w, above: max_above, below: max_below }
}

fn measure_fraction(numerator: &[EquationSegment], denominator: &[EquationSegment], parent_font_size: f64, depth: usize) -> MeasuredNode {
    let sub_font = if depth < 2 { (parent_font_size * FRACTION_SCALE).max(4.0) } else { parent_font_size };
    let num = measure_row(numerator, sub_font, depth + 1);
    let den = measure_row(denominator, sub_font, depth + 1);
    let col_w = num.width.max(den.width) + FRACTION_COL_PAD_PT;
    let bar_gap = (parent_font_size * 0.16).clamp(1.2, 3.0);
    let stroke_w = (parent_font_size * 0.075).clamp(0.5, 1.4);
    let above = bar_gap + num.above + num.below;
    let below = bar_gap + den.above + den.below;
    MeasuredNode::Fraction { num, den, width: col_w, above, below, bar_gap, stroke_w }
}

fn emit_row(scene: &mut ChartScene, row: &MeasuredRow, start_x: f64, mid_y: f64, color: Rgba8) {
    let mut cursor_x = start_x;
    let mut prev_was_frac = false;

    for node in &row.nodes {
        match node {
            MeasuredNode::Inline { rich, width, font_size, .. } => {
                if !rich.spans.is_empty() && rich.spans.iter().any(|s| !s.text.is_empty()) {
                    let baseline_y = mid_y + MATH_AXIS_FRAC * *font_size;
                    scene.primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: rich.clone(),
                        anchor_pt: PointPt::new(cursor_x, baseline_y),
                        anchor: TextAnchor::Start,
                        font_size_pt: *font_size,
                        rotation_deg: 0.0,
                        color,
                    }));
                }
                cursor_x += *width;
                prev_was_frac = false;
            }
            MeasuredNode::Fraction { num, den, width, bar_gap, stroke_w, .. } => {
                if prev_was_frac {
                    cursor_x += FRACTION_GAP_PT;
                }
                let num_x = cursor_x + 0.5 * (*width - num.width);
                let num_mid_y = mid_y - *bar_gap - num.below;
                emit_row(scene, num, num_x, num_mid_y, color);

                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(cursor_x, mid_y), PointPt::new(cursor_x + *width, mid_y)),
                    fill: None,
                    stroke: Some(StrokePaint::solid(color, *stroke_w)),
                    clip: None,
                });

                let den_x = cursor_x + 0.5 * (*width - den.width);
                let den_mid_y = mid_y + *bar_gap + den.above;
                emit_row(scene, den, den_x, den_mid_y, color);

                cursor_x += *width;
                prev_was_frac = true;
            }
        }
    }
}

fn wrap_segments(segments: &[EquationSegment], font_size: f64, max_width: f64) -> Vec<Vec<EquationSegment>> {
    let total = measure_row(segments, font_size, 0);
    if total.width <= max_width {
        return vec![segments.to_vec()];
    }

    let tokens = tokenize_segments(segments);
    let mut lines: Vec<Vec<EquationSegment>> = Vec::new();
    let mut current_line: Vec<EquationSegment> = Vec::new();
    let mut current_w = 0.0_f64;
    let mut pending_spaces: Vec<RichSpan> = Vec::new();
    let mut pending_w = 0.0_f64;

    for token in tokens {
        match token {
            EqToken::Space(span) => {
                let sw = measure_text_pt(&RichText { spans: vec![span.clone()] }, font_size).width_pt;
                pending_w += sw;
                pending_spaces.push(span);
            }
            EqToken::Word(rich) => {
                let ww = measure_text_pt(&rich, font_size).width_pt;
                if !current_line.is_empty() && current_w + pending_w + ww > max_width {
                    lines.push(current_line);
                    current_line = Vec::new();
                    current_w = 0.0;
                    pending_spaces.clear();
                    pending_w = 0.0;
                } else if !pending_spaces.is_empty() {
                    push_inline_spans(&mut current_line, std::mem::take(&mut pending_spaces));
                    current_w += pending_w;
                    pending_w = 0.0;
                }
                push_inline_spans(&mut current_line, rich.spans);
                current_w += ww;
            }
            EqToken::Fraction(segs) => {
                let frac_row = measure_row(&segs, font_size, 0);
                let fw = frac_row.width;
                if !current_line.is_empty() && current_w + pending_w + fw > max_width {
                    lines.push(current_line);
                    current_line = Vec::new();
                    current_w = 0.0;
                    pending_spaces.clear();
                    pending_w = 0.0;
                } else if !pending_spaces.is_empty() {
                    push_inline_spans(&mut current_line, std::mem::take(&mut pending_spaces));
                    current_w += pending_w;
                    pending_w = 0.0;
                }
                if let Some(seg) = segs.into_iter().next() {
                    current_line.push(seg);
                    current_w += fw;
                }
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() { vec![segments.to_vec()] } else { lines }
}

fn push_inline_spans(line: &mut Vec<EquationSegment>, spans: Vec<RichSpan>) {
    if spans.is_empty() {
        return;
    }
    if let Some(EquationSegment::Inline(existing)) = line.last_mut() {
        existing.spans.extend(spans);
    } else {
        line.push(EquationSegment::Inline(RichText { spans }));
    }
}

fn tokenize_segments(segments: &[EquationSegment]) -> Vec<EqToken> {
    let mut tokens = Vec::new();
    for seg in segments {
        match seg {
            EquationSegment::Inline(rich) => append_inline_to_tokens(rich, &mut tokens),
            EquationSegment::Fraction { .. } => tokens.push(EqToken::Fraction(vec![seg.clone()])),
        }
    }
    tokens
}

fn append_inline_to_tokens(rich: &RichText, tokens: &mut Vec<EqToken>) {
    let mut word_spans: Vec<RichSpan> = Vec::new();

    let flush_word = |spans: &mut Vec<RichSpan>, out: &mut Vec<EqToken>| {
        if !spans.is_empty() {
            out.push(EqToken::Word(RichText { spans: std::mem::take(spans) }));
        }
    };

    for span in &rich.spans {
        if span.script != SpanScript::Baseline {
            word_spans.push(span.clone());
            continue;
        }
        let mut start = 0_usize;
        for (idx, ch) in span.text.char_indices() {
            if ch == ' ' {
                if idx > start {
                    let mut part = span.clone();
                    part.text = span.text[start..idx].to_owned();
                    word_spans.push(part);
                }
                flush_word(&mut word_spans, tokens);
                let mut sp = span.clone();
                sp.text = " ".to_owned();
                tokens.push(EqToken::Space(sp));
                start = idx + ch.len_utf8();
            }
        }
        if start < span.text.len() {
            let mut part = span.clone();
            part.text = span.text[start..].to_owned();
            word_spans.push(part);
        }
    }
    flush_word(&mut word_spans, tokens);
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, EquationDisplayMode, ErrorCode, FittedCurveOverlay, LinearRegressionSpec, NonlinearFitSpec, ProjectDocument,
        RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId,
    };

    use super::*;

    fn empty_scene() -> ChartScene {
        ChartScene::new("graph-1", "Test", 400.0, 300.0, None)
    }

    fn make_base_doc_with_result(sheet: ResultSheet) -> (ProjectDocument, SheetId) {
        let mut doc = ProjectDocument::new("EquationBoxTest").expect("ProjectDocument::new");
        let id = sheet.id.clone();
        doc.result_sheets.push(sheet);
        (doc, id)
    }

    fn make_nonlinear_result(
        id: &str,
        equation_id: &str,
        custom_dsl: Option<&str>,
        scalars: Vec<ResultScalarEntry>,
        overlays: Vec<FittedCurveOverlay>,
    ) -> ResultSheet {
        let mut nl = NonlinearFitSpec::new(equation_id);
        nl.custom_dsl = custom_dsl.map(str::to_owned);
        let spec = AnalysisSpec::new(id, "data-1", AnalysisKind::NonlinearRegression(nl)).expect("AnalysisSpec::new");
        let mut sheet = ResultSheet::new(spec).expect("ResultSheet::new");
        sheet.status = ResultStatus::UpToDate;
        sheet.scalars = scalars;
        sheet.fitted_curves = overlays;
        sheet
    }

    fn collect_text_strings(scene: &ChartScene) -> Vec<String> {
        scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some(t.rich.spans.iter().map(|s| s.text.as_str()).collect::<String>()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn four_pl_symbolic_and_substituted_emit_fraction_vinculum_and_live_numbers() {
        let scalars = vec![
            ResultScalarEntry { key: "Bottom".to_owned(), data_set_index: Some(0), value: 10.5, units: None },
            ResultScalarEntry { key: "Top".to_owned(), data_set_index: Some(0), value: 98.2, units: None },
            ResultScalarEntry { key: "LogEC50".to_owned(), data_set_index: Some(0), value: -6.25, units: None },
            ResultScalarEntry { key: "HillSlope".to_owned(), data_set_index: Some(0), value: 1.45, units: None },
            ResultScalarEntry { key: "R2".to_owned(), data_set_index: Some(0), value: 0.9987, units: None },
        ];
        let sheet = make_nonlinear_result("res-nl", "log_agonist_vs_response_4pl", None, scalars, Vec::new());
        let (mut doc, id) = make_base_doc_with_result(sheet);

        let mut sym_scene = empty_scene();
        render_model_equation_annotation(
            &mut sym_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Symbolic,
                bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 60.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let sym_texts = collect_text_strings(&sym_scene).join(" | ");
        assert!(sym_texts.contains("Bottom"), "sym_texts={sym_texts}");
        assert!(sym_texts.contains("Top"), "sym_texts={sym_texts}");
        assert!(sym_texts.contains("HillSlope"), "sym_texts={sym_texts}");
        assert!(!sym_texts.contains("R2"), "must not include extra R2 callout: {sym_texts}");

        let path_count = sym_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { .. })).count();
        assert!(path_count >= 2, "expected box border + vinculum path, got {path_count}");

        let mut sub_scene = empty_scene();
        render_model_equation_annotation(
            &mut sub_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 60.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let sub_texts = collect_text_strings(&sub_scene).join(" | ");
        assert!(sub_texts.contains("10.5"), "sub_texts={sub_texts}");
        assert!(sub_texts.contains("98.2"), "sub_texts={sub_texts}");
        assert!(sub_texts.contains("1.45"), "sub_texts={sub_texts}");
        assert!(sub_texts.contains("6.25"), "sub_texts={sub_texts}");

        // Live refresh invariant: mutate ResultSheet::scalars without changing params
        doc.result_sheets[0].scalars[1].value = 142.0;
        let mut updated_scene = empty_scene();
        render_model_equation_annotation(
            &mut updated_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(80.0, 40.0, 260.0, 60.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let updated_texts = collect_text_strings(&updated_scene).join(" | ");
        assert!(updated_texts.contains("142"), "updated_texts={updated_texts}");
        assert!(!updated_texts.contains("98.2"), "stale Top remained: {updated_texts}");
    }

    #[test]
    fn overlay_winner_equation_id_overrides_spec_equation_id() {
        let sheet = make_nonlinear_result(
            "res-mm",
            "log_agonist_vs_response_4pl",
            None,
            vec![
                ResultScalarEntry { key: "Vmax".to_owned(), data_set_index: Some(0), value: 52.5, units: None },
                ResultScalarEntry { key: "Km".to_owned(), data_set_index: Some(0), value: 3.2, units: None },
            ],
            vec![FittedCurveOverlay {
                data_set_index: 0,
                equation_id: "michaelis_menten".to_owned(),
                converged: true,
                ambiguous: false,
                points: Vec::new(),
            }],
        );
        let (doc, id) = make_base_doc_with_result(sheet);
        let mut scene = empty_scene();
        render_model_equation_annotation(
            &mut scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(20.0, 20.0, 200.0, 50.0),
                font_size_pt: 9.0,
                bordered: false,
            },
        );
        let texts = collect_text_strings(&scene).join(" | ");
        assert!(texts.contains("52.5"), "texts={texts}");
        assert!(texts.contains("3.2"), "texts={texts}");
        let path_count = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { .. })).count();
        assert!(path_count >= 1, "expected vinculum line path for Michaelis-Menten");
    }

    #[test]
    fn linear_regression_origin_and_standard_modes() {
        let lin_std = LinearRegressionSpec { force_through_origin: false, ..Default::default() };
        let spec = AnalysisSpec::new("res-lin", "data-1", AnalysisKind::LinearRegression(lin_std)).expect("AnalysisSpec::new");
        let mut sheet = ResultSheet::new(spec).expect("ResultSheet::new");
        sheet.status = ResultStatus::UpToDate;
        sheet.scalars = vec![
            ResultScalarEntry { key: "Slope".to_owned(), data_set_index: Some(0), value: 2.5, units: None },
            ResultScalarEntry { key: "YIntercept".to_owned(), data_set_index: Some(0), value: -1.25, units: None },
            ResultScalarEntry { key: "R2".to_owned(), data_set_index: Some(0), value: 0.99, units: None },
        ];
        let (mut doc, id) = make_base_doc_with_result(sheet);

        let mut sym_scene = empty_scene();
        render_model_equation_annotation(
            &mut sym_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Symbolic,
                bounds_pt: DocRectPt::new(40.0, 40.0, 0.0, 0.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let sym_texts = collect_text_strings(&sym_scene).join(" ");
        assert!(sym_texts.contains("Slope"), "sym_texts={sym_texts}");
        assert!(sym_texts.contains("Intercept"), "sym_texts={sym_texts}");

        let mut sub_scene = empty_scene();
        render_model_equation_annotation(
            &mut sub_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(40.0, 40.0, 0.0, 0.0),
                font_size_pt: 10.0,
                bordered: true,
            },
        );
        let texts = collect_text_strings(&sub_scene).join(" ");
        assert!(texts.contains("2.5"), "texts={texts}");
        assert!(texts.contains("1.25"), "texts={texts}");
        assert!(!texts.contains("0.99"), "R2 must not leak: {texts}");

        // Force through origin
        doc.result_sheets[0].analysis.kind =
            AnalysisKind::LinearRegression(LinearRegressionSpec { force_through_origin: true, ..Default::default() });
        let mut origin_scene = empty_scene();
        render_model_equation_annotation(
            &mut origin_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(40.0, 40.0, 0.0, 0.0),
                font_size_pt: 10.0,
                bordered: false,
            },
        );
        let origin_texts = collect_text_strings(&origin_scene).join(" ");
        assert!(origin_texts.contains("2.5"), "origin_texts={origin_texts}");
        assert!(!origin_texts.contains("1.25"), "intercept should be omitted: {origin_texts}");
    }

    #[test]
    fn custom_dsl_and_shared_scalar_fallback_and_narrow_wrap() {
        let sheet = make_nonlinear_result(
            "res-dsl",
            "custom",
            Some("Y = k * X / (Kd + X)"),
            vec![
                ResultScalarEntry { key: "k".to_owned(), data_set_index: Some(1), value: 12.0, units: None },
                ResultScalarEntry { key: "Kd".to_owned(), data_set_index: None, value: 4.5, units: None },
            ],
            Vec::new(),
        );
        let (doc, id) = make_base_doc_with_result(sheet);
        let mut scene = empty_scene();
        render_model_equation_annotation(
            &mut scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &id,
                data_set_index: 1,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(10.0, 10.0, 75.0, 28.0),
                font_size_pt: f64::NAN,
                bordered: true,
            },
        );
        let texts = collect_text_strings(&scene).join(" ");
        assert!(texts.contains("12"), "texts={texts}");
        assert!(texts.contains("4.5"), "texts={texts}");
    }

    #[test]
    fn missing_and_failed_result_sheets_render_placeholders_without_nans() {
        let mut doc = ProjectDocument::new("Empty").expect("ProjectDocument::new");
        let missing_id = SheetId::new("does-not-exist");
        let mut scene = empty_scene();
        render_model_equation_annotation(
            &mut scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &missing_id,
                data_set_index: 0,
                mode: EquationDisplayMode::Symbolic,
                bounds_pt: DocRectPt::new(10.0, 10.0, 120.0, 24.0),
                font_size_pt: -3.0,
                bordered: true,
            },
        );
        let texts = collect_text_strings(&scene).join(" ");
        assert!(texts.contains("unavailable"), "texts={texts}");

        let mut failed_sheet = make_nonlinear_result("res-failed", "michaelis_menten", None, Vec::new(), Vec::new());
        failed_sheet.status = ResultStatus::Failed { code: ErrorCode::SolverFailed, message: "Singular Jacobian".to_owned() };
        let failed_id = failed_sheet.id.clone();
        doc.result_sheets.push(failed_sheet);
        let mut failed_scene = empty_scene();
        render_model_equation_annotation(
            &mut failed_scene,
            &doc,
            &ModelEquationParams {
                result_sheet_id: &failed_id,
                data_set_index: 0,
                mode: EquationDisplayMode::Substituted,
                bounds_pt: DocRectPt::new(10.0, 10.0, 140.0, 28.0),
                font_size_pt: 9.0,
                bordered: true,
            },
        );
        let failed_texts = collect_text_strings(&failed_scene).join(" ");
        assert!(failed_texts.contains("failed"), "failed_texts={failed_texts}");
    }
}
