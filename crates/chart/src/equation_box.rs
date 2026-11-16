//! Live TeX model equation box annotation compiler (`R9.16` / `QOL-16` / `R10.19`).
//!
//! Resolves regression equations dynamically from the target [`ResultSheet`] in
//! [`ProjectDocument`], parses a structured TeX-subset math grammar into recursive
//! two-dimensional math nodes (nested fractions, superscripts/subscripts, radicals,
//! scaled delimiters, Greek parameters, and roman math operators), and lays out
//! publication-grade boxes following Knuth, D. E. (1984), *The TeXbook*, Addison-Wesley,
//! Chapter 17 (pp. 140–145) and Appendix G ("Generating Boxes from Formulas", pp. 441–447):
//!
//! - **Math styles & cramped transitions** (Ch. 17 p. 140, Appendix G p. 441):
//!   eight styles (`D`, `D'`, `T`, `T'`, `S`, `S'`, `SS`, `SS'`) using `1.0 / 0.70 / 0.50`
//!   font scaling (`cmr10` / `cmr7` / `cmr5` ratios).
//! - **Appendix G fontdimens** (p. 447, normalized in current-style `em` and matched to
//!   `lumencraft-text` metrics with math axis $\sigma_{22} = 0.34\,\text{em}$ and x-height
//!   $\sigma_5 = 0.52\,\text{em}$, rule thickness $\theta = \xi_8 = \max(0.050\,\text{em}, 0.30\,\text{pt})$).
//! - **Rule 11** (`\sqrt`, p. 443): radicand in cramped style $C'$, clearance
//!   $\varphi = \theta + \frac{1}{4}\psi$ ($\psi = \sigma_5$ in display style, $\theta$ otherwise),
//!   vector radical surd joined to a horizontal overbar vinculum rule of thickness $\theta$.
//! - **Rules 15a–15e** (`\frac`, `\dfrac`, `\tfrac`, `\binom`, pp. 444–445): numerator shifts
//!   $\sigma_8 \dots \sigma_{10}$, denominator shifts $\sigma_{11} \dots \sigma_{12}$, math-axis
//!   $\sigma_{22}$ alignment, minimum clearances $3\theta$ / $\theta$ (with rule) and $7\theta$ / $3\theta$
//!   (rule-less `\binom`), centered numerator/denominator alignment, and `\nulldelimiterspace`.
//! - **Rules 18a–18f** (`^`, `_`, simultaneous `_sub^sup`, p. 445): nucleus box height/depth drop
//!   ($\sigma_{18}, \sigma_{19}$) for composite boxes vs zero initial drop for single-character nuclei,
//!   shift clamps ($\sigma_{13} \dots \sigma_{17}$), guaranteed $\ge 4\theta$ vertical separation between
//!   stacked superscripts and subscripts, $\frac{4}{5}\sigma_5$ adjustment, and `scriptspace`.
//! - **Rule 19** (scaled delimiters `\left...\right`, `( ... )`, `[ ... ]`, `| ... |`, `\{ ... \}`,
//!   p. 446): symmetric vertical span around the math axis $\sigma_{22}$ covering tall stacked
//!   subexpressions via smooth vector [`Path2D`] outlines.

use lumencraft_doc::{AnalysisKind, EquationDisplayMode, ProjectDocument, ResultSheet, ResultStatus, SheetId};
use lumencraft_fit::tex_format::{format_model_equation_tex, format_polynomial_equation_tex};
use lumencraft_palette::Rgba8;
use lumencraft_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, measure_text_pt, parse_markup};

use crate::scene::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
};

const DEFAULT_FONT_SIZE_PT: f64 = 10.0;
const EQUATION_SIG_FIGS: usize = 4;
const MAX_TEX_DEPTH: usize = 48;
const MIN_RULE_THICKNESS_PT: f64 = 0.30;

/// Inputs required to compile a [`lumencraft_doc::GraphAnnotation::ModelEquation`] annotation.
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

fn is_summary_or_gof_scalar(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "r2" | "r^2"
            | "adjusted r2"
            | "adjusted r^2"
            | "pseudo r2"
            | "rss"
            | "rsdr"
            | "sy.x"
            | "residual sd (sy.x)"
            | "rmse"
            | "aic"
            | "aicc"
            | "bic"
            | "auc"
            | "roc auc"
            | "dof"
    ) || lower.starts_with("auc (")
        || lower.starts_with("residual sd")
}

fn normalize_scalar_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn polynomial_order_for_equation_id(equation_id: &str) -> Option<(usize, bool)> {
    match normalize_scalar_key(equation_id).as_str() {
        "straight_line" => Some((1, false)),
        "line_through_origin" => Some((1, true)),
        "polynomial_2" | "quadratic" => Some((2, false)),
        "polynomial_3" | "cubic" => Some((3, false)),
        "polynomial_4" | "quartic" => Some((4, false)),
        "polynomial_5" => Some((5, false)),
        "polynomial_6" => Some((6, false)),
        _ => None,
    }
}

fn has_all_polynomial_coeffs(params: &[(String, f64)], order: usize, through_origin: bool) -> bool {
    if order <= 1 {
        let lin = collect_linear_params(params, through_origin);
        return lin.len() == if through_origin { 1 } else { 2 };
    }
    let start = usize::from(through_origin);
    (start..=order).all(|power| {
        let b0 = format!("b{power}");
        let b1 = format!("b_{power}");
        let a0 = format!("a{power}");
        let a1 = format!("a_{power}");
        let beta = format!("beta{power}");
        let c = format!("c{power}");
        params.iter().any(|(name, _)| {
            let key = normalize_scalar_key(name);
            key == b0
                || key == b1
                || key == a0
                || key == a1
                || key == beta
                || key == c
                || (power == 0 && matches!(key.as_str(), "intercept" | "yintercept" | "y_intercept"))
                || (power == 1 && key == "slope")
        })
    })
}

/// Collect scalar parameters for `data_set_index` from `res`, preferring
/// dataset-specific entries and falling back to shared (`data_set_index: None`)
/// parameters for keys not already bound to the dataset when another dataset
/// hasn't already claimed dataset-scoped parameters alone.
#[must_use]
pub(crate) fn collect_dataset_scalars(res: &ResultSheet, data_set_index: usize) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();

    for scalar in &res.scalars {
        if scalar.data_set_index == Some(data_set_index) && scalar.value.is_finite() && !is_summary_or_gof_scalar(&scalar.key) {
            let lower = scalar.key.trim().to_ascii_lowercase();
            if !seen_keys.iter().any(|k| k == &lower) {
                seen_keys.push(lower);
                out.push((scalar.key.clone(), scalar.value));
            }
        }
    }

    let has_other_dataset_scoped = res
        .scalars
        .iter()
        .any(|s| s.data_set_index.is_some_and(|idx| idx != data_set_index) && s.value.is_finite() && !is_summary_or_gof_scalar(&s.key));
    if !out.is_empty() || !has_other_dataset_scoped {
        for scalar in &res.scalars {
            if scalar.data_set_index.is_none() && scalar.value.is_finite() && !is_summary_or_gof_scalar(&scalar.key) {
                let lower = scalar.key.trim().to_ascii_lowercase();
                if !seen_keys.iter().any(|k| k == &lower) {
                    seen_keys.push(lower);
                    out.push((scalar.key.clone(), scalar.value));
                }
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
    if matches!(res.status, ResultStatus::Dirty) {
        return ResolvedEquation::Placeholder("(Fit pending)".to_owned());
    }

    let substituted = matches!(mode, EquationDisplayMode::Substituted);
    let params = collect_dataset_scalars(res, data_set_index);
    let overlay_for_dataset = if res.fitted_curves.is_empty() {
        None
    } else {
        let Some(ov) = res.fitted_curves.iter().find(|c| c.data_set_index == data_set_index) else {
            return ResolvedEquation::Placeholder("(Fit unavailable)".to_owned());
        };
        if !ov.converged {
            return ResolvedEquation::Placeholder("(Fit did not converge)".to_owned());
        }
        Some(ov)
    };

    match &res.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) => {
            let equation_id =
                overlay_for_dataset.map(|c| c.equation_id.as_str()).filter(|id| !id.trim().is_empty()).unwrap_or(nl.equation_id.as_str());
            if equation_id.trim().is_empty() {
                return ResolvedEquation::Placeholder("(No nonlinear model selected)".to_owned());
            }
            let custom_dsl = if overlay_for_dataset.is_none() || equation_id == nl.equation_id { nl.custom_dsl.as_deref() } else { None };
            if substituted {
                if params.is_empty() {
                    return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                }
                if custom_dsl.is_none()
                    && let Some((order, through_origin)) = polynomial_order_for_equation_id(equation_id)
                    && !has_all_polynomial_coeffs(&params, order, through_origin)
                {
                    return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                }
            }
            let tex = format_model_equation_tex(equation_id, custom_dsl, &params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::LinearRegression(lin) => {
            let through_origin = lin.force_through_origin && !lin.deming_model_ii;
            let lin_params = collect_linear_params(&params, through_origin);
            if substituted && !has_all_polynomial_coeffs(&lin_params, 1, through_origin) {
                return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
            }
            let tex = format_polynomial_equation_tex(1, through_origin, &lin_params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        AnalysisKind::PassingBablok(_) => {
            let pb_params = collect_linear_params(&params, false);
            if substituted && !has_all_polynomial_coeffs(&pb_params, 1, false) {
                return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
            }
            let tex = format_polynomial_equation_tex(1, false, &pb_params, substituted, EQUATION_SIG_FIGS);
            ResolvedEquation::Tex(tex)
        }
        _ => {
            if let Some(curve) = overlay_for_dataset.filter(|c| !c.equation_id.trim().is_empty()) {
                if substituted {
                    if params.is_empty() {
                        return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                    }
                    if let Some((order, through_origin)) = polynomial_order_for_equation_id(&curve.equation_id)
                        && !has_all_polynomial_coeffs(&params, order, through_origin)
                    {
                        return ResolvedEquation::Placeholder("(Fit parameters unavailable)".to_owned());
                    }
                }
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
    if let Some((_, slope)) = params.iter().find(|(k, _)| matches!(normalize_scalar_key(k).as_str(), "slope" | "b1" | "b_1")) {
        out.push(("Slope".to_owned(), *slope));
    }
    if !force_through_origin
        && let Some((_, intercept)) =
            params.iter().find(|(k, _)| matches!(normalize_scalar_key(k).as_str(), "yintercept" | "y_intercept" | "intercept" | "b0" | "b_0"))
    {
        out.push(("YIntercept".to_owned(), *intercept));
    }
    out
}

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
pub fn render_tex_equation_box(scene: &mut ChartScene, bounds_pt: lumencraft_doc::RectPt, tex: &str, font_size_pt: f64, bordered: bool) {
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

/// Resolve the effective hit-test bounding box for a [`lumencraft_doc::GraphAnnotation::ModelEquation`].
///
/// When either `bounds_pt.width` or `bounds_pt.height` is non-positive (auto-sized equation box),
/// recovers the compiled box dimensions from `scene.primitives` (background box, overflow clip rect,
/// or laid-out equation runs anchored at `(bounds_pt.x, bounds_pt.y)`).
#[must_use]
pub fn resolve_model_equation_hit_bounds(scene: &ChartScene, bounds_pt: lumencraft_doc::RectPt, font_size_pt: f64, bordered: bool) -> Option<RectPt> {
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
// Math AST & TeX Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FractionKind {
    Auto,
    Display,
    Text,
    Binomial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimKind {
    Paren,
    Bracket,
    Brace,
    Pipe,
    DoublePipe,
    Null,
}

#[derive(Debug, Clone)]
enum MathNode {
    Text { text: String, weight: SpanWeight, slope: SpanSlope, is_single_char: bool },
    Space { width_em: f64 },
    ThinSpace,
    Kern { width_em: f64 },
    Fraction { numerator: Vec<MathNode>, denominator: Vec<MathNode>, kind: FractionKind },
    Radical { degree: Option<Vec<MathNode>>, radicand: Vec<MathNode> },
    Delimited { left: DelimKind, inner: Vec<MathNode>, right: DelimKind, explicit: bool },
    Group(Vec<MathNode>),
    Scripted { nucleus: Box<MathNode>, sup: Option<Vec<MathNode>>, sub: Option<Vec<MathNode>> },
}

enum ParsedStep {
    Atom { node: MathNode, is_roman_op: bool },
    Raw(MathNode),
}

fn parse_math_nodes(input: &str) -> Vec<MathNode> {
    let trimmed = input.trim();
    let source = unwrap_outer_math_delimiters(trimmed).unwrap_or(input);
    let mut nodes = parse_math_slice(source, false, 0);
    trim_edge_spaces(&mut nodes);
    nodes
}

fn unwrap_outer_math_delimiters(trimmed: &str) -> Option<&str> {
    if trimmed.starts_with("$$") {
        let (inner, consumed) = match_balanced_math_delimiters(trimmed, 2)?;
        return (consumed == trimmed.len()).then_some(inner);
    }
    if trimmed.starts_with('$') {
        let (inner, consumed) = match_balanced_math_delimiters(trimmed, 1)?;
        return (consumed == trimmed.len()).then_some(inner);
    }
    None
}

fn match_balanced_math_delimiters(rest: &str, delim_len: usize) -> Option<(&str, usize)> {
    let after_open = rest.get(delim_len..)?;
    if delim_len == 1 && after_open.starts_with('$') {
        return None;
    }
    let mut i = 0usize;
    let mut brace_depth = 0usize;
    while i < after_open.len() {
        let ch = after_open.get(i..)?.chars().next()?;
        if ch == '\\' {
            i += 1;
            if let Some(next_ch) = after_open.get(i..).and_then(|s| s.chars().next()) {
                i += next_ch.len_utf8();
            }
            continue;
        }
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
            i += 1;
            continue;
        }
        if ch == '}' {
            if brace_depth == 0 {
                return None;
            }
            brace_depth -= 1;
            i += 1;
            continue;
        }
        let closing = (delim_len == 1 && ch == '$') || (delim_len == 2 && after_open.get(i..).is_some_and(|s| s.starts_with("$$")));
        if brace_depth == 0 && closing {
            let inner = after_open.get(..i)?;
            let after_close = after_open.get(i + delim_len..)?;
            if is_math_mode_content(inner, after_close) {
                return Some((inner, delim_len + i + delim_len));
            }
            return None;
        }
        i += ch.len_utf8();
    }
    None
}

fn is_math_mode_content(inner: &str, after_close: &str) -> bool {
    if inner.trim().is_empty() || after_close.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    if inner.ends_with(char::is_whitespace) && after_close.chars().next().is_some_and(char::is_alphanumeric) {
        return false;
    }
    inner.chars().any(is_math_indicator_char) || is_pure_math_identifier(inner)
}

const fn is_math_indicator_char(ch: char) -> bool {
    if matches!(ch, '=' | '\\' | '^' | '_' | '+' | '-' | '/' | '*' | '<' | '>' | '{' | '}') {
        return true;
    }
    let cp = ch as u32;
    matches!(cp, 0x00B0..=0x00B3 | 0x00B5 | 0x00B7 | 0x00B9 | 0x00D7 | 0x00F7 | 0x0370..=0x03FF | 0x2032..=0x2033 | 0x2190..=0x22FF)
}

fn is_pure_math_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || c == '\'')
}

fn parse_math_slice(input: &str, text_mode: bool, depth: usize) -> Vec<MathNode> {
    if depth >= MAX_TEX_DEPTH {
        if input.is_empty() {
            return Vec::new();
        }
        return vec![MathNode::Text { text: input.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false }];
    }

    let mut nodes = Vec::new();
    let mut pos = 0usize;
    while pos < input.len() {
        let Some(rest) = input.get(pos..) else {
            break;
        };
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            while let Some(c) = input.get(pos..).and_then(|s| s.chars().next())
                && c.is_whitespace()
            {
                pos += c.len_utf8();
            }
            if !matches!(nodes.last(), Some(MathNode::Space { .. })) {
                nodes.push(MathNode::Space { width_em: 0.278 });
            }
            continue;
        }
        if let Some(step) = parse_next_step(input, &mut pos, text_mode, depth) {
            match step {
                ParsedStep::Raw(node) => nodes.push(node),
                ParsedStep::Atom { node, is_roman_op } => {
                    let attached = attach_scripts_if_any(node, input, &mut pos, text_mode, depth);
                    nodes.push(attached);
                    if is_roman_op && needs_operator_thin_space(input.get(pos..).unwrap_or("")) {
                        nodes.push(MathNode::ThinSpace);
                    }
                }
            }
        }
    }
    nodes
}

fn parse_next_step(input: &str, pos: &mut usize, text_mode: bool, depth: usize) -> Option<ParsedStep> {
    let rest = input.get(*pos..)?;
    let ch = rest.chars().next()?;

    if ch == '<'
        && let Some((node, consumed)) = try_parse_html_step(rest, text_mode, depth)
    {
        *pos += consumed;
        return Some(ParsedStep::Atom { node, is_roman_op: false });
    }
    if ch == '\\' {
        return Some(parse_backslash_step(input, pos, depth));
    }
    if ch == '$'
        && let Some((inner, consumed)) = match_balanced_math_delimiters(rest, if rest.starts_with("$$") { 2 } else { 1 })
    {
        *pos += consumed;
        let mut inner_nodes = parse_math_slice(inner, false, depth + 1);
        trim_edge_spaces(&mut inner_nodes);
        return Some(ParsedStep::Atom { node: MathNode::Group(inner_nodes), is_roman_op: false });
    }
    if ch == '{'
        && let Some((inner, consumed)) = extract_braced(rest)
    {
        *pos += consumed;
        let mut inner_nodes = parse_math_slice(inner, text_mode, depth + 1);
        trim_edge_spaces(&mut inner_nodes);
        return Some(ParsedStep::Atom { node: MathNode::Group(inner_nodes), is_roman_op: false });
    }
    if !text_mode && matches!(ch, '(' | '[' | '|') {
        let delim = match ch {
            '(' => DelimKind::Paren,
            '[' => DelimKind::Bracket,
            _ => DelimKind::Pipe,
        };
        if let Some((inner_nodes, consumed)) = try_parse_matched_delim(rest, ch, depth) {
            *pos += consumed;
            return Some(ParsedStep::Atom {
                node: MathNode::Delimited { left: delim, inner: inner_nodes, right: delim, explicit: false },
                is_roman_op: false,
            });
        }
    }
    if ch == '^' || ch == '_' {
        // Standalone script marker without a preceding nucleus: attach to an empty nucleus.
        let empty = MathNode::Group(Vec::new());
        let attached = attach_scripts_if_any(empty, input, pos, text_mode, depth);
        return Some(ParsedStep::Raw(attached));
    }
    Some(parse_literal_atom(input, pos, text_mode))
}

fn parse_literal_atom(input: &str, pos: &mut usize, text_mode: bool) -> ParsedStep {
    let rest = input.get(*pos..).unwrap_or("");
    let Some(ch) = rest.chars().next() else {
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };

    if ch.is_ascii_digit() {
        let mut end = ch.len_utf8();
        while let Some(nxt) = rest.get(end..).and_then(|s| s.chars().next()) {
            if nxt.is_ascii_digit() || (nxt == '.' && rest.get(end + 1..).and_then(|s| s.chars().next()).is_some_and(|d| d.is_ascii_digit())) {
                end += nxt.len_utf8();
            } else {
                break;
            }
        }
        let slice = rest.get(..end).unwrap_or("").to_owned();
        *pos += end;
        return ParsedStep::Atom {
            node: MathNode::Text { text: slice, weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true },
            is_roman_op: false,
        };
    }

    if text_mode && (ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | ',' | ':' | ';')) {
        let mut end = ch.len_utf8();
        while let Some(nxt) = rest.get(end..).and_then(|s| s.chars().next()) {
            if nxt.is_ascii_alphanumeric() || matches!(nxt, '-' | '.' | '_') {
                end += nxt.len_utf8();
            } else {
                break;
            }
        }
        let slice = rest.get(..end).unwrap_or("").to_owned();
        *pos += end;
        return ParsedStep::Atom {
            node: MathNode::Text { text: slice, weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false },
            is_roman_op: false,
        };
    }

    *pos += ch.len_utf8();
    let slope = if !text_mode && ch.is_ascii_alphabetic() { SpanSlope::Italic } else { SpanSlope::Upright };
    let mut buf = [0u8; 4];
    let text = ch.encode_utf8(&mut buf).to_owned();
    ParsedStep::Atom { node: MathNode::Text { text, weight: SpanWeight::Regular, slope, is_single_char: true }, is_roman_op: false }
}

fn parse_backslash_step(input: &str, pos: &mut usize, depth: usize) -> ParsedStep {
    let rest = input.get(*pos..).unwrap_or("");
    let Some(after) = rest.get(1..) else {
        *pos = input.len();
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };
    let Some(first) = after.chars().next() else {
        *pos = input.len();
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };

    if first.is_ascii_alphabetic() {
        let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let name = after.get(..name_len).unwrap_or("");
        let tail = after.get(name_len..).unwrap_or("");
        if let Some((step, tail_consumed)) = parse_named_command(name, tail, depth) {
            *pos += 1 + name_len + tail_consumed;
            return step;
        }
        *pos += 1 + name_len;
        return ParsedStep::Atom {
            node: MathNode::Text { text: name.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false },
            is_roman_op: false,
        };
    }

    *pos += 1 + first.len_utf8();
    match first {
        ',' => ParsedStep::Raw(MathNode::Kern { width_em: 0.167 }),
        ':' => ParsedStep::Raw(MathNode::Kern { width_em: 0.222 }),
        ';' => ParsedStep::Raw(MathNode::Kern { width_em: 0.278 }),
        ' ' => ParsedStep::Raw(MathNode::Space { width_em: 0.278 }),
        '!' => ParsedStep::Raw(MathNode::Kern { width_em: 0.0 }),
        '|' => ParsedStep::Atom {
            node: MathNode::Text { text: "\u{2016}".to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true },
            is_roman_op: false,
        },
        _ => {
            let mut buf = [0u8; 4];
            ParsedStep::Atom {
                node: MathNode::Text {
                    text: first.encode_utf8(&mut buf).to_owned(),
                    weight: SpanWeight::Regular,
                    slope: SpanSlope::Upright,
                    is_single_char: true,
                },
                is_roman_op: false,
            }
        }
    }
}

fn parse_named_command(name: &str, tail: &str, depth: usize) -> Option<(ParsedStep, usize)> {
    if matches!(name, "frac" | "dfrac" | "tfrac" | "binom")
        && let Some((num_raw, den_raw, consumed)) = extract_two_math_args(tail)
    {
        let kind = match name {
            "dfrac" => FractionKind::Display,
            "tfrac" => FractionKind::Text,
            "binom" => FractionKind::Binomial,
            _ => FractionKind::Auto,
        };
        let mut numerator = parse_math_slice(num_raw, false, depth + 1);
        let mut denominator = parse_math_slice(den_raw, false, depth + 1);
        trim_edge_spaces(&mut numerator);
        trim_edge_spaces(&mut denominator);
        return Some((ParsedStep::Atom { node: MathNode::Fraction { numerator, denominator, kind }, is_roman_op: false }, consumed));
    }

    if name == "sqrt"
        && let Some((deg_raw, rad_raw, consumed)) = extract_sqrt_args(tail)
    {
        let degree = deg_raw.map(|d| {
            let mut parsed = parse_math_slice(d, false, depth + 1);
            trim_edge_spaces(&mut parsed);
            parsed
        });
        let mut radicand = parse_math_slice(rad_raw, false, depth + 1);
        trim_edge_spaces(&mut radicand);
        return Some((ParsedStep::Atom { node: MathNode::Radical { degree, radicand }, is_roman_op: false }, consumed));
    }

    if name == "left"
        && let Some((left_delim, inner_slice, right_delim, consumed)) = extract_left_right_pair(tail)
    {
        let mut inner = parse_math_slice(inner_slice, false, depth + 1);
        trim_edge_spaces(&mut inner);
        return Some((
            ParsedStep::Atom { node: MathNode::Delimited { left: left_delim, inner, right: right_delim, explicit: true }, is_roman_op: false },
            consumed,
        ));
    }

    if matches!(name, "textbf" | "mathbf" | "textit" | "mathit" | "text" | "mathrm" | "textrm" | "operatorname" | "mbox")
        && let Some((inner, consumed)) = extract_braced_opt_ws(tail)
    {
        let inner_text_mode = !matches!(name, "textit" | "mathit");
        let mut inner_nodes = parse_math_slice(inner, inner_text_mode, depth + 1);
        let override_weight = matches!(name, "textbf" | "mathbf").then_some(SpanWeight::Bold);
        let override_slope = if matches!(name, "textit" | "mathit") { Some(SpanSlope::Italic) } else { Some(SpanSlope::Upright) };
        apply_style_override(&mut inner_nodes, override_weight, override_slope);
        let node = MathNode::Group(inner_nodes);
        return Some((ParsedStep::Atom { node, is_roman_op: false }, consumed));
    }

    if let Some(op) = lookup_roman_operator(name) {
        return Some((
            ParsedStep::Atom {
                node: MathNode::Text { text: op.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false },
                is_roman_op: true,
            },
            0,
        ));
    }

    if name == "quad" {
        return Some((ParsedStep::Raw(MathNode::Kern { width_em: 1.0 }), 0));
    }
    if name == "qquad" {
        return Some((ParsedStep::Raw(MathNode::Kern { width_em: 2.0 }), 0));
    }
    if name == "space" {
        return Some((ParsedStep::Raw(MathNode::Space { width_em: 0.278 }), 0));
    }

    if let Some(sym) = lookup_greek_escape(name).or_else(|| lookup_math_escape(name)) {
        return Some((
            ParsedStep::Atom {
                node: MathNode::Text { text: sym.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true },
                is_roman_op: false,
            },
            0,
        ));
    }

    None
}

fn apply_style_override(nodes: &mut [MathNode], weight: Option<SpanWeight>, slope: Option<SpanSlope>) {
    for node in nodes {
        match node {
            MathNode::Text { weight: w, slope: s, is_single_char, .. } => {
                if let Some(nw) = weight {
                    *w = nw;
                }
                if let Some(ns) = slope {
                    *s = ns;
                }
                *is_single_char = false;
            }
            MathNode::Group(inner) => apply_style_override(inner, weight, slope),
            MathNode::Scripted { nucleus, sup, sub } => {
                apply_style_override(std::slice::from_mut(nucleus.as_mut()), weight, slope);
                if let Some(sp) = sup {
                    apply_style_override(sp, weight, slope);
                }
                if let Some(sb) = sub {
                    apply_style_override(sb, weight, slope);
                }
            }
            _ => {}
        }
    }
}

fn try_parse_html_step(rest: &str, text_mode: bool, depth: usize) -> Option<(MathNode, usize)> {
    let close = rest.find('>')?;
    if close == 0 || close > 8 {
        return None;
    }
    let tag = rest.get(1..close)?;
    let (end_tag, script_kind) = if tag.eq_ignore_ascii_case("b") {
        ("</b>", None)
    } else if tag.eq_ignore_ascii_case("i") {
        ("</i>", None)
    } else if tag.eq_ignore_ascii_case("sup") {
        ("</sup>", Some(SpanScript::Superscript))
    } else if tag.eq_ignore_ascii_case("sub") {
        ("</sub>", Some(SpanScript::Subscript))
    } else {
        return None;
    };
    let after_open = rest.get(close + 1..)?;
    let close_pos = find_ascii_case_insensitive(after_open, end_tag)?;
    let inner = after_open.get(..close_pos)?;
    let total = close + 1 + close_pos + end_tag.len();
    let mut inner_nodes = parse_math_slice(inner, text_mode, depth + 1);
    match script_kind {
        Some(SpanScript::Superscript) => {
            Some((MathNode::Scripted { nucleus: Box::new(MathNode::Group(Vec::new())), sup: Some(inner_nodes), sub: None }, total))
        }
        Some(SpanScript::Subscript) => {
            Some((MathNode::Scripted { nucleus: Box::new(MathNode::Group(Vec::new())), sup: None, sub: Some(inner_nodes) }, total))
        }
        _ => {
            let weight = tag.eq_ignore_ascii_case("b").then_some(SpanWeight::Bold);
            let slope = tag.eq_ignore_ascii_case("i").then_some(SpanSlope::Italic);
            apply_style_override(&mut inner_nodes, weight, slope);
            Some((MathNode::Group(inner_nodes), total))
        }
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.char_indices().find_map(|(idx, _)| haystack.get(idx..idx + needle.len()).and_then(|sub| sub.eq_ignore_ascii_case(needle).then_some(idx)))
}

fn attach_scripts_if_any(mut nucleus: MathNode, input: &str, pos: &mut usize, text_mode: bool, depth: usize) -> MathNode {
    if depth >= MAX_TEX_DEPTH {
        return nucleus;
    }
    let mut sup: Option<Vec<MathNode>> = None;
    let mut sub: Option<Vec<MathNode>> = None;

    while let Some(rest) = input.get(*pos..) {
        let Some(marker) = rest.chars().next() else {
            break;
        };
        if marker != '^' && marker != '_' {
            break;
        }
        let after = rest.get(1..).unwrap_or("");
        let Some((arg_nodes, arg_consumed)) = parse_script_arg(after, text_mode, depth + 1) else {
            break;
        };
        *pos += 1 + arg_consumed;
        if marker == '^' {
            if sup.is_some() {
                nucleus = MathNode::Scripted { nucleus: Box::new(nucleus), sup: sup.take(), sub: sub.take() };
            }
            sup = Some(arg_nodes);
        } else {
            if sub.is_some() {
                nucleus = MathNode::Scripted { nucleus: Box::new(nucleus), sup: sup.take(), sub: sub.take() };
            }
            sub = Some(arg_nodes);
        }
    }

    if sup.is_some() || sub.is_some() { MathNode::Scripted { nucleus: Box::new(nucleus), sup, sub } } else { nucleus }
}

fn parse_script_arg(s: &str, text_mode: bool, depth: usize) -> Option<(Vec<MathNode>, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    let first = trimmed.chars().next()?;
    if first == '{' {
        let (inner, consumed) = extract_braced(trimmed)?;
        let mut nodes = parse_math_slice(inner, text_mode, depth + 1);
        trim_edge_spaces(&mut nodes);
        return Some((nodes, ws + consumed));
    }
    if first == '\\' {
        let mut local_pos = 0usize;
        let step = parse_backslash_step(trimmed, &mut local_pos, depth + 1);
        let node = match step {
            ParsedStep::Atom { node, .. } | ParsedStep::Raw(node) => node,
        };
        return Some((vec![node], ws + local_pos));
    }
    let mut local_pos = 0usize;
    let step = parse_literal_atom(trimmed, &mut local_pos, text_mode);
    let node = match step {
        ParsedStep::Atom { node, .. } | ParsedStep::Raw(node) => node,
    };
    Some((vec![node], ws + local_pos))
}

fn needs_operator_thin_space(after: &str) -> bool {
    let trimmed = after.trim_start_matches([' ', '\t']);
    if after.len() > trimmed.len() {
        return false; // Explicit space already follows.
    }
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if matches!(first, '(' | '[' | '|' | ',' | '.' | ';' | ':' | ')' | ']' | '}' | '^' | '_') {
        return false;
    }
    !trimmed.starts_with("\\left") && !trimmed.starts_with("\\right")
}

fn trim_edge_spaces(nodes: &mut Vec<MathNode>) {
    while matches!(nodes.first(), Some(MathNode::Space { .. })) {
        nodes.remove(0);
    }
    while matches!(nodes.last(), Some(MathNode::Space { .. })) {
        nodes.pop();
    }
}

fn extract_braced(s: &str) -> Option<(&str, usize)> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '{' {
            depth = depth.saturating_add(1);
            if depth > MAX_TEX_DEPTH {
                return None;
            }
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((s.get(1..idx)?, idx + 1));
            }
        }
    }
    None
}

fn extract_braced_opt_ws(s: &str) -> Option<(&str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws_len = s.len() - trimmed.len();
    let (inner, consumed) = extract_braced(trimmed)?;
    Some((inner, ws_len + consumed))
}

fn extract_one_math_arg(s: &str) -> Option<(&str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if trimmed.starts_with('{') {
        let (inner, consumed) = extract_braced(trimmed)?;
        return Some((inner, ws + consumed));
    }
    let ch = trimmed.chars().next()?;
    if ch == '\\' {
        let after = trimmed.get(1..)?;
        let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let end = if nlen > 0 { 1 + nlen } else { 1 + after.chars().next().map_or(0, char::len_utf8) };
        return Some((trimmed.get(..end)?, ws + end));
    }
    Some((trimmed.get(..ch.len_utf8())?, ws + ch.len_utf8()))
}

fn extract_two_math_args(s: &str) -> Option<(&str, &str, usize)> {
    let (a, ca) = extract_one_math_arg(s)?;
    let (b, cb) = extract_one_math_arg(s.get(ca..)?)?;
    Some((a, b, ca + cb))
}

fn extract_sqrt_args(s: &str) -> Option<(Option<&str>, &str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if let Some(after_bracket) = trimmed.strip_prefix('[')
        && let Some(close_idx) = find_matching_square_bracket(after_bracket)
    {
        let deg = after_bracket.get(..close_idx)?;
        let rest_after_deg = after_bracket.get(close_idx + 1..)?;
        let (rad, rad_consumed) = extract_one_math_arg(rest_after_deg)?;
        return Some((Some(deg), rad, ws + 1 + close_idx + 1 + rad_consumed));
    }
    let (rad, rad_consumed) = extract_one_math_arg(s)?;
    Some((None, rad, rad_consumed))
}

fn find_matching_square_bracket(s: &str) -> Option<usize> {
    let mut brace_depth = 0usize;
    let mut sq_depth = 0usize;
    let mut i = 0usize;
    while i < s.len() {
        let ch = s.get(i..)?.chars().next()?;
        if ch == '\\' {
            i += 1;
            if let Some(nc) = s.get(i..).and_then(|t| t.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        match ch {
            '{' => brace_depth = brace_depth.saturating_add(1),
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' if brace_depth == 0 => sq_depth = sq_depth.saturating_add(1),
            ']' if brace_depth == 0 => {
                if sq_depth == 0 {
                    return Some(i);
                }
                sq_depth -= 1;
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn parse_tex_delim_token(s: &str) -> Option<(DelimKind, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    let (kind, len) = if trimmed.starts_with('(') || trimmed.starts_with(')') {
        (DelimKind::Paren, 1)
    } else if trimmed.starts_with('[') || trimmed.starts_with(']') {
        (DelimKind::Bracket, 1)
    } else if trimmed.starts_with('|') {
        (DelimKind::Pipe, 1)
    } else if trimmed.starts_with('.') {
        (DelimKind::Null, 1)
    } else if trimmed.starts_with("\\{") || trimmed.starts_with("\\}") {
        (DelimKind::Brace, 2)
    } else if trimmed.starts_with("\\|") {
        (DelimKind::DoublePipe, 2)
    } else {
        return None;
    };
    Some((kind, ws + len))
}

fn extract_left_right_pair(tail: &str) -> Option<(DelimKind, &str, DelimKind, usize)> {
    let (left_delim, left_len) = parse_tex_delim_token(tail)?;
    let body = tail.get(left_len..)?;
    let mut depth = 0usize;
    let mut brace_depth = 0usize;
    let mut i = 0usize;
    while i < body.len() {
        let rest = body.get(i..)?;
        if rest.starts_with('\\') {
            if brace_depth == 0
                && rest.starts_with("\\left")
                && !rest.get(5..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                depth = depth.saturating_add(1);
                i += 5;
                continue;
            }
            if brace_depth == 0
                && rest.starts_with("\\right")
                && !rest.get(6..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                if depth == 0 {
                    let after_right = rest.get(6..)?;
                    let (right_delim, right_len) = parse_tex_delim_token(after_right)?;
                    return Some((left_delim, body.get(..i)?, right_delim, left_len + i + 6 + right_len));
                }
                depth -= 1;
                i += 6;
                continue;
            }
            i += 1;
            if let Some(nc) = body.get(i..).and_then(|s| s.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        let ch = rest.chars().next()?;
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
        } else if ch == '}' {
            brace_depth = brace_depth.saturating_sub(1);
        }
        i += ch.len_utf8();
    }
    None
}

fn try_parse_matched_delim(rest: &str, open_ch: char, depth: usize) -> Option<(Vec<MathNode>, usize)> {
    if depth >= MAX_TEX_DEPTH {
        return None;
    }
    let close_ch = match open_ch {
        '(' => ')',
        '[' => ']',
        '|' => '|',
        _ => return None,
    };
    let body = rest.get(open_ch.len_utf8()..)?;
    let mut brace_depth = 0usize;
    let mut left_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut sq_depth = 0usize;
    let mut i = 0usize;

    while i < body.len() {
        let cur = body.get(i..)?;
        if cur.starts_with('\\') {
            if brace_depth == 0 && cur.starts_with("\\left") && !cur.get(5..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                left_depth = left_depth.saturating_add(1);
                i += 5;
                continue;
            }
            if brace_depth == 0 && cur.starts_with("\\right") && !cur.get(6..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                left_depth = left_depth.saturating_sub(1);
                i += 6;
                continue;
            }
            i += 1;
            if let Some(nc) = body.get(i..).and_then(|s| s.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        let ch = cur.chars().next()?;
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
        } else if ch == '}' {
            if brace_depth == 0 {
                return None;
            }
            brace_depth -= 1;
        } else if brace_depth == 0 && left_depth == 0 {
            if open_ch == '|' {
                match ch {
                    '(' => paren_depth = paren_depth.saturating_add(1),
                    ')' => {
                        if paren_depth == 0 {
                            return None;
                        }
                        paren_depth -= 1;
                    }
                    '[' => sq_depth = sq_depth.saturating_add(1),
                    ']' => {
                        if sq_depth == 0 {
                            return None;
                        }
                        sq_depth -= 1;
                    }
                    '|' if paren_depth == 0 && sq_depth == 0 => {
                        let inner = body.get(..i)?;
                        let mut inner_nodes = parse_math_slice(inner, false, depth + 1);
                        trim_edge_spaces(&mut inner_nodes);
                        return Some((inner_nodes, open_ch.len_utf8() + i + close_ch.len_utf8()));
                    }
                    _ => {}
                }
            } else if ch == open_ch {
                paren_depth = paren_depth.saturating_add(1);
            } else if ch == close_ch {
                if paren_depth == 0 {
                    let inner = body.get(..i)?;
                    let mut inner_nodes = parse_math_slice(inner, false, depth + 1);
                    trim_edge_spaces(&mut inner_nodes);
                    return Some((inner_nodes, open_ch.len_utf8() + i + close_ch.len_utf8()));
                }
                paren_depth -= 1;
            }
        }
        i += ch.len_utf8();
    }
    None
}

fn lookup_roman_operator(name: &str) -> Option<&'static str> {
    match name {
        "log" => Some("log"),
        "ln" => Some("ln"),
        "exp" => Some("exp"),
        "sin" => Some("sin"),
        "cos" => Some("cos"),
        "tan" => Some("tan"),
        "sinh" => Some("sinh"),
        "cosh" => Some("cosh"),
        "tanh" => Some("tanh"),
        "arcsin" => Some("arcsin"),
        "arccos" => Some("arccos"),
        "arctan" => Some("arctan"),
        "sec" => Some("sec"),
        "csc" => Some("csc"),
        "cot" => Some("cot"),
        "max" => Some("max"),
        "min" => Some("min"),
        "lim" => Some("lim"),
        "sup" => Some("sup"),
        "inf" => Some("inf"),
        "arg" => Some("arg"),
        "det" => Some("det"),
        "dim" => Some("dim"),
        "gcd" => Some("gcd"),
        "lg" => Some("lg"),
        "erf" => Some("erf"),
        "erfc" => Some("erfc"),
        _ => None,
    }
}

fn lookup_greek_escape(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("\u{03B1}"),
        "beta" => Some("\u{03B2}"),
        "gamma" => Some("\u{03B3}"),
        "delta" => Some("\u{03B4}"),
        "epsilon" => Some("\u{03B5}"),
        "varepsilon" => Some("\u{03F5}"),
        "zeta" => Some("\u{03B6}"),
        "eta" => Some("\u{03B7}"),
        "theta" => Some("\u{03B8}"),
        "vartheta" => Some("\u{03D1}"),
        "iota" => Some("\u{03B9}"),
        "kappa" => Some("\u{03BA}"),
        "varkappa" => Some("\u{03F0}"),
        "lambda" => Some("\u{03BB}"),
        "mu" | "micro" => Some("\u{03BC}"),
        "nu" => Some("\u{03BD}"),
        "xi" => Some("\u{03BE}"),
        "omicron" => Some("\u{03BF}"),
        "pi" => Some("\u{03C0}"),
        "varpi" => Some("\u{03D6}"),
        "rho" => Some("\u{03C1}"),
        "varrho" => Some("\u{03F1}"),
        "varsigma" => Some("\u{03C2}"),
        "sigma" => Some("\u{03C3}"),
        "tau" => Some("\u{03C4}"),
        "upsilon" => Some("\u{03C5}"),
        "phi" => Some("\u{03C6}"),
        "varphi" => Some("\u{03D5}"),
        "chi" => Some("\u{03C7}"),
        "psi" => Some("\u{03C8}"),
        "omega" => Some("\u{03C9}"),
        "Alpha" => Some("\u{0391}"),
        "Beta" => Some("\u{0392}"),
        "Gamma" => Some("\u{0393}"),
        "Delta" => Some("\u{0394}"),
        "Epsilon" => Some("\u{0395}"),
        "Zeta" => Some("\u{0396}"),
        "Eta" => Some("\u{0397}"),
        "Theta" => Some("\u{0398}"),
        "Iota" => Some("\u{0399}"),
        "Kappa" => Some("\u{039A}"),
        "Lambda" => Some("\u{039B}"),
        "Mu" => Some("\u{039C}"),
        "Nu" => Some("\u{039D}"),
        "Xi" => Some("\u{039E}"),
        "Omicron" => Some("\u{039F}"),
        "Pi" => Some("\u{03A0}"),
        "Rho" => Some("\u{03A1}"),
        "Sigma" => Some("\u{03A3}"),
        "Tau" => Some("\u{03A4}"),
        "Upsilon" => Some("\u{03A5}"),
        "Phi" => Some("\u{03A6}"),
        "Chi" => Some("\u{03A7}"),
        "Psi" => Some("\u{03A8}"),
        "Omega" | "ohm" => Some("\u{03A9}"),
        _ => None,
    }
}

fn lookup_math_escape(name: &str) -> Option<&'static str> {
    match name {
        "pm" => Some("\u{00B1}"),
        "mp" => Some("\u{2213}"),
        "times" => Some("\u{00D7}"),
        "div" => Some("\u{00F7}"),
        "cdot" => Some("\u{00B7}"),
        "bullet" => Some("\u{2022}"),
        "le" | "leq" => Some("\u{2264}"),
        "ge" | "geq" => Some("\u{2265}"),
        "ne" | "neq" => Some("\u{2260}"),
        "approx" => Some("\u{2248}"),
        "equiv" => Some("\u{2261}"),
        "sim" => Some("\u{223C}"),
        "propto" => Some("\u{221D}"),
        "ll" => Some("\u{226A}"),
        "gg" => Some("\u{226B}"),
        "infty" => Some("\u{221E}"),
        "minus" => Some("\u{2212}"),
        "degree" | "deg" | "textdegree" => Some("\u{00B0}"),
        "circ" => Some("\u{2218}"),
        "prime" => Some("\u{2032}"),
        "dprime" => Some("\u{2033}"),
        "perthousand" | "permil" => Some("\u{2030}"),
        "angstrom" | "AA" => Some("\u{00C5}"),
        "partial" => Some("\u{2202}"),
        "nabla" => Some("\u{2207}"),
        "sum" => Some("\u{2211}"),
        "prod" => Some("\u{220F}"),
        "int" => Some("\u{222B}"),
        "sqrt" => Some("\u{221A}"),
        "to" | "rightarrow" => Some("\u{2192}"),
        "leftarrow" => Some("\u{2190}"),
        "leftrightarrow" => Some("\u{2194}"),
        "uparrow" => Some("\u{2191}"),
        "downarrow" => Some("\u{2193}"),
        "therefore" => Some("\u{2234}"),
        "forall" => Some("\u{2200}"),
        "exists" => Some("\u{2203}"),
        "in" => Some("\u{2208}"),
        "notin" => Some("\u{2209}"),
        "subset" => Some("\u{2282}"),
        "subseteq" => Some("\u{2286}"),
        "cup" => Some("\u{222A}"),
        "cap" => Some("\u{2229}"),
        "emptyset" => Some("\u{2205}"),
        "ldots" | "dots" => Some("\u{2026}"),
        "ndash" => Some("\u{2013}"),
        "mdash" => Some("\u{2014}"),
        "percent" => Some("%"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Knuth Appendix G 2D Math Styles, Fontdimens & Box Layout Engine
// ---------------------------------------------------------------------------

/// TeXbook Chapter 17 (p. 140) & Appendix G (p. 441) math layout styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathStyle {
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
struct TeXMathDimens {
    em: f64,
    theta: f64,    // xi_8: default rule thickness
    axis: f64,     // sigma_22: math axis height above baseline
    x_h: f64,      // sigma_5: x-height
    num1: f64,     // sigma_8: display fraction numerator shift
    num2: f64,     // sigma_9: non-display fraction numerator shift (with rule)
    num3: f64,     // sigma_10: non-display rule-less fraction numerator shift
    denom1: f64,   // sigma_11: display denominator shift
    denom2: f64,   // sigma_12: non-display denominator shift
    sup1: f64,     // sigma_13: display superscript shift
    sup2: f64,     // sigma_14: non-display uncramped superscript shift
    sup3: f64,     // sigma_15: cramped superscript shift
    sub1: f64,     // sigma_16: subscript-only shift
    sub2: f64,     // sigma_17: subscript shift with superscript
    sup_drop: f64, // sigma_18
    sub_drop: f64, // sigma_19
    script_space: f64,
    null_delim_space: f64,
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
struct LayoutCtx {
    style: MathStyle,
    base_font: f64,
    color: Rgba8,
    depth: usize,
}

impl LayoutCtx {
    fn with_style(self, style: MathStyle) -> Self {
        Self { style, depth: self.depth.saturating_add(1), ..self }
    }

    fn dimens(self) -> TeXMathDimens {
        TeXMathDimens::for_style(self.style, self.base_font)
    }
}

#[derive(Debug, Clone)]
enum BoxOp {
    Text { x: f64, baseline_y: f64, width: f64, span: RichSpan, font_size: f64, color: Rgba8 },
    Path { path: Path2D, fill: Option<FillPaint>, stroke: Option<StrokePaint> },
}

#[derive(Debug, Clone)]
struct LayoutBox {
    width: f64,
    height: f64,
    depth: f64,
    is_single_char: bool,
    italic_end: bool,
    ops: Vec<BoxOp>,
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

fn translate_path(path: &Path2D, dx: f64, dy: f64) -> Path2D {
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

fn layout_nodes(nodes: &[MathNode], ctx: LayoutCtx) -> LayoutBox {
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

fn layout_single_node(node: &MathNode, ctx: LayoutCtx) -> LayoutBox {
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
    let nuc_box = layout_single_node(nucleus, ctx);
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
    let mut out = Vec::new();
    for node in nodes {
        match node {
            MathNode::Group(inner) => out.extend(flatten_for_wrap(inner, ctx)),
            MathNode::Delimited { left, inner, right, explicit: false } => {
                let d = ctx.dimens();
                let inner_box = layout_nodes(inner, ctx);
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
                    if !l_text.is_empty() {
                        out.push(MathNode::Text {
                            text: l_text.to_owned(),
                            weight: SpanWeight::Regular,
                            slope: SpanSlope::Upright,
                            is_single_char: true,
                        });
                    }
                    out.extend(flatten_for_wrap(inner, ctx));
                    if !r_text.is_empty() {
                        out.push(MathNode::Text {
                            text: r_text.to_owned(),
                            weight: SpanWeight::Regular,
                            slope: SpanSlope::Upright,
                            is_single_char: true,
                        });
                    }
                } else {
                    out.push(node.clone());
                }
            }
            _ => out.push(node.clone()),
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
