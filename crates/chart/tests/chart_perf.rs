//! Wall-clock guards for the chart crate. Selected by `PERFORMANCE_FILTER` (`binary(*perf*)`), kept out of the
//! general `chart_suite` so timing noise never fails the regular gate.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_chart::ChartScene;
use curvebook_chart::equation_box::render_tex_equation_box;
use curvebook_doc::RectPt as DocRectPt;

#[test]
fn gap_01_02_perf_adversarial_equation_inputs_parse_in_linear_time() {
    let cases = [
        ("open_parens", "(".repeat(100_000)),
        ("open_brackets", "[".repeat(100_000)),
        ("pipe_parens", "|(".repeat(50_000)),
        ("bare_lt", "<".repeat(100_000)),
        ("unclosed_b", "<b>".repeat(33_333)),
        ("unclosed_sup", "<sup>".repeat(20_000)),
        ("unclosed_left", "\\left(".repeat(16_000)),
        ("unclosed_sqrt_opt", "\\sqrt[".repeat(16_000)),
        ("many_dollars", "$x$ ".repeat(30_000)),
        ("nested_dollar_groups", "{$x$} ".repeat(20_000)),
    ];
    for (label, payload) in &cases {
        let start = Instant::now();
        let mut scene = ChartScene::new("g-adv", *label, 400.0, 200.0, None);
        render_tex_equation_box(&mut scene, DocRectPt::new(10.0, 10.0, 0.0, 0.0), payload, 10.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs_f64() < 5.0, "{label} took too long ({elapsed:?}); suspected quadratic rescan");
    }
}
