//! Regression tests for Round 2 Slice S09 (`R2-S09-03`): structured 2D equation segmentation.

use curvebook_text::{EquationSegment, RichSpan, SpanScript, SpanSlope, parse_equation_segments};

#[test]
fn r2_s09_03_equation_segments_preserve_math_multiplication_braces_and_script_fractions() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Math-mode multiplication `*` is preserved literally and never paired as Markdown *italic*.
    let mult = parse_equation_segments(r"$Y = a*x + b*z$");
    assert_eq!(mult.len(), 1);
    let EquationSegment::Inline(rich) = &mult[0] else {
        return Err(format!("expected inline segment for multiplication equation, got {mult:?}").into());
    };
    assert_eq!(rich.plain_text(), "Y = a*x + b*z");
    assert!(rich.spans.iter().all(|s| s.slope == SpanSlope::Upright), "math * must stay upright: {:?}", rich.spans);

    // 2. Braced grouping `{...}` inside math delimiters and fractions collapses cleanly without literal braces.
    let grouped = parse_equation_segments(r"$Y = {A + B} * X$");
    assert_eq!(grouped.len(), 1);
    let EquationSegment::Inline(grouped_rich) = &grouped[0] else {
        return Err(format!("expected inline segment for grouped math equation, got {grouped:?}").into());
    };
    assert_eq!(grouped_rich.plain_text(), "Y = A + B * X");

    // 3. Numerator and denominator inside `\frac` stay in math mode (preserving `*` and stripping `{...}`).
    let frac = parse_equation_segments(r"\frac{a*x + {b + c}}{d*y}");
    assert_eq!(frac.len(), 1);
    let EquationSegment::Fraction { numerator, denominator } = &frac[0] else {
        return Err(format!("expected stacked fraction segment, got {frac:?}").into());
    };
    let [EquationSegment::Inline(num_rich)] = numerator.as_slice() else {
        return Err(format!("expected single inline numerator, got {numerator:?}").into());
    };
    let [EquationSegment::Inline(den_rich)] = denominator.as_slice() else {
        return Err(format!("expected single inline denominator, got {denominator:?}").into());
    };
    assert_eq!(num_rich.plain_text(), "a*x + b + c");
    assert_eq!(den_rich.plain_text(), "d*y");
    assert!(num_rich.spans.iter().all(|s| s.slope == SpanSlope::Upright));
    assert!(den_rich.spans.iter().all(|s| s.slope == SpanSlope::Upright));

    // 4. Unbraced `^\frac{num}{den}` and `_\frac{num}{den}` remain inline superscript/subscript fractions.
    let exp_seg = parse_equation_segments(r"$e^\frac{1}{2}$");
    assert_eq!(exp_seg.len(), 1, "superscript frac must stay inline: {exp_seg:?}");
    let EquationSegment::Inline(exp_rich) = &exp_seg[0] else {
        return Err(format!("expected inline segment for e^\\frac{{1}}{{2}}, got {exp_seg:?}").into());
    };
    assert_eq!(exp_rich.plain_text(), "e1/2");
    assert_eq!(exp_rich.spans[0], RichSpan { text: "e".to_owned(), script: SpanScript::Baseline, ..exp_rich.spans[0].clone() });
    assert!(exp_rich.spans.iter().any(|s| s.text == "1/2" && s.script == SpanScript::Superscript), "spans={:?}", exp_rich.spans);

    let sub_seg = parse_equation_segments(r"$x_\frac{1}{2}$");
    assert_eq!(sub_seg.len(), 1, "subscript frac must stay inline: {sub_seg:?}");
    let EquationSegment::Inline(sub_rich) = &sub_seg[0] else {
        return Err(format!("expected inline segment for x_\\frac{{1}}{{2}}, got {sub_seg:?}").into());
    };
    assert_eq!(sub_rich.plain_text(), "x1/2");
    assert!(sub_rich.spans.iter().any(|s| s.text == "1/2" && s.script == SpanScript::Subscript), "spans={:?}", sub_rich.spans);

    // 5. Following baseline `\frac` after `^\frac` still splits cleanly as a stacked Fraction segment.
    let mixed = parse_equation_segments(r"$a^\frac{1}{2} + \frac{3}{4}$");
    assert_eq!(mixed.len(), 2, "expected inline script-frac plus baseline Fraction: {mixed:?}");
    assert!(matches!(&mixed[0], EquationSegment::Inline(r) if r.plain_text() == "a1/2 + "));
    assert!(matches!(&mixed[1], EquationSegment::Fraction { .. }));
    Ok(())
}
