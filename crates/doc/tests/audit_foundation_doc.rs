//! Targeted regression tests for foundation audit fixes in `curvebook-doc` (#4, #94).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{BlandAltmanSpec, ColumnTitleHookError, parse_column_title_numeric_hook};

#[test]
fn audit_04_bland_altman_xy_compare_mode_serde_and_defaults() {
    let def = BlandAltmanSpec::default();
    assert_eq!(def.xy_compare_x_vs_y, None);

    // Default omits `xy_compare_x_vs_y` cleanly and legacy payloads deserialize as `None`.
    let json_def = serde_json::to_string(&def).unwrap();
    assert!(!json_def.contains("xy_compare_x_vs_y"), "default JSON unexpectedly serialized None field: {json_def}");
    let legacy = r#"{"column_a":0,"column_b":1,"percent_difference":false}"#;
    assert_eq!(serde_json::from_str::<BlandAltmanSpec>(legacy).unwrap(), def);

    for mode in [None, Some(true), Some(false)] {
        let spec = BlandAltmanSpec { xy_compare_x_vs_y: mode, ..BlandAltmanSpec::default() };
        let rt: BlandAltmanSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(rt, spec);
    }
}

#[test]
fn audit_94_column_title_numeric_hook_accepts_valid_forms_and_rejects_ambiguous_titles() {
    // 1. Standalone finite numbers.
    for (title, expected) in [
        ("0", 0.0),
        ("-0.0", 0.0),
        ("10", 10.0),
        ("+10", 10.0),
        ("-2.5", -2.5),
        (".5", 0.5),
        ("-.25", -0.25),
        ("1e-7", 1e-7),
        ("  25.0  ", 25.0),
        ("007", 7.0),
    ] {
        let parsed = parse_column_title_numeric_hook(title).unwrap_or_else(|e| panic!("expected `{title}` to parse, got {e}"));
        assert!((parsed - expected).abs() <= 1e-15 * expected.abs().max(1.0), "`{title}` -> {parsed} vs {expected}");
        assert!(!parsed.is_sign_negative() || expected < 0.0);
    }

    // 2. Finite numbers with trailing unit.
    for (title, expected) in [
        ("10 nM", 10.0),
        ("25.0 uM", 25.0),
        ("1e-6 M", 1e-6),
        ("-7.5 logM", -7.5),
        ("10 ng/mL", 10.0),
        ("5%", 5.0),
        ("5 %", 5.0),
        ("100 U/mL", 100.0),
        ("12.5 µM", 12.5),
    ] {
        let parsed = parse_column_title_numeric_hook(title).unwrap_or_else(|e| panic!("expected `{title}` to parse, got {e}"));
        assert!((parsed - expected).abs() <= 1e-15 * expected.abs().max(1.0), "`{title}` -> {parsed} vs {expected}");
    }

    // 3. Explicit assignment `[name] = value [unit]` or `name = value [unit]`.
    for (title, expected) in [
        ("[I] = 10 nM", 10.0),
        ("I = 10 nM", 10.0),
        ("[AntagConc] = 1e-7 M", 1e-7),
        ("[B]=3.5 uM", 3.5),
        ("Conc = -6.5", -6.5),
        ("[O2] = 5%", 5.0),
        ("[IL-6] = 10 ng/mL", 10.0),
    ] {
        let parsed = parse_column_title_numeric_hook(title).unwrap_or_else(|e| panic!("expected `{title}` to parse, got {e}"));
        assert!((parsed - expected).abs() <= 1e-15 * expected.abs().max(1.0), "`{title}` -> {parsed} vs {expected}");
    }

    // 4. Reject embedded hyphens/digits inside identifiers, ranges, multi-number strings, and non-finite inputs.
    for bad in [
        "",
        "   ",
        "Inhibitor-10 nM",
        "IL-6 10 ng/mL",
        "CO2 5%",
        "Hot1.5",
        "Group A",
        "10-20 nM",
        "10-fold",
        "10 20",
        "10 nM 20 uM",
        "10 - 5",
        "10,5 nM",
        "NaN",
        "NaN nM",
        "inf",
        "-inf M",
        "999e999 nM",
        "= 10 nM",
        "5 = 10",
        "-1 = 10",
        "[10] = 5",
        "[I = 10 nM",
        "[I] = ",
        "[I] = Inhibitor-10 nM",
        "[I] = 10 = 20",
        "5%%",
    ] {
        let err = parse_column_title_numeric_hook(bad).unwrap_err();
        assert!(!err.to_string().is_empty(), "empty error message for `{bad}`");
    }

    assert!(matches!(parse_column_title_numeric_hook("  "), Err(ColumnTitleHookError::EmptyTitle)));
    assert!(matches!(parse_column_title_numeric_hook("Inhibitor-10 nM"), Err(ColumnTitleHookError::MissingNumericValue { .. })));
    assert!(matches!(parse_column_title_numeric_hook("IL-6 10 ng/mL"), Err(ColumnTitleHookError::MissingNumericValue { .. })));
    assert!(matches!(parse_column_title_numeric_hook("CO2 5%"), Err(ColumnTitleHookError::MissingNumericValue { .. })));
    assert!(matches!(parse_column_title_numeric_hook("NaN"), Err(ColumnTitleHookError::NonFiniteValue { .. })));
    assert!(matches!(parse_column_title_numeric_hook("5 = 10"), Err(ColumnTitleHookError::InvalidAssignment { .. })));
    assert!(matches!(parse_column_title_numeric_hook("10 20"), Err(ColumnTitleHookError::InvalidTrailingUnit { .. })));
}
