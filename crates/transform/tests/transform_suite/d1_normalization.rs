//! Independent D1 oracles for normalization, baseline removal, and mathematical transform
//! uncertainty propagation (`crates/transform/src/ops/`).
//!
//! Reference provenance:
//! - Exact rational arithmetic (`fractions.Fraction`) for affine normalization and linear
//!   windowed/row-wise baseline removal.
//! - High-precision nonlinear evaluations and analytical derivatives (`JCGM 100:2008` / GUM §5.1.2)
//!   via `mpmath 1.3.0` at 80 and 120 decimal digits (agreeing within `7.05e-80`), corroborated
//!   with `R 4.5.2` to full IEEE-754 binary64 precision.
//! - Reproduce all stored reference constants with `reference/normalization.py`.
//!
//! Tolerance justification:
//! - `LINEAR_TOL = 1e-12`: bounds IEEE-754 binary64 rounding across small affine rescalings,
//!   compensated means, and row-index linear interpolations.
//! - `NONLINEAR_ABS_TOL = 1e-12`, `NONLINEAR_REL_TOL = 1e-12`: bounds libm elementary function
//!   rounding (`ln`, `log10`, `asin`, `sqrt`) and `norm_ppf` / `norm_pdf` probit evaluation against
//!   120-digit `mpmath` constants rounded once to binary64.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_6, LN_2, LOG10_2};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, CellValue, ErrorPropagationMode, HookSource, MathTransformPreset, NormalizeAnchor, ResultCell,
    ResultStatus, ResultTabKind, SheetId, SubcolumnMode, TransformAnalysisSpec,
};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    BaselineKind, MathTransformOp, TransformContext, TransformError, execute_analysis_transform, execute_math_op, execute_math_transform,
    execute_normalize, execute_remove_baseline, execute_remove_baseline_kind, execute_transform,
};

use crate::common::{
    SubExp::{Empty, Num},
    Xy3Col, as_grouped, as_xy, assert_cells_3, assert_col_vals, assert_scalar_anchor, col_sheet, empty, excl, grouped_sheet, info_with_numbers, num,
    xy_3sub, xy_3sub_clean,
};

const LINEAR_TOL: f64 = 1e-12;
const NONLINEAR_ABS_TOL: f64 = 1e-12;
const NONLINEAR_REL_TOL: f64 = 1e-12;
const NL_TOL: (f64, f64) = (NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL);

#[test]
fn normalization_per_dataset_and_pooled_fraction_and_percent_match_rationals() {
    // Unequal dataset ranges with excluded extremes (500* high in Ctrl, -200* low in Drug) and blank row 3.
    let s = col_sheet(
        "norm-src",
        &[
            ("Ctrl", &[num(10.0), num(30.0), num(50.0), empty(), num(90.0), excl(500.0)]),
            ("Drug", &[excl(-200.0), num(30.0), num(70.0), empty(), num(110.0), num(190.0)]),
        ],
    );
    let ctx = TransformContext::default();

    struct NormCase<'a> {
        label: &'a str,
        pct: bool,
        per_ds: bool,
        anchors: &'a [(Option<usize>, f64, f64)],
        col0: &'a [Option<f64>],
        col1: &'a [Option<f64>],
    }

    let cases = [
        NormCase {
            label: "per_frac",
            pct: false,
            per_ds: true,
            anchors: &[(Some(0), 10.0, 90.0), (Some(1), 30.0, 190.0)],
            col0: &[Some(0.0), Some(0.25), Some(0.5), None, Some(1.0), Some(6.125)],
            col1: &[Some(-1.4375), Some(0.0), Some(0.25), None, Some(0.5), Some(1.0)],
        },
        NormCase {
            label: "per_pct",
            pct: true,
            per_ds: true,
            anchors: &[(Some(0), 10.0, 90.0), (Some(1), 30.0, 190.0)],
            col0: &[Some(0.0), Some(25.0), Some(50.0), None, Some(100.0), Some(612.5)],
            col1: &[Some(-143.75), Some(0.0), Some(25.0), None, Some(50.0), Some(100.0)],
        },
        NormCase {
            label: "pool_frac",
            pct: false,
            per_ds: false,
            anchors: &[(None, 10.0, 190.0)],
            col0: &[
                Some(0.0),
                Some(0.111_111_111_111_111_1),
                Some(0.222_222_222_222_222_2),
                None,
                Some(0.444_444_444_444_444_4),
                Some(2.722_222_222_222_222_3),
            ],
            col1: &[
                Some(-1.166_666_666_666_666_7),
                Some(0.111_111_111_111_111_1),
                Some(0.333_333_333_333_333_3),
                None,
                Some(0.555_555_555_555_555_6),
                Some(1.0),
            ],
        },
    ];

    for c in cases {
        let out = if c.pct {
            let spec = TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::ColumnMin,
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: c.per_ds,
            };
            execute_transform(&s, &spec, &ctx).unwrap()
        } else {
            execute_normalize(&s, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, false, c.per_ds, &ctx).unwrap()
        };
        assert_eq!(out.scalars.len(), c.anchors.len() * 2, "{}", c.label);
        for &(ds, z, h) in c.anchors {
            assert_scalar_anchor(&out.scalars, "ZeroAnchor", ds, z, LINEAR_TOL, c.label);
            assert_scalar_anchor(&out.scalars, "HundredAnchor", ds, h, LINEAR_TOL, c.label);
        }
        assert_col_vals(&out.data_sheet, 0, c.col0, &[5], LINEAR_TOL, c.label);
        assert_col_vals(&out.data_sheet, 1, c.col1, &[0], LINEAR_TOL, c.label);
    }

    // 1d. Pooled percentage via typed AnalysisSpec -> ResultSheet (AC #4).
    let norm_spec = TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::ColumnMin,
        hundred_anchor: NormalizeAnchor::ColumnMax,
        output_percent: true,
        per_data_set: false,
    };
    let analysis = AnalysisSpec::new("an-norm-pooled", "norm-src", AnalysisKind::Transform(norm_spec)).unwrap();
    let res = execute_analysis_transform(&s, &analysis, &ctx).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);
    assert_eq!((res.title(), res.tabs.len(), res.tabs[0].kind), ("Normalize", 1, ResultTabKind::MainSummary));
    assert_approx_eq(res.lookup_scalar("ZeroAnchor", None).unwrap(), 10.0, LINEAR_TOL, 0.0, "ResultSheet ZeroAnchor");
    assert_approx_eq(res.lookup_scalar("zeroanchor", None).unwrap(), 10.0, LINEAR_TOL, 0.0, "ResultSheet zeroanchor");
    assert_approx_eq(res.lookup_scalar("HundredAnchor", None).unwrap(), 190.0, LINEAR_TOL, 0.0, "ResultSheet HundredAnchor");
    assert_eq!(
        res.tabs[0].rows[0..3],
        [vec![ResultCell::Boolean(false)], vec![ResultCell::Boolean(true)], vec![ResultCell::Number { value: 2.0, prefix: None, units: None }]]
    );

    let norm_data = res.analyzable_data.as_ref().unwrap();
    let pool_pct_c0 =
        [Some(0.0), Some(11.111_111_111_111_11), Some(22.222_222_222_222_22), None, Some(44.444_444_444_444_44), Some(272.222_222_222_222_23)];
    let pool_pct_c1 =
        [Some(-116.666_666_666_666_67), Some(11.111_111_111_111_11), Some(33.333_333_333_333_336), None, Some(55.555_555_555_555_56), Some(100.0)];
    assert_col_vals(norm_data, 0, &pool_pct_c0, &[5], LINEAR_TOL, "pool_pct_result");
    assert_col_vals(norm_data, 1, &pool_pct_c1, &[0], LINEAR_TOL, "pool_pct_result");
}

#[test]
fn normalization_coincident_anchors_return_invalid_spec_and_hooked_summary_scales() {
    let ctx = TransformContext::default();
    let const_col = col_sheet(
        "norm-coincident",
        &[("Varying", &[num(1.0), num(5.0), empty(), num(9.0)]), ("Constant", &[num(25.0), num(25.0), empty(), excl(99.0)])],
    );
    let all_const = col_sheet("norm-all-const", &[("A", &[num(12.5), excl(-50.0)]), ("B", &[num(12.5), empty()])]);
    for (label, s, z, h, pct, per) in [
        ("per-dataset coincident", &const_col, NormalizeAnchor::ColumnMin, NormalizeAnchor::ColumnMax, true, true),
        ("pooled coincident", &all_const, NormalizeAnchor::ColumnMin, NormalizeAnchor::ColumnMax, false, false),
        ("fixed coincident", &const_col, NormalizeAnchor::FixedValue(7.5), NormalizeAnchor::FixedValue(7.5), true, false),
    ] {
        assert!(matches!(execute_normalize(s, &z, &h, pct, per, &ctx), Err(TransformError::InvalidSpec { .. })), "{label}");
    }

    // ControlColumnMean (col 0 active mean = 20.0) + Hooked InfoConstant (100.0) on MeanSemN.
    let info = info_with_numbers("info-norm", &[("MaxPlateau", 100.0)]);
    let hook_ctx = TransformContext { info_sheet: Some(&info), selected_data_sets: vec![1], ..TransformContext::default() };
    let xy_sem = xy_3sub_clean(
        "xy-norm-sem",
        SubcolumnMode::MeanSemN,
        &[1.0, 2.0, 3.0],
        &[
            ("Ctrl", &[[Some(10.0), Some(1.0), Some(3.0)], [Some(20.0), Some(1.0), Some(3.0)], [Some(30.0), Some(1.0), Some(3.0)]]),
            ("Trt", &[[Some(40.0), Some(4.0), Some(5.0)], [Some(60.0), Some(8.0), Some(5.0)], [Some(100.0), Some(4.0), Some(5.0)]]),
        ],
    );
    let hook = NormalizeAnchor::Hooked(HookSource::InfoConstant {
        info_sheet_id: Some(SheetId::new("info-norm")),
        constant_name: String::from("MaxPlateau"),
    });
    let sem_norm = execute_normalize(&xy_sem, &NormalizeAnchor::ControlColumnMean { column: 0 }, &hook, true, true, &hook_ctx).unwrap();
    assert_scalar_anchor(&sem_norm.scalars, "ZeroAnchor", Some(1), 20.0, LINEAR_TOL, "MeanSemN hooked");
    assert_scalar_anchor(&sem_norm.scalars, "HundredAnchor", Some(1), 100.0, LINEAR_TOL, "MeanSemN hooked");
    let sem_xy = as_xy(&sem_norm.data_sheet);
    assert_cells_3(&sem_xy.data_sets[0].cells[0], [Num(10.0), Num(1.0), Num(3.0)], (LINEAR_TOL, 0.0), "unselected Ctrl r0");
    for (r, exp) in [(0, [Num(25.0), Num(5.0), Num(5.0)]), (1, [Num(50.0), Num(10.0), Num(5.0)]), (2, [Num(100.0), Num(5.0), Num(5.0)])] {
        assert_cells_3(&sem_xy.data_sets[1].cells[r], exp, (LINEAR_TOL, 0.0), &format!("MeanSemN r{r}"));
    }

    // Decreasing normalization (ZeroAnchor = 100 > HundredAnchor = 20) reverses MeanLowHigh & UpperLowerLimits.
    let xy_lh = xy_3sub_clean("xy-norm-lh", SubcolumnMode::MeanLowHigh, &[1.0], &[("Series", &[[Some(40.0), Some(4.0), Some(12.0)]])]);
    let dec_lh = execute_normalize(&xy_lh, &NormalizeAnchor::FixedValue(100.0), &NormalizeAnchor::FixedValue(20.0), true, true, &ctx).unwrap();
    assert_cells_3(&as_xy(&dec_lh.data_sheet).data_sets[0].cells[0], [Num(75.0), Num(15.0), Num(5.0)], (LINEAR_TOL, 0.0), "dec MeanLowHigh");

    let xy_ul = xy_3sub_clean(
        "xy-norm-ul",
        SubcolumnMode::UpperLowerLimits,
        &[1.0, 2.0],
        &[("Limits", &[[Some(40.0), Some(36.0), Some(52.0)], [Some(40.0), Some(36.0), None]])],
    );
    let dec_ul = execute_normalize(&xy_ul, &NormalizeAnchor::FixedValue(100.0), &NormalizeAnchor::FixedValue(20.0), true, true, &ctx).unwrap();
    let ul_xy = as_xy(&dec_ul.data_sheet);
    assert_cells_3(&ul_xy.data_sets[0].cells[0], [Num(75.0), Num(60.0), Num(80.0)], (LINEAR_TOL, 0.0), "dec UL r0");
    assert_cells_3(&ul_xy.data_sets[0].cells[1], [Num(75.0), Empty, Num(80.0)], (LINEAR_TOL, 0.0), "dec UL r1");
}

#[test]
fn baseline_first_and_last_linear_windows_zero_denominator_and_missing_anchor() {
    // 9 rows, first_k = 2 (rows 0..2), last_k = 4 (rows 5..9). Anchors use exact active
    // observation row-index centroids: Col 0 B(r) = -3 + 72*(r - 1/2)/37; Col 1 B(r) = 1 + 20*r/13;
    // Col 2 has no active leading anchor (6 active cells blanked + warned); Col 3 B(r) = 2r - 4 so B(2) = 0.
    let s = col_sheet(
        "base-lin",
        &[
            ("ExcludedTrailingCentroid", &[num(-4.0), num(-2.0), num(5.0), num(8.0), empty(), num(6.0), excl(100.0), num(9.0), num(12.0)]),
            ("MissingInWindow", &[num(1.0), empty(), num(4.0), num(7.0), num(10.0), num(7.0), num(9.0), num(13.0), num(15.0)]),
            ("EmptyLeadingWindow", &[empty(), excl(50.0), num(10.0), num(20.0), empty(), num(30.0), num(40.0), num(50.0), num(60.0)]),
            ("LinearZeroCrossingCentroid", &[num(-4.0), num(-2.0), num(5.0), num(8.0), empty(), num(6.0), excl(100.0), num(10.0), num(12.0)]),
        ],
    );
    let col2_empty = [None; 9];

    struct OpCase<'a> {
        label: &'a str,
        op: BaselineOperation,
        warn_count: usize,
        zero_warn: bool,
        col0: [Option<f64>; 9],
        col1: [Option<f64>; 9],
        check_col2: bool,
        col3: [Option<f64>; 9],
    }

    let cases = [
        OpCase {
            label: "lin diff",
            op: BaselineOperation::Difference,
            warn_count: 6,
            zero_warn: false,
            col0: [
                Some(-0.027_027_027_027_027_03),
                Some(0.027_027_027_027_027_03),
                Some(5.081_081_081_081_081),
                Some(6.135_135_135_135_135),
                None,
                Some(0.243_243_243_243_243_26),
                Some(92.297_297_297_297_29),
                Some(-0.648_648_648_648_648_7),
                Some(0.405_405_405_405_405_43),
            ],
            col1: [
                Some(0.0),
                None,
                Some(-0.076_923_076_923_076_93),
                Some(1.384_615_384_615_384_6),
                Some(2.846_153_846_153_846_3),
                Some(-1.692_307_692_307_692_3),
                Some(-1.230_769_230_769_230_8),
                Some(1.230_769_230_769_230_8),
                Some(1.692_307_692_307_692_3),
            ],
            check_col2: true,
            col3: [Some(0.0), Some(0.0), Some(5.0), Some(6.0), None, Some(0.0), Some(92.0), Some(0.0), Some(0.0)],
        },
        OpCase {
            label: "lin ratio",
            op: BaselineOperation::Ratio,
            warn_count: 7,
            zero_warn: true,
            col0: [
                Some(1.006_802_721_088_435_4),
                Some(0.986_666_666_666_666_7),
                Some(-61.666_666_666_666_664),
                Some(4.289_855_072_463_768_5),
                None,
                Some(1.042_253_521_126_760_5),
                Some(12.982_456_140_350_877),
                Some(0.932_773_109_243_697_5),
                Some(1.034_965_034_965_035),
            ],
            col1: [
                Some(1.0),
                None,
                Some(0.981_132_075_471_698_1),
                Some(1.246_575_342_465_753_5),
                Some(1.397_849_462_365_591_5),
                Some(0.805_309_734_513_274_4),
                Some(0.879_699_248_120_300_8),
                Some(1.104_575_163_398_692_9),
                Some(1.127_167_630_057_803_5),
            ],
            check_col2: true,
            col3: [Some(1.0), Some(1.0), None, Some(4.0), None, Some(1.0), Some(12.5), Some(1.0), Some(1.0)],
        },
        OpCase {
            label: "lin pct",
            op: BaselineOperation::PercentChange,
            warn_count: 7,
            zero_warn: true,
            col0: [
                Some(0.680_272_108_843_537_4),
                Some(-1.333_333_333_333_333_3),
                Some(-6_266.666_666_666_667),
                Some(328.985_507_246_376_8),
                None,
                Some(4.225_352_112_676_056),
                Some(1_198.245_614_035_087_8),
                Some(-6.722_689_075_630_252),
                Some(3.496_503_496_503_496_7),
            ],
            col1: [
                Some(0.0),
                None,
                Some(-1.886_792_452_830_188_7),
                Some(24.657_534_246_575_34),
                Some(39.784_946_236_559_136),
                Some(-19.469_026_548_672_566),
                Some(-12.030_075_187_969_924),
                Some(10.457_516_339_869_281),
                Some(12.716_763_005_780_347),
            ],
            check_col2: false,
            col3: [Some(0.0), Some(0.0), None, Some(300.0), None, Some(0.0), Some(1150.0), Some(0.0), Some(0.0)],
        },
        OpCase {
            label: "lin frac",
            op: BaselineOperation::FractionalChange,
            warn_count: 7,
            zero_warn: true,
            col0: [
                Some(0.006_802_721_088_435_374),
                Some(-0.013_333_333_333_333_334),
                Some(-62.666_666_666_666_664),
                Some(3.289_855_072_463_768),
                None,
                Some(0.042_253_521_126_760_56),
                Some(11.982_456_140_350_877),
                Some(-0.067_226_890_756_302_52),
                Some(0.034_965_034_965_034_97),
            ],
            col1: [
                Some(0.0),
                None,
                Some(-0.018_867_924_528_301_886),
                Some(0.246_575_342_465_753_4),
                Some(0.397_849_462_365_591_4),
                Some(-0.194_690_265_486_725_66),
                Some(-0.120_300_751_879_699_24),
                Some(0.104_575_163_398_692_81),
                Some(0.127_167_630_057_803_46),
            ],
            check_col2: false,
            col3: [Some(0.0), Some(0.0), None, Some(3.0), None, Some(0.0), Some(11.5), Some(0.0), Some(0.0)],
        },
    ];

    let ctx = TransformContext::default();
    let kind = BaselineKind::FirstAndLastLinear { first_k: 2, last_k: 4 };
    for c in cases {
        let out = execute_remove_baseline_kind(&s, &kind, c.op, &ctx).unwrap();
        assert_eq!(out.warnings.len(), c.warn_count, "{}: {:?}", c.label, out.warnings);
        if c.zero_warn {
            assert!(out.warnings.iter().any(|w| w.contains("row 2 col 3") && w.contains("baseline=0")), "{}: {:?}", c.label, out.warnings);
        } else {
            assert!(out.warnings.iter().all(|w| w.contains("col 2") && w.contains("missing baseline")), "{}: {:?}", c.label, out.warnings);
        }
        assert_col_vals(&out.data_sheet, 0, &c.col0, &[6], LINEAR_TOL, c.label);
        assert_col_vals(&out.data_sheet, 1, &c.col1, &[], LINEAR_TOL, c.label);
        if c.check_col2 {
            assert_col_vals(&out.data_sheet, 2, &col2_empty, &[1], LINEAR_TOL, c.label);
        }
        assert_col_vals(&out.data_sheet, 3, &c.col3, &[6], LINEAR_TOL, c.label);
    }
}

#[test]
fn baseline_columns_rowwise_preserves_sem_handles_zero_missing_excluded_and_negative() {
    let s = xy_3sub(
        "xy-rowwise-base",
        SubcolumnMode::MeanSemN,
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[
            Xy3Col {
                title: "Baseline",
                triples: &[
                    [Some(4.0), Some(0.2), Some(4.0)],
                    [Some(0.0), Some(0.1), Some(4.0)],
                    [None, None, None],
                    [Some(10.0), Some(0.5), Some(4.0)],
                    [Some(-2.0), Some(0.25), Some(4.0)],
                ],
                excl_primary: &[false, false, false, true, false],
            },
            Xy3Col {
                title: "Treated",
                triples: &[
                    [Some(10.0), Some(0.8), Some(6.0)],
                    [Some(5.0), Some(0.5), Some(6.0)],
                    [Some(12.0), Some(0.6), Some(6.0)],
                    [Some(20.0), Some(1.0), Some(6.0)],
                    [Some(7.0), Some(0.5), Some(6.0)],
                ],
                excl_primary: &[false; 5],
            },
        ],
    );
    let ctx = TransformContext { selected_data_sets: vec![1], ..TransformContext::default() };
    let lin = (LINEAR_TOL, 0.0);

    let diff_out = execute_remove_baseline(&s, &[0], BaselineOperation::Difference, &ctx).unwrap();
    assert_eq!(diff_out.warnings.len(), 4, "missing/excluded baseline rows 2 and 3 blank Mean+SEM: {:?}", diff_out.warnings);
    let d_xy = as_xy(&diff_out.data_sheet);
    for (r, exp) in [
        (0, [Num(6.0), Num(0.8), Num(6.0)]),
        (1, [Num(5.0), Num(0.5), Num(6.0)]),
        (2, [Empty, Empty, Num(6.0)]),
        (3, [Empty, Empty, Num(6.0)]),
        (4, [Num(9.0), Num(0.5), Num(6.0)]),
    ] {
        assert_cells_3(&d_xy.data_sets[1].cells[r], exp, lin, &format!("diff r{r}"));
    }

    for (op, r0, r4) in [
        (BaselineOperation::Ratio, [Num(2.5), Num(0.2), Num(6.0)], [Num(-3.5), Num(0.25), Num(6.0)]),
        (BaselineOperation::PercentChange, [Num(150.0), Num(20.0), Num(6.0)], [Num(-450.0), Num(25.0), Num(6.0)]),
        (BaselineOperation::FractionalChange, [Num(1.5), Num(0.2), Num(6.0)], [Num(-4.5), Num(0.25), Num(6.0)]),
    ] {
        let out = execute_remove_baseline(&s, &[0], op, &ctx).unwrap();
        assert_eq!(out.warnings.len(), 6, "{op:?} warnings: {:?}", out.warnings);
        let xy = as_xy(&out.data_sheet);
        assert_cells_3(&xy.data_sets[1].cells[0], r0, lin, &format!("{op:?} r0"));
        assert_cells_3(&xy.data_sets[1].cells[4], r4, lin, &format!("{op:?} r4"));
        for r in [1_usize, 2, 3] {
            assert_cells_3(&xy.data_sets[1].cells[r], [Empty, Empty, Num(6.0)], lin, &format!("{op:?} r{r}"));
        }
    }

    // Negative baseline Ratio / PercentChange / FractionalChange on MeanLowHigh swap -Error/+Error; Difference does not.
    let lh_s = xy_3sub_clean(
        "xy-base-lh",
        SubcolumnMode::MeanLowHigh,
        &[1.0],
        &[("Base", &[[Some(-2.0), Some(0.0), Some(0.0)]]), ("Trt", &[[Some(8.0), Some(1.0), Some(3.0)]])],
    );
    for (op, exp) in [
        (BaselineOperation::Difference, [Num(10.0), Num(1.0), Num(3.0)]),
        (BaselineOperation::Ratio, [Num(-4.0), Num(1.5), Num(0.5)]),
        (BaselineOperation::PercentChange, [Num(-500.0), Num(150.0), Num(50.0)]),
        (BaselineOperation::FractionalChange, [Num(-5.0), Num(1.5), Num(0.5)]),
    ] {
        let out = execute_remove_baseline(&lh_s, &[0], op, &ctx).unwrap();
        assert_cells_3(&as_xy(&out.data_sheet).data_sets[1].cells[0], exp, lin, &format!("lh {op:?}"));
    }

    // Typed AnalysisSpec -> ResultSheet for RemoveBaseline (AC #4).
    let base_spec = TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Ratio };
    let base_analysis = AnalysisSpec::new("an-base-ratio", "xy-rowwise-base", AnalysisKind::Transform(base_spec)).unwrap();
    let base_res = execute_analysis_transform(&s, &base_analysis, &ctx).unwrap();
    assert_eq!(base_res.status, ResultStatus::UpToDate);
    assert_eq!(base_res.title(), "Remove baseline");
    let base_xy = as_xy(base_res.analyzable_data.as_ref().unwrap());
    assert_cells_3(&base_xy.data_sets[1].cells[0], [Num(2.5), Num(0.2), Num(6.0)], lin, "ResultSheet baseline r0");
    assert_eq!(base_xy.data_sets[1].cells[1][0].value, CellValue::Empty);
}

#[test]
fn math_transform_uncertainty_propagation_delta_and_endpoints_match_mpmath() {
    let ctx = TransformContext::default();
    let pos_s = xy_3sub(
        "xy-math-pos",
        SubcolumnMode::MeanSemN,
        &[1.0, 2.0, 3.0, 4.0],
        &[Xy3Col {
            title: "Conc",
            triples: &[
                [Some(2.5), Some(0.35), Some(6.0)],
                [Some(0.5), Some(0.75), Some(5.0)],
                [Some(-0.5), Some(0.2), Some(4.0)],
                [Some(-1.0), Some(0.2), Some(4.0)],
            ],
            excl_primary: &[false, false, false, true],
        }],
    );

    // 3a. Ln with FirstOrderDeltaMethod vs ReplicateEndpoints (including OOD behavior).
    let ln_delta = execute_math_op(&pos_s, &MathTransformOp::Ln, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    assert_eq!(ln_delta.warnings.len(), 1, "{:?}", ln_delta.warnings);
    assert!(ln_delta.warnings[0].contains("row 2 col 0") && ln_delta.warnings[0].contains("outside transform domain"));
    let ld_xy = as_xy(&ln_delta.data_sheet);
    assert_cells_3(&ld_xy.data_sets[0].cells[0], [Num(0.916_290_731_874_155_1), Num(0.14), Num(6.0)], NL_TOL, "ln delta r0");
    assert_cells_3(&ld_xy.data_sets[0].cells[1], [Num(-LN_2), Num(1.5), Num(5.0)], NL_TOL, "ln delta r1");
    assert_cells_3(&ld_xy.data_sets[0].cells[2], [Empty, Empty, Num(4.0)], (LINEAR_TOL, 0.0), "ln delta r2");
    assert!(ld_xy.data_sets[0].cells[3][0].excluded && ld_xy.data_sets[0].cells[3][0].value == CellValue::Empty);
    assert_eq!(ld_xy.data_sets[0].cells[3][1].value, CellValue::Empty);

    let ln_ep = execute_math_op(&pos_s, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(ln_ep.warnings.len(), 2);
    assert!(ln_ep.warnings[0].contains("spread blanked"));
    let le_xy = as_xy(&ln_ep.data_sheet);
    assert_cells_3(&le_xy.data_sets[0].cells[0], [Num(0.916_290_731_874_155_1), Num(0.140_925_576_070_493_86), Num(6.0)], NL_TOL, "ln ep r0");
    assert_cells_3(&le_xy.data_sets[0].cells[1], [Num(-LN_2), Empty, Num(5.0)], NL_TOL, "ln ep r1");

    // 3b. Increasing & decreasing presets on MeanSdN row (2.5, 0.35, N=6).
    let clean_sd = xy_3sub_clean("xy-math-sd", SubcolumnMode::MeanSdN, &[2.5], &[("Series", &[[Some(2.5), Some(0.35), Some(6.0)]])]);
    for (label, preset, exp_c, exp_d, exp_e) in [
        ("Log10", MathTransformPreset::Log10, 0.397_940_008_672_037_6, 0.060_801_227_466_455_25, 0.061_203_200_046_452_44),
        ("Reciprocal (decreasing)", MathTransformPreset::Reciprocal, 0.4, 0.056, 0.057_119_543_043_655_65),
        (
            "ConcentrationToNegativeLogM uM (decreasing)",
            MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 },
            5.602_059_991_327_962_5,
            0.060_801_227_466_455_25,
            0.061_203_200_046_452_44,
        ),
    ] {
        let op = MathTransformOp::Preset(preset);
        let d_out = execute_math_transform(&clean_sd, None, Some(&op), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
        let e_out = execute_math_transform(&clean_sd, None, Some(&op), ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
        assert!(d_out.warnings.is_empty() && e_out.warnings.is_empty(), "{label}");
        assert_cells_3(&as_xy(&d_out.data_sheet).data_sets[0].cells[0], [Num(exp_c), Num(exp_d), Num(6.0)], NL_TOL, &format!("{label} delta"));
        assert_cells_3(&as_xy(&e_out.data_sheet).data_sets[0].cells[0], [Num(exp_c), Num(exp_e), Num(6.0)], NL_TOL, &format!("{label} ep"));
    }

    // MeanCvN (Mean=2.5, %CV=14.0 -> SD=0.35, N=6) under decreasing Reciprocal.
    let clean_cv = xy_3sub_clean("xy-math-cv", SubcolumnMode::MeanCvN, &[2.5], &[("Series", &[[Some(2.5), Some(14.0), Some(6.0)]])]);
    let cv_d = execute_math_op(&clean_cv, &MathTransformOp::Reciprocal, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let cv_e = execute_math_op(&clean_cv, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_cells_3(&as_xy(&cv_d.data_sheet).data_sets[0].cells[0], [Num(0.4), Num(14.0), Num(6.0)], NL_TOL, "cv delta");
    assert_cells_3(&as_xy(&cv_e.data_sheet).data_sets[0].cells[0], [Num(0.4), Num(14.279_885_760_913_913), Num(6.0)], NL_TOL, "cv ep");
}

#[test]
fn math_transform_proportion_uncertainty_and_result_sheet() {
    let ctx = TransformContext::default();
    let prop_s = xy_3sub_clean(
        "xy-math-prop",
        SubcolumnMode::MeanSemN,
        &[1.0, 2.0],
        &[("Prop", &[[Some(0.25), Some(0.08), Some(9.0)], [Some(1.0), Some(0.05), Some(4.0)]])],
    );

    for (label, preset, exp_c, exp_d, exp_e, p1_is_ood) in [
        ("Logit", MathTransformPreset::Logit, -1.098_612_288_668_109_8, 0.426_666_666_666_666_7, 0.438_721_102_907_948, true),
        ("Probit", MathTransformPreset::Probit, 4.325_510_249_803_918, 0.251_749_206_444_887_35, 0.257_126_043_736_480_3, true),
        ("ArcsinSqrt", MathTransformPreset::ArcsinSqrt, FRAC_PI_6, 0.092_376_043_070_340_13, 0.093_475_465_835_731_98, false),
    ] {
        let d_spec = TransformAnalysisSpec::Math {
            x_transform: Some(MathTransformPreset::Log10),
            y_transform: Some(preset),
            error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
        };
        let e_spec =
            TransformAnalysisSpec::Math { x_transform: None, y_transform: Some(preset), error_propagation: ErrorPropagationMode::ReplicateEndpoints };
        let (d_out, e_out) = (execute_transform(&prop_s, &d_spec, &ctx).unwrap(), execute_transform(&prop_s, &e_spec, &ctx).unwrap());
        let (d_xy, e_xy) = (as_xy(&d_out.data_sheet), as_xy(&e_out.data_sheet));
        assert_approx_eq(d_xy.x_value_at(0).unwrap(), 0.0, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "x0 log10");
        assert_approx_eq(d_xy.x_value_at(1).unwrap(), LOG10_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "x1 log10");
        assert_cells_3(&d_xy.data_sets[0].cells[0], [Num(exp_c), Num(exp_d), Num(9.0)], NL_TOL, &format!("{label} delta r0"));
        assert_cells_3(&e_xy.data_sets[0].cells[0], [Num(exp_c), Num(exp_e), Num(9.0)], NL_TOL, &format!("{label} ep r0"));

        let (warn_substr, r1_exp) =
            if p1_is_ood { ("outside transform domain", [Empty, Empty, Num(4.0)]) } else { ("spread blanked", [Num(FRAC_PI_2), Empty, Num(4.0)]) };
        assert_eq!((d_out.warnings.len(), e_out.warnings.len()), (1, 1), "{label}");
        assert!(d_out.warnings[0].contains(warn_substr) && e_out.warnings[0].contains(warn_substr), "{label}");
        assert_cells_3(&d_xy.data_sets[0].cells[1], r1_exp, NL_TOL, &format!("{label} delta r1"));
        assert_cells_3(&e_xy.data_sets[0].cells[1], r1_exp, NL_TOL, &format!("{label} ep r1"));
    }

    // Typed AnalysisSpec -> ResultSheet check for Math transform (Logit + ReplicateEndpoints).
    let math_spec = TransformAnalysisSpec::Math {
        x_transform: None,
        y_transform: Some(MathTransformPreset::Logit),
        error_propagation: ErrorPropagationMode::ReplicateEndpoints,
    };
    let math_analysis = AnalysisSpec::new("an-math-logit", "xy-math-prop", AnalysisKind::Transform(math_spec)).unwrap();
    let math_res = execute_analysis_transform(&prop_s, &math_analysis, &ctx).unwrap();
    assert_eq!(math_res.status, ResultStatus::UpToDate);
    assert_eq!(math_res.title(), "Transform");
    let math_data = as_xy(math_res.analyzable_data.as_ref().unwrap());
    assert_approx_eq(
        math_data.data_sets[0].cells[0][1].active_number().unwrap(),
        0.438_721_102_907_948,
        NONLINEAR_ABS_TOL,
        NONLINEAR_REL_TOL,
        "ResultSheet logit ep sem",
    );
}

#[test]
fn math_transform_decreasing_monotone_endpoints_preserve_lower_upper_ordering() {
    let ctx = TransformContext::default();
    let ul_s = xy_3sub_clean(
        "xy-ul-dec",
        SubcolumnMode::UpperLowerLimits,
        &[1.0, 2.0],
        &[("Limits", &[[Some(2.0), Some(1.2), Some(4.0)], [Some(2.0), Some(-1.0), Some(4.0)]])],
    );

    for (label, preset, exp) in [
        ("Reciprocal", MathTransformPreset::Reciprocal, [0.5, 0.25, 0.833_333_333_333_333_4]),
        (
            "ConcentrationToNegativeLogM",
            MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 },
            [5.698_970_004_336_019, 5.397_940_008_672_037_5, 5.920_818_753_952_375],
        ),
    ] {
        let spec =
            TransformAnalysisSpec::Math { x_transform: None, y_transform: Some(preset), error_propagation: ErrorPropagationMode::ReplicateEndpoints };
        let out = execute_transform(&ul_s, &spec, &ctx).unwrap();
        let row0 = &as_xy(&out.data_sheet).data_sets[0].cells[0];
        assert_cells_3(row0, exp.map(Num), NL_TOL, label);
        let bounds = SubcolumnMode::UpperLowerLimits.explicit_bounds(row0).unwrap();
        assert!(bounds.lower < bounds.center && bounds.center < bounds.upper, "{label}: {bounds:?}");
    }

    // Out-of-domain lower limit (-1.0) under Ln blanks Lower while keeping Mean & Upper and emitting a warning (#55).
    let ln_ul = execute_math_op(&ul_s, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(ln_ul.warnings.len(), 1);
    assert!(ln_ul.warnings[0].contains("lower bound -1 outside transform domain"));
    assert_cells_3(&as_xy(&ln_ul.data_sheet).data_sets[0].cells[1], [Num(LN_2), Empty, Num(4.0_f64.ln())], NL_TOL, "ln ul r1");

    // Partial-OOD lower limit (0.0) under decreasing Reciprocal maps Upper=4.0 to Lower=0.25 and leaves Upper Empty.
    let rec_partial_s = xy_3sub_clean("xy-ul-rec-ood", SubcolumnMode::UpperLowerLimits, &[1.0], &[("Lim", &[[Some(2.0), Some(0.0), Some(4.0)]])]);
    let rec_partial = execute_math_op(&rec_partial_s, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_cells_3(&as_xy(&rec_partial.data_sheet).data_sets[0].cells[0], [Num(0.5), Num(0.25), Empty], NL_TOL, "rec partial");

    // MedianIqr decreasing endpoint ordering: [Median=2.0, Q1=1.2, Q3=4.0] under Reciprocal.
    let iqr_s = xy_3sub_clean("xy-iqr-dec", SubcolumnMode::MedianIqr, &[1.0], &[("IQR", &[[Some(2.0), Some(1.2), Some(4.0)]])]);
    let iqr_out = execute_math_op(&iqr_s, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    let iqr_row = &as_xy(&iqr_out.data_sheet).data_sets[0].cells[0];
    assert_cells_3(iqr_row, [Num(0.5), Num(0.25), Num(0.833_333_333_333_333_4)], NL_TOL, "rec iqr");
    let iqr_bounds = SubcolumnMode::MedianIqr.explicit_bounds(iqr_row).unwrap();
    assert!(iqr_bounds.lower < iqr_bounds.center && iqr_bounds.center < iqr_bounds.upper);

    // MeanLowHigh half-OOD & full-OOD under Ln.
    let lh_ood_s = xy_3sub_clean(
        "xy-lh-ood",
        SubcolumnMode::MeanLowHigh,
        &[1.0, 2.0],
        &[("LH", &[[Some(0.5), Some(0.75), Some(0.5)], [Some(-0.5), Some(0.2), Some(0.3)]])],
    );
    let lh_ood_d = execute_math_op(&lh_ood_s, &MathTransformOp::Ln, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let lh_ood_e = execute_math_op(&lh_ood_s, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(lh_ood_d.warnings.len(), 1);
    assert!(lh_ood_d.warnings[0].contains("row 1 col 0") && lh_ood_d.warnings[0].contains("outside transform domain"));
    assert_eq!(lh_ood_e.warnings.len(), 2);
    assert!(lh_ood_e.warnings[0].contains("row 0 col 0") && lh_ood_e.warnings[0].contains("error bounds blanked"));
    assert_cells_3(&as_xy(&lh_ood_d.data_sheet).data_sets[0].cells[0], [Num(-LN_2), Num(1.5), Num(1.0)], NL_TOL, "lh ood delta r0");
    assert_cells_3(&as_xy(&lh_ood_e.data_sheet).data_sets[0].cells[0], [Num(-LN_2), Empty, Empty], NL_TOL, "lh ood ep r0");
    assert_cells_3(&as_xy(&lh_ood_d.data_sheet).data_sets[0].cells[1], [Empty, Empty, Empty], NL_TOL, "lh ood delta r1");
    assert_cells_3(&as_xy(&lh_ood_e.data_sheet).data_sets[0].cells[1], [Empty, Empty, Empty], NL_TOL, "lh ood ep r1");

    // MeanLowHigh: [Mean=2.0, -Error=0.8, +Error=2.0] under decreasing transforms.
    let lh_s = xy_3sub_clean("xy-lh-dec", SubcolumnMode::MeanLowHigh, &[1.0], &[("Asym", &[[Some(2.0), Some(0.8), Some(2.0)]])]);
    for (label, preset, exp_ep, exp_dl) in [
        ("Reciprocal", MathTransformPreset::Reciprocal, [0.5, 0.25, 0.333_333_333_333_333_3], [0.5, 0.5, 0.2]),
        (
            "ConcentrationToNegativeLogM",
            MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 },
            [5.698_970_004_336_019, LOG10_2, 0.221_848_749_616_356_37],
            [5.698_970_004_336_019, 0.434_294_481_903_251_76, 0.173_717_792_761_300_73],
        ),
    ] {
        let op = MathTransformOp::Preset(preset);
        let ep_out = execute_math_transform(&lh_s, None, Some(&op), ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
        let dl_out = execute_math_transform(&lh_s, None, Some(&op), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
        assert_cells_3(&as_xy(&ep_out.data_sheet).data_sets[0].cells[0], exp_ep.map(Num), NL_TOL, &format!("{label} lh ep"));
        assert_cells_3(&as_xy(&dl_out.data_sheet).data_sets[0].cells[0], exp_dl.map(Num), NL_TOL, &format!("{label} lh delta"));
    }
}

#[test]
fn baseline_first_and_last_linear_affine_invariance_unbalanced_and_equal_anchor() {
    // Noiseless affine series y(r) = -5 + 3*r across 8 rows with blanks, excluded outliers, and unbalanced replicates.
    let e = empty();
    let rep_s = grouped_sheet(
        "aff-unbal",
        SubcolumnMode::RawReplicates(3),
        &[],
        &[(
            "UnbalancedAffine",
            &[
                &[num(-5.0), e.clone(), e.clone()],
                &[num(-2.0), num(-2.0), num(-2.0)],
                &[num(1.0), excl(999.0), num(1.0)],
                &[e.clone(), e.clone(), e.clone()],
                &[num(7.0), num(7.0), e.clone()],
                &[num(10.0), excl(500.0), num(10.0)],
                &[num(13.0), e.clone(), e],
                &[num(16.0), num(16.0), num(16.0)],
            ],
        )],
    );
    let ctx = TransformContext::default();
    let kind = BaselineKind::FirstAndLastLinear { first_k: 3, last_k: 3 };

    for (op, expected) in [(BaselineOperation::Difference, 0.0), (BaselineOperation::Ratio, 1.0)] {
        let out = execute_remove_baseline_kind(&rep_s, &kind, op, &ctx).unwrap();
        for (r, row) in as_grouped(&out.data_sheet).data_sets[0].cells.iter().enumerate() {
            for cell in row {
                if let Some(v) = cell.active_number() {
                    assert_approx_eq(v, expected, LINEAR_TOL, 0.0, &format!("affine {op:?} invariance row {r}"));
                }
            }
        }
    }

    // Overlapping windows collapsing to identical active row coordinate (r = 2, B = 10.0) avoid 0/0 NaN.
    let overlap_s = col_sheet("base-equal-anchor", &[("SingleShared", &[empty(), empty(), num(10.0), empty(), empty()])]);
    let overlap_diff = execute_remove_baseline_kind(&overlap_s, &kind, BaselineOperation::Difference, &ctx).unwrap();
    assert!(overlap_diff.warnings.is_empty());
    assert_col_vals(&overlap_diff.data_sheet, 0, &[None, None, Some(0.0), None, None], &[], LINEAR_TOL, "overlapping identical anchor row");
}
