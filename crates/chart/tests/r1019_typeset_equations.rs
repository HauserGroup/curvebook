//! Integration tests for `R10.19` two-dimensional mathematical equation typesetting
//! (`crates/chart/src/equation_box.rs` + `crates/fit/src/tex_format.rs`).
//!
//! Verifies Knuth, *The TeXbook* (1984), Appendix G invariants:
//! - Rule 11 (`\sqrt`): radical surd + horizontal overbar vinculum clearance and width coverage;
//! - Rules 15a–15e (`\frac`, nested fractions): non-overlapping numerator/vinculum/denominator boxes,
//!   math-axis $\sigma_{22}$ alignment, cramped style font scaling (`1.0 / 0.70 / 0.50`);
//! - Rules 18a–18f (`^`, `_`, `_sub^sup`): simultaneous stacking clearance $\ge 4\theta$,
//!   nested subscript-in-superscript height containment, composite nucleus box-drop lifting;
//! - Rule 19 (scaled delimiters `\left(...\right)`, `(...)`, `[...]`, `\left|...\right|`):
//!   math-axis symmetric vertical coverage around tall subexpressions;
//! - Greek parameters, operators, polynomials, custom DSL, deterministic bit-exact scene output,
//!   backend readiness invariants, and `WF-49` live coefficient refresh on upstream edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::equation_box::{ModelEquationParams, render_model_equation_annotation, render_tex_equation_box};
use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, ScenePrimitive, compile_graph, preflight_check_scene};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, EquationDisplayMode, GraphAnnotation, GraphSheet, LinearRegressionSpec, NonlinearFitSpec,
    ProjectDocument, RectPt as DocRectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis, list_builtin_equations};
use curvebook_palette::Rgba8;
use curvebook_text::{RichText, measure_text_pt};

#[derive(Debug, Clone)]
struct TextBox {
    text: String,
    font_size: f64,
    x0: f64,
    x1: f64,
    y_top: f64,
    y_bot: f64,
    baseline_y: f64,
}

#[derive(Debug, Clone, Copy)]
struct RuleSegment {
    x0: f64,
    x1: f64,
    y: f64,
    stroke_w: f64,
}

fn empty_scene() -> ChartScene {
    ChartScene::new("eq-test", "Equation Test", 480.0, 320.0, Some(Rgba8::WHITE))
}

fn text_boxes(scene: &ChartScene) -> Vec<TextBox> {
    let mut out = Vec::new();
    for p in &scene.primitives {
        let ScenePrimitive::Text(t) = p else {
            continue;
        };
        let mut cursor_x = t.anchor_pt.x;
        for span in &t.rich.spans {
            let m = measure_text_pt(&RichText { spans: vec![span.clone()] }, t.font_size_pt);
            out.push(TextBox {
                text: span.text.clone(),
                font_size: t.font_size_pt,
                x0: cursor_x,
                x1: cursor_x + m.width_pt,
                y_top: t.anchor_pt.y - m.ascent_pt,
                y_bot: t.anchor_pt.y + m.descent_pt,
                baseline_y: t.anchor_pt.y,
            });
            cursor_x += m.width_pt;
        }
    }
    out
}

fn horizontal_rules(scene: &ChartScene) -> Vec<RuleSegment> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: None, stroke: Some(st), clip: None } if path.commands.len() == 2 => {
                match (path.commands.first(), path.commands.get(1)) {
                    (Some(PathCmd::MoveTo(a)), Some(PathCmd::LineTo(b))) if (a.y - b.y).abs() < 1e-6 && (b.x - a.x).abs() > 2.0 => {
                        Some(RuleSegment { x0: a.x.min(b.x), x1: a.x.max(b.x), y: a.y, stroke_w: st.width_pt })
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn path_y_bounds(path: &Path2D) -> Option<(f64, f64)> {
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    let mut push_pt = |pt: PointPt| {
        any = true;
        min_y = min_y.min(pt.y);
        max_y = max_y.max(pt.y);
    };
    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => push_pt(p),
            PathCmd::QuadTo { control, to } => {
                push_pt(control);
                push_pt(to);
            }
            PathCmd::CubicTo { c1, c2, to } => {
                push_pt(c1);
                push_pt(c2);
                push_pt(to);
            }
            PathCmd::Close => {}
        }
    }
    any.then_some((min_y, max_y))
}

fn path_x_bounds(path: &Path2D) -> Option<(f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut any = false;
    let mut push_pt = |pt: PointPt| {
        any = true;
        min_x = min_x.min(pt.x);
        max_x = max_x.max(pt.x);
    };
    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => push_pt(p),
            PathCmd::QuadTo { control, to } => {
                push_pt(control);
                push_pt(to);
            }
            PathCmd::CubicTo { c1, c2, to } => {
                push_pt(c1);
                push_pt(c2);
                push_pt(to);
            }
            PathCmd::Close => {}
        }
    }
    any.then_some((min_x, max_x))
}

fn assert_paths_bit_identical(p1: &Path2D, p2: &Path2D, idx: usize) {
    assert_eq!(p1.commands.len(), p2.commands.len(), "cmd len mismatch at {idx}");
    for (ca, cb) in p1.commands.iter().zip(&p2.commands) {
        match (ca, cb) {
            (PathCmd::MoveTo(u), PathCmd::MoveTo(v)) | (PathCmd::LineTo(u), PathCmd::LineTo(v)) => {
                assert_eq!(u.x.to_bits(), v.x.to_bits());
                assert_eq!(u.y.to_bits(), v.y.to_bits());
            }
            (PathCmd::QuadTo { control: c1a, to: t1 }, PathCmd::QuadTo { control: c1b, to: t2 }) => {
                assert_eq!(c1a.x.to_bits(), c1b.x.to_bits());
                assert_eq!(c1a.y.to_bits(), c1b.y.to_bits());
                assert_eq!(t1.x.to_bits(), t2.x.to_bits());
                assert_eq!(t1.y.to_bits(), t2.y.to_bits());
            }
            (PathCmd::CubicTo { c1: a1, c2: a2, to: t1 }, PathCmd::CubicTo { c1: b1, c2: b2, to: t2 }) => {
                assert_eq!(a1.x.to_bits(), b1.x.to_bits());
                assert_eq!(a1.y.to_bits(), b1.y.to_bits());
                assert_eq!(a2.x.to_bits(), b2.x.to_bits());
                assert_eq!(a2.y.to_bits(), b2.y.to_bits());
                assert_eq!(t1.x.to_bits(), t2.x.to_bits());
                assert_eq!(t1.y.to_bits(), t2.y.to_bits());
            }
            (PathCmd::Close, PathCmd::Close) => {}
            _ => panic!("path cmd mismatch at {idx}"),
        }
    }
}

fn assert_scenes_bit_identical(a: &ChartScene, b: &ChartScene) {
    assert_eq!(a.width_pt.to_bits(), b.width_pt.to_bits());
    assert_eq!(a.height_pt.to_bits(), b.height_pt.to_bits());
    assert_eq!(a.primitives.len(), b.primitives.len(), "primitive count mismatch");
    for (idx, (pa, pb)) in a.primitives.iter().zip(&b.primitives).enumerate() {
        match (pa, pb) {
            (ScenePrimitive::Text(ta), ScenePrimitive::Text(tb)) => {
                assert_eq!(ta.rich, tb.rich, "text mismatch at primitive {idx}");
                assert_eq!(ta.anchor_pt.x.to_bits(), tb.anchor_pt.x.to_bits());
                assert_eq!(ta.anchor_pt.y.to_bits(), tb.anchor_pt.y.to_bits());
                assert_eq!(ta.font_size_pt.to_bits(), tb.font_size_pt.to_bits());
                assert_eq!(ta.rotation_deg.to_bits(), tb.rotation_deg.to_bits());
                assert_eq!(ta.anchor, tb.anchor);
                assert_eq!(ta.color, tb.color);
            }
            (
                ScenePrimitive::Path { path: p1, fill: f1, stroke: s1, clip: c1 },
                ScenePrimitive::Path { path: p2, fill: f2, stroke: s2, clip: c2 },
            ) => {
                assert_eq!(f1, f2, "fill mismatch at {idx}");
                assert_eq!(c1.is_some(), c2.is_some());
                assert_paths_bit_identical(p1, p2, idx);
                match (s1, s2) {
                    (Some(sa), Some(sb)) => {
                        assert_eq!(sa.color, sb.color);
                        assert_eq!(sa.width_pt.to_bits(), sb.width_pt.to_bits());
                        assert_eq!(sa.cap, sb.cap);
                        assert_eq!(sa.join, sb.join);
                    }
                    (None, None) => {}
                    _ => panic!("stroke mismatch at {idx}"),
                }
            }
            (ScenePrimitive::PushClip { rect: r1 }, ScenePrimitive::PushClip { rect: r2 }) => {
                assert_eq!(r1.x.to_bits(), r2.x.to_bits());
                assert_eq!(r1.y.to_bits(), r2.y.to_bits());
                assert_eq!(r1.width.to_bits(), r2.width.to_bits());
                assert_eq!(r1.height.to_bits(), r2.height.to_bits());
            }
            (ScenePrimitive::PopClip, ScenePrimitive::PopClip) => {}
            _ => panic!("primitive variant mismatch at {idx}"),
        }
    }
}

fn assert_scene_backend_ready(scene: &ChartScene) {
    let mut clip_depth = 0usize;
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, stroke, clip, .. } => {
                if let Some(st) = stroke {
                    assert!(st.width_pt.is_finite() && st.width_pt > 0.0, "invalid stroke width {}", st.width_pt);
                }
                if let Some(r) = clip {
                    assert!(r.x.is_finite() && r.y.is_finite() && r.width >= 0.0 && r.height >= 0.0);
                }
                for cmd in &path.commands {
                    match *cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => assert!(p.x.is_finite() && p.y.is_finite()),
                        PathCmd::QuadTo { control, to } => {
                            assert!(control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite());
                        }
                        PathCmd::CubicTo { c1, c2, to } => {
                            assert!(
                                c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite()
                            );
                        }
                        PathCmd::Close => {}
                    }
                }
            }
            ScenePrimitive::Text(t) => {
                assert!(t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite());
                assert!(t.font_size_pt.is_finite() && t.font_size_pt >= 3.5);
                assert!(t.rotation_deg.is_finite());
                let plain = t.rich.plain_text();
                for raw in ["\\frac", "\\sqrt", "\\text", "\\cdot", "\\alpha", "\\Delta", "\\left", "\\right"] {
                    assert!(!plain.contains(raw), "unexpanded TeX escape `{raw}` in `{plain}`");
                }
                assert!(!plain.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')), "XML-unsafe control character in `{plain}`");
            }
            ScenePrimitive::PushClip { rect } => {
                assert!(rect.x.is_finite() && rect.y.is_finite() && rect.width >= 0.0 && rect.height >= 0.0);
                clip_depth += 1;
            }
            ScenePrimitive::PopClip => {
                assert!(clip_depth > 0, "unbalanced PopClip");
                clip_depth -= 1;
            }
        }
    }
    assert_eq!(clip_depth, 0, "unclosed PushClip");
}

#[test]
fn appendix_g_rule_15_nested_fractions_and_math_axis_alignment() {
    // Outer display fraction with nested fraction in denominator: Y = \frac{1}{1 + \frac{a}{b}}
    let mut scene = empty_scene();
    render_tex_equation_box(&mut scene, DocRectPt::new(40.0, 40.0, 0.0, 0.0), r"Y = \frac{1}{1 + \frac{a}{b}}", 10.0, false);
    assert_scene_backend_ready(&scene);
    assert!(preflight_check_scene(&scene).is_empty(), "preflight issues: {:?}", preflight_check_scene(&scene));

    let boxes = text_boxes(&scene);
    let rules = horizontal_rules(&scene);
    assert_eq!(rules.len(), 2, "expected outer + inner fraction vinculum rules, got {rules:?}");

    // Outer rule is wider than inner rule and sits on the top-level Display math axis (baseline - 0.34*10).
    let (outer_rule, inner_rule) =
        if (rules[0].x1 - rules[0].x0) >= (rules[1].x1 - rules[1].x0) { (rules[0], rules[1]) } else { (rules[1], rules[0]) };
    let main_y = boxes.iter().find(|b| b.text == "Y").expect("italic Y baseline run");
    let main_eq = boxes.iter().find(|b| b.text.contains('=')).expect("= baseline run");
    assert!((main_y.baseline_y - main_eq.baseline_y).abs() < 1e-6);
    let expected_outer_axis = main_eq.baseline_y - 0.34 * 10.0;
    assert!((outer_rule.y - expected_outer_axis).abs() < 1e-4, "outer_y={} axis={expected_outer_axis}", outer_rule.y);

    // Numerator `1` (Text style 10pt) strictly clears outer vinculum by >= 3*theta.
    let num_1 = boxes.iter().find(|b| b.text == "1" && b.y_bot < outer_rule.y).expect("outer numerator '1'");
    assert!((num_1.font_size - 10.0).abs() < 1e-6);
    assert!(
        num_1.y_bot <= outer_rule.y - 0.5 * outer_rule.stroke_w - 3.0 * outer_rule.stroke_w + 1e-6,
        "numerator bottom {} must clear outer rule top by 3*theta",
        num_1.y_bot
    );
    assert!(num_1.x0 >= outer_rule.x0 && num_1.x1 <= outer_rule.x1);

    // Inner fraction `a / b` (Script 7pt) sits strictly below outer vinculum and aligns to inner axis.
    let den_prefix = boxes.iter().find(|b| b.text == "1 + " && b.y_top > outer_rule.y).expect("denominator '1 + '");
    let inner_a = boxes.iter().find(|b| b.text == "a").expect("inner numerator 'a'");
    let inner_b = boxes.iter().find(|b| b.text == "b").expect("inner denominator 'b'");
    assert!((den_prefix.font_size - 10.0).abs() < 1e-6);
    assert!((inner_a.font_size - 7.0).abs() < 1e-6, "inner num should scale to Script 7pt, got {}", inner_a.font_size);
    assert!((inner_b.font_size - 7.0).abs() < 1e-6, "inner den should scale to Script 7pt, got {}", inner_b.font_size);
    assert!((inner_rule.y - (den_prefix.baseline_y - 0.34 * 10.0)).abs() < 1e-4);

    // Outer rule bottom clearance over highest element in denominator (`inner_a` top).
    assert!(inner_a.y_top >= outer_rule.y + 0.5 * outer_rule.stroke_w + 3.0 * outer_rule.stroke_w - 1e-6);
    // Inner rule strictly separates `a` and `b` by >= theta_T.
    assert!(inner_a.y_bot <= inner_rule.y - 0.5 * inner_rule.stroke_w - inner_rule.stroke_w + 1e-6);
    assert!(inner_b.y_top >= inner_rule.y + 0.5 * inner_rule.stroke_w + inner_rule.stroke_w - 1e-6);
    assert!(inner_a.x0 >= inner_rule.x0 && inner_b.x1 <= inner_rule.x1);

    // Nested fraction inside an exponent: `e^{-\frac{x}{2}}`
    let mut exp_scene = empty_scene();
    render_tex_equation_box(&mut exp_scene, DocRectPt::new(20.0, 20.0, 0.0, 0.0), r"Y = e^{-\frac{x}{2}}", 10.0, false);
    assert_scene_backend_ready(&exp_scene);
    let exp_boxes = text_boxes(&exp_scene);
    let exp_rules = horizontal_rules(&exp_scene);
    assert_eq!(exp_rules.len(), 1, "expected stacked vinculum inside exponent");
    let e_base = exp_boxes.iter().find(|b| b.text.ends_with('e')).expect("base e");
    let exp_num = exp_boxes.iter().find(|b| b.text == "x").expect("exponent numerator x");
    let exp_den = exp_boxes.iter().find(|b| b.text == "2").expect("exponent denominator 2");
    assert!((exp_num.font_size - 5.0).abs() < 1e-6, "expected ScriptScript 5pt in exponent fraction, got {}", exp_num.font_size);
    assert!(exp_num.y_bot < exp_rules[0].y && exp_den.y_top > exp_rules[0].y);
    assert!(exp_den.y_bot < e_base.baseline_y, "exponent fraction bottom {} must remain above base baseline {}", exp_den.y_bot, e_base.baseline_y);
}

#[test]
fn appendix_g_rule_18_stacked_sub_sup_and_nested_scripts() {
    // Test simultaneous _sub^sup stacking in both orderings (`x_i^2` and `S^n_{j}`)
    let mut scene = empty_scene();
    render_tex_equation_box(&mut scene, DocRectPt::new(30.0, 30.0, 0.0, 0.0), r"x_i^2 + S^n_{j}", 10.0, false);
    assert_scene_backend_ready(&scene);
    let boxes = text_boxes(&scene);

    let x_nuc = boxes.iter().find(|b| b.text == "x").expect("x");
    let x_sup = boxes.iter().find(|b| b.text == "2").expect("2");
    let x_sub = boxes.iter().find(|b| b.text == "i").expect("i");
    assert!((x_sup.font_size - 7.0).abs() < 1e-6);
    assert!((x_sub.font_size - 7.0).abs() < 1e-6);
    assert!(x_sup.x0 >= x_nuc.x1 - 1e-6 && x_sub.x0 >= x_nuc.x1 - 1e-6);
    assert!((x_sup.x0 - x_sub.x0).abs() <= 0.6, "stacked sup and sub must share x column: sup={} sub={}", x_sup.x0, x_sub.x0);
    // Rule 18e: gap >= 4*theta (theta = 0.5pt -> gap >= 2.0pt).
    let gap_x = x_sub.y_top - x_sup.y_bot;
    assert!(gap_x >= 2.0 - 1e-6, "Rule 18e clearance violated for x_i^2: gap={gap_x}");

    let s_nuc = boxes.iter().find(|b| b.text.ends_with('S')).expect("S");
    let s_sup = boxes.iter().find(|b| b.text == "n").expect("n");
    let s_sub = boxes.iter().find(|b| b.text == "j").expect("j");
    assert!((s_sup.x0 - s_sub.x0).abs() <= 0.6);
    assert!(s_sub.y_top - s_sup.y_bot >= 2.0 - 1e-6, "Rule 18e violated for S^n_j");
    assert!(s_sup.x0 >= s_nuc.x1 - 1e-6);

    // Nested subscript inside superscript (`10^{\text{HillSlope} \cdot (\log\text{EC}_{50} - X)}`)
    let mut nested = empty_scene();
    render_tex_equation_box(&mut nested, DocRectPt::new(30.0, 30.0, 0.0, 0.0), r"10^{\text{HillSlope} \cdot (\log\text{EC}_{50} - X)}", 10.0, false);
    assert_scene_backend_ready(&nested);
    let nboxes = text_boxes(&nested);
    let base_10 = nboxes.iter().find(|b| b.text == "10").expect("10");
    let hill = nboxes.iter().find(|b| b.text.contains("HillSlope")).expect("HillSlope");
    let sub_50 = nboxes.iter().find(|b| b.text == "50").expect("nested 50");
    assert!((hill.font_size - 7.0).abs() < 1e-6);
    assert!((sub_50.font_size - 5.0).abs() < 1e-6);
    assert!(sub_50.baseline_y > hill.baseline_y, "50 must be subscripted below EC baseline inside exponent");
    assert!(sub_50.y_bot < base_10.baseline_y, "nested subscript 50 bottom {} must stay above base 10 baseline {}", sub_50.y_bot, base_10.baseline_y);
}

#[test]
fn appendix_g_rule_19_scaled_delimiters_cover_tall_subexpressions() {
    for tex in [
        r"Y = \left(\frac{a + b}{c + d}\right)^2",
        r"Y = \left[\frac{1}{1 + \frac{X}{K_m}}\right]",
        r"Y = \left|\frac{X - \mu}{\sigma}\right|",
        r"Y = (\frac{X}{t_e})^{\frac{t_e}{t_e - t_m}}",
    ] {
        let mut scene = empty_scene();
        render_tex_equation_box(&mut scene, DocRectPt::new(20.0, 20.0, 0.0, 0.0), tex, 10.0, false);
        assert_scene_backend_ready(&scene);

        let boxes = text_boxes(&scene);
        let rules = horizontal_rules(&scene);
        assert!(!rules.is_empty(), "expected stacked fraction rule for `{tex}`");

        // Find non-horizontal-rule delimiter paths (vertical height > 1.5 * base_font).
        let delim_paths: Vec<(f64, f64, f64)> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } => {
                    let (y0, y1) = path_y_bounds(path)?;
                    let (x0, _x1) = path_x_bounds(path)?;
                    if y1 - y0 > 15.0 { Some((x0, y0, y1)) } else { None }
                }
                _ => None,
            })
            .collect();
        assert_eq!(delim_paths.len(), 2, "expected left & right scaled delimiter paths for `{tex}`, got {delim_paths:?}");

        let (lx, ly0, ly1) = delim_paths[0];
        let (rx, ry0, ry1) = delim_paths[1];
        assert!(lx < rx, "left delimiter must precede right delimiter");
        assert!((ly0 - ry0).abs() < 1e-4 && (ly1 - ry1).abs() < 1e-4, "left/right delimiters must share symmetric vertical bounds");

        // All enclosed boxes inside [lx, rx] must be strictly covered vertically by [ly0, ly1].
        let inner_boxes: Vec<&TextBox> = boxes.iter().filter(|b| b.x0 > lx && b.x1 < rx).collect();
        assert!(!inner_boxes.is_empty());
        let inner_min_y = inner_boxes.iter().map(|b| b.y_top).fold(f64::INFINITY, f64::min);
        let inner_max_y = inner_boxes.iter().map(|b| b.y_bot).fold(f64::NEG_INFINITY, f64::max);
        assert!(ly0 <= inner_min_y && ly1 >= inner_max_y, "delimiter [{ly0}, {ly1}] must cover inner [{inner_min_y}, {inner_max_y}] for `{tex}`");
    }
}

#[test]
fn appendix_g_rule_11_square_root_surd_and_overbar_clearance() {
    for tex in [r"Y = \sqrt{a^2 + b^2}", r"Y = \sqrt{1 + \frac{X}{K_m}}", r"Y = \sqrt[3]{X + \Delta}"] {
        let mut scene = empty_scene();
        render_tex_equation_box(&mut scene, DocRectPt::new(20.0, 20.0, 0.0, 0.0), tex, 10.0, false);
        assert_scene_backend_ready(&scene);

        // Identify surd path (multi-segment polyline) and overbar vinculum (horizontal 2-point line).
        let surd_bounds = scene
            .primitives
            .iter()
            .find_map(|p| match p {
                ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None } if path.commands.len() == 4 => {
                    let (x0, x1) = path_x_bounds(path)?;
                    let (y0, y1) = path_y_bounds(path)?;
                    Some((x0, x1, y0, y1))
                }
                _ => None,
            })
            .expect("expected 4-point radical surd path");

        let rules = horizontal_rules(&scene);
        // The highest horizontal rule (smallest y in Y-down) that starts at surd's right join x is the overbar!
        let overbar = rules.iter().min_by(|a, b| a.y.total_cmp(&b.y)).copied().expect("expected radical overbar vinculum");
        assert!((overbar.x0 - surd_bounds.1).abs() < 1e-4, "surd join x {} must meet overbar start {}", surd_bounds.1, overbar.x0);

        let boxes = text_boxes(&scene);
        let rad_boxes: Vec<&TextBox> = boxes.iter().filter(|b| b.x0 >= overbar.x0 && b.x1 <= overbar.x1).collect();
        assert!(!rad_boxes.is_empty(), "expected radicand texts under overbar for `{tex}`");
        let min_rad_top = rad_boxes.iter().map(|b| b.y_top).fold(f64::INFINITY, f64::min);
        let max_rad_bot = rad_boxes.iter().map(|b| b.y_bot).fold(f64::NEG_INFINITY, f64::max);

        // Rule 11 clearance: overbar bottom strictly above highest radicand element (>= theta).
        let clearance = min_rad_top - (overbar.y + 0.5 * overbar.stroke_w);
        assert!(clearance >= overbar.stroke_w - 1e-6, "Rule 11 overbar clearance too small ({clearance}) for `{tex}`");
        assert!(surd_bounds.3 >= max_rad_bot, "surd V bottom {} must reach radicand bottom {}", surd_bounds.3, max_rad_bot);
    }
}

#[test]
fn greek_parameters_builtins_polynomials_dsl_and_determinism() {
    let greek_dsl = "Y = alpha + beta * X + gamma * Delta + theta * kappa + lambda * mu + sigma * tau + phi * psi + Omega";
    let mut sheet = ResultSheet::new(
        AnalysisSpec::new(
            "res-greek",
            "ds-1",
            AnalysisKind::NonlinearRegression(NonlinearFitSpec { custom_dsl: Some(greek_dsl.to_owned()), ..NonlinearFitSpec::new("custom") }),
        )
        .unwrap(),
    )
    .unwrap();
    sheet.status = ResultStatus::UpToDate;
    let mut doc = ProjectDocument::new("GreekTest").unwrap();
    doc.result_sheets.push(sheet);

    let params = ModelEquationParams {
        result_sheet_id: &SheetId::new("res-greek"),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: DocRectPt::new(20.0, 20.0, 0.0, 0.0),
        font_size_pt: 10.0,
        bordered: true,
    };
    let mut s1 = empty_scene();
    render_model_equation_annotation(&mut s1, &doc, &params);
    let mut s2 = empty_scene();
    render_model_equation_annotation(&mut s2, &doc, &params);
    assert_scenes_bit_identical(&s1, &s2);
    assert_scene_backend_ready(&s1);

    let joined: String = text_boxes(&s1).into_iter().map(|b| b.text).collect::<Vec<_>>().join(" ");
    for expected in ['α', 'β', 'γ', 'Δ', 'θ', 'κ', 'λ', 'μ', 'σ', 'τ', 'φ', 'ψ', 'Ω'] {
        assert!(joined.contains(expected), "missing Greek character `{expected}` in `{joined}`");
    }

    // Verify every single built-in equation (117+ models) renders deterministically without NaNs,
    // panics, or unexpanded TeX macros in both Symbolic and Substituted modes!
    let catalog = list_builtin_equations();
    assert!(catalog.len() >= 117);
    for entry in &catalog {
        let mut nl_doc = ProjectDocument::new("AllModels").unwrap();
        let mut nl_sheet =
            ResultSheet::new(AnalysisSpec::new("res-all", "ds-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new(&entry.id))).unwrap())
                .unwrap();
        nl_sheet.status = ResultStatus::UpToDate;
        nl_sheet.scalars = entry
            .param_names
            .iter()
            .enumerate()
            .map(|(i, p)| ResultScalarEntry { key: p.clone(), data_set_index: Some(0), value: 2.5 + (i as f64) * 1.25, units: None })
            .collect();
        nl_doc.result_sheets.push(nl_sheet);

        for mode in [EquationDisplayMode::Symbolic, EquationDisplayMode::Substituted] {
            let p = ModelEquationParams {
                result_sheet_id: &SheetId::new("res-all"),
                data_set_index: 0,
                mode,
                bounds_pt: DocRectPt::new(20.0, 20.0, 280.0, 64.0),
                font_size_pt: 10.0,
                bordered: true,
            };
            let mut a = empty_scene();
            render_model_equation_annotation(&mut a, &nl_doc, &p);
            let mut b = empty_scene();
            render_model_equation_annotation(&mut b, &nl_doc, &p);
            assert_scenes_bit_identical(&a, &b);
            assert_scene_backend_ready(&a);
        }
    }
}

#[test]
fn wf49_live_coefficient_refresh_on_upstream_table_edits_across_models() {
    let mut doc = ProjectDocument::new("WF49LiveRefresh").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
    let pts: [(f64, f64); 8] =
        [(0.0, 100.0), (1.0, 81.108), (2.0, 66.555), (4.0, 46.270), (6.0, 33.966), (8.0, 26.503), (12.0, 19.232), (16.0, 16.557)];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("xy-decay", "Decay", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    let nl_spec = AnalysisSpec::new("res-decay", "xy-decay", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("one_phase_decay"))).unwrap();
    let res_before = execute_analysis(&nl_spec, &ds, &HookContext::default()).unwrap();
    assert_eq!(res_before.status, ResultStatus::UpToDate);
    doc.add_result_sheet(res_before).unwrap();

    let lin_spec = AnalysisSpec::new("res-lin", "xy-decay", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    let lin_before = execute_analysis(&lin_spec, &ds, &HookContext::default()).unwrap();
    doc.add_result_sheet(lin_before).unwrap();

    let mut graph = GraphSheet::new_uninitialized_for_data("g-decay", doc.data_sheet(&SheetId::new("xy-decay")).unwrap()).unwrap();
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-decay"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(80.0, 30.0, 240.0, 44.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-lin"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: DocRectPt::new(80.0, 82.0, 200.0, 30.0),
        font_size_pt: 10.0,
        bordered: false,
    });

    let scene_v1 = compile_graph(&graph, &doc).unwrap();
    let scene_v1_repeat = compile_graph(&graph, &doc).unwrap();
    assert_scenes_bit_identical(&scene_v1, &scene_v1_repeat);
    assert_scene_backend_ready(&scene_v1);
    let texts_v1: Vec<String> = text_boxes(&scene_v1).into_iter().map(|b| b.text).collect();
    assert!(texts_v1.join(" ").contains("100"), "expected Y0~100 in initial fit: {texts_v1:?}");
    let coalesced_lines: Vec<String> = scene_v1
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect();
    assert!(
        coalesced_lines.iter().any(|line| line.contains("Y = ")),
        "expected space-preserving coalesced linear equation run (`Y = ...`), got: {coalesced_lines:?}"
    );

    // Mutate upstream XY cell (Y(t=0): 100 -> 220) and re-execute fits -> equation boxes refresh!
    if let TableArchetype::Xy(xy_mut) = &mut doc.data_sheet_mut(&SheetId::new("xy-decay")).unwrap().archetype {
        xy_mut.data_sets[0].cells[0][0] = Cell::number(220.0);
    }
    let ds_updated = doc.data_sheet(&SheetId::new("xy-decay")).unwrap().clone();
    let res_after = execute_analysis(&nl_spec, &ds_updated, &HookContext::default()).unwrap();
    let lin_after = execute_analysis(&lin_spec, &ds_updated, &HookContext::default()).unwrap();
    doc.result_sheets[0] = res_after;
    doc.result_sheets[1] = lin_after;

    let scene_v2 = compile_graph(&graph, &doc).unwrap();
    assert_scene_backend_ready(&scene_v2);
    let texts_v2: Vec<String> = text_boxes(&scene_v2).into_iter().map(|b| b.text).collect();
    assert_ne!(texts_v1, texts_v2, "ModelEquation annotations must refresh when upstream data changes");
}
