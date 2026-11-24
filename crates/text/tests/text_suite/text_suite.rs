//! Comprehensive integration, edge-case, and property tests for `curvebook-text`.

#![allow(clippy::cognitive_complexity, clippy::too_many_arguments)]

use curvebook_math::{AxisScale, LogitDomain, PValueFormat};
use curvebook_testkit::{assert_approx_eq, assert_close as assert_near};
use curvebook_text::glyph_paths::{GlyphOutlineCmd, PositionedGlyphOutline, outline_rich_text, outline_rich_text_rotated, outline_ttf_glyph};
use curvebook_text::markup::{
    BidiRun, RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, TextDirection, contains_rtl_script, expand_template_hooks, is_rtl_locale,
    parse_markup, reorder_visual_bidi, shape_bidi_runs,
};
use curvebook_text::metrics::{
    TextMetricsPt, TrueTypeFont, TrueTypeFontInfo, build_minimal_truetype_font, char_advance_pt, core14_advance_em, has_bundled_or_ttf_font,
    measure_text_pt, parse_truetype_font_info, system_cjk_font_candidates,
};
use curvebook_text::number::{AxisTickFormat, LocaleNumberPolicy, format_axis_tick, format_axis_tick_with_format, format_pvalue_label};
use curvebook_text::{BUNDLED_FONTS, EquationSegment, TextError, has_bundled_fonts_feature, parse_equation_segments};
use proptest::{prelude::any, prop_assert, prop_assert_eq, proptest};

// ---------------------------------------------------------------------------
// 1. Markup & Template Hook Tests
// ---------------------------------------------------------------------------

#[test]
fn markup_parses_nested_html_and_markdown_styles() {
    let rich = parse_markup("<b>IC<sub>50</sub></b> = <i>12.4</i> nM (<b><i>R<sup>2</sup></i></b>)");
    assert_eq!(rich.plain_text(), "IC50 = 12.4 nM (R2)");

    let ic = rich.spans.iter().find(|s| s.text == "IC");
    assert!(matches!(ic, Some(s) if s.weight == SpanWeight::Bold && s.script == SpanScript::Baseline));

    let sub50 = rich.spans.iter().find(|s| s.text == "50");
    assert!(matches!(
        sub50,
        Some(s) if s.weight == SpanWeight::Bold && s.script == SpanScript::Subscript && (s.scale - 0.7).abs() < 1e-6
    ));

    let italic_val = rich.spans.iter().find(|s| s.text == "12.4");
    assert!(matches!(italic_val, Some(s) if s.slope == SpanSlope::Italic && s.weight == SpanWeight::Regular));

    let r_span = rich.spans.iter().find(|s| s.text == "R");
    assert!(matches!(r_span, Some(s) if s.weight == SpanWeight::Bold && s.slope == SpanSlope::Italic));

    let sup2 = rich.spans.iter().find(|s| s.text == "2");
    assert!(matches!(
        sup2,
        Some(s) if s.weight == SpanWeight::Bold && s.slope == SpanSlope::Italic && s.script == SpanScript::Superscript
    ));
}

#[test]
fn markup_parses_markdown_bold_italic_and_preserves_scientific_asterisks() {
    let rich = parse_markup("**Mean** ± *SD* (***n = 6***)");
    assert_eq!(rich.plain_text(), "Mean ± SD (n = 6)");
    assert!(rich.spans.iter().any(|s| s.text == "Mean" && s.weight == SpanWeight::Bold && s.slope == SpanSlope::Upright));
    assert!(rich.spans.iter().any(|s| s.text == "SD" && s.weight == SpanWeight::Regular && s.slope == SpanSlope::Italic));
    assert!(rich.spans.iter().any(|s| s.text == "n = 6" && s.weight == SpanWeight::Bold && s.slope == SpanSlope::Italic));

    // P-value significance stars and digit-flanked multiplication must stay literal.
    for literal in ["P < 0.001 ***", "****", "1*10^3 + 2*10^4", "2 * 3 = 6"] {
        let parsed = parse_markup(literal);
        assert!(parsed.spans.iter().all(|s| s.weight == SpanWeight::Regular && s.slope == SpanSlope::Upright), "failed on {literal}");
    }
    assert_eq!(parse_markup("****").plain_text(), "****");
    assert_eq!(parse_markup("1*10^3 + 2*10^4").plain_text(), "1*103 + 2*104");
}

#[test]
fn markup_parses_braced_unbraced_and_nested_scripts_and_latex_macros() {
    let p1 = parse_markup("10^{−6} M and 10^-3 M");
    assert_eq!(p1.plain_text(), "10−6 M and 10-3 M");
    assert!(p1.spans.iter().any(|s| s.text == "−6" && s.script == SpanScript::Superscript));
    assert!(p1.spans.iter().any(|s| s.text == "-3" && s.script == SpanScript::Superscript));

    let p2 = parse_markup(r"EC_{50} + K_d + \textbf{V_{\max}} + \textit{t_{1/2}}");
    assert!(p2.spans.iter().any(|s| s.text == "50" && s.script == SpanScript::Subscript));
    assert!(p2.spans.iter().any(|s| s.text == "d" && s.script == SpanScript::Subscript));
    assert!(p2.spans.iter().any(|s| s.text == "V" && s.weight == SpanWeight::Bold));
    assert!(p2.spans.iter().any(|s| s.text == "t" && s.slope == SpanSlope::Italic));

    // Escaped caret / underscore remain literal baseline characters.
    let escaped = parse_markup(r"a\^b \_ c \{d\}");
    assert_eq!(escaped.plain_text(), "a^b _ c {d}");
    assert!(escaped.spans.iter().all(|s| s.script == SpanScript::Baseline));

    // Nested HTML scripts and unbalanced syntax degrade safely without panicking.
    let nested = parse_markup("<sup>a<sub>b</sub>c</sup>");
    assert_eq!(nested.plain_text(), "abc");
    assert!(nested.spans.iter().any(|s| s.text == "a" && s.script == SpanScript::Superscript));
    assert!(nested.spans.iter().any(|s| s.text == "b" && s.script == SpanScript::Subscript));
    assert!(nested.spans.iter().any(|s| s.text == "c" && s.script == SpanScript::Superscript));

    let unbalanced = parse_markup("<b>unclosed <sup>top {brace _");
    assert!(!unbalanced.plain_text().is_empty());
}

#[test]
fn markup_expands_all_greek_and_math_escapes_and_entities() {
    let lower_cases = [
        (r"\alpha", '\u{03B1}'),
        (r"\beta", '\u{03B2}'),
        (r"\gamma", '\u{03B3}'),
        (r"\delta", '\u{03B4}'),
        (r"\epsilon", '\u{03B5}'),
        (r"\zeta", '\u{03B6}'),
        (r"\eta", '\u{03B7}'),
        (r"\theta", '\u{03B8}'),
        (r"\iota", '\u{03B9}'),
        (r"\kappa", '\u{03BA}'),
        (r"\lambda", '\u{03BB}'),
        (r"\mu", '\u{03BC}'),
        (r"\nu", '\u{03BD}'),
        (r"\xi", '\u{03BE}'),
        (r"\omicron", '\u{03BF}'),
        (r"\pi", '\u{03C0}'),
        (r"\rho", '\u{03C1}'),
        (r"\sigma", '\u{03C3}'),
        (r"\tau", '\u{03C4}'),
        (r"\upsilon", '\u{03C5}'),
        (r"\phi", '\u{03C6}'),
        (r"\chi", '\u{03C7}'),
        (r"\psi", '\u{03C8}'),
        (r"\omega", '\u{03C9}'),
        (r"\varsigma", '\u{03C2}'),
    ];
    for (escape, expected) in lower_cases {
        assert_eq!(parse_markup(escape).plain_text(), expected.to_string(), "for {escape}");
    }

    let upper_cases = [
        (r"\Alpha", '\u{0391}'),
        (r"\Beta", '\u{0392}'),
        (r"\Gamma", '\u{0393}'),
        (r"\Delta", '\u{0394}'),
        (r"\Epsilon", '\u{0395}'),
        (r"\Zeta", '\u{0396}'),
        (r"\Eta", '\u{0397}'),
        (r"\Theta", '\u{0398}'),
        (r"\Iota", '\u{0399}'),
        (r"\Kappa", '\u{039A}'),
        (r"\Lambda", '\u{039B}'),
        (r"\Mu", '\u{039C}'),
        (r"\Nu", '\u{039D}'),
        (r"\Xi", '\u{039E}'),
        (r"\Omicron", '\u{039F}'),
        (r"\Pi", '\u{03A0}'),
        (r"\Rho", '\u{03A1}'),
        (r"\Sigma", '\u{03A3}'),
        (r"\Tau", '\u{03A4}'),
        (r"\Upsilon", '\u{03A5}'),
        (r"\Phi", '\u{03A6}'),
        (r"\Chi", '\u{03A7}'),
        (r"\Psi", '\u{03A8}'),
        (r"\Omega", '\u{03A9}'),
        (r"\ohm", '\u{03A9}'),
    ];
    for (escape, expected) in upper_cases {
        assert_eq!(parse_markup(escape).plain_text(), expected.to_string(), "for {escape}");
    }

    let operators = parse_markup(r"\alpha + \Delta\chi^2 \pm \mu \le \ge \times \approx \infty \neq \degree");
    assert_eq!(operators.plain_text(), "α + Δχ2 ± μ ≤ ≥ × ≈ ∞ ≠ °");

    let entities = parse_markup("&alpha; &beta; &Delta; &mu; &plusmn; &times; &le; &ge; &approx; &infin; &deg; &lt; &gt; &amp;");
    assert_eq!(entities.plain_text(), "α β Δ μ ± × ≤ ≥ ≈ ∞ ° < > &");
}

#[test]
fn expand_template_hooks_resolves_single_pass_and_preserves_missing_or_literal_braces() {
    let expanded = expand_template_hooks("{{info:Study}}: Slope = {{slope}}, {{result:IC50}} ({{missing}})", |key| match key {
        "info:Study" => Some(String::from("Dose-{{cohort}}")),
        "cohort" => Some(String::from("A1")),
        "slope" => Some(String::from("-0.85")),
        "result:IC50" => Some(String::from("4.2 nM")),
        _ => None,
    });
    // Single-pass avoids re-expanding user-supplied values or recursive cycles.
    assert_eq!(expanded, "Dose-{{cohort}}: Slope = -0.85, 4.2 nM ({{missing}})");

    // Self-referential expansion stays literal without infinite recursion.
    let cyclic = expand_template_hooks("Value={{a}}", |key| (key == "a").then(|| String::from("{{a}}")));
    assert_eq!(cyclic, "Value={{a}}");

    // Unclosed hook stays literal.
    let unclosed = expand_template_hooks("open {{incomplete", |_| Some(String::from("ignored")));
    assert_eq!(unclosed, "open {{incomplete");
}

#[test]
fn tex_math_delimiters_fractions_sqrt_and_equation_segments_layout_cleanly() {
    // 4PL Dose-response equation renders both flat RichText and stacked EquationSegment tree.
    let eq_4pl = r"$Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log\text{EC}_{50} - X) \cdot \text{HillSlope}}}$";
    let rich_4pl = parse_markup(eq_4pl);
    assert_eq!(rich_4pl.plain_text(), "Y = Bottom + (Top - Bottom)/(1 + 10(logEC50 - X) · HillSlope)");
    let metrics_4pl = measure_text_pt(&rich_4pl, 10.0);
    assert!(metrics_4pl.width_pt > 0.0 && metrics_4pl.height_pt() > 0.0);
    let outlines_4pl = outline_rich_text(&rich_4pl, 0.0, 20.0, 10.0);
    assert_eq!(outlines_4pl.len(), rich_4pl.plain_text().chars().count());
    assert_valid_closed_outlines(&outlines_4pl);

    let segs_4pl = parse_equation_segments(eq_4pl);
    assert_eq!(segs_4pl.len(), 2);
    assert!(matches!(&segs_4pl[0], EquationSegment::Inline(r) if r.plain_text() == "Y = Bottom + "));
    assert!(matches!(&segs_4pl[1], EquationSegment::Fraction { numerator, denominator } if numerator.len() == 1 && denominator.len() == 1));

    // Michaelis-Menten, Exponential decay, and square roots.
    let mm = parse_markup(r"Y = \frac{V_{\max} \cdot X}{K_m + X}");
    assert_eq!(mm.plain_text(), "Y = (Vmax · X)/(Km + X)");

    let decay = parse_markup(r"Y = (Y_0 - \text{Plateau}) \cdot \exp(-K \cdot X) + \text{Plateau}");
    assert_eq!(decay.plain_text(), "Y = (Y0 - Plateau) · exp(-K · X) + Plateau");

    let sqrt_multi = parse_markup(r"\sqrt{a^2 + b^2}");
    assert_eq!(sqrt_multi.plain_text(), "√(a2 + b2)");
    assert_valid_closed_outlines(&outline_rich_text(&sqrt_multi, 0.0, 20.0, 10.0));

    // Currency and unpaired dollar delimiters remain intact.
    assert_eq!(parse_markup("$100").plain_text(), "$100");
    assert_eq!(parse_markup("Costs $5 to $10").plain_text(), "Costs $5 to $10");
    assert_eq!(parse_equation_segments("$100"), vec![EquationSegment::Inline(RichText::plain("$100"))]);
}

// ---------------------------------------------------------------------------
// 2. Number Formatting & P-Value Tests
// ---------------------------------------------------------------------------

#[test]
fn format_axis_tick_covers_all_formats_and_locales() {
    let pub_policy = LocaleNumberPolicy::publication();
    let en_grouped = LocaleNumberPolicy::english_grouped();
    let eu_policy = LocaleNumberPolicy::european();

    // Auto selection across Linear & Log10.
    assert_eq!(format_axis_tick(0.0, AxisScale::Linear, &pub_policy), "0");
    assert_eq!(format_axis_tick(-0.0, AxisScale::Linear, &pub_policy), "0");
    assert_eq!(format_axis_tick(-42.5, AxisScale::Linear, &pub_policy), "\u{2212}42.5");
    assert_eq!(format_axis_tick(1e-6, AxisScale::Log10, &pub_policy), "10^{\u{2212}6}");
    assert_eq!(format_axis_tick(1e6, AxisScale::Log10, &pub_policy), "10^{6}");

    // Explicit PowerOfTenSuperscript including negative values and non-unit mantissas.
    assert_eq!(
        format_axis_tick_with_format(2.5e-6, AxisScale::Log10, AxisTickFormat::PowerOfTenSuperscript, &pub_policy),
        "2.5\u{00D7}10^{\u{2212}6}"
    );
    assert_eq!(format_axis_tick_with_format(-1e4, AxisScale::Log10, AxisTickFormat::PowerOfTenSuperscript, &pub_policy), "\u{2212}10^{4}");

    // ScientificE
    assert_eq!(format_axis_tick_with_format(-0.000125, AxisScale::Linear, AxisTickFormat::ScientificE, &pub_policy), "\u{2212}1.25e\u{2212}4");

    // Decimal & Antilog with grouped English & European locales.
    assert_eq!(format_axis_tick_with_format(1_234_567.89, AxisScale::Linear, AxisTickFormat::Decimal, &en_grouped), "1,234,567.89");
    assert_eq!(format_axis_tick_with_format(-1_234_567.89, AxisScale::Linear, AxisTickFormat::Decimal, &eu_policy), "\u{2212}1.234.567,89");
    assert_eq!(format_axis_tick_with_format(0.0001, AxisScale::Log10, AxisTickFormat::Antilog, &pub_policy), "0.0001");

    // EngineeringSi across prefixes and boundary rollover.
    assert_eq!(format_axis_tick_with_format(1e-6, AxisScale::Linear, AxisTickFormat::EngineeringSi, &pub_policy), "1\u{03BC}");
    assert_eq!(format_axis_tick_with_format(-0.005, AxisScale::Linear, AxisTickFormat::EngineeringSi, &pub_policy), "\u{2212}5m");
    assert_eq!(format_axis_tick_with_format(1500.0, AxisScale::Linear, AxisTickFormat::EngineeringSi, &pub_policy), "1.5k");
    assert_eq!(format_axis_tick_with_format(2_500_000.0, AxisScale::Linear, AxisTickFormat::EngineeringSi, &pub_policy), "2.5M");
    assert_eq!(format_axis_tick_with_format(999.999_999_999_9, AxisScale::Linear, AxisTickFormat::EngineeringSi, &pub_policy), "1k");
    assert_eq!(format_axis_tick_with_format(1e33, AxisScale::Linear, AxisTickFormat::EngineeringSi, &pub_policy), "1e33");

    // Non-finite handling across scales.
    assert_eq!(format_axis_tick(f64::NAN, AxisScale::Logit(LogitDomain::Percent), &pub_policy), "NaN");
    assert_eq!(format_axis_tick(f64::INFINITY, AxisScale::Probit, &pub_policy), "Inf");
    assert_eq!(format_axis_tick(f64::NEG_INFINITY, AxisScale::Reciprocal, &pub_policy), "\u{2212}Inf");
    assert_eq!(format_axis_tick(f64::NEG_INFINITY, AxisScale::Linear, &LocaleNumberPolicy::default()), "-Inf");
}

#[test]
fn format_pvalue_label_covers_all_formats_boundaries_and_non_finite() {
    let default_policy = LocaleNumberPolicy::default();
    let eu_policy = LocaleNumberPolicy::european();

    assert_eq!(format_pvalue_label(0.0023, PValueFormat::FourDecimal, true, true, &default_policy), "P = 0.0023 **");
    assert_eq!(format_pvalue_label(0.00005, PValueFormat::FourDecimal, true, true, &default_policy), "P < 0.0001 ****");
    assert_eq!(format_pvalue_label(0.00005, PValueFormat::Apa, true, true, &default_policy), "P < .001 ***");
    assert_eq!(format_pvalue_label(0.9995, PValueFormat::Apa, true, false, &default_policy), "P > .999");
    assert_eq!(format_pvalue_label(1.0, PValueFormat::Apa, true, false, &default_policy), "P = 1.000");
    assert_eq!(format_pvalue_label(0.0004, PValueFormat::Nejm, true, true, &default_policy), "P < 0.001 ***");
    assert_eq!(format_pvalue_label(0.25, PValueFormat::Nejm, false, true, &default_policy), "0.250 ns");
    assert_eq!(format_pvalue_label(0.0023, PValueFormat::Custom { decimals: 4, scientific_below: None }, true, true, &eu_policy), "P = 0,0023 **");

    // Invalid / non-finite / out-of-range p-values never panic and never emit significance stars.
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, 1.05] {
        let lbl = format_pvalue_label(bad, PValueFormat::FourDecimal, true, true, &default_policy);
        assert_eq!(lbl, "P = NaN");
    }
    assert_eq!(format_pvalue_label(0.0, PValueFormat::FourDecimal, true, true, &default_policy), "P < 0.0001 ****");
    assert_eq!(format_pvalue_label(-0.0, PValueFormat::FourDecimal, true, true, &default_policy), "P < 0.0001 ****");
}

// ---------------------------------------------------------------------------
// 3. Metrics & Clean-Room SFNT / TrueType Tests
// ---------------------------------------------------------------------------

#[test]
fn core14_widths_and_rich_metrics_respect_style_and_scripts() {
    // Verify proportional Helvetica Core-14 glyph widths regardless of bundled fonts feature.
    let w_em = core14_advance_em('W', SpanWeight::Regular, SpanSlope::Upright);
    let a_em = core14_advance_em('a', SpanWeight::Regular, SpanSlope::Upright);
    let i_em = core14_advance_em('i', SpanWeight::Regular, SpanSlope::Upright);
    assert!(w_em > a_em && a_em > i_em);

    // Tabular ASCII digits '0'..='9' share identical advance width in Core-14 Helvetica.
    let d0 = core14_advance_em('0', SpanWeight::Regular, SpanSlope::Upright);
    for d in '1'..='9' {
        assert_near(core14_advance_em(d, SpanWeight::Regular, SpanSlope::Upright), d0, 1e-6, "tabular digit width");
    }

    // Adobe Symbol Greek/math advances.
    assert!(
        core14_advance_em('\u{03A9}', SpanWeight::Regular, SpanSlope::Upright)
            > core14_advance_em('\u{03B9}', SpanWeight::Regular, SpanSlope::Upright)
    );
    assert!(char_advance_pt('\u{00B1}', 12.0, SpanWeight::Regular, SpanSlope::Upright) > 0.0);

    // Superscript & subscript vertical box expansion.
    let base = measure_text_pt(&parse_markup("10"), 12.0);
    let sup = measure_text_pt(&parse_markup("10^{−6}"), 12.0);
    let sub = measure_text_pt(&parse_markup("IC_{50}"), 12.0);
    assert!(sup.ascent_pt > base.ascent_pt);
    assert!(sub.descent_pt > base.descent_pt);
    assert!(sup.width_pt > base.width_pt);

    // Manually constructed Superscript span with scale=1.0 matches parsed `x^2` (no double-scaling).
    let parsed_sq = measure_text_pt(&parse_markup("x^2"), 12.0);
    let manual_sq =
        measure_text_pt(&RichText::new(vec![RichSpan::plain("x"), RichSpan { script: SpanScript::Superscript, ..RichSpan::plain("2") }]), 12.0);
    assert_near(parsed_sq.width_pt, manual_sq.width_pt, 1e-6, "manual vs parsed superscript width");
    assert_near(parsed_sq.ascent_pt, manual_sq.ascent_pt, 1e-6, "manual vs parsed superscript ascent");

    // Multiline total bounding-box height and width.
    let single = measure_text_pt(&parse_markup("Alpha"), 10.0);
    let multi = measure_text_pt(&parse_markup("Alpha\nLonger Beta"), 10.0);
    let second_only = measure_text_pt(&parse_markup("Longer Beta"), 10.0);
    assert!(multi.height_pt() > single.height_pt() * 1.9);
    assert_approx_eq(multi.line_height_pt, single.line_height_pt, 1e-6, 1e-6, "per-line height unchanged");
    assert_near(multi.width_pt, second_only.width_pt, 1e-6, "multiline max line width");

    // Invalid / empty inputs.
    assert_eq!(measure_text_pt(&RichText::default(), 10.0), TextMetricsPt::default());
    assert_eq!(measure_text_pt(&parse_markup("abc"), 0.0), TextMetricsPt::default());
    assert_eq!(measure_text_pt(&parse_markup("abc"), f64::NAN), TextMetricsPt::default());
}

#[test]
fn bundled_fonts_feature_and_tables_match_configuration() {
    assert_eq!(has_bundled_fonts_feature(), cfg!(feature = "bundled-fonts"));
    if !has_bundled_fonts_feature() {
        assert!(BUNDLED_FONTS.is_empty());
    }
    for &(name, bytes) in BUNDLED_FONTS {
        let parsed = TrueTypeFont::parse(bytes);
        assert!(parsed.is_some(), "bundled font {name} failed TrueTypeFont::parse");
    }
}

/// Build a valid minimal SFNT/TrueType buffer with `head`, `hhea`, `maxp`, `cmap` (format 4 + optional format 12),
/// `hmtx`, `loca`, and `glyf` (glyph 0 empty, glyph 1 simple square, glyph 2 compound shifted glyph).
fn build_synthetic_ttf(include_cmap_fmt12: bool) -> Vec<u8> {
    fn push_u16(buf: &mut Vec<u8>, v: u16) {
        buf.extend_from_slice(&v.to_be_bytes());
    }
    fn push_i16(buf: &mut Vec<u8>, v: i16) {
        buf.extend_from_slice(&v.to_be_bytes());
    }
    fn push_u32(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_be_bytes());
    }

    // `head` (54 bytes): unitsPerEm = 1000, indexToLocFormat = 0 (short)
    let mut head = vec![0u8; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&0i16.to_be_bytes());

    // `hhea` (36 bytes): ascender = 800, descender = -200, lineGap = 100, numberOfHMetrics = 3
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[8..10].copy_from_slice(&100i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&3u16.to_be_bytes());

    // `maxp` (6 bytes): numGlyphs = 3
    let mut maxp = Vec::new();
    push_u32(&mut maxp, 0x0000_5000);
    push_u16(&mut maxp, 3);

    // `hmtx`: glyph 0 -> 500, glyph 1 -> 600, glyph 2 -> 650
    let mut hmtx = Vec::new();
    for adv in [500u16, 600, 650] {
        push_u16(&mut hmtx, adv);
        push_i16(&mut hmtx, 50);
    }

    // `cmap` format 4 mapping 'A' (65) -> gid 1, 'B' (66) -> gid 2, sentinel 0xFFFF -> 0.
    let mut cmap4 = Vec::new();
    push_u16(&mut cmap4, 4); // format
    push_u16(&mut cmap4, 32); // length
    push_u16(&mut cmap4, 0); // language
    push_u16(&mut cmap4, 4); // segCountX2 = 2 * 2
    push_u16(&mut cmap4, 4); // searchRange
    push_u16(&mut cmap4, 1); // entrySelector
    push_u16(&mut cmap4, 0); // rangeShift
    push_u16(&mut cmap4, 66); // endCode[0]
    push_u16(&mut cmap4, 0xFFFF); // endCode[1]
    push_u16(&mut cmap4, 0); // reservedPad
    push_u16(&mut cmap4, 65); // startCode[0]
    push_u16(&mut cmap4, 0xFFFF); // startCode[1]
    push_i16(&mut cmap4, -64); // idDelta[0]: 65 + (-64) = 1
    push_i16(&mut cmap4, 1); // idDelta[1]: 65535 + 1 = 0 (mod 65536)
    push_u16(&mut cmap4, 0); // idRangeOffset[0]
    push_u16(&mut cmap4, 0); // idRangeOffset[1]

    // Optional `cmap` format 12 mapping U+1D400 ('𝐀') -> gid 1.
    let mut cmap12 = Vec::new();
    if include_cmap_fmt12 {
        push_u16(&mut cmap12, 12);
        push_u16(&mut cmap12, 0);
        push_u32(&mut cmap12, 28); // length
        push_u32(&mut cmap12, 0); // language
        push_u32(&mut cmap12, 1); // numGroups
        push_u32(&mut cmap12, 0x1D400);
        push_u32(&mut cmap12, 0x1D400);
        push_u32(&mut cmap12, 1);
    }

    let num_subtables: u16 = if include_cmap_fmt12 { 2 } else { 1 };
    let header_len = 4usize + usize::from(num_subtables) * 8;
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0); // version
    push_u16(&mut cmap, num_subtables);
    push_u16(&mut cmap, 3); // platform Microsoft
    push_u16(&mut cmap, 1); // Unicode BMP
    push_u32(&mut cmap, header_len as u32);
    if include_cmap_fmt12 {
        push_u16(&mut cmap, 3);
        push_u16(&mut cmap, 10); // Unicode full repertoire
        push_u32(&mut cmap, (header_len + cmap4.len()) as u32);
    }
    cmap.extend_from_slice(&cmap4);
    cmap.extend_from_slice(&cmap12);

    // `glyf`:
    // gid 0 -> empty (0 bytes)
    // gid 1 -> simple rectangle (100,0) -> (100,700) -> (500,700) -> (500,0), all on-curve
    let mut glyph1 = Vec::new();
    push_i16(&mut glyph1, 1); // numberOfContours = 1
    push_i16(&mut glyph1, 100);
    push_i16(&mut glyph1, 0);
    push_i16(&mut glyph1, 500);
    push_i16(&mut glyph1, 700);
    push_u16(&mut glyph1, 3); // endPtsOfContours[0] = 3 (4 points)
    push_u16(&mut glyph1, 0); // instructionLength
    // flags: on-curve (0x01), non-short x/y (16-bit signed)
    glyph1.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
    // x deltas: +100, 0, +400, 0
    for dx in [100i16, 0, 400, 0] {
        push_i16(&mut glyph1, dx);
    }
    // y deltas: 0, +700, 0, -700
    for dy in [0i16, 700, 0, -700] {
        push_i16(&mut glyph1, dy);
    }
    if !glyph1.len().is_multiple_of(2) {
        glyph1.push(0);
    }

    // gid 2 -> compound glyph referencing gid 1 shifted by (+50, +100)
    let mut glyph2 = Vec::new();
    push_i16(&mut glyph2, -1); // numberOfContours < 0 => compound
    push_i16(&mut glyph2, 150);
    push_i16(&mut glyph2, 100);
    push_i16(&mut glyph2, 550);
    push_i16(&mut glyph2, 800);
    push_u16(&mut glyph2, 0x0003); // ARG_1_AND_2_ARE_WORDS (0x01) | ARGS_ARE_XY_VALUES (0x02)
    push_u16(&mut glyph2, 1); // component glyph_index = 1
    push_i16(&mut glyph2, 50); // dx = 50
    push_i16(&mut glyph2, 100); // dy = 100
    if !glyph2.len().is_multiple_of(2) {
        glyph2.push(0);
    }

    let g1_off = 0usize;
    let g2_off = glyph1.len();
    let end_off = glyph1.len() + glyph2.len();
    let mut glyf = Vec::with_capacity(end_off);
    glyf.extend_from_slice(&glyph1);
    glyf.extend_from_slice(&glyph2);

    // `loca` (short format: offsets divided by 2)
    let mut loca = Vec::new();
    for off in [0usize, g1_off, g2_off, end_off] {
        push_u16(&mut loca, (off / 2) as u16);
    }

    let tables: [([u8; 4], Vec<u8>); 7] =
        [(*b"cmap", cmap), (*b"glyf", glyf), (*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"loca", loca), (*b"maxp", maxp)];

    let num_tables = tables.len() as u16;
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, 0x0001_0000);
    push_u16(&mut font, num_tables);
    push_u16(&mut font, 64);
    push_u16(&mut font, 2);
    push_u16(&mut font, 48);

    let mut cur_offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32(&mut font, 0); // checksum (not checked)
        push_u32(&mut font, cur_offset as u32);
        push_u32(&mut font, data.len() as u32);
        cur_offset += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        while !font.len().is_multiple_of(4) {
            font.push(0);
        }
    }
    font
}

#[test]
fn truetype_parser_decodes_metrics_cmap_and_compound_glyf_safely() {
    let bytes4 = build_synthetic_ttf(false);
    let ttf4 = TrueTypeFont::parse(&bytes4).expect("synthetic format-4 TTF must parse");
    assert_eq!(ttf4.units_per_em(), 1000);
    assert_eq!(ttf4.ascent(), 800);
    assert_eq!(ttf4.descent(), -200);
    assert_eq!(ttf4.line_gap(), 100);
    assert_eq!(ttf4.num_glyphs(), 3);
    assert_eq!(ttf4.index_to_loc_format(), 0);
    assert!(ttf4.has_glyf_outlines());

    assert_eq!(ttf4.glyph_id('A'), Some(1));
    assert_eq!(ttf4.glyph_id('B'), Some(2));
    assert_eq!(ttf4.glyph_id('Z'), None);
    assert_eq!(ttf4.glyph_advance_units(1), Some(600));
    assert_eq!(ttf4.glyph_advance_units(2), Some(650));
    assert_near(ttf4.advance_em('A').expect("mapped"), 0.6, 1e-6, "ttf4 A advance_em");
    assert!(!ttf4.glyf_slice(1).expect("gid 1").is_empty());

    let simple_cmds = outline_ttf_glyph(&ttf4, 'A', 10.0, 100.0, 20.0, false).expect("simple glyph A");
    assert!(!simple_cmds.is_empty());
    // Y-down check: font y=700 at size=20, upem=1000 shifts Y by -14 => 86.0 < origin_y 100.0
    let min_y = simple_cmds
        .iter()
        .filter_map(|cmd| match *cmd {
            GlyphOutlineCmd::MoveTo { y_pt, .. } | GlyphOutlineCmd::LineTo { y_pt, .. } => Some(y_pt),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    assert_near(min_y, 86.0, 1e-4, "ttf simple glyph Y-down ascender");

    // Compound glyph 'B' shifts gid 1 by dx=+50, dy=+100 font units (+1pt X, -2pt Y at 20pt).
    let comp_cmds = outline_ttf_glyph(&ttf4, 'B', 10.0, 100.0, 20.0, false).expect("compound glyph B");
    let min_y_comp = comp_cmds
        .iter()
        .filter_map(|cmd| match *cmd {
            GlyphOutlineCmd::MoveTo { y_pt, .. } | GlyphOutlineCmd::LineTo { y_pt, .. } => Some(y_pt),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    assert_near(min_y_comp, 84.0, 1e-4, "ttf compound glyph Y-down shift");

    // Format 12 non-BMP mapping U+1D400 -> gid 1.
    let bytes12 = build_synthetic_ttf(true);
    let ttf12 = TrueTypeFont::parse(&bytes12).expect("synthetic format-12 TTF must parse");
    assert_eq!(ttf12.glyph_id('\u{1D400}'), Some(1));

    // Corrupt/truncated buffers must return None and never panic.
    assert!(TrueTypeFont::parse(&[]).is_none());
    assert!(TrueTypeFont::parse(b"NOT_A_FONT_HEADER").is_none());
    for cut in (0..bytes4.len().min(64)).step_by(7) {
        let _ = TrueTypeFont::parse(&bytes4[..cut]);
    }
}

// ---------------------------------------------------------------------------
// 4. Glyph Outlines & Rotation Tests
// ---------------------------------------------------------------------------

fn contour_signed_areas(cmds: &[GlyphOutlineCmd]) -> Vec<f64> {
    let mut areas = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut last = (0.0_f64, 0.0_f64);

    for &cmd in cmds {
        match cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                current.clear();
                current.push((x_pt, y_pt));
                last = (x_pt, y_pt);
            }
            GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                current.push((x_pt, y_pt));
                last = (x_pt, y_pt);
            }
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                for step in 1..=8 {
                    let t = f64::from(step) / 8.0;
                    let mt = 1.0 - t;
                    current.push((mt * mt * last.0 + 2.0 * mt * t * cx_pt + t * t * x_pt, mt * mt * last.1 + 2.0 * mt * t * cy_pt + t * t * y_pt));
                }
                last = (x_pt, y_pt);
            }
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                for step in 1..=8 {
                    let t = f64::from(step) / 8.0;
                    let mt = 1.0 - t;
                    current.push((
                        mt * mt * mt * last.0 + 3.0 * mt * mt * t * c1x_pt + 3.0 * mt * t * t * c2x_pt + t * t * t * x_pt,
                        mt * mt * mt * last.1 + 3.0 * mt * mt * t * c1y_pt + 3.0 * mt * t * t * c2y_pt + t * t * t * y_pt,
                    ));
                }
                last = (x_pt, y_pt);
            }
            GlyphOutlineCmd::Close => {
                if current.len() >= 3 {
                    let mut sum = 0.0_f64;
                    for i in 0..current.len() {
                        let (x1, y1) = current[i];
                        let (x2, y2) = current[(i + 1) % current.len()];
                        sum += x1 * y2 - x2 * y1;
                    }
                    areas.push(sum * 0.5);
                }
                current.clear();
            }
        }
    }
    areas
}

fn assert_valid_closed_outlines(outlines: &[PositionedGlyphOutline]) {
    for glyph in outlines {
        assert!(glyph.origin_x_pt.is_finite() && glyph.origin_y_pt.is_finite());
        assert!(glyph.advance_x_pt.is_finite() && glyph.advance_x_pt >= 0.0);
        if glyph.codepoint.is_whitespace() {
            assert!(glyph.commands.is_empty(), "whitespace {:?} should emit empty commands", glyph.codepoint);
            continue;
        }
        assert!(!glyph.commands.is_empty(), "visible glyph {:?} emitted empty outline", glyph.codepoint);

        let mut in_contour = false;
        for &cmd in &glyph.commands {
            match cmd {
                GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                    assert!(x_pt.is_finite() && y_pt.is_finite());
                    in_contour = true;
                }
                GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                    assert!(in_contour && x_pt.is_finite() && y_pt.is_finite());
                }
                GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                    assert!(in_contour && cx_pt.is_finite() && cy_pt.is_finite() && x_pt.is_finite() && y_pt.is_finite());
                }
                GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                    assert!(
                        in_contour
                            && c1x_pt.is_finite()
                            && c1y_pt.is_finite()
                            && c2x_pt.is_finite()
                            && c2y_pt.is_finite()
                            && x_pt.is_finite()
                            && y_pt.is_finite()
                    );
                }
                GlyphOutlineCmd::Close => {
                    assert!(in_contour, "Close without MoveTo on {:?}", glyph.codepoint);
                    in_contour = false;
                }
            }
        }
        assert!(!in_contour, "unclosed contour on {:?}", glyph.codepoint);
    }
}

fn outline_bbox(cmds: &[GlyphOutlineCmd]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &cmd in cmds {
        let pts: &[(f64, f64)] = match cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } | GlyphOutlineCmd::LineTo { x_pt, y_pt } => &[(x_pt, y_pt)],
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => &[(cx_pt, cy_pt), (x_pt, y_pt)],
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => &[(c1x_pt, c1y_pt), (c2x_pt, c2y_pt), (x_pt, y_pt)],
            GlyphOutlineCmd::Close => &[],
        };
        for &(x, y) in pts {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    (min_x, min_y, max_x, max_y)
}

#[test]
fn outline_rich_text_covers_ascii_greek_symbols_winding_styles_and_scripts() {
    let ascii: String = (' '..='~').collect();
    let rich_ascii = RichText::plain(ascii.clone());
    let ascii_outlines = outline_rich_text(&rich_ascii, 0.0, 50.0, 12.0);
    assert_eq!(ascii_outlines.len(), ascii.chars().count());
    assert_valid_closed_outlines(&ascii_outlines);

    let greek_and_math = "αβγδεζηθικλμνξοπρσςτυφχψω ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ ±×÷≤≥≠≈∞°−";
    let rich_greek = RichText::plain(greek_and_math);
    let greek_outlines = outline_rich_text(&rich_greek, 0.0, 50.0, 12.0);
    assert_eq!(greek_outlines.len(), greek_and_math.chars().count());
    assert_valid_closed_outlines(&greek_outlines);

    // Nonzero fill orientation in Y-down coordinates: 'O' has positive outer area & negative inner hole.
    if !has_bundled_fonts_feature() {
        let o_outlines = outline_rich_text(&RichText::plain("O"), 0.0, 50.0, 12.0);
        let areas = contour_signed_areas(&o_outlines[0].commands);
        assert_eq!(areas.len(), 2);
        assert!(areas[0] > 0.0 && areas[1] < 0.0, "expected (+outer, -hole), got {areas:?}");

        // Descender check on 'p': extends below baseline (y > 50.0), while ascender extends above (y < 50.0).
        let p_outlines = outline_rich_text(&RichText::plain("p"), 0.0, 50.0, 12.0);
        let (_, min_y, _, max_y) = outline_bbox(&p_outlines[0].commands);
        assert!(min_y < 50.0 && max_y > 50.0);

        // Bold increases stroke thickness/area vs Regular.
        let reg_area: f64 = contour_signed_areas(&outline_rich_text(&RichText::plain("I"), 0.0, 50.0, 12.0)[0].commands).into_iter().sum();
        let bold_area: f64 = contour_signed_areas(&outline_rich_text(&parse_markup("<b>I</b>"), 0.0, 50.0, 12.0)[0].commands).into_iter().sum();
        assert!(bold_area > reg_area);
    }

    // Italic slant shifts top of 'I' further right than bottom.
    let italic = outline_rich_text(&parse_markup("<i>I</i>"), 0.0, 50.0, 20.0);
    let (min_x_it, min_y_it, max_x_it, _) = outline_bbox(&italic[0].commands);
    assert!(max_x_it > min_x_it && min_y_it < 50.0);

    // Superscript & subscript vertical origin shift relative to baseline.
    let scripted = outline_rich_text(&parse_markup("A<sup>2</sup><sub>i</sub>"), 10.0, 100.0, 12.0);
    assert_near(scripted[0].origin_y_pt, 100.0, 1e-6, "baseline glyph y");
    assert!(scripted[1].origin_y_pt < 100.0);
    assert!(scripted[2].origin_y_pt > 100.0);
}

#[test]
fn outline_rich_text_rotated_transforms_anchor_advances_and_contours() {
    let rich = parse_markup("AB CD");
    let flat = outline_rich_text_rotated(&rich, 100.0, 200.0, 12.0, 0.0);
    let rot90 = outline_rich_text_rotated(&rich, 100.0, 200.0, 12.0, 90.0);
    let rot180 = outline_rich_text_rotated(&rich, 100.0, 200.0, 12.0, 180.0);
    let rot_neg90 = outline_rich_text_rotated(&rich, 100.0, 200.0, 12.0, -90.0);

    assert_eq!(flat.len(), rot90.len());
    assert_valid_closed_outlines(&rot90);
    assert_valid_closed_outlines(&rot180);
    assert_valid_closed_outlines(&rot_neg90);

    // At +90 deg clockwise in Y-down, horizontal pen advance along +X becomes downward along +Y.
    let dx_flat = flat[3].origin_x_pt - flat[0].origin_x_pt;
    assert_near(rot90[3].origin_x_pt, 100.0, 1e-6, "rot90 x unchanged");
    assert_near(rot90[3].origin_y_pt, 200.0 + dx_flat, 1e-6, "rot90 y advance");

    // At -90 deg (vertical left Y-axis label), pen advances upward along -Y.
    assert_near(rot_neg90[3].origin_x_pt, 100.0, 1e-6, "rot-90 x unchanged");
    assert_near(rot_neg90[3].origin_y_pt, 200.0 - dx_flat, 1e-6, "rot-90 y advance");

    // Distance from rotation anchor is invariant at arbitrary angles (e.g. 37 deg).
    let rot37 = outline_rich_text_rotated(&rich, 100.0, 200.0, 12.0, 37.0);
    let dist_flat = (flat[4].origin_x_pt - 100.0).hypot(flat[4].origin_y_pt - 200.0);
    let dist_37 = (rot37[4].origin_x_pt - 100.0).hypot(rot37[4].origin_y_pt - 200.0);
    assert_near(dist_flat, dist_37, 1e-6, "rotation radius invariant");
}

// ---------------------------------------------------------------------------
// 5. Property Tests
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_markup_and_hooks_never_panic_on_arbitrary_input(s in "\\PC{0,80}") {
        let rich = parse_markup(&s);
        let outlines = outline_rich_text(&rich, 0.0, 0.0, 10.0);
        prop_assert_eq!(outlines.len(), rich.plain_text().chars().count());
        let metrics = measure_text_pt(&rich, 10.0);
        prop_assert!(metrics.width_pt.is_finite() && metrics.width_pt >= 0.0);
        prop_assert!(metrics.height_pt().is_finite() && metrics.height_pt() >= 0.0);

        let expanded = expand_template_hooks(&s, |key| Some(format!("[{key}]")));
        prop_assert!(expanded.is_ascii() || !expanded.is_empty() || s.is_empty());

        let segments = parse_equation_segments(&s);
        for seg in &segments {
            if let EquationSegment::Inline(inner) = seg {
                prop_assert!(!inner.is_empty());
            }
        }
    }

    #[test]
    fn prop_number_and_pvalue_formatters_never_panic(v in proptest::num::f64::ANY) {
        let policy = LocaleNumberPolicy::publication();
        for fmt in [
            AxisTickFormat::Decimal,
            AxisTickFormat::PowerOfTenSuperscript,
            AxisTickFormat::ScientificE,
            AxisTickFormat::Antilog,
            AxisTickFormat::EngineeringSi,
        ] {
            let out = format_axis_tick_with_format(v, AxisScale::Linear, fmt, &policy);
            prop_assert!(!out.is_empty());
        }
        for pfmt in [
            PValueFormat::FourDecimal,
            PValueFormat::Apa,
            PValueFormat::Nejm,
            PValueFormat::Custom { decimals: 3, scientific_below: Some(1e-4) },
        ] {
            let lbl = format_pvalue_label(v, pfmt, true, true, &policy);
            prop_assert!(!lbl.is_empty());
        }
    }

    #[test]
    fn prop_rotated_outlines_are_finite_and_closed(
        raw in "[A-Za-z0-9 ±αβΔμ]{0,16}",
        size in 2.0f64..48.0,
        angle in -360.0f64..360.0
    ) {
        let rich = parse_markup(&raw);
        let outlines = outline_rich_text_rotated(&rich, 10.0, 20.0, size, angle);
        prop_assert_eq!(outlines.len(), rich.plain_text().chars().count());
        assert_valid_closed_outlines(&outlines);
    }

    #[test]
    fn prop_bidi_shaping_never_panics_and_preserves_byte_bounds(s in "\\PC{0,80}") {
        for base_dir in [TextDirection::Ltr, TextDirection::Rtl] {
            let runs = shape_bidi_runs(&s, base_dir);
            let visual = reorder_visual_bidi(&s, base_dir);
            let mut joined = String::new();
            for run in &runs {
                prop_assert!(run.start_byte < run.end_byte);
                prop_assert!(run.end_byte <= s.len());
                prop_assert!(s.is_char_boundary(run.start_byte));
                prop_assert!(s.is_char_boundary(run.end_byte));
                joined.push_str(&run.text);
            }
            prop_assert_eq!(joined, visual);
        }
    }

    #[test]
    fn prop_parse_truetype_font_info_never_panics_on_hostile_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = parse_truetype_font_info(&bytes);
    }
}

// ---------------------------------------------------------------------------
// 6. BiDi / RTL Shaping, SFNT Inspection & Builder, and CJK Fallbacks
// ---------------------------------------------------------------------------

#[test]
fn bidi_locale_and_script_detection_covers_rtl_blocks() {
    for rtl_loc in ["ar", "ar-SA", "AR_EG", "he", "he-IL", "fa", "fa-IR", "ur", "ur-PK"] {
        assert!(is_rtl_locale(rtl_loc), "expected RTL locale for {rtl_loc}");
    }
    for ltr_loc in ["", "en", "en-US", "de", "fr", "ja", "ko", "zh-CN", "zh-TW", "pt-BR", "ar-Latn"] {
        assert!(!is_rtl_locale(ltr_loc), "expected LTR locale for {ltr_loc}");
    }

    assert!(contains_rtl_script("שלום")); // Hebrew U+0590..=U+05FF
    assert!(contains_rtl_script("مرحبا")); // Arabic U+0600..=U+06FF
    assert!(contains_rtl_script("abc \u{0750} def")); // Arabic Supplement U+0750..=U+077F
    assert!(contains_rtl_script("\u{FB50}")); // Arabic Presentation Forms-A
    assert!(contains_rtl_script("\u{FE8E}")); // Arabic Presentation Forms-B
    assert!(!contains_rtl_script("IC50 = 12.4 ± 0.5 μM (Δαβ)"));
    assert!(!contains_rtl_script("濃度 日本語 한국어"));
    assert!(!contains_rtl_script(""));
}

#[test]
fn bidi_shaping_handles_mixed_hebrew_arabic_numbers_math_and_bracket_mirroring() {
    assert!(shape_bidi_runs("", TextDirection::Ltr).is_empty());
    assert_eq!(reorder_visual_bidi("", TextDirection::Rtl), "");

    let pure_ltr = "IC50 = 12.4 ± 0.5 μM";
    let ltr_runs = shape_bidi_runs(pure_ltr, TextDirection::Ltr);
    assert_eq!(ltr_runs, vec![BidiRun { text: pure_ltr.to_owned(), direction: TextDirection::Ltr, start_byte: 0, end_byte: pure_ltr.len() }]);

    // Pure Hebrew & pure Arabic reverse into visual LTR order.
    let he = "שלום";
    let he_runs = shape_bidi_runs(he, TextDirection::Rtl);
    assert_eq!(he_runs, vec![BidiRun { text: String::from("םולש"), direction: TextDirection::Rtl, start_byte: 0, end_byte: he.len() }]);
    assert_eq!(reorder_visual_bidi(he, TextDirection::Ltr), "םולש");
    assert_eq!(reorder_visual_bidi("مرحبا", TextDirection::Rtl), "ابحرم");

    // Bracket mirroring () / [] / {} / <> with numbers & math identifiers in RTL runs.
    assert_eq!(reorder_visual_bidi("שלום (123)", TextDirection::Rtl), "(123) םולש");
    assert_eq!(reorder_visual_bidi("שלום (עולם)", TextDirection::Rtl), "(םלוע) םולש");
    assert_eq!(reorder_visual_bidi("التركيز [IC50] = 5.2", TextDirection::Rtl), "5.2 = [IC50] زيكرتلا");
    assert_eq!(reorder_visual_bidi("قيمة {α + β}", TextDirection::Rtl), "{α + β} ةميق");
    assert_eq!(reorder_visual_bidi("ערך <נתון>", TextDirection::Rtl), "<ןותנ> ךרע");

    // Mixed LTR base sentence containing parenthesized Hebrew.
    assert_eq!(reorder_visual_bidi("a (שלום) b", TextDirection::Ltr), "a (םולש) b");
    assert_eq!(reorder_visual_bidi("א (ב) ג", TextDirection::Ltr), "ג (ב) א");

    // Combining niqqud stays attached after its base Hebrew letter.
    let with_niqqud = "ש\u{05BC}לום";
    assert_eq!(reorder_visual_bidi(with_niqqud, TextDirection::Rtl), "םולש\u{05BC}");

    // Multi-line paragraphs keep line 1 above line 2.
    assert_eq!(reorder_visual_bidi("שלום\nעולם", TextDirection::Rtl), "םולש\nםלוע");

    // Byte spans remain valid logical slices of original input.
    let mixed = "التركيز [IC50]";
    let mixed_runs = shape_bidi_runs(mixed, TextDirection::Rtl);
    assert!(mixed_runs.iter().any(|r| r.direction == TextDirection::Ltr && r.text == "IC50"));
    for run in &mixed_runs {
        assert!(mixed.get(run.start_byte..run.end_byte).is_some());
    }
}

#[test]
fn minimal_truetype_font_builder_roundtrips_through_sfnt_parsers_and_outlines() {
    let ttf_bytes = build_minimal_truetype_font();
    assert_eq!(&ttf_bytes[0..4], &[0x00, 0x01, 0x00, 0x00]);
    for tag in [b"OS/2", b"cmap", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name", b"post"] {
        assert!(ttf_bytes.windows(4).any(|w| w == tag), "missing SFNT tag {:?}", std::str::from_utf8(tag));
    }

    let ttf = TrueTypeFont::parse(&ttf_bytes).expect("minimal TTF must parse with TrueTypeFont");
    assert_eq!(ttf.units_per_em(), 1000);
    assert_eq!(ttf.ascent(), 800);
    assert_eq!(ttf.descent(), -200);
    assert_eq!(ttf.num_glyphs(), 198);
    assert!(ttf.has_glyf_outlines());

    let info: TrueTypeFontInfo = parse_truetype_font_info(&ttf_bytes).expect("minimal TTF must inspect cleanly");
    assert_eq!(info.postscript_name, "CurveBookSans-Regular");
    assert_eq!(info.family_name, "CurveBook Sans");
    assert_eq!(info.units_per_em, 1000);
    assert_eq!(info.ascender_1000, 800);
    assert_eq!(info.descender_1000, -200);
    assert_eq!(info.cap_height_1000, 720);
    assert_near(info.italic_angle, 0.0, 1e-6, "minimal TTF italic angle");
    assert_eq!(info.flags, 32);
    assert_eq!(info.stem_v, 82);
    assert_eq!(info.bbox_1000[1], -200);
    assert_eq!(info.bbox_1000[3], 720);
    assert_eq!(info.widths_1000.len(), 224);

    assert!(info.width_pt('W', 10.0) > info.width_pt('a', 10.0));
    assert!(info.width_pt('a', 10.0) > info.width_pt('i', 10.0));
    assert_near(info.width_pt('\n', 10.0), 0.0, 1e-9, "newline width");

    for code in 32u8..=126u8 {
        let ch = char::from(code);
        assert_ne!(info.glyph_id(ch), 0, "ASCII {ch:?} should be mapped");
        assert!(info.width_pt(ch, 12.0) > 0.0);
    }
    for code in 160u8..=255u8 {
        let ch = char::from(code);
        assert_ne!(info.glyph_id(ch), 0, "Latin-1 U+{:04X} should be mapped", u32::from(ch));
        assert!(info.width_pt(ch, 12.0) > 0.0);
    }
    for greek in ['α', 'β', 'γ', 'Δ', 'μ', 'σ'] {
        assert_ne!(info.glyph_id(greek), 0, "Greek {greek:?} should be mapped");
        assert!(info.width_pt(greek, 12.0) > 0.0);
        let cmds = outline_ttf_glyph(&ttf, greek, 10.0, 50.0, 12.0, false).expect("Greek glyph outline");
        assert!(!cmds.is_empty());
    }

    assert_eq!(info.glyph_id('中'), 0);
    assert!(info.width_pt('中', 12.0) > 0.0);
    let space_cmds = outline_ttf_glyph(&ttf, ' ', 0.0, 50.0, 12.0, false).expect("space glyph");
    assert!(space_cmds.is_empty());
}

#[test]
fn parse_truetype_font_info_handles_format12_and_rejects_hostile_sfnt() {
    let syn_bytes = build_synthetic_ttf(true);
    let syn_info = parse_truetype_font_info(&syn_bytes).expect("synthetic format-12 TTF must inspect");
    assert_eq!(syn_info.glyph_id('A'), 1);
    assert_eq!(syn_info.glyph_id('B'), 2);
    assert_eq!(syn_info.glyph_id('\u{1D400}'), 1);
    assert_eq!(syn_info.widths_1000.len(), 224);
    assert_eq!(syn_info.widths_1000[usize::from(b'A' - 32)], 600);

    // Every single truncated prefix of minimal TTF must return Err cleanly.
    let full = build_minimal_truetype_font();
    for cut in 0..full.len() {
        assert!(parse_truetype_font_info(&full[..cut]).is_err(), "prefix len {cut} should error");
    }

    // Invalid SFNT magic and directory anomalies.
    assert!(matches!(parse_truetype_font_info(&[]), Err(TextError::InvalidSfnt { .. })));
    assert!(matches!(parse_truetype_font_info(b"wOFF\x00\x01\x00\x00\x00\x00\x00\x00"), Err(TextError::InvalidSfnt { .. })));

    // Missing required table (`cmap` renamed to `xxxx`) yields MissingSfntTable.
    let mut missing_cmap = full.clone();
    if let Some(pos) = missing_cmap.windows(4).position(|w| w == b"cmap") {
        missing_cmap[pos..pos + 4].copy_from_slice(b"xxxx");
    }
    assert!(matches!(parse_truetype_font_info(&missing_cmap), Err(TextError::MissingSfntTable { tag }) if tag == *b"cmap"));

    // Corrupt directory length exceeding EOF.
    let mut oob = full.clone();
    oob[12 + 12..12 + 16].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
    assert!(matches!(parse_truetype_font_info(&oob), Err(TextError::InvalidSfnt { .. })));
}

#[test]
fn cjk_candidates_and_bundled_font_helpers_behave_consistently() {
    let ja = system_cjk_font_candidates("ja-JP");
    assert!(ja.iter().any(|name| name.contains("Hiragino") || name.contains("Hirakaku")));
    assert!(ja.iter().any(|name| name.contains("Yu Gothic")));
    assert!(ja.iter().any(|name| name.contains("Noto Sans CJK")));

    let ko = system_cjk_font_candidates("ko-KR");
    assert!(ko.iter().any(|name| name.contains("Malgun Gothic")));

    let zh_cn = system_cjk_font_candidates("zh-CN");
    assert!(zh_cn.iter().any(|name| name.contains("PingFang")));

    let zh_tw = system_cjk_font_candidates("zh-TW");
    assert!(zh_tw.iter().any(|name| name.contains("PingFang TC")));

    let fallback = system_cjk_font_candidates("en-US");
    assert!(fallback.iter().any(|name| name.contains("Noto Sans CJK")));
    assert!(fallback.iter().any(|name| name.contains("PingFang")));
    assert!(fallback.iter().any(|name| name.contains("Malgun Gothic")));
    assert!(fallback.iter().any(|name| name.contains("Yu Gothic")));
    assert!(fallback.iter().any(|name| name.contains("Hiragino")));

    assert_eq!(has_bundled_or_ttf_font(), !BUNDLED_FONTS.is_empty());
}
