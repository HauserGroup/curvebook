use curvebook_doc::{AnalysisKind, AnalysisSpec, ResultCell, SimulationNoise, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable};
use curvebook_transform::{TransformContext, TransformError, execute_analysis_transform, execute_transform, parse_formula};

use crate::common::{mv_sheet, num, sheet};

fn assert_seed_summary_cell(summary_rows: &[Vec<ResultCell>], row_labels: &[String], expected_seed: u64) {
    let idx = row_labels.iter().position(|label| label == "Seed").expect("summary tab must include a Seed row");
    assert_eq!(summary_rows.get(idx).map(Vec::as_slice), Some([ResultCell::Text(expected_seed.to_string())].as_slice()));
}

#[test]
fn r1_s04_12_simulate_seed_preserves_full_u64_precision() {
    let src = sheet("sim-src", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 1).unwrap()));
    let ctx = TransformContext::default();

    for seed in [0_u64, 9_007_199_254_740_993_u64, u64::MAX] {
        let spec = TransformAnalysisSpec::Simulate {
            seed,
            row_count: 3,
            replicates: 1,
            x_min: 0.0,
            x_max: 2.0,
            equation_id: String::from("linear"),
            parameters: vec![(String::from("slope"), 1.0), (String::from("intercept"), 0.0)],
            noise: SimulationNoise::Gaussian { sd: 0.5 },
            simulation_mode: None,
        };
        let out = execute_transform(&src, &spec, &ctx).unwrap();
        assert_seed_summary_cell(&out.summary_tab.rows, &out.summary_tab.row_labels, seed);
        assert!(format!("{:?}", out.summary_tab).contains(&seed.to_string()));

        let analysis = AnalysisSpec::new("an-sim", "sim-src", AnalysisKind::Transform(spec)).unwrap();
        let res = execute_analysis_transform(&src, &analysis, &ctx).unwrap();
        assert_seed_summary_cell(&res.tabs[0].rows, &res.tabs[0].row_labels, seed);
    }
}

#[test]
fn r1_s04_12_random_sample_seed_preserves_full_u64_precision_and_reproduces() {
    let cells: Vec<_> = (0..20).map(|r| num(r as f64)).collect();
    let src = mv_sheet("mv-src", &[("A", &cells)]);
    let ctx = TransformContext::default();

    for seed in [9_007_199_254_740_993_u64, u64::MAX] {
        let spec = TransformAnalysisSpec::RandomSample { seed, sample_size: 5, stratify_column: None, proportional_allocation: false };
        let out = execute_transform(&src, &spec, &ctx).unwrap();
        assert_seed_summary_cell(&out.summary_tab.rows, &out.summary_tab.row_labels, seed);

        let seed_idx = out.summary_tab.row_labels.iter().position(|label| label == "Seed").unwrap();
        let ResultCell::Text(displayed_seed) = &out.summary_tab.rows[seed_idx][0] else {
            panic!("expected ResultCell::Text for Seed row, got {:?}", out.summary_tab.rows[seed_idx][0]);
        };
        let parsed_seed = displayed_seed.parse::<u64>().unwrap();
        assert_eq!(parsed_seed, seed);

        let rerun = execute_transform(
            &src,
            &TransformAnalysisSpec::RandomSample { seed: parsed_seed, sample_size: 5, stratify_column: None, proportional_allocation: false },
            &ctx,
        )
        .unwrap();
        assert_eq!(rerun.data_sheet.archetype, out.data_sheet.archetype);
    }
}

#[test]
fn r1_s04_13_non_ascii_unexpected_char_reports_exact_span() {
    let cases = [
        ("[A]\u{a0}+ 1", 3_usize, '\u{a0}'),
        ("[A] + €", 6_usize, '€'),
        ("  = [A] + €   ", 10_usize, '€'),
        ("  = [A]\u{a0}+ 1  ", 7_usize, '\u{a0}'),
        ("x × y", 2_usize, '×'),
        ("² + 1", 0_usize, '²'),
        ("1 − 2", 2_usize, '−'),
        ("1 +\u{200b} 2", 3_usize, '\u{200b}'),
        ("😀", 0_usize, '😀'),
    ];
    for (formula, expected_pos, expected_ch) in cases {
        let TransformError::ParseError { position, message } = parse_formula(formula).unwrap_err() else {
            panic!("expected ParseError for `{formula}`");
        };
        assert!(!message.contains("token limit"), "unexpected token-limit error for `{formula}`: {message}");
        assert!(message.contains("unexpected character") && message.contains(expected_ch), "bad message `{message}` for `{formula}`");
        assert_eq!(position, expected_pos, "wrong byte span for `{formula}`");
        assert!(formula.get(position..).is_some_and(|tail| tail.starts_with(expected_ch)), "span {position} mismatch in `{formula}`");
    }

    let dollar_err = parse_formula("  = $€  ").unwrap_err();
    assert!(matches!(dollar_err, TransformError::ParseError { position: 4, ref message } if message.contains("expected variable name")));
}

#[test]
fn r1_s04_13_valid_unicode_identifiers_and_r1_s04_02_drop_remain_intact() {
    for valid in ["α + β * µM", "[Price (€)] * 1.2"] {
        assert!(parse_formula(valid).is_ok(), "expected valid formula `{valid}`");
    }
    let chain = vec!["1"; 8_000].join("+");
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || drop(parse_formula(&chain).unwrap()))
        .unwrap()
        .join()
        .expect("R1-S04-02 iterative Drop must not overflow 2 MiB stack");
}
