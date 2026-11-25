//! Axis scale mapping, guarded domain diagnostics, major/minor nice-tick computation,
//! discontinuous multi-segment axes, frame presets, thresholds, legends, and annotations.

mod annotations;
mod mapping;
mod segments;
mod ticks;

use curvebook_palette::Rgba8;

pub use crate::markers::ellipse_path;
pub(crate) use annotations::{
    AnnotationBudget, LegendPlacement, auto_legend_box_width_pt, render_annotations_with_budget, render_legend_with_placement,
};
pub use annotations::{LegendEntry, legend_entries_from_graph_spec, render_annotations, render_legend, summarize_warnings};
pub use mapping::{AxisMapping, ErrorBarMappedPt, ScaleDiagnostics, scale_inverse, scale_transform};
#[cfg(test)]
pub(crate) use segments::resolve_sample_segment_domain;
pub use segments::{
    AutoRangeHeadroomPolicy, AxisBreakGap, AxisOrientation, ClippedSegmentPath, DEFAULT_AUTORANGE_HEADROOM, ResolvedAxis, ResolvedSegment,
    SegmentIntervalPt, SegmentedErrorBar, SegmentedPointMatch, segment_cell_rect, segment_has_manual_limits, segment_is_auto_range,
    split_polyline_across_segments,
};
pub(crate) use segments::{resolve_exact_domain_with_warnings, sanitize_linear_range};
pub(crate) use ticks::{
    AdditionalYTickLayout, compute_additional_ticks_for_segments, compute_segment_tick_values, emit_additional_tick_gridline_segments,
    emit_additional_x_tick_primitives, emit_additional_y_tick_primitives, format_scalar_clean, format_tick_markup, resolve_template_hooks_with_info,
    tick_span_offsets,
};
pub use ticks::{
    AxisTick, CategoricalTick, ResolvedAdditionalTick, TickComputeConfig, TickKind, compute_additional_ticks, compute_axis_ticks,
    compute_axis_ticks_with_warnings, compute_segment_ticks, compute_segment_ticks_with_warnings, format_tick_rich, resolve_template_hooks,
};

/// Upper bound on ticks emitted per segment to prevent unbounded allocation on adversarial specs.
pub const MAX_TICKS_PER_SEGMENT: usize = 256;

/// Fallback strictly positive lower floor used when clamping invalid log-scale bounds.
pub const LOG_POSITIVE_FLOOR_DEFAULT: f64 = 1e-12;

/// Default gridline colour used for additional-tick gridlines.
pub const DEFAULT_GRID_COLOR: Rgba8 = Rgba8::rgba(210, 210, 210, 180);

const MIN_FINITE_AXIS_SPAN: f64 = f64::MIN_POSITIVE * 4.0;

#[cfg(test)]
mod tests {
    use curvebook_doc::{AxisSegment, AxisSpec, TickLabelFormat};
    use curvebook_math::AxisScale;
    use curvebook_text::{LocaleNumberPolicy, SpanScript};

    use super::segments::{nice_linear_bounds, resolve_sample_segment_domain};
    use super::ticks::compute_safe_linear_ticks;
    use super::{
        AxisMapping, AxisOrientation, ResolvedAxis, ResolvedSegment, TickComputeConfig, TickKind, compute_segment_ticks_with_warnings,
        format_tick_rich, resolve_exact_domain_with_warnings, sanitize_linear_range,
    };
    use crate::scene::RectPt;

    #[test]
    fn rv20_tiny_linear_domains_and_constant_padding_preserve_scale() {
        // 1. Explicit `[0.0, 1e-16]` maps cleanly without returning `None` or expanding to `[-0.5, 0.5]`.
        let tiny_map = AxisMapping { min: 0.0, max: 1e-16, scale: AxisScale::Linear };
        let p0 = tiny_map.map_value(0.0, 0.0, 200.0).expect("map 0.0 on [0, 1e-16]");
        let p_mid = tiny_map.map_value(0.5e-16, 0.0, 200.0).expect("map midpoint on [0, 1e-16]");
        let p1 = tiny_map.map_value(1e-16, 0.0, 200.0).expect("map 1e-16 on [0, 1e-16]");
        assert!((p0 - 0.0).abs() < 1e-12);
        assert!((p_mid - 100.0).abs() < 1e-12);
        assert!((p1 - 200.0).abs() < 1e-12);
        let roundtrip = tiny_map.unmap_coord(p_mid, 0.0, 200.0).expect("unmap midpoint on [0, 1e-16]");
        assert!((roundtrip - 0.5e-16).abs() < 1e-30);

        let mut explicit_spec = AxisSpec::linear("Tiny");
        explicit_spec.segments[0].min = Some(0.0);
        explicit_spec.segments[0].max = Some(1e-16);
        let mut warnings = Vec::new();
        let resolved_tiny =
            ResolvedAxis::resolve(&explicit_spec, AxisOrientation::Horizontal, RectPt::new(0.0, 0.0, 200.0, 100.0), None, &mut warnings);
        assert!(warnings.is_empty(), "explicit [0, 1e-16] should not warn: {warnings:?}");
        assert_eq!((resolved_tiny.segments[0].domain_min, resolved_tiny.segments[0].domain_max), (0.0, 1e-16));
        assert!((resolved_tiny.map_value(0.5e-16).expect("segment map").coord_pt - 100.0).abs() < 1e-12);

        // 2. Auto-domain on picomolar/femtomolar `[1e-13, 6e-13]` (span `5e-13`) is NOT treated as degenerate or padded by +/-1.0.
        let (n_lo, n_hi) = nice_linear_bounds(1e-13, 6e-13);
        assert!((0.0..=1e-13).contains(&n_lo), "unexpected nice_lo: {n_lo:e}");
        assert!((6e-13..=1e-12).contains(&n_hi), "unexpected nice_hi: {n_hi:e}");

        let (san_lo, san_hi) = sanitize_linear_range(1e-13, 6e-13, (0.0, 1.0));
        assert_eq!((san_lo, san_hi), (1e-13, 6e-13));

        let seg_auto = AxisSegment::default();
        let mut sample_warnings = Vec::new();
        let (s_lo, s_hi) = resolve_sample_segment_domain(&seg_auto, &[1e-13, 6e-13], false, 0.05, &mut sample_warnings);
        assert!(sample_warnings.is_empty());
        assert!(s_lo <= 1e-13 && s_hi >= 6e-13 && (s_hi - s_lo) < 1e-11, "picomolar sample auto-domain collapsed: [{s_lo:e}, {s_hi:e}]");

        // 3. Single-value `1e-12` pads proportionally by `|lo| * 0.1` (`1e-13`), and `1.0` ONLY when `lo == 0.0`.
        let (single_lo, single_hi) = nice_linear_bounds(1e-12, 1e-12);
        assert!((single_lo - 0.9e-12).abs() < 1e-24, "expected 0.9e-12, got {single_lo:e}");
        assert!((single_hi - 1.1e-12).abs() < 1e-24, "expected 1.1e-12, got {single_hi:e}");

        let (san_single_lo, san_single_hi) = sanitize_linear_range(1e-12, 1e-12, (0.0, 1.0));
        assert!((san_single_lo - 0.9e-12).abs() < 1e-24);
        assert!((san_single_hi - 1.1e-12).abs() < 1e-24);

        let (single_samp_lo, single_samp_hi) = resolve_sample_segment_domain(&seg_auto, &[1e-12], false, 0.0, &mut sample_warnings);
        assert!(single_samp_lo > 0.0 && single_samp_lo < 1e-12 && single_samp_hi > 1e-12 && single_samp_hi < 2e-12);

        let (zero_lo, zero_hi) = nice_linear_bounds(0.0, 0.0);
        assert_eq!((zero_lo, zero_hi), (-1.0, 1.0));
    }

    #[test]
    fn rv21_scientific_and_engineering_si_ticks_carry_after_rounding() {
        let policy = LocaleNumberPolicy::publication();

        // Scientific superscript: `9.9996e5` rounds to `1.000e6` -> `10^6` (never `10×10^5`).
        let pos_sup = format_tick_rich(9.9996e5, AxisScale::Log10, TickLabelFormat::PowerOfTenSuperscript, &policy);
        assert_eq!(pos_sup.spans.len(), 2, "expected 10^6 spans, got {:?}", pos_sup.spans);
        assert_eq!(pos_sup.spans[0].text, "10");
        assert_eq!(pos_sup.spans[1].script, SpanScript::Superscript);
        assert_eq!(pos_sup.spans[1].text, "6");

        let neg_sup = format_tick_rich(-9.9996e5, AxisScale::Linear, TickLabelFormat::PowerOfTenSuperscript, &policy);
        assert_eq!(neg_sup.spans.len(), 2, "expected -10^6 spans, got {:?}", neg_sup.spans);
        assert_eq!(neg_sup.spans[0].text, "\u{2212}10");
        assert_eq!(neg_sup.spans[1].script, SpanScript::Superscript);
        assert_eq!(neg_sup.spans[1].text, "6");

        // Scientific E: `9.9996e5` -> `1e6`, `-9.9996e5` -> `−1e6`.
        let pos_sci = format_tick_rich(9.9996e5, AxisScale::Linear, TickLabelFormat::ScientificE, &policy);
        assert_eq!(pos_sci.plain_text(), "1e6");
        let neg_sci = format_tick_rich(-9.9996e5, AxisScale::Linear, TickLabelFormat::ScientificE, &policy);
        assert_eq!(neg_sci.plain_text(), "\u{2212}1e6");

        // Engineering SI: `999.9996` -> `1 k` (not `1000`), `999_999.6` -> `1 M` (not `1000 k`), plus negative counterparts.
        let pos_si = format_tick_rich(999.9996, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(pos_si.plain_text(), "1 k");
        let neg_si = format_tick_rich(-999.9996, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(neg_si.plain_text(), "\u{2212}1 k");

        let pos_si_mega = format_tick_rich(999_999.6, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(pos_si_mega.plain_text(), "1 M");
        let neg_si_mega = format_tick_rich(-999_999.6, AxisScale::Linear, TickLabelFormat::EngineeringSi, &policy);
        assert_eq!(neg_si_mega.plain_text(), "\u{2212}1 M");
    }

    #[test]
    fn sec09_s1_log_explicit_and_extreme_domain_regressions() {
        let make_log_axis = |title: &str| {
            let mut axis = AxisSpec::linear(title);
            axis.segments[0].scale = AxisScale::Log10;
            axis
        };
        // 1. Sub-picomolar log10 domain `[1e-14, 1e-12]` is preserved without false degeneracy fallback.
        let mut log_axis = make_log_axis("Conc");
        let mut w_sub = Vec::new();
        let (lo_sub, hi_sub) = resolve_exact_domain_with_warnings(&log_axis, 1e-14, 1e-12, &mut w_sub);
        assert!(w_sub.is_empty(), "sub-picomolar log10 domain should not warn: {w_sub:?}");
        assert!((lo_sub - 1e-14).abs() < 1e-28 && (hi_sub - 1e-12).abs() < 1e-26);

        let mut w_samp_sub = Vec::new();
        let (slo_sub, shi_sub) = resolve_sample_segment_domain(&log_axis.segments[0], &[1e-14, 1e-12], false, 0.0, &mut w_samp_sub);
        assert!(w_samp_sub.is_empty(), "sub-picomolar log sample should not warn: {w_samp_sub:?}");
        assert!(slo_sub > 0.0 && slo_sub <= 1e-14 && (1e-12..=1e-11).contains(&shi_sub), "got [{slo_sub:e}, {shi_sub:e}]");

        let sub_seg = ResolvedSegment {
            segment_index: 0,
            domain_min: 1e-14,
            domain_max: 1e-12,
            scale: AxisScale::Log10,
            length_fraction: 1.0,
            span_start_pt: 0.0,
            span_end_pt: 200.0,
            major_tick_interval: None,
            minor_ticks_per_interval: 0,
        };
        let policy = LocaleNumberPolicy::publication();
        let tick_cfg =
            TickComputeConfig { tick_format: TickLabelFormat::default(), policy: &policy, auto_log_minor_ticks: false, categorical_ticks: None };
        let mut w_ticks = Vec::new();
        let sub_ticks = compute_segment_ticks_with_warnings(&sub_seg, tick_cfg, &mut w_ticks);
        let major_vals: Vec<f64> = sub_ticks.iter().filter(|t| t.kind == TickKind::Major).map(|t| t.value).collect();
        assert!(major_vals.len() >= 3, "expected log10 ticks for [1e-14, 1e-12], got {major_vals:?}");
        assert!((major_vals[0] - 1e-14).abs() < 1e-26 && (major_vals[major_vals.len() - 1] - 1e-12).abs() < 1e-24);

        // 2. Non-positive lower log bound repairs relative to `hi` (never hardcoded `0.1` that inverts `[0, 0.05]` or `[0, 1e-12]`).
        let mut w_tiny_log = Vec::new();
        let (lo_t, hi_t) = resolve_exact_domain_with_warnings(&log_axis, 0.0, 1e-12, &mut w_tiny_log);
        assert!(lo_t > 0.0 && lo_t < hi_t && (hi_t - 1e-12).abs() < 1e-26, "got [{lo_t:e}, {hi_t:e}]");
        let (lo_s, hi_s) = resolve_exact_domain_with_warnings(&log_axis, 0.0, 0.05, &mut w_tiny_log);
        assert!(lo_s > 0.0 && lo_s < hi_s && (hi_s - 0.05).abs() < 1e-12, "got [{lo_s:e}, {hi_s:e}]");

        // 3. Single-sided explicit bounds extend non-degenerately, while two-sided reversed explicit bounds stay preserved.
        let mut lin_one_lo = AxisSpec::linear("X");
        lin_one_lo.segments[0].min = Some(100.0);
        let mut w_one = Vec::new();
        let (ol_lo, ol_hi) = resolve_exact_domain_with_warnings(&lin_one_lo, 1.0, 5.0, &mut w_one);
        assert!(ol_lo == 100.0 && ol_hi > ol_lo, "single-sided min=100 must extend max > 100, got [{ol_lo}, {ol_hi}]");
        let (sl_lo, sl_hi) = resolve_sample_segment_domain(&lin_one_lo.segments[0], &[1.0, 5.0], false, 0.0, &mut w_one);
        assert!(sl_lo == 100.0 && sl_hi > sl_lo, "sample single-sided min=100 must extend max > 100, got [{sl_lo}, {sl_hi}]");

        log_axis.segments[0].min = None;
        log_axis.segments[0].max = Some(1.0);
        let (lm_lo, lm_hi) = resolve_sample_segment_domain(&log_axis.segments[0], &[10.0, 100.0], false, 0.0, &mut w_one);
        assert!(lm_lo > 0.0 && lm_lo < 1.0 && (lm_hi - 1.0).abs() < 1e-12, "single-sided log max=1 must drop lo < 1, got [{lm_lo}, {lm_hi}]");

        let mut rev_lin = AxisSpec::linear("RevLin");
        rev_lin.segments[0].min = Some(100.0);
        rev_lin.segments[0].max = Some(0.0);
        assert_eq!(resolve_exact_domain_with_warnings(&rev_lin, 10.0, 50.0, &mut w_one), (100.0, 0.0));

        let mut rev_log = make_log_axis("RevLog");
        rev_log.segments[0].min = Some(100.0);
        rev_log.segments[0].max = Some(1.0);
        assert_eq!(resolve_exact_domain_with_warnings(&rev_log, 2.0, 50.0, &mut w_one), (100.0, 1.0));

        // 4. Extreme opposite-sign linear range `[-1e308, 1e308]` maps, unmaps, resolves, and ticks without `f64` overflow.
        let extreme_lin = AxisSpec::linear("Extreme");
        let mut w_ext = Vec::new();
        let (ex_lo, ex_hi) = resolve_exact_domain_with_warnings(&extreme_lin, -1e308, 1e308, &mut w_ext);
        assert!(ex_lo.is_finite() && ex_hi.is_finite() && ex_lo <= -1e308 && ex_hi >= 1e308);
        let (exs_lo, exs_hi) = resolve_sample_segment_domain(&extreme_lin.segments[0], &[-1e308, 1e308], false, 0.0, &mut w_ext);
        assert!(exs_lo.is_finite() && exs_hi.is_finite() && exs_lo <= -1e308 && exs_hi >= 1e308);

        let ex_map = AxisMapping { min: -1e308, max: 1e308, scale: AxisScale::Linear };
        let px_mid = ex_map.map_value(0.0, 0.0, 200.0).expect("map 0 on [-1e308, 1e308]");
        assert!((px_mid - 100.0).abs() < 1e-9, "px_mid={px_mid}");
        let px_lo = ex_map.map_value(-1e308, 0.0, 200.0).expect("map -1e308");
        let px_hi = ex_map.map_value(1e308, 0.0, 200.0).expect("map +1e308");
        assert!((px_lo - 0.0).abs() < 1e-9 && (px_hi - 200.0).abs() < 1e-9);
        let un_q1 = ex_map.unmap_coord(50.0, 0.0, 200.0).expect("unmap quarter coord");
        assert!((un_q1 - (-5e307)).abs() / 5e307 < 1e-9, "un_q1={un_q1:e}");

        let (ext_ticks, ext_step, _) = compute_safe_linear_ticks(-1e308, 1e308, None, 6, &mut w_ext);
        assert!(ext_step.is_finite() && ext_step > 0.0 && ext_ticks.len() >= 2);
        assert!(ext_ticks.iter().all(|v| v.is_finite()));
        assert!(ext_ticks.iter().any(|&v| v < 0.0) && ext_ticks.iter().any(|&v| v > 0.0));
    }
}
