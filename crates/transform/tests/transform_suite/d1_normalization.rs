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
    AnalysisKind, AnalysisSpec, BaselineOperation, Cell, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorPropagationMode,
    GroupedTable, HookSource, InfoSheet, MathTransformPreset, NormalizeAnchor, ResultCell, ResultScalarEntry, ResultStatus, ResultTabKind, SheetId,
    SubcolumnMode, TableArchetype, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    BaselineKind, MathTransformOp, TransformContext, TransformError, execute_analysis_transform, execute_math_op, execute_math_transform,
    execute_normalize, execute_remove_baseline, execute_remove_baseline_kind, execute_transform,
};

const LINEAR_TOL: f64 = 1e-12;
const NONLINEAR_ABS_TOL: f64 = 1e-12;
const NONLINEAR_REL_TOL: f64 = 1e-12;

type Xy3SubCol<'a> = (&'a str, &'a [f64], &'a [[Option<f64>; 3]], &'a [bool]);

fn make_col_sheet(id: &str, cols: &[(&str, &[Cell])]) -> Result<DataSheet, TransformError> {
    let n_rows = cols.first().map_or(0, |(_, cells)| cells.len());
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows)?;
    for (c, (title, cells)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*title).to_owned();
        for (r, cell) in cells.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = cell.clone();
        }
    }
    Ok(DataSheet::new(id, format!("Sheet {id}"), TableArchetype::Column(tbl))?)
}

fn make_xy_3sub(id: &str, mode: SubcolumnMode, rows_by_col: &[Xy3SubCol<'_>]) -> Result<DataSheet, TransformError> {
    let n_rows = rows_by_col.first().map_or(0, |(_, xs, _, _)| xs.len());
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, rows_by_col.len(), n_rows)?;
    for (r, &x) in rows_by_col[0].1.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
    }
    for (c, (title, _, triples, excl_primary)) in rows_by_col.iter().enumerate() {
        xy.data_sets[c].title = (*title).to_owned();
        for (r, (vals, &excl)) in triples.iter().zip(excl_primary.iter()).enumerate() {
            xy.data_sets[c].cells[r] = vec![
                vals[0].map_or_else(|| Cell { value: CellValue::Empty, excluded: excl }, |v| Cell { value: CellValue::Number(v), excluded: excl }),
                vals[1].map_or_else(Cell::empty, Cell::number),
                vals[2].map_or_else(Cell::empty, Cell::number),
            ];
        }
    }
    Ok(DataSheet::new(id, format!("Sheet {id}"), TableArchetype::Xy(xy))?)
}

fn extract_xy_table<'a>(sheet: &'a DataSheet, label: &str) -> Result<&'a XyTable, TransformError> {
    match &sheet.archetype {
        TableArchetype::Xy(xy) => Ok(xy),
        other => Err(TransformError::InvalidInput { message: format!("{label}: expected Xy archetype, got {:?}", other.kind()) }),
    }
}

fn assert_col_cells(sheet: &DataSheet, col: usize, expected: &[(Option<f64>, bool)], label: &str) {
    let TableArchetype::Column(tbl) = &sheet.archetype else {
        assert!(matches!(&sheet.archetype, TableArchetype::Column(_)), "{label}: expected Column archetype, got {:?}", sheet.kind());
        return;
    };
    assert_eq!(tbl.data_sets[col].cells.len(), expected.len(), "{label} col {col} row count");
    for (r, (exp_val, exp_excl)) in expected.iter().enumerate() {
        let cell = &tbl.data_sets[col].cells[r][0];
        assert_eq!(cell.excluded, *exp_excl, "{label} col {col} row {r} excluded mismatch");
        match exp_val {
            Some(v) => {
                let actual = cell.value.finite_f64();
                assert!(actual.is_some(), "{label} col {col} row {r} expected finite {v}, got {cell:?}");
                if let Some(val) = actual {
                    assert_approx_eq(val, *v, LINEAR_TOL, LINEAR_TOL, &format!("{label} col {col} row {r}"));
                }
            }
            None => assert_eq!(cell.value, CellValue::Empty, "{label} col {col} row {r} expected Empty"),
        }
    }
}

fn assert_scalar_anchor(scalars: &[ResultScalarEntry], key: &str, ds_idx: Option<usize>, expected: f64, label: &str) {
    let found = scalars.iter().find(|s| s.key == key && s.data_set_index == ds_idx);
    assert!(found.is_some(), "{label}: missing scalar ({key}, {ds_idx:?}) in {scalars:?}");
    if let Some(entry) = found {
        assert_eq!(entry.units, None, "{label}: scalar {key} units");
        assert_approx_eq(entry.value, expected, LINEAR_TOL, 0.0, &format!("{label} {key} {ds_idx:?}"));
    }
}

// ============================================================================
// 1. Normalization (Per-Dataset vs Pooled, Fraction vs Percent, Error Cases)
// ============================================================================

#[test]
fn normalization_per_dataset_and_pooled_fraction_and_percent_match_rationals() {
    // Unequal dataset ranges with excluded extremes (500* high in Ctrl, -200* low in Drug) and blank row 3.
    let sheet = make_col_sheet(
        "norm-src",
        &[
            ("Ctrl", &[Cell::number(10.0), Cell::number(30.0), Cell::number(50.0), Cell::empty(), Cell::number(90.0), Cell::excluded_number(500.0)]),
            (
                "Drug",
                &[Cell::excluded_number(-200.0), Cell::number(30.0), Cell::number(70.0), Cell::empty(), Cell::number(110.0), Cell::number(190.0)],
            ),
        ],
    )
    .unwrap();
    let ctx = TransformContext::default();

    // 1a. Per-dataset fraction (output_percent = false)
    let per_frac = execute_normalize(&sheet, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, false, true, &ctx).unwrap();
    assert_eq!(per_frac.scalars.len(), 4);
    assert_scalar_anchor(&per_frac.scalars, "ZeroAnchor", Some(0), 10.0, "per_frac");
    assert_scalar_anchor(&per_frac.scalars, "HundredAnchor", Some(0), 90.0, "per_frac");
    assert_scalar_anchor(&per_frac.scalars, "ZeroAnchor", Some(1), 30.0, "per_frac");
    assert_scalar_anchor(&per_frac.scalars, "HundredAnchor", Some(1), 190.0, "per_frac");
    assert_col_cells(
        &per_frac.data_sheet,
        0,
        &[(Some(0.0), false), (Some(0.25), false), (Some(0.5), false), (None, false), (Some(1.0), false), (Some(6.125), true)],
        "per_frac",
    );
    assert_col_cells(
        &per_frac.data_sheet,
        1,
        &[(Some(-1.4375), true), (Some(0.0), false), (Some(0.25), false), (None, false), (Some(0.5), false), (Some(1.0), false)],
        "per_frac",
    );

    // 1b. Per-dataset percentage (output_percent = true)
    let per_pct = execute_transform(
        &sheet,
        &TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        },
        &ctx,
    )
    .unwrap();
    assert_eq!(per_pct.scalars.len(), 4);
    assert_scalar_anchor(&per_pct.scalars, "ZeroAnchor", Some(0), 10.0, "per_pct");
    assert_scalar_anchor(&per_pct.scalars, "HundredAnchor", Some(0), 90.0, "per_pct");
    assert_scalar_anchor(&per_pct.scalars, "ZeroAnchor", Some(1), 30.0, "per_pct");
    assert_scalar_anchor(&per_pct.scalars, "HundredAnchor", Some(1), 190.0, "per_pct");
    assert_col_cells(
        &per_pct.data_sheet,
        0,
        &[(Some(0.0), false), (Some(25.0), false), (Some(50.0), false), (None, false), (Some(100.0), false), (Some(612.5), true)],
        "per_pct",
    );
    assert_col_cells(
        &per_pct.data_sheet,
        1,
        &[(Some(-143.75), true), (Some(0.0), false), (Some(25.0), false), (None, false), (Some(50.0), false), (Some(100.0), false)],
        "per_pct",
    );

    // 1c. Pooled fraction (per_data_set = false, output_percent = false)
    let pool_frac = execute_normalize(&sheet, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, false, false, &ctx).unwrap();
    assert_eq!(pool_frac.scalars.len(), 2);
    assert_scalar_anchor(&pool_frac.scalars, "ZeroAnchor", None, 10.0, "pool_frac");
    assert_scalar_anchor(&pool_frac.scalars, "HundredAnchor", None, 190.0, "pool_frac");
    assert_col_cells(
        &pool_frac.data_sheet,
        0,
        &[
            (Some(0.0), false),
            (Some(0.111_111_111_111_111_1), false),
            (Some(0.222_222_222_222_222_2), false),
            (None, false),
            (Some(0.444_444_444_444_444_4), false),
            (Some(2.722_222_222_222_222_3), true),
        ],
        "pool_frac",
    );
    assert_col_cells(
        &pool_frac.data_sheet,
        1,
        &[
            (Some(-1.166_666_666_666_666_7), true),
            (Some(0.111_111_111_111_111_1), false),
            (Some(0.333_333_333_333_333_3), false),
            (None, false),
            (Some(0.555_555_555_555_555_6), false),
            (Some(1.0), false),
        ],
        "pool_frac",
    );

    // 1d. Pooled percentage via typed AnalysisSpec -> ResultSheet (AC #4)
    let analysis = AnalysisSpec::new(
        "an-norm-pooled",
        "norm-src",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: false,
        }),
    )
    .unwrap();
    let res_sheet = execute_analysis_transform(&sheet, &analysis, &ctx).unwrap();
    assert_eq!(res_sheet.status, ResultStatus::UpToDate);
    assert_eq!(res_sheet.title(), "Normalize");
    assert_approx_eq(res_sheet.lookup_scalar("ZeroAnchor", None).unwrap(), 10.0, LINEAR_TOL, 0.0, "ResultSheet ZeroAnchor");
    assert_approx_eq(res_sheet.lookup_scalar("zeroanchor", None).unwrap(), 10.0, LINEAR_TOL, 0.0, "ResultSheet zeroanchor");
    assert_approx_eq(res_sheet.lookup_scalar("HundredAnchor", None).unwrap(), 190.0, LINEAR_TOL, 0.0, "ResultSheet HundredAnchor");
    assert_eq!(res_sheet.tabs.len(), 1);
    assert_eq!(res_sheet.tabs[0].kind, ResultTabKind::MainSummary);
    assert_eq!(res_sheet.tabs[0].rows[0], vec![ResultCell::Boolean(false)]);
    assert_eq!(res_sheet.tabs[0].rows[1], vec![ResultCell::Boolean(true)]);
    assert_eq!(res_sheet.tabs[0].rows[2], vec![ResultCell::Number { value: 2.0, prefix: None, units: None }]);

    let norm_data = res_sheet.analyzable_data.as_ref().unwrap();
    assert_col_cells(
        norm_data,
        0,
        &[
            (Some(0.0), false),
            (Some(11.111_111_111_111_11), false),
            (Some(22.222_222_222_222_22), false),
            (None, false),
            (Some(44.444_444_444_444_44), false),
            (Some(272.222_222_222_222_23), true),
        ],
        "pool_pct_result",
    );
    assert_col_cells(
        norm_data,
        1,
        &[
            (Some(-116.666_666_666_666_67), true),
            (Some(11.111_111_111_111_11), false),
            (Some(33.333_333_333_333_336), false),
            (None, false),
            (Some(55.555_555_555_555_56), false),
            (Some(100.0), false),
        ],
        "pool_pct_result",
    );
}

#[test]
fn normalization_coincident_anchors_return_invalid_spec_and_hooked_summary_scales() {
    let ctx = TransformContext::default();

    // Per-dataset coincident anchors: column 1 has active [25, 25] plus blank and excluded 99*,
    // so active min == active max == 25.0 (excluded extreme must not mask coincident anchors).
    let const_col = make_col_sheet(
        "norm-coincident",
        &[
            ("Varying", &[Cell::number(1.0), Cell::number(5.0), Cell::empty(), Cell::number(9.0)]),
            ("Constant", &[Cell::number(25.0), Cell::number(25.0), Cell::empty(), Cell::excluded_number(99.0)]),
        ],
    )
    .unwrap();
    let err_per = execute_normalize(&const_col, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, true, true, &ctx).unwrap_err();
    assert!(matches!(err_per, TransformError::InvalidSpec { .. }), "expected InvalidSpec for per-dataset coincident anchors, got {err_per:?}");

    // Pooled coincident anchors across two constant columns
    let all_const =
        make_col_sheet("norm-all-const", &[("A", &[Cell::number(12.5), Cell::excluded_number(-50.0)]), ("B", &[Cell::number(12.5), Cell::empty()])])
            .unwrap();
    let err_pool = execute_normalize(&all_const, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, false, false, &ctx).unwrap_err();
    assert!(matches!(err_pool, TransformError::InvalidSpec { .. }), "expected InvalidSpec for pooled coincident anchors, got {err_pool:?}");

    // Fixed equal anchors
    let err_fixed =
        execute_normalize(&const_col, &NormalizeAnchor::FixedValue(7.5), &NormalizeAnchor::FixedValue(7.5), true, false, &ctx).unwrap_err();
    assert!(matches!(err_fixed, TransformError::InvalidSpec { .. }), "expected InvalidSpec for fixed coincident anchors, got {err_fixed:?}");

    // ControlColumnMean (col 0 active mean = 20.0) + Hooked InfoConstant (100.0) on MeanSemN
    let mut info = InfoSheet::new_preseeded("info-norm", "Norm Info").unwrap();
    info.set_constant("MaxPlateau", ConstantValue::Number(100.0), None).unwrap();
    let hook_ctx = TransformContext { info_sheet: Some(&info), selected_data_sets: vec![1], ..TransformContext::default() };
    let xy_sem = make_xy_3sub(
        "xy-norm-sem",
        SubcolumnMode::MeanSemN,
        &[
            (
                "Ctrl",
                &[1.0, 2.0, 3.0],
                &[[Some(10.0), Some(1.0), Some(3.0)], [Some(20.0), Some(1.0), Some(3.0)], [Some(30.0), Some(1.0), Some(3.0)]],
                &[false, false, false],
            ),
            (
                "Trt",
                &[1.0, 2.0, 3.0],
                &[[Some(40.0), Some(4.0), Some(5.0)], [Some(60.0), Some(8.0), Some(5.0)], [Some(100.0), Some(4.0), Some(5.0)]],
                &[false, false, false],
            ),
        ],
    )
    .unwrap();
    let sem_norm = execute_normalize(
        &xy_sem,
        &NormalizeAnchor::ControlColumnMean { column: 0 },
        &NormalizeAnchor::Hooked(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-norm")),
            constant_name: String::from("MaxPlateau"),
        }),
        true,
        true,
        &hook_ctx,
    )
    .unwrap();
    assert_scalar_anchor(&sem_norm.scalars, "ZeroAnchor", Some(1), 20.0, "MeanSemN hooked");
    assert_scalar_anchor(&sem_norm.scalars, "HundredAnchor", Some(1), 100.0, "MeanSemN hooked");
    let sem_xy = extract_xy_table(&sem_norm.data_sheet, "MeanSemN hooked").unwrap();
    // Unselected Ctrl col 0 remains unchanged
    assert_approx_eq(sem_xy.data_sets[0].cells[0][0].active_number().unwrap(), 10.0, LINEAR_TOL, 0.0, "unselected Ctrl r0");
    assert_approx_eq(sem_xy.data_sets[0].cells[0][1].active_number().unwrap(), 1.0, LINEAR_TOL, 0.0, "unselected Ctrl r0 sem");
    for (r, (exp_m, exp_sem)) in [(0, (25.0, 5.0)), (1, (50.0, 10.0)), (2, (100.0, 5.0))] {
        assert_approx_eq(sem_xy.data_sets[1].cells[r][0].active_number().unwrap(), exp_m, LINEAR_TOL, 0.0, &format!("MeanSemN r{r} mean"));
        assert_approx_eq(sem_xy.data_sets[1].cells[r][1].active_number().unwrap(), exp_sem, LINEAR_TOL, 0.0, &format!("MeanSemN r{r} sem"));
        assert_approx_eq(sem_xy.data_sets[1].cells[r][2].active_number().unwrap(), 5.0, LINEAR_TOL, 0.0, &format!("MeanSemN r{r} N preserved"));
    }

    // Decreasing normalization (ZeroAnchor = 100 > HundredAnchor = 20) on MeanLowHigh reverses -Error and +Error,
    // and on UpperLowerLimits reverses Lower/Upper (including partial-missing limits).
    let xy_lh =
        make_xy_3sub("xy-norm-lh", SubcolumnMode::MeanLowHigh, &[("Series", &[1.0], &[[Some(40.0), Some(4.0), Some(12.0)]], &[false])]).unwrap();
    let dec_norm = execute_normalize(&xy_lh, &NormalizeAnchor::FixedValue(100.0), &NormalizeAnchor::FixedValue(20.0), true, true, &ctx).unwrap();
    let lh_xy = extract_xy_table(&dec_norm.data_sheet, "dec MeanLowHigh").unwrap();
    assert_approx_eq(lh_xy.data_sets[0].cells[0][0].active_number().unwrap(), 75.0, LINEAR_TOL, 0.0, "dec MeanLowHigh mean");
    assert_approx_eq(lh_xy.data_sets[0].cells[0][1].active_number().unwrap(), 15.0, LINEAR_TOL, 0.0, "dec MeanLowHigh -Error");
    assert_approx_eq(lh_xy.data_sets[0].cells[0][2].active_number().unwrap(), 5.0, LINEAR_TOL, 0.0, "dec MeanLowHigh +Error");

    let xy_ul = make_xy_3sub(
        "xy-norm-ul",
        SubcolumnMode::UpperLowerLimits,
        &[("Limits", &[1.0, 2.0], &[[Some(40.0), Some(36.0), Some(52.0)], [Some(40.0), Some(36.0), None]], &[false, false])],
    )
    .unwrap();
    let dec_ul = execute_normalize(&xy_ul, &NormalizeAnchor::FixedValue(100.0), &NormalizeAnchor::FixedValue(20.0), true, true, &ctx).unwrap();
    let ul_xy = extract_xy_table(&dec_ul.data_sheet, "dec UpperLowerLimits").unwrap();
    assert_approx_eq(ul_xy.data_sets[0].cells[0][0].active_number().unwrap(), 75.0, LINEAR_TOL, 0.0, "dec UL r0 mean");
    assert_approx_eq(ul_xy.data_sets[0].cells[0][1].active_number().unwrap(), 60.0, LINEAR_TOL, 0.0, "dec UL r0 lower");
    assert_approx_eq(ul_xy.data_sets[0].cells[0][2].active_number().unwrap(), 80.0, LINEAR_TOL, 0.0, "dec UL r0 upper");
    assert_approx_eq(ul_xy.data_sets[0].cells[1][0].active_number().unwrap(), 75.0, LINEAR_TOL, 0.0, "dec UL r1 mean");
    assert_eq!(ul_xy.data_sets[0].cells[1][1].value, CellValue::Empty, "dec UL r1 missing lower from missing upper");
    assert_approx_eq(ul_xy.data_sets[0].cells[1][2].active_number().unwrap(), 80.0, LINEAR_TOL, 0.0, "dec UL r1 upper from lower");
}

// ============================================================================
// 2. Baseline Removal (FirstAndLastLinear + ColumnsRowWise, Zero Denominator, Missing/Excluded Anchor)
// ============================================================================

#[test]
fn baseline_first_and_last_linear_windows_zero_denominator_and_missing_anchor() {
    // 9 rows, first_k = 2 (rows 0..2), last_k = 4 (rows 5..9). Anchors use exact active
    // observation row-index centroids (x_start, y_start) and (x_end, y_end):
    // Col 0: leading {(0,-4),(1,-2)} -> (0.5,-3); trailing active {(5,6),(7,9),(8,12)} (100* excl)
    //   -> x_end=20/3, y_end=9, B(r) = -3 + 72*(r - 1/2)/37.
    // Col 1: leading {(0,1)} (row 1 Empty) -> (0,1); trailing {(5,7),(6,9),(7,13),(8,15)}
    //   -> (6.5,11), B(r) = 1 + 20*r/13.
    // Col 2: leading {Empty, 50*(excl)} -> no active leading anchor -> all active cells blanked + warnings.
    // Col 3: leading {(0,-4),(1,-2)} -> (0.5,-3); trailing active {(5,6),(7,10),(8,12)} (100* excl)
    //   -> x_end=20/3, y_end=28/3, B(r) = 2r - 4 so B(2) = 0 (zero-denominator under true centroid).
    let sheet = make_col_sheet(
        "base-lin",
        &[
            (
                "ExcludedTrailingCentroid",
                &[
                    Cell::number(-4.0),
                    Cell::number(-2.0),
                    Cell::number(5.0),
                    Cell::number(8.0),
                    Cell::empty(),
                    Cell::number(6.0),
                    Cell::excluded_number(100.0),
                    Cell::number(9.0),
                    Cell::number(12.0),
                ],
            ),
            (
                "MissingInWindow",
                &[
                    Cell::number(1.0),
                    Cell::empty(),
                    Cell::number(4.0),
                    Cell::number(7.0),
                    Cell::number(10.0),
                    Cell::number(7.0),
                    Cell::number(9.0),
                    Cell::number(13.0),
                    Cell::number(15.0),
                ],
            ),
            (
                "EmptyLeadingWindow",
                &[
                    Cell::empty(),
                    Cell::excluded_number(50.0),
                    Cell::number(10.0),
                    Cell::number(20.0),
                    Cell::empty(),
                    Cell::number(30.0),
                    Cell::number(40.0),
                    Cell::number(50.0),
                    Cell::number(60.0),
                ],
            ),
            (
                "LinearZeroCrossingCentroid",
                &[
                    Cell::number(-4.0),
                    Cell::number(-2.0),
                    Cell::number(5.0),
                    Cell::number(8.0),
                    Cell::empty(),
                    Cell::number(6.0),
                    Cell::excluded_number(100.0),
                    Cell::number(10.0),
                    Cell::number(12.0),
                ],
            ),
        ],
    )
    .unwrap();
    let ctx = TransformContext::default();
    let kind = BaselineKind::FirstAndLastLinear { first_k: 2, last_k: 4 };

    // 2a. Difference
    let diff_out = execute_remove_baseline_kind(&sheet, &kind, BaselineOperation::Difference, &ctx).unwrap();
    assert_eq!(diff_out.warnings.len(), 6, "only 6 active cells in col 2 should warn for missing baseline: {:?}", diff_out.warnings);
    assert!(diff_out.warnings.iter().all(|w| w.contains("col 2") && w.contains("missing baseline")));
    assert_col_cells(
        &diff_out.data_sheet,
        0,
        &[
            (Some(-0.027_027_027_027_027_03), false),
            (Some(0.027_027_027_027_027_03), false),
            (Some(5.081_081_081_081_081), false),
            (Some(6.135_135_135_135_135), false),
            (None, false),
            (Some(0.243_243_243_243_243_26), false),
            (Some(92.297_297_297_297_29), true),
            (Some(-0.648_648_648_648_648_7), false),
            (Some(0.405_405_405_405_405_43), false),
        ],
        "lin diff col0",
    );
    assert_col_cells(
        &diff_out.data_sheet,
        1,
        &[
            (Some(0.0), false),
            (None, false),
            (Some(-0.076_923_076_923_076_93), false),
            (Some(1.384_615_384_615_384_6), false),
            (Some(2.846_153_846_153_846_3), false),
            (Some(-1.692_307_692_307_692_3), false),
            (Some(-1.230_769_230_769_230_8), false),
            (Some(1.230_769_230_769_230_8), false),
            (Some(1.692_307_692_307_692_3), false),
        ],
        "lin diff col1",
    );
    assert_col_cells(
        &diff_out.data_sheet,
        2,
        &[(None, false), (None, true), (None, false), (None, false), (None, false), (None, false), (None, false), (None, false), (None, false)],
        "lin diff col2",
    );
    assert_col_cells(
        &diff_out.data_sheet,
        3,
        &[
            (Some(0.0), false),
            (Some(0.0), false),
            (Some(5.0), false),
            (Some(6.0), false),
            (None, false),
            (Some(0.0), false),
            (Some(92.0), true),
            (Some(0.0), false),
            (Some(0.0), false),
        ],
        "lin diff col3",
    );

    // 2b. Ratio (zero-denominator row 2 in col 3 blanks + warns)
    let ratio_out = execute_remove_baseline_kind(&sheet, &kind, BaselineOperation::Ratio, &ctx).unwrap();
    assert_eq!(ratio_out.warnings.len(), 7, "expected 1 zero-denominator warning (row 2 col 3) + 6 missing warnings: {:?}", ratio_out.warnings);
    assert!(ratio_out.warnings.iter().any(|w| w.contains("row 2 col 3") && w.contains("baseline=0")));
    assert_col_cells(
        &ratio_out.data_sheet,
        0,
        &[
            (Some(1.006_802_721_088_435_4), false),
            (Some(0.986_666_666_666_666_7), false),
            (Some(-61.666_666_666_666_664), false),
            (Some(4.289_855_072_463_768_5), false),
            (None, false),
            (Some(1.042_253_521_126_760_5), false),
            (Some(12.982_456_140_350_877), true),
            (Some(0.932_773_109_243_697_5), false),
            (Some(1.034_965_034_965_035), false),
        ],
        "lin ratio col0",
    );
    assert_col_cells(
        &ratio_out.data_sheet,
        1,
        &[
            (Some(1.0), false),
            (None, false),
            (Some(0.981_132_075_471_698_1), false),
            (Some(1.246_575_342_465_753_5), false),
            (Some(1.397_849_462_365_591_5), false),
            (Some(0.805_309_734_513_274_4), false),
            (Some(0.879_699_248_120_300_8), false),
            (Some(1.104_575_163_398_692_9), false),
            (Some(1.127_167_630_057_803_5), false),
        ],
        "lin ratio col1",
    );
    assert_col_cells(
        &ratio_out.data_sheet,
        2,
        &[(None, false), (None, true), (None, false), (None, false), (None, false), (None, false), (None, false), (None, false), (None, false)],
        "lin ratio col2",
    );
    assert_col_cells(
        &ratio_out.data_sheet,
        3,
        &[
            (Some(1.0), false),
            (Some(1.0), false),
            (None, false),
            (Some(4.0), false),
            (None, false),
            (Some(1.0), false),
            (Some(12.5), true),
            (Some(1.0), false),
            (Some(1.0), false),
        ],
        "lin ratio col3",
    );

    // 2c. PercentChange
    let pct_out = execute_remove_baseline_kind(&sheet, &kind, BaselineOperation::PercentChange, &ctx).unwrap();
    assert_eq!(pct_out.warnings.len(), 7);
    assert!(pct_out.warnings.iter().any(|w| w.contains("row 2 col 3") && w.contains("baseline=0")));
    assert_col_cells(
        &pct_out.data_sheet,
        0,
        &[
            (Some(0.680_272_108_843_537_4), false),
            (Some(-1.333_333_333_333_333_3), false),
            (Some(-6_266.666_666_666_667), false),
            (Some(328.985_507_246_376_8), false),
            (None, false),
            (Some(4.225_352_112_676_056), false),
            (Some(1_198.245_614_035_087_8), true),
            (Some(-6.722_689_075_630_252), false),
            (Some(3.496_503_496_503_496_7), false),
        ],
        "lin pct col0",
    );
    assert_col_cells(
        &pct_out.data_sheet,
        1,
        &[
            (Some(0.0), false),
            (None, false),
            (Some(-1.886_792_452_830_188_7), false),
            (Some(24.657_534_246_575_34), false),
            (Some(39.784_946_236_559_136), false),
            (Some(-19.469_026_548_672_566), false),
            (Some(-12.030_075_187_969_924), false),
            (Some(10.457_516_339_869_281), false),
            (Some(12.716_763_005_780_347), false),
        ],
        "lin pct col1",
    );
    assert_col_cells(
        &pct_out.data_sheet,
        3,
        &[
            (Some(0.0), false),
            (Some(0.0), false),
            (None, false),
            (Some(300.0), false),
            (None, false),
            (Some(0.0), false),
            (Some(1150.0), true),
            (Some(0.0), false),
            (Some(0.0), false),
        ],
        "lin pct col3",
    );

    // 2d. FractionalChange
    let frac_out = execute_remove_baseline_kind(&sheet, &kind, BaselineOperation::FractionalChange, &ctx).unwrap();
    assert_eq!(frac_out.warnings.len(), 7);
    assert!(frac_out.warnings.iter().any(|w| w.contains("row 2 col 3") && w.contains("baseline=0")));
    assert_col_cells(
        &frac_out.data_sheet,
        0,
        &[
            (Some(0.006_802_721_088_435_374), false),
            (Some(-0.013_333_333_333_333_334), false),
            (Some(-62.666_666_666_666_664), false),
            (Some(3.289_855_072_463_768), false),
            (None, false),
            (Some(0.042_253_521_126_760_56), false),
            (Some(11.982_456_140_350_877), true),
            (Some(-0.067_226_890_756_302_52), false),
            (Some(0.034_965_034_965_034_97), false),
        ],
        "lin frac col0",
    );
    assert_col_cells(
        &frac_out.data_sheet,
        1,
        &[
            (Some(0.0), false),
            (None, false),
            (Some(-0.018_867_924_528_301_886), false),
            (Some(0.246_575_342_465_753_4), false),
            (Some(0.397_849_462_365_591_4), false),
            (Some(-0.194_690_265_486_725_66), false),
            (Some(-0.120_300_751_879_699_24), false),
            (Some(0.104_575_163_398_692_81), false),
            (Some(0.127_167_630_057_803_46), false),
        ],
        "lin frac col1",
    );
    assert_col_cells(
        &frac_out.data_sheet,
        3,
        &[
            (Some(0.0), false),
            (Some(0.0), false),
            (None, false),
            (Some(3.0), false),
            (None, false),
            (Some(0.0), false),
            (Some(11.5), true),
            (Some(0.0), false),
            (Some(0.0), false),
        ],
        "lin frac col3",
    );
}

#[test]
fn baseline_columns_rowwise_preserves_sem_handles_zero_missing_excluded_and_negative() {
    let sheet = make_xy_3sub(
        "xy-rowwise-base",
        SubcolumnMode::MeanSemN,
        &[
            (
                "Baseline",
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[
                    [Some(4.0), Some(0.2), Some(4.0)],
                    [Some(0.0), Some(0.1), Some(4.0)],
                    [None, None, None],
                    [Some(10.0), Some(0.5), Some(4.0)],
                    [Some(-2.0), Some(0.25), Some(4.0)],
                ],
                &[false, false, false, true, false],
            ),
            (
                "Treated",
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[
                    [Some(10.0), Some(0.8), Some(6.0)],
                    [Some(5.0), Some(0.5), Some(6.0)],
                    [Some(12.0), Some(0.6), Some(6.0)],
                    [Some(20.0), Some(1.0), Some(6.0)],
                    [Some(7.0), Some(0.5), Some(6.0)],
                ],
                &[false, false, false, false, false],
            ),
        ],
    )
    .unwrap();
    let ctx = TransformContext { selected_data_sets: vec![1], ..TransformContext::default() };

    let diff_out = execute_remove_baseline(&sheet, &[0], BaselineOperation::Difference, &ctx).unwrap();
    assert_eq!(diff_out.warnings.len(), 4, "missing/excluded baseline rows 2 and 3 blank Mean+SEM: {:?}", diff_out.warnings);
    let d_xy = extract_xy_table(&diff_out.data_sheet, "rowwise diff").unwrap();
    for (r, exp) in [(0, Some((6.0, 0.8))), (1, Some((5.0, 0.5))), (2, None), (3, None), (4, Some((9.0, 0.5)))] {
        match exp {
            Some((m, s)) => {
                assert_approx_eq(d_xy.data_sets[1].cells[r][0].active_number().unwrap(), m, LINEAR_TOL, 0.0, "diff mean");
                assert_approx_eq(d_xy.data_sets[1].cells[r][1].active_number().unwrap(), s, LINEAR_TOL, 0.0, "diff sem");
            }
            None => {
                assert_eq!(d_xy.data_sets[1].cells[r][0].value, CellValue::Empty);
                assert_eq!(d_xy.data_sets[1].cells[r][1].value, CellValue::Empty);
            }
        }
        assert_approx_eq(d_xy.data_sets[1].cells[r][2].active_number().unwrap(), 6.0, LINEAR_TOL, 0.0, "N preserved");
    }

    for (op, expected) in [
        (BaselineOperation::Ratio, [(0, Some((2.5, 0.2))), (4, Some((-3.5, 0.25)))]),
        (BaselineOperation::PercentChange, [(0, Some((150.0, 20.0))), (4, Some((-450.0, 25.0)))]),
        (BaselineOperation::FractionalChange, [(0, Some((1.5, 0.2))), (4, Some((-4.5, 0.25)))]),
    ] {
        let out = execute_remove_baseline(&sheet, &[0], op, &ctx).unwrap();
        assert_eq!(out.warnings.len(), 6, "{op:?} should warn for Mean+SEM on zero row 1 and missing/excluded rows 2,3");
        let xy = extract_xy_table(&out.data_sheet, &format!("rowwise {op:?}")).unwrap();
        for (r, exp_ms) in expected {
            let (m, s) = exp_ms.unwrap();
            assert_approx_eq(xy.data_sets[1].cells[r][0].active_number().unwrap(), m, LINEAR_TOL, 0.0, &format!("{op:?} r{r} mean"));
            assert_approx_eq(xy.data_sets[1].cells[r][1].active_number().unwrap(), s, LINEAR_TOL, 0.0, &format!("{op:?} r{r} sem"));
        }
        for r in [1_usize, 2, 3] {
            assert_eq!(xy.data_sets[1].cells[r][0].value, CellValue::Empty);
            assert_eq!(xy.data_sets[1].cells[r][1].value, CellValue::Empty);
        }
    }

    // Negative baseline Ratio / PercentChange / FractionalChange on MeanLowHigh swap -Error and +Error; Difference does not.
    let lh_sheet = make_xy_3sub(
        "xy-base-lh",
        SubcolumnMode::MeanLowHigh,
        &[("Base", &[1.0], &[[Some(-2.0), Some(0.0), Some(0.0)]], &[false]), ("Trt", &[1.0], &[[Some(8.0), Some(1.0), Some(3.0)]], &[false])],
    )
    .unwrap();
    for (op, exp_m, exp_lo, exp_hi) in [
        (BaselineOperation::Difference, 10.0, 1.0, 3.0),
        (BaselineOperation::Ratio, -4.0, 1.5, 0.5),
        (BaselineOperation::PercentChange, -500.0, 150.0, 50.0),
        (BaselineOperation::FractionalChange, -5.0, 1.5, 0.5),
    ] {
        let lh_out = execute_remove_baseline(&lh_sheet, &[0], op, &ctx).unwrap();
        let lh_xy = extract_xy_table(&lh_out.data_sheet, &format!("lh {op:?}")).unwrap();
        assert_approx_eq(lh_xy.data_sets[1].cells[0][0].active_number().unwrap(), exp_m, LINEAR_TOL, 0.0, &format!("{op:?} mean"));
        assert_approx_eq(lh_xy.data_sets[1].cells[0][1].active_number().unwrap(), exp_lo, LINEAR_TOL, 0.0, &format!("{op:?} -Error"));
        assert_approx_eq(lh_xy.data_sets[1].cells[0][2].active_number().unwrap(), exp_hi, LINEAR_TOL, 0.0, &format!("{op:?} +Error"));
    }

    // Typed AnalysisSpec -> ResultSheet for RemoveBaseline (AC #4)
    let base_analysis = AnalysisSpec::new(
        "an-base-ratio",
        "xy-rowwise-base",
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Ratio }),
    )
    .unwrap();
    let base_res = execute_analysis_transform(&sheet, &base_analysis, &ctx).unwrap();
    assert_eq!(base_res.status, ResultStatus::UpToDate);
    assert_eq!(base_res.title(), "Remove baseline");
    let base_xy = extract_xy_table(base_res.analyzable_data.as_ref().unwrap(), "ResultSheet baseline").unwrap();
    assert_approx_eq(base_xy.data_sets[1].cells[0][0].active_number().unwrap(), 2.5, LINEAR_TOL, 0.0, "ResultSheet baseline ratio mean");
    assert_approx_eq(base_xy.data_sets[1].cells[0][1].active_number().unwrap(), 0.2, LINEAR_TOL, 0.0, "ResultSheet baseline ratio sem");
    assert_eq!(base_xy.data_sets[1].cells[1][0].value, CellValue::Empty);
}

// ============================================================================
// 3. Math Transform Uncertainty Propagation (JCGM 100:2008) & Endpoint Ordering
// ============================================================================

#[test]
fn math_transform_uncertainty_propagation_delta_and_endpoints_match_mpmath() {
    let ctx = TransformContext::default();

    // Positive-domain MeanSemN sheet with:
    // row 0: in-domain (2.5, 0.35, N=6)
    // row 1: half-OOD under Ln (0.5, 0.75, N=5) -> center valid, lo = -0.25 OOD
    // row 2: active OOD center under Ln (-0.5, 0.2, N=4) -> center & spread blanked + warning
    // row 3: excluded OOD center under Ln (-1.0*, 0.2, N=4) -> center & spread blanked, no warning
    let pos_sheet = make_xy_3sub(
        "xy-math-pos",
        SubcolumnMode::MeanSemN,
        &[(
            "Conc",
            &[1.0, 2.0, 3.0, 4.0],
            &[
                [Some(2.5), Some(0.35), Some(6.0)],
                [Some(0.5), Some(0.75), Some(5.0)],
                [Some(-0.5), Some(0.2), Some(4.0)],
                [Some(-1.0), Some(0.2), Some(4.0)],
            ],
            &[false, false, false, true],
        )],
    )
    .unwrap();

    // 3a. Ln with FirstOrderDeltaMethod vs ReplicateEndpoints (including OOD behavior)
    let ln_delta = execute_math_op(&pos_sheet, &MathTransformOp::Ln, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    assert_eq!(ln_delta.warnings.len(), 1, "only active OOD row 2 should warn: {:?}", ln_delta.warnings);
    assert!(ln_delta.warnings[0].contains("row 2 col 0") && ln_delta.warnings[0].contains("outside transform domain"));
    let ld_xy = extract_xy_table(&ln_delta.data_sheet, "ln delta").unwrap();
    assert_approx_eq(
        ld_xy.data_sets[0].cells[0][0].active_number().unwrap(),
        0.916_290_731_874_155_1,
        NONLINEAR_ABS_TOL,
        NONLINEAR_REL_TOL,
        "ln center",
    );
    assert_approx_eq(ld_xy.data_sets[0].cells[0][1].active_number().unwrap(), 0.14, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "ln delta sem");
    assert_approx_eq(ld_xy.data_sets[0].cells[1][0].active_number().unwrap(), -LN_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "ln half-OOD center");
    assert_approx_eq(ld_xy.data_sets[0].cells[1][1].active_number().unwrap(), 1.5, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "ln half-OOD delta sem");
    assert_eq!(ld_xy.data_sets[0].cells[2][0].value, CellValue::Empty);
    assert_eq!(ld_xy.data_sets[0].cells[2][1].value, CellValue::Empty);
    assert_approx_eq(ld_xy.data_sets[0].cells[2][2].active_number().unwrap(), 4.0, LINEAR_TOL, 0.0, "OOD N preserved");
    assert!(ld_xy.data_sets[0].cells[3][0].excluded && ld_xy.data_sets[0].cells[3][0].value == CellValue::Empty);
    assert_eq!(ld_xy.data_sets[0].cells[3][1].value, CellValue::Empty);

    let ln_ep = execute_math_op(&pos_sheet, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(ln_ep.warnings.len(), 2);
    assert!(ln_ep.warnings[0].contains("spread blanked"));
    let le_xy = extract_xy_table(&ln_ep.data_sheet, "ln endpoints").unwrap();
    assert_approx_eq(
        le_xy.data_sets[0].cells[0][0].active_number().unwrap(),
        0.916_290_731_874_155_1,
        NONLINEAR_ABS_TOL,
        NONLINEAR_REL_TOL,
        "ln ep center",
    );
    assert_approx_eq(
        le_xy.data_sets[0].cells[0][1].active_number().unwrap(),
        0.140_925_576_070_493_86,
        NONLINEAR_ABS_TOL,
        NONLINEAR_REL_TOL,
        "ln ep sem",
    );
    // Half-OOD row 1: center ln(0.5) stays finite while endpoint spread becomes Empty.
    assert_approx_eq(le_xy.data_sets[0].cells[1][0].active_number().unwrap(), -LN_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "ln ep half-OOD center");
    assert_eq!(le_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
    assert_approx_eq(le_xy.data_sets[0].cells[1][2].active_number().unwrap(), 5.0, LINEAR_TOL, 0.0, "half-OOD N preserved");

    // 3b. Increasing & decreasing presets on MeanSdN row (2.5, 0.35, N=6)
    let clean_sd =
        make_xy_3sub("xy-math-sd", SubcolumnMode::MeanSdN, &[("Series", &[2.5], &[[Some(2.5), Some(0.35), Some(6.0)]], &[false])]).unwrap();
    let preset_cases = [
        ("Log10", MathTransformPreset::Log10, 0.397_940_008_672_037_6, 0.060_801_227_466_455_25, 0.061_203_200_046_452_44),
        ("Reciprocal (decreasing)", MathTransformPreset::Reciprocal, 0.4, 0.056, 0.057_119_543_043_655_65),
        (
            "ConcentrationToNegativeLogM uM (decreasing)",
            MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 },
            5.602_059_991_327_962_5,
            0.060_801_227_466_455_25,
            0.061_203_200_046_452_44,
        ),
    ];
    for (label, preset, exp_center, exp_delta, exp_ep) in preset_cases {
        let d_out =
            execute_math_transform(&clean_sd, None, Some(&MathTransformOp::Preset(preset)), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)
                .unwrap();
        let e_out =
            execute_math_transform(&clean_sd, None, Some(&MathTransformOp::Preset(preset)), ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
        assert!(d_out.warnings.is_empty(), "{label} delta unexpected warnings: {:?}", d_out.warnings);
        assert!(e_out.warnings.is_empty(), "{label} ep unexpected warnings: {:?}", e_out.warnings);
        let d_xy = extract_xy_table(&d_out.data_sheet, label).unwrap();
        let e_xy = extract_xy_table(&e_out.data_sheet, label).unwrap();
        assert_approx_eq(
            d_xy.data_sets[0].cells[0][0].active_number().unwrap(),
            exp_center,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} delta center"),
        );
        assert_approx_eq(
            d_xy.data_sets[0].cells[0][1].active_number().unwrap(),
            exp_delta,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} delta sd"),
        );
        assert_approx_eq(
            e_xy.data_sets[0].cells[0][0].active_number().unwrap(),
            exp_center,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} ep center"),
        );
        assert_approx_eq(
            e_xy.data_sets[0].cells[0][1].active_number().unwrap(),
            exp_ep,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} ep sd"),
        );
        assert_approx_eq(e_xy.data_sets[0].cells[0][2].active_number().unwrap(), 6.0, LINEAR_TOL, 0.0, &format!("{label} N"));
    }

    // MeanCvN (Mean=2.5, %CV=14.0 -> SD=0.35, N=6) under decreasing Reciprocal
    let clean_cv =
        make_xy_3sub("xy-math-cv", SubcolumnMode::MeanCvN, &[("Series", &[2.5], &[[Some(2.5), Some(14.0), Some(6.0)]], &[false])]).unwrap();
    let cv_delta = execute_math_op(&clean_cv, &MathTransformOp::Reciprocal, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let cv_ep = execute_math_op(&clean_cv, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    let cv_d_xy = extract_xy_table(&cv_delta.data_sheet, "cv delta").unwrap();
    let cv_e_xy = extract_xy_table(&cv_ep.data_sheet, "cv ep").unwrap();
    assert_approx_eq(cv_d_xy.data_sets[0].cells[0][0].active_number().unwrap(), 0.4, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "cv delta mean");
    assert_approx_eq(cv_d_xy.data_sets[0].cells[0][1].active_number().unwrap(), 14.0, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "cv delta %CV");
    assert_approx_eq(cv_e_xy.data_sets[0].cells[0][0].active_number().unwrap(), 0.4, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "cv ep mean");
    assert_approx_eq(
        cv_e_xy.data_sets[0].cells[0][1].active_number().unwrap(),
        14.279_885_760_913_913,
        NONLINEAR_ABS_TOL,
        NONLINEAR_REL_TOL,
        "cv ep %CV",
    );
}

#[test]
fn math_transform_proportion_uncertainty_and_result_sheet() {
    let ctx = TransformContext::default();
    // 3c. Unit-proportion transforms (Logit, Probit, ArcsinSqrt) on (0.25, 0.08, N=9) plus OOD row (1.0, 0.05, N=4)
    let prop_sheet = make_xy_3sub(
        "xy-math-prop",
        SubcolumnMode::MeanSemN,
        &[("Prop", &[1.0, 2.0], &[[Some(0.25), Some(0.08), Some(9.0)], [Some(1.0), Some(0.05), Some(4.0)]], &[false, false])],
    )
    .unwrap();
    let unit_cases = [
        ("Logit", MathTransformPreset::Logit, -1.098_612_288_668_109_8, 0.426_666_666_666_666_7, 0.438_721_102_907_948, true),
        ("Probit", MathTransformPreset::Probit, 4.325_510_249_803_918, 0.251_749_206_444_887_35, 0.257_126_043_736_480_3, true),
        (
            "ArcsinSqrt",
            MathTransformPreset::ArcsinSqrt,
            FRAC_PI_6,
            0.092_376_043_070_340_13,
            0.093_475_465_835_731_98,
            false, // p=1.0 is inside ArcsinSqrt domain [0, 1], though derivative is undefined
        ),
    ];
    for (label, preset, exp_c, exp_d, exp_e, p1_is_ood) in unit_cases {
        let d_out = execute_transform(
            &prop_sheet,
            &TransformAnalysisSpec::Math {
                x_transform: Some(MathTransformPreset::Log10),
                y_transform: Some(preset),
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
            },
            &ctx,
        )
        .unwrap();
        let e_out = execute_transform(
            &prop_sheet,
            &TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(preset),
                error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            },
            &ctx,
        )
        .unwrap();
        let d_xy = extract_xy_table(&d_out.data_sheet, label).unwrap();
        let e_xy = extract_xy_table(&e_out.data_sheet, label).unwrap();
        assert_approx_eq(d_xy.x_value_at(0).unwrap(), 0.0, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "x0 log10");
        assert_approx_eq(d_xy.x_value_at(1).unwrap(), LOG10_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "x1 log10");
        assert_approx_eq(
            d_xy.data_sets[0].cells[0][0].active_number().unwrap(),
            exp_c,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} delta center"),
        );
        assert_approx_eq(
            d_xy.data_sets[0].cells[0][1].active_number().unwrap(),
            exp_d,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} delta sem"),
        );
        assert_approx_eq(
            e_xy.data_sets[0].cells[0][0].active_number().unwrap(),
            exp_c,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} ep center"),
        );
        assert_approx_eq(
            e_xy.data_sets[0].cells[0][1].active_number().unwrap(),
            exp_e,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} ep sem"),
        );

        if p1_is_ood {
            assert_eq!(d_out.warnings.len(), 1);
            assert!(d_out.warnings[0].contains("outside transform domain"));
            assert_eq!(e_out.warnings.len(), 1);
            assert!(e_out.warnings[0].contains("outside transform domain"));
            assert_eq!(d_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
            assert_eq!(d_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
            assert_eq!(e_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
            assert_eq!(e_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
            assert_approx_eq(e_xy.data_sets[0].cells[1][2].active_number().unwrap(), 4.0, LINEAR_TOL, 0.0, "OOD N preserved");
        } else {
            // ArcsinSqrt(1.0) = pi/2, derivative undefined at boundary -> spread Empty + warning (#55)
            assert_eq!(d_out.warnings.len(), 1);
            assert_eq!(e_out.warnings.len(), 1);
            assert!(d_out.warnings[0].contains("spread blanked"));
            assert!(e_out.warnings[0].contains("spread blanked"));
            assert_approx_eq(d_xy.data_sets[0].cells[1][0].active_number().unwrap(), FRAC_PI_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "arcsin(1)");
            assert_eq!(d_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
            assert_eq!(e_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
        }
    }

    // Typed AnalysisSpec -> ResultSheet check for Math transform (Logit + ReplicateEndpoints)
    let math_analysis = AnalysisSpec::new(
        "an-math-logit",
        "xy-math-prop",
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: Some(MathTransformPreset::Logit),
            error_propagation: ErrorPropagationMode::ReplicateEndpoints,
        }),
    )
    .unwrap();
    let math_res = execute_analysis_transform(&prop_sheet, &math_analysis, &ctx).unwrap();
    assert_eq!(math_res.status, ResultStatus::UpToDate);
    assert_eq!(math_res.title(), "Transform");
    let math_data = extract_xy_table(math_res.analyzable_data.as_ref().unwrap(), "ResultSheet math").unwrap();
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

    // UpperLowerLimits: [Mean=2.0, Lower=1.2, Upper=4.0], plus row 1 with OOD Lower=-1.0 for Ln.
    let ul_sheet = make_xy_3sub(
        "xy-ul-dec",
        SubcolumnMode::UpperLowerLimits,
        &[("Limits", &[1.0, 2.0], &[[Some(2.0), Some(1.2), Some(4.0)], [Some(2.0), Some(-1.0), Some(4.0)]], &[false, false])],
    )
    .unwrap();

    for (label, preset, exp_mean, exp_lo, exp_hi) in [
        ("Reciprocal", MathTransformPreset::Reciprocal, 0.5, 0.25, 0.833_333_333_333_333_4),
        (
            "ConcentrationToNegativeLogM",
            MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 },
            5.698_970_004_336_019,
            5.397_940_008_672_037_5,
            5.920_818_753_952_375,
        ),
    ] {
        let out = execute_transform(
            &ul_sheet,
            &TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: Some(preset),
                error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            },
            &ctx,
        )
        .unwrap();
        let xy = extract_xy_table(&out.data_sheet, label).unwrap();
        let row0 = &xy.data_sets[0].cells[0];
        let m = row0[0].active_number().unwrap();
        let lo = row0[1].active_number().unwrap();
        let hi = row0[2].active_number().unwrap();
        assert!(lo < m && m < hi, "{label}: expected Lower < Mean < Upper, got {lo}, {m}, {hi}");
        assert_approx_eq(m, exp_mean, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, &format!("{label} mean"));
        assert_approx_eq(lo, exp_lo, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, &format!("{label} lower (from upper input)"));
        assert_approx_eq(hi, exp_hi, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, &format!("{label} upper (from lower input)"));
        let bounds = SubcolumnMode::UpperLowerLimits.explicit_bounds(row0).unwrap();
        assert_eq!((bounds.lower, bounds.center, bounds.upper), (lo, m, hi));
    }

    // Out-of-domain lower limit (-1.0) under Ln blanks Lower while keeping Mean & Upper and emitting a warning (#55).
    let ln_ul = execute_math_op(&ul_sheet, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(ln_ul.warnings.len(), 1);
    assert!(ln_ul.warnings[0].contains("lower bound -1 outside transform domain"));
    let ln_xy = extract_xy_table(&ln_ul.data_sheet, "ln ul").unwrap();
    assert_approx_eq(ln_xy.data_sets[0].cells[1][0].active_number().unwrap(), LN_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "ln ul mean");
    assert_eq!(ln_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
    assert_approx_eq(ln_xy.data_sets[0].cells[1][2].active_number().unwrap(), 4.0_f64.ln(), NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "ln ul upper");

    // Partial-OOD lower limit (0.0) under decreasing Reciprocal maps Upper=4.0 to Lower=0.25 and leaves Upper Empty.
    let rec_partial_sheet =
        make_xy_3sub("xy-ul-rec-ood", SubcolumnMode::UpperLowerLimits, &[("Lim", &[1.0], &[[Some(2.0), Some(0.0), Some(4.0)]], &[false])]).unwrap();
    let rec_partial = execute_math_op(&rec_partial_sheet, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    let rec_partial_xy = extract_xy_table(&rec_partial.data_sheet, "rec partial").unwrap();
    assert_approx_eq(rec_partial_xy.data_sets[0].cells[0][0].active_number().unwrap(), 0.5, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "rec partial mean");
    assert_approx_eq(
        rec_partial_xy.data_sets[0].cells[0][1].active_number().unwrap(),
        0.25,
        NONLINEAR_ABS_TOL,
        NONLINEAR_REL_TOL,
        "rec partial lower",
    );
    assert_eq!(rec_partial_xy.data_sets[0].cells[0][2].value, CellValue::Empty, "rec partial upper blanked");

    // MedianIqr decreasing endpoint ordering: [Median=2.0, Q1=1.2, Q3=4.0] under Reciprocal
    let iqr_sheet = make_xy_3sub("xy-iqr-dec", SubcolumnMode::MedianIqr, &[("IQR", &[1.0], &[[Some(2.0), Some(1.2), Some(4.0)]], &[false])]).unwrap();
    let iqr_out = execute_math_op(&iqr_sheet, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    let iqr_xy = extract_xy_table(&iqr_out.data_sheet, "rec iqr").unwrap();
    let iqr_row = &iqr_xy.data_sets[0].cells[0];
    assert_approx_eq(iqr_row[0].active_number().unwrap(), 0.5, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "iqr median");
    assert_approx_eq(iqr_row[1].active_number().unwrap(), 0.25, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "iqr Q1");
    assert_approx_eq(iqr_row[2].active_number().unwrap(), 0.833_333_333_333_333_4, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "iqr Q3");
    let iqr_bounds = SubcolumnMode::MedianIqr.explicit_bounds(iqr_row).unwrap();
    assert!(iqr_bounds.lower < iqr_bounds.center && iqr_bounds.center < iqr_bounds.upper);

    // MeanLowHigh half-OOD (0.5, -Err=0.75, +Err=0.5) and full-OOD (-0.5, -Err=0.2, +Err=0.3) under Ln
    let lh_ood_sheet = make_xy_3sub(
        "xy-lh-ood",
        SubcolumnMode::MeanLowHigh,
        &[("LH", &[1.0, 2.0], &[[Some(0.5), Some(0.75), Some(0.5)], [Some(-0.5), Some(0.2), Some(0.3)]], &[false, false])],
    )
    .unwrap();
    let lh_ood_delta = execute_math_op(&lh_ood_sheet, &MathTransformOp::Ln, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let lh_ood_ep = execute_math_op(&lh_ood_sheet, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(lh_ood_delta.warnings.len(), 1);
    assert!(lh_ood_delta.warnings[0].contains("row 1 col 0") && lh_ood_delta.warnings[0].contains("outside transform domain"));
    assert_eq!(lh_ood_ep.warnings.len(), 2);
    assert!(lh_ood_ep.warnings[0].contains("row 0 col 0") && lh_ood_ep.warnings[0].contains("error bounds blanked"));
    let lh_d_xy = extract_xy_table(&lh_ood_delta.data_sheet, "lh ood delta").unwrap();
    let lh_e_xy = extract_xy_table(&lh_ood_ep.data_sheet, "lh ood ep").unwrap();
    assert_approx_eq(lh_d_xy.data_sets[0].cells[0][0].active_number().unwrap(), -LN_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "lh half-OOD mean");
    assert_approx_eq(lh_d_xy.data_sets[0].cells[0][1].active_number().unwrap(), 1.5, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "lh half-OOD delta -Err");
    assert_approx_eq(lh_d_xy.data_sets[0].cells[0][2].active_number().unwrap(), 1.0, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "lh half-OOD delta +Err");
    assert_approx_eq(lh_e_xy.data_sets[0].cells[0][0].active_number().unwrap(), -LN_2, NONLINEAR_ABS_TOL, NONLINEAR_REL_TOL, "lh half-OOD ep mean");
    assert_eq!(lh_e_xy.data_sets[0].cells[0][1].value, CellValue::Empty);
    assert_eq!(lh_e_xy.data_sets[0].cells[0][2].value, CellValue::Empty);
    for c in 0..3 {
        assert_eq!(lh_d_xy.data_sets[0].cells[1][c].value, CellValue::Empty);
        assert_eq!(lh_e_xy.data_sets[0].cells[1][c].value, CellValue::Empty);
    }

    // MeanLowHigh: [Mean=2.0, -Error=0.8, +Error=2.0] (interval [1.2, 4.0]) under decreasing transforms
    let lh_sheet =
        make_xy_3sub("xy-lh-dec", SubcolumnMode::MeanLowHigh, &[("Asym", &[1.0], &[[Some(2.0), Some(0.8), Some(2.0)]], &[false])]).unwrap();
    for (label, preset, exp_m, exp_ep_lo, exp_ep_hi, exp_d_lo, exp_d_hi) in [
        ("Reciprocal", MathTransformPreset::Reciprocal, 0.5, 0.25, 0.333_333_333_333_333_3, 0.5, 0.2),
        (
            "ConcentrationToNegativeLogM",
            MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: 1e-6 },
            5.698_970_004_336_019,
            LOG10_2,
            0.221_848_749_616_356_37,
            0.434_294_481_903_251_76,
            0.173_717_792_761_300_73,
        ),
    ] {
        let ep_out =
            execute_math_transform(&lh_sheet, None, Some(&MathTransformOp::Preset(preset)), ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
        let dl_out =
            execute_math_transform(&lh_sheet, None, Some(&MathTransformOp::Preset(preset)), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)
                .unwrap();
        let ep_xy = extract_xy_table(&ep_out.data_sheet, &format!("{label} lh ep")).unwrap();
        let dl_xy = extract_xy_table(&dl_out.data_sheet, &format!("{label} lh delta")).unwrap();
        assert_approx_eq(
            ep_xy.data_sets[0].cells[0][0].active_number().unwrap(),
            exp_m,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} lh ep mean"),
        );
        assert_approx_eq(
            ep_xy.data_sets[0].cells[0][1].active_number().unwrap(),
            exp_ep_lo,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} lh ep -Err"),
        );
        assert_approx_eq(
            ep_xy.data_sets[0].cells[0][2].active_number().unwrap(),
            exp_ep_hi,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} lh ep +Err"),
        );

        assert_approx_eq(
            dl_xy.data_sets[0].cells[0][0].active_number().unwrap(),
            exp_m,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} lh delta mean"),
        );
        assert_approx_eq(
            dl_xy.data_sets[0].cells[0][1].active_number().unwrap(),
            exp_d_lo,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} lh delta -Err"),
        );
        assert_approx_eq(
            dl_xy.data_sets[0].cells[0][2].active_number().unwrap(),
            exp_d_hi,
            NONLINEAR_ABS_TOL,
            NONLINEAR_REL_TOL,
            &format!("{label} lh delta +Err"),
        );
    }
}

#[test]
fn baseline_first_and_last_linear_affine_invariance_unbalanced_and_equal_anchor() {
    // Noiseless affine series y(r) = -5 + 3*r across 8 rows with blanks, excluded outliers,
    // and unbalanced replicate counts [1, 3, 2, 0, 2, 2, 1, 3].
    let mut rep_grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 1, 8).unwrap();
    rep_grp.data_sets[0].title = String::from("UnbalancedAffine");
    rep_grp.data_sets[0].cells = vec![
        vec![Cell::number(-5.0), Cell::empty(), Cell::empty()],
        vec![Cell::number(-2.0), Cell::number(-2.0), Cell::number(-2.0)],
        vec![Cell::number(1.0), Cell::excluded_number(999.0), Cell::number(1.0)],
        vec![Cell::empty(), Cell::empty(), Cell::empty()],
        vec![Cell::number(7.0), Cell::number(7.0), Cell::empty()],
        vec![Cell::number(10.0), Cell::excluded_number(500.0), Cell::number(10.0)],
        vec![Cell::number(13.0), Cell::empty(), Cell::empty()],
        vec![Cell::number(16.0), Cell::number(16.0), Cell::number(16.0)],
    ];
    let rep_sheet = DataSheet::new("aff-unbal", "Affine Unbalanced", TableArchetype::Grouped(rep_grp)).unwrap();
    let ctx = TransformContext::default();
    let kind = BaselineKind::FirstAndLastLinear { first_k: 3, last_k: 3 };

    let diff = execute_remove_baseline_kind(&rep_sheet, &kind, BaselineOperation::Difference, &ctx).unwrap();
    let TableArchetype::Grouped(diff_tbl) = &diff.data_sheet.archetype else {
        panic!("expected Grouped");
    };
    for r in 0..8 {
        for cell in &diff_tbl.data_sets[0].cells[r] {
            if let Some(v) = cell.active_number() {
                assert_approx_eq(v, 0.0, LINEAR_TOL, 0.0, &format!("affine Difference invariance row {r}"));
            }
        }
    }

    let ratio = execute_remove_baseline_kind(&rep_sheet, &kind, BaselineOperation::Ratio, &ctx).unwrap();
    let TableArchetype::Grouped(ratio_tbl) = &ratio.data_sheet.archetype else {
        panic!("expected Grouped");
    };
    for r in 0..8 {
        for cell in &ratio_tbl.data_sets[0].cells[r] {
            if let Some(v) = cell.active_number() {
                assert_approx_eq(v, 1.0, LINEAR_TOL, 0.0, &format!("affine Ratio invariance row {r}"));
            }
        }
    }

    // Overlapping windows collapsing to identical active row coordinate (r = 2, B = 10.0):
    // must avoid 0/0 NaN and yield flat baseline B(r) = 10.0.
    let overlap_sheet =
        make_col_sheet("base-equal-anchor", &[("SingleShared", &[Cell::empty(), Cell::empty(), Cell::number(10.0), Cell::empty(), Cell::empty()])])
            .unwrap();
    let overlap_diff = execute_remove_baseline_kind(
        &overlap_sheet,
        &BaselineKind::FirstAndLastLinear { first_k: 3, last_k: 3 },
        BaselineOperation::Difference,
        &ctx,
    )
    .unwrap();
    assert!(overlap_diff.warnings.is_empty());
    assert_col_cells(
        &overlap_diff.data_sheet,
        0,
        &[(None, false), (None, false), (Some(0.0), false), (None, false), (None, false)],
        "overlapping identical anchor row",
    );
}
